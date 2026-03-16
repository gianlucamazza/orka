use std::sync::Arc;

use async_trait::async_trait;
use orka_core::traits::Skill;
use orka_core::{Error, Result, SkillInput, SkillOutput, SkillSchema};

use crate::config::PermissionLevel;
use crate::guard::PermissionGuard;

// ── service_status ──

pub struct ServiceStatusSkill {
    guard: Arc<PermissionGuard>,
}

impl ServiceStatusSkill {
    pub fn new(guard: Arc<PermissionGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
impl Skill for ServiceStatusSkill {
    fn name(&self) -> &str {
        "service_status"
    }

    fn description(&self) -> &str {
        "Get the status of a systemd service."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema {
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "unit": { "type": "string", "description": "Service unit name (e.g. 'sshd.service')" }
                },
                "required": ["unit"]
            }),
        }
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        self.guard.check_permission(PermissionLevel::Admin)?;

        let unit = input
            .args
            .get("unit")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Skill("missing 'unit' argument".into()))?;

        let output = tokio::process::Command::new("systemctl")
            .args(["status", unit, "--no-pager"])
            .output()
            .await
            .map_err(|e| Error::Skill(format!("systemctl failed: {}", e)))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();

        Ok(SkillOutput {
            data: serde_json::json!({
                "unit": unit,
                "status": stdout,
                "exit_code": output.status.code(),
            }),
        })
    }
}

// ── service_list ──

pub struct ServiceListSkill {
    guard: Arc<PermissionGuard>,
}

impl ServiceListSkill {
    pub fn new(guard: Arc<PermissionGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
impl Skill for ServiceListSkill {
    fn name(&self) -> &str {
        "service_list"
    }

    fn description(&self) -> &str {
        "List systemd service units."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema {
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "state": {
                        "type": "string",
                        "description": "Filter by state (running, failed, etc.)"
                    }
                },
                "required": []
            }),
        }
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        self.guard.check_permission(PermissionLevel::Admin)?;

        let state = input.args.get("state").and_then(|v| v.as_str());

        let mut cmd = tokio::process::Command::new("systemctl");
        cmd.args(["list-units", "--type=service", "--no-pager", "--plain"]);
        if let Some(s) = state {
            cmd.arg(format!("--state={}", s));
        }

        let output = cmd
            .output()
            .await
            .map_err(|e| Error::Skill(format!("systemctl failed: {}", e)))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();

        Ok(SkillOutput {
            data: serde_json::json!({
                "services": stdout,
                "success": output.status.success(),
            }),
        })
    }
}

// ── journal_read ──

pub struct JournalReadSkill {
    guard: Arc<PermissionGuard>,
}

impl JournalReadSkill {
    pub fn new(guard: Arc<PermissionGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
impl Skill for JournalReadSkill {
    fn name(&self) -> &str {
        "journal_read"
    }

    fn description(&self) -> &str {
        "Read systemd journal logs. Can filter by unit, time range, and priority."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema {
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "unit": { "type": "string", "description": "Filter by unit name" },
                    "since": { "type": "string", "description": "Show entries since (e.g. '1 hour ago', '2024-01-01')" },
                    "lines": { "type": "integer", "default": 100, "maximum": 500 },
                    "priority": {
                        "type": "string",
                        "description": "Filter by priority (emerg, alert, crit, err, warning, notice, info, debug)"
                    }
                },
                "required": []
            }),
        }
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        self.guard.check_permission(PermissionLevel::Admin)?;

        let unit = input.args.get("unit").and_then(|v| v.as_str());
        let since = input.args.get("since").and_then(|v| v.as_str());
        let lines = input
            .args
            .get("lines")
            .and_then(|v| v.as_u64())
            .unwrap_or(100)
            .min(500);
        let priority = input.args.get("priority").and_then(|v| v.as_str());

        let mut cmd = tokio::process::Command::new("journalctl");
        cmd.args(["--no-pager", "-n", &lines.to_string()]);

        if let Some(u) = unit {
            cmd.arg("-u").arg(u);
        }
        if let Some(s) = since {
            cmd.arg("--since").arg(s);
        }
        if let Some(p) = priority {
            cmd.arg("-p").arg(p);
        }

        let output = cmd
            .output()
            .await
            .map_err(|e| Error::Skill(format!("journalctl failed: {}", e)))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();

        Ok(SkillOutput {
            data: serde_json::json!({
                "logs": stdout,
                "success": output.status.success(),
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
            permission_level: "admin".into(),
            ..OsConfig::default()
        }))
    }

    #[test]
    fn service_status_schema_valid() {
        let skill = ServiceStatusSkill::new(make_guard());
        let schema = skill.schema();
        assert_eq!(schema.parameters["required"][0], "unit");
    }

    #[test]
    fn journal_read_schema_valid() {
        let skill = JournalReadSkill::new(make_guard());
        let _schema = skill.schema();
    }

    #[tokio::test]
    async fn service_status_requires_admin() {
        use orka_core::config::OsConfig;
        let guard = Arc::new(PermissionGuard::new(&OsConfig {
            permission_level: "execute".into(),
            ..OsConfig::default()
        }));
        let skill = ServiceStatusSkill::new(guard);
        let mut args = std::collections::HashMap::new();
        args.insert("unit".into(), serde_json::json!("sshd.service"));
        assert!(skill
            .execute(SkillInput {
                args,
                context: None
            })
            .await
            .is_err());
    }
}
