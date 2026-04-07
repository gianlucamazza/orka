//! `git_diff` skill.

use std::sync::Arc;

use async_trait::async_trait;
use orka_core::{Result, SkillInput, SkillOutput, SkillSchema, traits::Skill};

use crate::{cli::resolve_work_dir, guard::GitGuard};

/// Returns the diff of the working tree, index, or between two commits.
pub struct GitDiffSkill {
    guard: Arc<GitGuard>,
}

impl GitDiffSkill {
    /// Create from the shared guard.
    pub fn new(guard: Arc<GitGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
impl Skill for GitDiffSkill {
    fn name(&self) -> &'static str {
        "git_diff"
    }

    fn category(&self) -> &'static str {
        "git"
    }

    fn description(&self) -> &'static str {
        "Show the diff of changes. By default shows unstaged changes. \
         Pass `staged = true` for staged changes, or `commit_a`/`commit_b` \
         to diff between commits. Optionally filter to a specific `path`."
    }

    fn budget_cost(&self) -> f32 {
        0.5
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Repository or file path"
                },
                "staged": {
                    "type": "boolean",
                    "description": "Show staged (index) changes instead of working-tree changes"
                },
                "commit_a": {
                    "type": "string",
                    "description": "Base commit/ref for comparison"
                },
                "commit_b": {
                    "type": "string",
                    "description": "Target commit/ref for comparison (requires commit_a)"
                },
                "file_path": {
                    "type": "string",
                    "description": "Limit diff to this specific file path"
                },
                "max_lines": {
                    "type": "integer",
                    "description": "Truncate output at this many lines",
                    "default": 500
                }
            }
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let cwd_str = input.args.get("path").and_then(serde_json::Value::as_str);
        let worktree_cwd = input
            .context
            .as_ref()
            .and_then(|c| c.worktree_cwd.as_deref());
        let user_cwd = input.context.as_ref().and_then(|c| c.user_cwd.as_deref());
        let work_dir = resolve_work_dir(cwd_str.or(worktree_cwd).or(user_cwd))
            .map_err(orka_core::Error::from)?;

        let staged = input
            .args
            .get("staged")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let commit_a = input
            .args
            .get("commit_a")
            .and_then(serde_json::Value::as_str);
        let commit_b = input
            .args
            .get("commit_b")
            .and_then(serde_json::Value::as_str);
        let file_path = input
            .args
            .get("file_path")
            .and_then(serde_json::Value::as_str);
        let max_lines = input
            .args
            .get("max_lines")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(500) as usize;
        let max_lines = max_lines.min(self.guard.max_diff_lines);

        // Build argument list
        let mut args: Vec<&str> = vec!["diff", "--stat", "--patch"];
        if staged {
            args.push("--staged");
        }
        let range;
        if let (Some(a), Some(b)) = (commit_a, commit_b) {
            range = format!("{a}..{b}");
            args.push(&range);
        } else if let Some(a) = commit_a {
            args.push(a);
        }
        if let Some(fp) = file_path {
            args.push("--");
            args.push(fp);
        }

        let out = crate::cli::run_git(&work_dir, &args, self.guard.command_timeout_secs)
            .await
            .map_err(orka_core::Error::from)?
            .into_result()
            .map_err(orka_core::Error::from)?;

        // Truncate output
        let mut lines: Vec<&str> = out.stdout.lines().collect();
        let truncated = lines.len() > max_lines;
        lines.truncate(max_lines);
        let diff_text = lines.join("\n");

        Ok(SkillOutput::new(serde_json::json!({
            "diff": diff_text,
            "truncated": truncated,
            "lines": lines.len(),
        })))
    }
}
