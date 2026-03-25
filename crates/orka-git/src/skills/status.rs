//! `git_status` skill.

use std::sync::Arc;

use async_trait::async_trait;
use orka_core::{Result, SkillInput, SkillOutput, SkillSchema, traits::Skill};

use crate::{cli::resolve_work_dir, guard::GitGuard};

/// Returns the working tree status (staged, modified, untracked, branch info).
///
/// Uses `git status --porcelain=v2 --branch` for machine-readable output.
pub struct GitStatusSkill {
    guard: Arc<GitGuard>,
}

impl GitStatusSkill {
    /// Create a new skill from the shared guard.
    pub fn new(guard: Arc<GitGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
impl Skill for GitStatusSkill {
    fn name(&self) -> &'static str {
        "git_status"
    }

    fn category(&self) -> &'static str {
        "git"
    }

    fn description(&self) -> &'static str {
        "Show the working tree status: staged files, unstaged changes, untracked files, \
         and current branch info (ahead/behind counts)."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Repository path (defaults to user's current working directory)"
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

        let out = crate::cli::run_git(
            &work_dir,
            &["status", "--porcelain=v2", "--branch"],
            self.guard.command_timeout_secs,
        )
        .await
        .map_err(orka_core::Error::from)?
        .into_result()
        .map_err(orka_core::Error::from)?;

        let parsed = parse_porcelain_v2(&out.stdout);
        Ok(SkillOutput::new(serde_json::json!({
            "branch": parsed.branch,
            "ahead": parsed.ahead,
            "behind": parsed.behind,
            "staged": parsed.staged,
            "modified": parsed.modified,
            "untracked": parsed.untracked,
            "conflicted": parsed.conflicted,
        })))
    }
}

// ── Parser ────────────────────────────────────────────────────────────────────

struct StatusResult {
    branch: String,
    ahead: u32,
    behind: u32,
    staged: Vec<String>,
    modified: Vec<String>,
    untracked: Vec<String>,
    conflicted: Vec<String>,
}

fn parse_porcelain_v2(output: &str) -> StatusResult {
    let mut branch = String::new();
    let mut ahead = 0u32;
    let mut behind = 0u32;
    let mut staged = Vec::new();
    let mut modified = Vec::new();
    let mut untracked = Vec::new();
    let mut conflicted = Vec::new();

    for line in output.lines() {
        if let Some(rest) = line.strip_prefix("# branch.head ") {
            branch = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("# branch.ab ") {
            // Format: "+N -M"
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() == 2 {
                ahead = parts[0].trim_start_matches('+').parse().unwrap_or(0);
                behind = parts[1].trim_start_matches('-').parse().unwrap_or(0);
            }
        } else if line.starts_with("1 ") || line.starts_with("2 ") {
            // Changed tracked entry: "1 XY N... mode mode mode hash hash path"
            let parts: Vec<&str> = line.splitn(9, ' ').collect();
            if parts.len() >= 9 {
                let xy = parts[1];
                let path = parts[8].split('\t').next().unwrap_or(parts[8]);
                let x = xy.chars().next().unwrap_or('.');
                let y = xy.chars().nth(1).unwrap_or('.');
                if x != '.' {
                    staged.push(path.to_string());
                }
                if y != '.' {
                    modified.push(path.to_string());
                }
            }
        } else if line.starts_with("u ") {
            // Unmerged entry
            let parts: Vec<&str> = line.splitn(11, ' ').collect();
            if parts.len() >= 11 {
                conflicted.push(parts[10].to_string());
            }
        } else if let Some(rest) = line.strip_prefix("? ") {
            untracked.push(rest.to_string());
        }
    }

    StatusResult {
        branch,
        ahead,
        behind,
        staged,
        modified,
        untracked,
        conflicted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_clean_repo() {
        let output = "# branch.oid abc123\n# branch.head main\n# branch.ab +0 -0\n";
        let r = parse_porcelain_v2(output);
        assert_eq!(r.branch, "main");
        assert_eq!(r.ahead, 0);
        assert_eq!(r.behind, 0);
        assert!(r.staged.is_empty());
        assert!(r.untracked.is_empty());
    }

    #[test]
    fn parse_modified_file() {
        let output = concat!(
            "# branch.head feat/x\n",
            "# branch.ab +2 -1\n",
            "1 .M N... 100644 100644 100644 aaa bbb src/main.rs\n",
            "? untracked.txt\n"
        );
        let r = parse_porcelain_v2(output);
        assert_eq!(r.branch, "feat/x");
        assert_eq!(r.ahead, 2);
        assert_eq!(r.behind, 1);
        assert!(r.staged.is_empty());
        assert_eq!(r.modified, vec!["src/main.rs"]);
        assert_eq!(r.untracked, vec!["untracked.txt"]);
    }
}
