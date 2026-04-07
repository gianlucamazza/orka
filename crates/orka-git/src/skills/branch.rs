//! Branch-related skills: `git_branch_list`, `git_branch_create`,
//! `git_checkout`.

use std::sync::Arc;

use async_trait::async_trait;
use orka_core::{Result, SkillInput, SkillOutput, SkillSchema, traits::Skill};

use crate::{
    cli::{resolve_work_dir, run_git},
    guard::GitGuard,
    repo,
};

// ── git_branch_list
// ────────────────────────────────────────────────────────────

/// Lists all local and optionally remote branches.
///
/// Uses gix for efficient, no-fork branch enumeration.
pub struct GitBranchListSkill {
    #[allow(dead_code)] // held for future per-call policy hooks
    guard: Arc<GitGuard>,
}

impl GitBranchListSkill {
    /// Create from the shared guard.
    pub fn new(guard: Arc<GitGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
impl Skill for GitBranchListSkill {
    fn name(&self) -> &'static str {
        "git_branch_list"
    }

    fn category(&self) -> &'static str {
        "git"
    }

    fn description(&self) -> &'static str {
        "List local branches (and optionally remote-tracking branches). \
         Marks the currently checked-out branch."
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
                    "description": "Repository path (defaults to user's current working directory)"
                },
                "include_remote": {
                    "type": "boolean",
                    "description": "Include remote-tracking branches",
                    "default": false
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

        let include_remote = input
            .args
            .get("include_remote")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);

        let (branches, current) = repo::list_branches(work_dir)
            .await
            .map_err(orka_core::Error::from)?;

        let branch_values: Vec<serde_json::Value> = branches
            .into_iter()
            .filter(|b| include_remote || !b.is_remote)
            .map(|b| {
                serde_json::json!({
                    "name": b.name,
                    "current": b.is_current,
                    "remote": b.is_remote,
                })
            })
            .collect();

        Ok(SkillOutput::new(serde_json::json!({
            "current": current,
            "branches": branch_values,
        })))
    }
}

// ── git_branch_create
// ──────────────────────────────────────────────────────────

/// Creates a new branch.
pub struct GitBranchCreateSkill {
    guard: Arc<GitGuard>,
}

impl GitBranchCreateSkill {
    /// Create from the shared guard.
    pub fn new(guard: Arc<GitGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
impl Skill for GitBranchCreateSkill {
    fn name(&self) -> &'static str {
        "git_branch_create"
    }

    fn category(&self) -> &'static str {
        "git"
    }

    fn description(&self) -> &'static str {
        "Create a new branch. Optionally specify a start point (commit, branch, or tag)."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "required": ["name"],
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Branch name to create"
                },
                "start_point": {
                    "type": "string",
                    "description": "Commit, branch, or tag to branch from (default: HEAD)"
                },
                "path": {
                    "type": "string",
                    "description": "Repository path"
                }
            }
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let name = input
            .args
            .get("name")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| orka_core::Error::SkillCategorized {
                message: "missing 'name' argument".to_string(),
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

        let start_point = input
            .args
            .get("start_point")
            .and_then(serde_json::Value::as_str);

        let mut args = vec!["branch", name];
        if let Some(sp) = start_point {
            args.push(sp);
        }

        run_git(&work_dir, &args, self.guard.command_timeout_secs)
            .await
            .map_err(orka_core::Error::from)?
            .into_result()
            .map_err(orka_core::Error::from)?;

        Ok(SkillOutput::new(serde_json::json!({
            "created": name,
            "start_point": start_point,
        })))
    }
}

// ── git_checkout
// ───────────────────────────────────────────────────────────────

/// Checks out a branch or commit.
pub struct GitCheckoutSkill {
    guard: Arc<GitGuard>,
}

impl GitCheckoutSkill {
    /// Create from the shared guard.
    pub fn new(guard: Arc<GitGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
impl Skill for GitCheckoutSkill {
    fn name(&self) -> &'static str {
        "git_checkout"
    }

    fn category(&self) -> &'static str {
        "git"
    }

    fn description(&self) -> &'static str {
        "Switch to a branch or detach HEAD to a commit. \
         Use `git_branch_create` first to create a new branch."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "required": ["target"],
            "properties": {
                "target": {
                    "type": "string",
                    "description": "Branch name or commit SHA to check out"
                },
                "path": {
                    "type": "string",
                    "description": "Repository path"
                }
            }
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let target = input
            .args
            .get("target")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| orka_core::Error::SkillCategorized {
                message: "missing 'target' argument".to_string(),
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

        run_git(
            &work_dir,
            &["checkout", target],
            self.guard.command_timeout_secs,
        )
        .await
        .map_err(orka_core::Error::from)?
        .into_result()
        .map_err(orka_core::Error::from)?;

        Ok(SkillOutput::new(serde_json::json!({
            "checked_out": target,
        })))
    }
}
