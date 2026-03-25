//! Remote skills: `git_fetch`, `git_pull`, `git_push`, `git_merge`.

use std::sync::Arc;

use async_trait::async_trait;
use orka_core::{Result, SkillInput, SkillOutput, SkillSchema, traits::Skill};

use crate::{
    cli::{resolve_work_dir, run_git},
    guard::GitGuard,
};

// ── git_fetch
// ─────────────────────────────────────────────────────────────────

/// Fetches from a remote.
pub struct GitFetchSkill {
    guard: Arc<GitGuard>,
}

impl GitFetchSkill {
    /// Create from the shared guard.
    pub fn new(guard: Arc<GitGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
impl Skill for GitFetchSkill {
    fn name(&self) -> &'static str {
        "git_fetch"
    }

    fn category(&self) -> &'static str {
        "git"
    }

    fn description(&self) -> &'static str {
        "Fetch from a remote repository (does not merge). \
         Use `git_pull` to fetch and merge/rebase."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "remote": {
                    "type": "string",
                    "description": "Remote name (default: origin)",
                    "default": "origin"
                },
                "prune": {
                    "type": "boolean",
                    "description": "Remove remote-tracking references that no longer exist on the remote",
                    "default": false
                },
                "path": {
                    "type": "string",
                    "description": "Repository path"
                }
            }
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let remote = input
            .args
            .get("remote")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("origin");
        let prune = input
            .args
            .get("prune")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);

        let cwd_str = input.args.get("path").and_then(serde_json::Value::as_str);
        let worktree_cwd = input
            .context
            .as_ref()
            .and_then(|c| c.worktree_cwd.as_deref());
        let user_cwd = input.context.as_ref().and_then(|c| c.user_cwd.as_deref());
        let work_dir = resolve_work_dir(cwd_str.or(worktree_cwd).or(user_cwd))
            .map_err(orka_core::Error::from)?;

        self.guard
            .check_remote(remote)
            .map_err(orka_core::Error::from)?;

        let mut args = vec!["fetch", remote];
        if prune {
            args.push("--prune");
        }

        run_git(&work_dir, &args, self.guard.command_timeout_secs)
            .await
            .map_err(orka_core::Error::from)?
            .into_result()
            .map_err(orka_core::Error::from)?;

        Ok(SkillOutput::new(serde_json::json!({
            "fetched": true,
            "remote": remote,
            "pruned": prune,
        })))
    }
}

// ── git_pull
// ──────────────────────────────────────────────────────────────────

/// Pulls (fetch + merge/rebase) from a remote.
pub struct GitPullSkill {
    guard: Arc<GitGuard>,
}

impl GitPullSkill {
    /// Create from the shared guard.
    pub fn new(guard: Arc<GitGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
impl Skill for GitPullSkill {
    fn name(&self) -> &'static str {
        "git_pull"
    }

    fn category(&self) -> &'static str {
        "git"
    }

    fn description(&self) -> &'static str {
        "Fetch and integrate changes from a remote. \
         Defaults to `--rebase` to keep a linear history."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "remote": {
                    "type": "string",
                    "description": "Remote name (default: origin)",
                    "default": "origin"
                },
                "branch": {
                    "type": "string",
                    "description": "Remote branch to pull (default: tracking branch)"
                },
                "rebase": {
                    "type": "boolean",
                    "description": "Use rebase instead of merge (default: true)",
                    "default": true
                },
                "path": {
                    "type": "string",
                    "description": "Repository path"
                }
            }
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let remote = input
            .args
            .get("remote")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("origin");
        let branch = input.args.get("branch").and_then(serde_json::Value::as_str);
        let rebase = input
            .args
            .get("rebase")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);

        let cwd_str = input.args.get("path").and_then(serde_json::Value::as_str);
        let worktree_cwd = input
            .context
            .as_ref()
            .and_then(|c| c.worktree_cwd.as_deref());
        let user_cwd = input.context.as_ref().and_then(|c| c.user_cwd.as_deref());
        let work_dir = resolve_work_dir(cwd_str.or(worktree_cwd).or(user_cwd))
            .map_err(orka_core::Error::from)?;

        self.guard
            .check_remote(remote)
            .map_err(orka_core::Error::from)?;

        let mut args = vec!["pull"];
        if rebase {
            args.push("--rebase");
        }
        args.push(remote);
        if let Some(b) = branch {
            args.push(b);
        }

        let out = run_git(&work_dir, &args, self.guard.command_timeout_secs)
            .await
            .map_err(orka_core::Error::from)?
            .into_result()
            .map_err(orka_core::Error::from)?;

        Ok(SkillOutput::new(serde_json::json!({
            "pulled": true,
            "remote": remote,
            "branch": branch,
            "rebase": rebase,
            "output": out.stdout.trim(),
        })))
    }
}

// ── git_push
// ──────────────────────────────────────────────────────────────────

/// Pushes local commits to a remote.
///
/// **Guardrails:**
/// - Force-push is blocked by default (`allow_force_push = false`).
/// - Protected branches (e.g. `main`, `release/*`) block direct pushes.
/// - Remote must be on the `allowed_remotes` list (if configured).
pub struct GitPushSkill {
    guard: Arc<GitGuard>,
}

impl GitPushSkill {
    /// Create from the shared guard.
    pub fn new(guard: Arc<GitGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
impl Skill for GitPushSkill {
    fn name(&self) -> &'static str {
        "git_push"
    }

    fn category(&self) -> &'static str {
        "git"
    }

    fn description(&self) -> &'static str {
        "Push commits to a remote. \
         Force-push is disabled by default. \
         Protected branches (main, release/*) cannot be pushed to directly. \
         Requires HITL approval by default — check agent configuration."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "remote": {
                    "type": "string",
                    "description": "Remote name (default: origin)",
                    "default": "origin"
                },
                "branch": {
                    "type": "string",
                    "description": "Local branch to push (default: current branch)"
                },
                "force": {
                    "type": "boolean",
                    "description": "Force push — blocked by default policy",
                    "default": false
                },
                "set_upstream": {
                    "type": "boolean",
                    "description": "Set upstream tracking (-u flag)",
                    "default": false
                },
                "path": {
                    "type": "string",
                    "description": "Repository path"
                }
            }
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let remote = input
            .args
            .get("remote")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("origin");
        let branch = input.args.get("branch").and_then(serde_json::Value::as_str);
        let force = input
            .args
            .get("force")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let set_upstream = input
            .args
            .get("set_upstream")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);

        let cwd_str = input.args.get("path").and_then(serde_json::Value::as_str);
        let worktree_cwd = input
            .context
            .as_ref()
            .and_then(|c| c.worktree_cwd.as_deref());
        let user_cwd = input.context.as_ref().and_then(|c| c.user_cwd.as_deref());
        let work_dir = resolve_work_dir(cwd_str.or(worktree_cwd).or(user_cwd))
            .map_err(orka_core::Error::from)?;

        // Policy checks
        self.guard
            .check_remote(remote)
            .map_err(orka_core::Error::from)?;

        if let Some(b) = branch {
            self.guard
                .check_push_allowed(b, force)
                .map_err(orka_core::Error::from)?;
        } else if force {
            // Even without a named branch, block force if policy forbids it
            self.guard
                .check_push_allowed("", force)
                .map_err(orka_core::Error::from)?;
        }

        let mut args = vec!["push"];
        if force {
            args.push("--force-with-lease"); // safer than --force
        }
        if set_upstream {
            args.push("-u");
        }
        args.push(remote);
        if let Some(b) = branch {
            args.push(b);
        }

        let out = run_git(&work_dir, &args, self.guard.command_timeout_secs)
            .await
            .map_err(orka_core::Error::from)?
            .into_result()
            .map_err(orka_core::Error::from)?;

        Ok(SkillOutput::new(serde_json::json!({
            "pushed": true,
            "remote": remote,
            "branch": branch,
            "force": force,
            "output": out.stderr.trim(), // git push prints progress to stderr
        })))
    }
}

// ── git_merge
// ─────────────────────────────────────────────────────────────────

/// Merges a branch into the current branch.
pub struct GitMergeSkill {
    guard: Arc<GitGuard>,
}

impl GitMergeSkill {
    /// Create from the shared guard.
    pub fn new(guard: Arc<GitGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
impl Skill for GitMergeSkill {
    fn name(&self) -> &'static str {
        "git_merge"
    }

    fn category(&self) -> &'static str {
        "git"
    }

    fn description(&self) -> &'static str {
        "Merge a branch into the current branch. \
         Use `--no-ff` to always create a merge commit. \
         Requires HITL approval by default — check agent configuration."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "required": ["branch"],
            "properties": {
                "branch": {
                    "type": "string",
                    "description": "Branch to merge into the current branch"
                },
                "no_ff": {
                    "type": "boolean",
                    "description": "Always create a merge commit (no fast-forward)",
                    "default": false
                },
                "message": {
                    "type": "string",
                    "description": "Custom merge commit message"
                },
                "path": {
                    "type": "string",
                    "description": "Repository path"
                }
            }
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let branch = input
            .args
            .get("branch")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| orka_core::Error::SkillCategorized {
                message: "missing 'branch' argument".to_string(),
                category: orka_core::ErrorCategory::Input,
            })?;
        let no_ff = input
            .args
            .get("no_ff")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let message = input
            .args
            .get("message")
            .and_then(serde_json::Value::as_str);

        let cwd_str = input.args.get("path").and_then(serde_json::Value::as_str);
        let worktree_cwd = input
            .context
            .as_ref()
            .and_then(|c| c.worktree_cwd.as_deref());
        let user_cwd = input.context.as_ref().and_then(|c| c.user_cwd.as_deref());
        let work_dir = resolve_work_dir(cwd_str.or(worktree_cwd).or(user_cwd))
            .map_err(orka_core::Error::from)?;

        let mut args = vec!["merge"];
        if no_ff {
            args.push("--no-ff");
        }
        let msg_arg;
        if let Some(m) = message {
            args.push("-m");
            msg_arg = m.to_string();
            args.push(&msg_arg);
        }
        args.push(branch);

        let out = run_git(&work_dir, &args, self.guard.command_timeout_secs)
            .await
            .map_err(orka_core::Error::from)?
            .into_result()
            .map_err(orka_core::Error::from)?;

        Ok(SkillOutput::new(serde_json::json!({
            "merged": branch,
            "no_ff": no_ff,
            "output": out.stdout.trim(),
        })))
    }
}
