use std::sync::Arc;

use async_trait::async_trait;
use orka_core::traits::Skill;
use orka_core::{Error, Result, SkillInput, SkillOutput, SkillSchema};

use crate::config::PermissionLevel;
use crate::guard::PermissionGuard;

pub struct NotifySendSkill {
    guard: Arc<PermissionGuard>,
}

impl NotifySendSkill {
    pub fn new(guard: Arc<PermissionGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
impl Skill for NotifySendSkill {
    fn name(&self) -> &str {
        "notify_send"
    }

    fn description(&self) -> &str {
        "Send a desktop notification using notify-send."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema {
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "Notification title" },
                    "body": { "type": "string", "description": "Notification body text" },
                    "urgency": {
                        "type": "string",
                        "enum": ["low", "normal", "critical"],
                        "default": "normal"
                    },
                    "icon": { "type": "string", "description": "Icon name or path" },
                    "timeout_ms": { "type": "integer", "description": "Display duration in milliseconds" }
                },
                "required": ["title"]
            }),
        }
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        self.guard.check_permission(PermissionLevel::Write)?;

        let title = input
            .args
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Skill("missing 'title' argument".into()))?;
        let body = input.args.get("body").and_then(|v| v.as_str());
        let urgency = input
            .args
            .get("urgency")
            .and_then(|v| v.as_str())
            .unwrap_or("normal");
        let icon = input
            .args
            .get("icon")
            .and_then(|v| v.as_str())
            .unwrap_or("orka");
        let timeout_ms = input.args.get("timeout_ms").and_then(|v| v.as_u64());

        let mut cmd = tokio::process::Command::new("notify-send");
        cmd.arg("--app-name").arg("orka");
        cmd.arg("--urgency").arg(urgency);
        cmd.arg("--icon").arg(icon);
        if let Some(ms) = timeout_ms {
            cmd.arg("--expire-time").arg(ms.to_string());
        }

        cmd.arg(title);
        if let Some(body) = body {
            cmd.arg(body);
        }

        let output = cmd
            .output()
            .await
            .map_err(|e| Error::Skill(format!("notify-send failed: {}", e)))?;

        Ok(SkillOutput {
            data: serde_json::json!({
                "success": output.status.success(),
                "title": title,
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_guard() -> Arc<PermissionGuard> {
        use orka_core::config::OsConfig;
        Arc::new(PermissionGuard::new(&OsConfig {
            permission_level: "write".into(),
            ..OsConfig::default()
        }))
    }

    #[test]
    fn schema_valid() {
        let skill = NotifySendSkill::new(make_guard());
        let schema = skill.schema();
        assert_eq!(schema.parameters["required"][0], "title");
    }

    #[tokio::test]
    async fn requires_write_permission() {
        use orka_core::config::OsConfig;
        let guard = Arc::new(PermissionGuard::new(&OsConfig {
            permission_level: "read-only".into(),
            ..OsConfig::default()
        }));
        let skill = NotifySendSkill::new(guard);
        let mut args = std::collections::HashMap::new();
        args.insert("title".into(), serde_json::json!("test"));
        assert!(
            skill
                .execute(SkillInput {
                    args,
                    context: None
                })
                .await
                .is_err()
        );
    }
}
