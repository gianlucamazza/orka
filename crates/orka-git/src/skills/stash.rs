//! `git_stash` skill.

use std::sync::Arc;

use async_trait::async_trait;
use orka_core::{Result, SkillInput, SkillOutput, SkillSchema, traits::Skill};

use crate::{
    cli::{resolve_work_dir, run_git},
    guard::GitGuard,
};

/// Manages the stash: push, pop, list, or drop.
pub struct GitStashSkill {
    guard: Arc<GitGuard>,
}

impl GitStashSkill {
    /// Create from the shared guard.
    pub fn new(guard: Arc<GitGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
#[allow(clippy::too_many_lines)]
impl Skill for GitStashSkill {
    fn name(&self) -> &'static str {
        "git_stash"
    }

    fn category(&self) -> &'static str {
        "git"
    }

    fn description(&self) -> &'static str {
        "Manage the git stash. \
         Use `action=push` to stash current changes, `action=pop` to restore, \
         `action=list` to see all stash entries, `action=drop` to discard one."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "required": ["action"],
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["push", "pop", "list", "drop"],
                    "description": "Stash operation to perform"
                },
                "message": {
                    "type": "string",
                    "description": "Optional description for `push` action"
                },
                "index": {
                    "type": "integer",
                    "description": "Stash index for `pop` or `drop` (default: 0 = most recent)",
                    "default": 0
                },
                "path": {
                    "type": "string",
                    "description": "Repository path"
                }
            }
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let action = input
            .args
            .get("action")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| orka_core::Error::SkillCategorized {
                message: "missing 'action' argument".to_string(),
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

        match action {
            "push" => {
                let mut args = vec!["stash", "push"];
                let message = input
                    .args
                    .get("message")
                    .and_then(serde_json::Value::as_str);
                let msg_arg;
                if let Some(m) = message {
                    args.push("-m");
                    msg_arg = m.to_string();
                    args.push(&msg_arg);
                }
                run_git(&work_dir, &args, self.guard.command_timeout_secs)
                    .await
                    .map_err(orka_core::Error::from)?
                    .into_result()
                    .map_err(orka_core::Error::from)?;

                Ok(SkillOutput::new(serde_json::json!({
                    "action": "push",
                    "message": message,
                })))
            }
            "pop" => {
                let index = input
                    .args
                    .get("index")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                let ref_arg = format!("stash@{{{index}}}");
                run_git(
                    &work_dir,
                    &["stash", "pop", &ref_arg],
                    self.guard.command_timeout_secs,
                )
                .await
                .map_err(orka_core::Error::from)?
                .into_result()
                .map_err(orka_core::Error::from)?;

                Ok(SkillOutput::new(serde_json::json!({
                    "action": "pop",
                    "index": index,
                })))
            }
            "list" => {
                let out = run_git(
                    &work_dir,
                    &["stash", "list", "--format=%gd%x1f%s"],
                    self.guard.command_timeout_secs,
                )
                .await
                .map_err(orka_core::Error::from)?
                .into_result()
                .map_err(orka_core::Error::from)?;

                let entries: Vec<serde_json::Value> = out
                    .stdout
                    .lines()
                    .filter(|l| !l.is_empty())
                    .map(|l| {
                        let mut parts = l.splitn(2, '\x1f');
                        let ref_name = parts.next().unwrap_or("").trim();
                        let subject = parts.next().unwrap_or("").trim();
                        serde_json::json!({ "ref": ref_name, "subject": subject })
                    })
                    .collect();

                Ok(SkillOutput::new(serde_json::json!({
                    "action": "list",
                    "entries": entries,
                    "count": entries.len(),
                })))
            }
            "drop" => {
                let index = input
                    .args
                    .get("index")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                let ref_arg = format!("stash@{{{index}}}");
                run_git(
                    &work_dir,
                    &["stash", "drop", &ref_arg],
                    self.guard.command_timeout_secs,
                )
                .await
                .map_err(orka_core::Error::from)?
                .into_result()
                .map_err(orka_core::Error::from)?;

                Ok(SkillOutput::new(serde_json::json!({
                    "action": "drop",
                    "index": index,
                })))
            }
            other => Err(orka_core::Error::SkillCategorized {
                message: format!("unknown stash action '{other}'; expected push|pop|list|drop"),
                category: orka_core::ErrorCategory::Input,
            }),
        }
    }
}
