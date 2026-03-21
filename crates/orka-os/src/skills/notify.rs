use std::sync::Arc;

use async_trait::async_trait;
use orka_core::traits::Skill;
use orka_core::{Error, ErrorCategory, Result, SkillInput, SkillOutput, SkillSchema};

use crate::config::PermissionLevel;
use crate::guard::PermissionGuard;

/// Skill that sends a desktop notification via `notify-send`.
pub struct NotifySendSkill {
    guard: Arc<PermissionGuard>,
}

impl NotifySendSkill {
    /// Create a new `notify_send` skill with the given permission guard.
    pub fn new(guard: Arc<PermissionGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
impl Skill for NotifySendSkill {
    fn name(&self) -> &str {
        "notify_send"
    }

    fn category(&self) -> &str {
        "desktop"
    }

    fn description(&self) -> &str {
        "Send a desktop notification using notify-send."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
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
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        self.guard.check_permission(PermissionLevel::Interact)?;

        let title = input
            .args
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::SkillCategorized {
                message: "missing 'title' argument".into(),
                category: ErrorCategory::Input,
            })?;
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

        let output = cmd.output().await.map_err(|e| Error::SkillCategorized {
            message: format!("notify-send failed: {}", e),
            category: ErrorCategory::Environmental,
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::SkillCategorized {
                message: format!(
                    "notify-send failed (exit {}): {}",
                    output.status.code().unwrap_or(-1),
                    stderr.trim()
                ),
                category: ErrorCategory::Environmental,
            });
        }

        Ok(SkillOutput::new(serde_json::json!({
            "title": title,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_guard() -> Arc<PermissionGuard> {
        use orka_core::config::OsConfig;
        Arc::new(PermissionGuard::new(&OsConfig {
            permission_level: "interact".into(),
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
        assert!(skill.execute(SkillInput::new(args)).await.is_err());
    }
}
