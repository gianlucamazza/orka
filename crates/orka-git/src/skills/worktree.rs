//! Worktree skills: `git_worktree_create`, `git_worktree_list`,
//! `git_worktree_remove`.

use std::sync::Arc;

use async_trait::async_trait;
use orka_core::{Result, SkillInput, SkillOutput, SkillSchema, traits::Skill};

use crate::{guard::GitGuard, worktree::WorktreeManager};

// ── git_worktree_create
// ───────────────────────────────────────────────────────

/// Creates an isolated worktree for a task or agent.
pub struct GitWorktreeCreateSkill {
    guard: Arc<GitGuard>,
    manager: Arc<WorktreeManager>,
}

impl GitWorktreeCreateSkill {
    /// Create from the shared guard and worktree manager.
    pub fn new(guard: Arc<GitGuard>, manager: Arc<WorktreeManager>) -> Self {
        Self { guard, manager }
    }
}

#[async_trait]
impl Skill for GitWorktreeCreateSkill {
    fn name(&self) -> &'static str {
        "git_worktree_create"
    }

    fn category(&self) -> &'static str {
        "git_worktree"
    }

    fn description(&self) -> &'static str {
        "Create an isolated git worktree for a task. \
         Each worktree shares the object database but has its own working tree and branch, \
         enabling parallel agent work without interference. \
         `.env` files are copied; cache directories are symlinked."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "required": ["branch"],
            "properties": {
                "branch": {
                    "type": "string",
                    "description": "New branch name to create in the worktree"
                },
                "base": {
                    "type": "string",
                    "description": "Base ref to branch from (default: HEAD)"
                },
                "agent_id": {
                    "type": "string",
                    "description": "Identifier of the agent owning this worktree"
                }
            }
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let _ = &self.guard; // guard is held for policy context

        let branch = input
            .args
            .get("branch")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| orka_core::Error::SkillCategorized {
                message: "missing 'branch' argument".to_string(),
                category: orka_core::ErrorCategory::Input,
            })?;
        let base = input
            .args
            .get("base")
            .and_then(serde_json::Value::as_str)
            .map(String::from);
        let agent_id = input
            .args
            .get("agent_id")
            .and_then(serde_json::Value::as_str)
            .map(String::from);

        let info = self
            .manager
            .create(branch, base.as_deref(), agent_id)
            .await
            .map_err(orka_core::Error::from)?;

        Ok(SkillOutput::new(serde_json::json!({
            "created": true,
            "name": info.name,
            "path": info.path.to_string_lossy(),
            "branch": info.branch,
            "head_sha": info.head_sha,
            "created_at": info.created_at,
        })))
    }
}

// ── git_worktree_list
// ─────────────────────────────────────────────────────────

/// Lists all active worktrees.
pub struct GitWorktreeListSkill {
    guard: Arc<GitGuard>,
    manager: Arc<WorktreeManager>,
}

impl GitWorktreeListSkill {
    /// Create from the shared guard and worktree manager.
    pub fn new(guard: Arc<GitGuard>, manager: Arc<WorktreeManager>) -> Self {
        Self { guard, manager }
    }
}

#[async_trait]
impl Skill for GitWorktreeListSkill {
    fn name(&self) -> &'static str {
        "git_worktree_list"
    }

    fn category(&self) -> &'static str {
        "git_worktree"
    }

    fn description(&self) -> &'static str {
        "List all active git worktrees with their paths, branches, and metadata."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {}
        }))
    }

    async fn execute(&self, _input: SkillInput) -> Result<SkillOutput> {
        let _ = &self.guard;

        let worktrees = self.manager.list().await.map_err(orka_core::Error::from)?;

        let entries: Vec<serde_json::Value> = worktrees
            .iter()
            .map(|w| {
                serde_json::json!({
                    "name": w.name,
                    "path": w.path.to_string_lossy(),
                    "branch": w.branch,
                    "head_sha": w.head_sha,
                    "created_at": w.created_at,
                    "agent_id": w.agent_id,
                })
            })
            .collect();

        Ok(SkillOutput::new(serde_json::json!({
            "worktrees": entries,
            "count": entries.len(),
        })))
    }
}

// ── git_worktree_remove
// ───────────────────────────────────────────────────────

/// Removes a worktree.
pub struct GitWorktreeRemoveSkill {
    guard: Arc<GitGuard>,
    manager: Arc<WorktreeManager>,
}

impl GitWorktreeRemoveSkill {
    /// Create from the shared guard and worktree manager.
    pub fn new(guard: Arc<GitGuard>, manager: Arc<WorktreeManager>) -> Self {
        Self { guard, manager }
    }
}

#[async_trait]
impl Skill for GitWorktreeRemoveSkill {
    fn name(&self) -> &'static str {
        "git_worktree_remove"
    }

    fn category(&self) -> &'static str {
        "git_worktree"
    }

    fn description(&self) -> &'static str {
        "Remove a worktree. Use `force=true` only if the worktree has uncommitted changes \
         that should be discarded. Runs `git worktree prune` after removal."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "required": ["name"],
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Worktree name (as returned by git_worktree_list)"
                },
                "force": {
                    "type": "boolean",
                    "description": "Force removal even if the worktree has uncommitted changes",
                    "default": false
                }
            }
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let _ = &self.guard;

        let name = input
            .args
            .get("name")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| orka_core::Error::SkillCategorized {
                message: "missing 'name' argument".to_string(),
                category: orka_core::ErrorCategory::Input,
            })?;
        let force = input
            .args
            .get("force")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);

        self.manager
            .remove(name, force)
            .await
            .map_err(orka_core::Error::from)?;

        Ok(SkillOutput::new(serde_json::json!({
            "removed": name,
            "force": force,
        })))
    }
}
