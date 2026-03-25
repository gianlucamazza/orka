//! `git_commit` skill.
//!
//! Enforces Conventional Commits, secret detection, and AI authorship
//! attribution per best-practice 2026 agent policy.

use std::sync::Arc;

use async_trait::async_trait;
use orka_core::{Result, SkillInput, SkillOutput, SkillSchema, traits::Skill};

use crate::{
    cli::{resolve_work_dir, run_git},
    guard::{AuthorshipArgs, GitGuard},
};

/// Stages specific files and creates a commit.
///
/// **Safety rules (2026 agent best practices)**:
/// - Never accepts `git add -A` or wildcard staging.
/// - Validates Conventional Commits format.
/// - Detects and blocks secrets (`.env`, `*.key`, etc.).
/// - Appends AI authorship attribution trailer or sets `--author`.
pub struct GitCommitSkill {
    guard: Arc<GitGuard>,
}

impl GitCommitSkill {
    /// Create from the shared guard.
    pub fn new(guard: Arc<GitGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
impl Skill for GitCommitSkill {
    fn name(&self) -> &'static str {
        "git_commit"
    }

    fn category(&self) -> &'static str {
        "git"
    }

    fn description(&self) -> &'static str {
        "Stage specific files and create a commit. \
         Message must follow Conventional Commits format: `type(scope): description`. \
         Never commits secrets (.env, *.key, *.pem, etc.). \
         Automatically appends AI authorship attribution."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "required": ["files", "message"],
            "properties": {
                "files": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Files to stage and commit. Must be explicit paths — never use wildcards or '.' to avoid committing secrets."
                },
                "message": {
                    "type": "string",
                    "description": "Commit message in Conventional Commits format: `type(scope): description`"
                },
                "path": {
                    "type": "string",
                    "description": "Repository path"
                }
            }
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let files_val =
            input
                .args
                .get("files")
                .ok_or_else(|| orka_core::Error::SkillCategorized {
                    message: "missing 'files' argument".to_string(),
                    category: orka_core::ErrorCategory::Input,
                })?;

        let files: Vec<&str> = files_val
            .as_array()
            .ok_or_else(|| orka_core::Error::SkillCategorized {
                message: "'files' must be an array".to_string(),
                category: orka_core::ErrorCategory::Input,
            })?
            .iter()
            .filter_map(|v| v.as_str())
            .collect();

        if files.is_empty() {
            return Err(orka_core::Error::SkillCategorized {
                message: "'files' array is empty".to_string(),
                category: orka_core::ErrorCategory::Input,
            });
        }

        let message = input
            .args
            .get("message")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| orka_core::Error::SkillCategorized {
                message: "missing 'message' argument".to_string(),
                category: orka_core::ErrorCategory::Input,
            })?;

        let cwd_str = input.args.get("path").and_then(serde_json::Value::as_str);
        let worktree_cwd = input
            .context
            .as_ref()
            .and_then(|c| c.worktree_cwd.as_deref());
        let user_cwd = input.context.as_ref().and_then(|c| c.user_cwd.as_deref());
        let work_dir = resolve_work_dir(cwd_str.or(worktree_cwd).or(user_cwd))
            .map_err(orka_core::Error::from)?;

        // --- Policy checks ---

        // 1. Secret detection (block before any I/O)
        self.guard
            .check_no_secrets(&files)
            .map_err(orka_core::Error::from)?;

        // 2. Conventional commit format
        self.guard
            .check_commit_message(message)
            .map_err(orka_core::Error::from)?;

        // --- Stage files (explicitly named, never -A) ---
        let mut add_args = vec!["add", "--"];
        add_args.extend_from_slice(&files);

        run_git(&work_dir, &add_args, self.guard.command_timeout_secs)
            .await
            .map_err(orka_core::Error::from)?
            .into_result()
            .map_err(orka_core::Error::from)?;

        // --- Build commit args ---
        let mut commit_args = vec!["commit"];

        if self.guard.sign_commits_flag {
            commit_args.push("-S");
        }

        // Authorship: trailer vs. author override
        let full_message;
        let author_arg;
        match self.guard.authorship_args() {
            Some(AuthorshipArgs::Trailer { name, email }) => {
                full_message = format!("{message}\n\nCo-Authored-By: {name} <{email}>");
                commit_args.push("-m");
                commit_args.push(&full_message);
            }
            Some(AuthorshipArgs::Author { name, email }) => {
                author_arg = format!("{name} <{email}>");
                commit_args.push("--author");
                commit_args.push(&author_arg);
                commit_args.push("-m");
                commit_args.push(message);
            }
            None => {
                commit_args.push("-m");
                commit_args.push(message);
            }
        }

        let out = run_git(&work_dir, &commit_args, self.guard.command_timeout_secs)
            .await
            .map_err(orka_core::Error::from)?
            .into_result()
            .map_err(orka_core::Error::from)?;

        // Parse SHA from "1 file changed, ...  abc1234 ..." output
        let sha = out
            .stdout
            .lines()
            .find_map(|l| {
                // "main abc1234] message" or "[branch abc1234]"
                if l.starts_with('[') {
                    l.split_whitespace().nth(1).map(|s| s.trim_end_matches(']'))
                } else {
                    None
                }
            })
            .unwrap_or("")
            .to_string();

        Ok(SkillOutput::new(serde_json::json!({
            "committed": true,
            "sha": sha,
            "message": message,
            "files": files,
        })))
    }
}
