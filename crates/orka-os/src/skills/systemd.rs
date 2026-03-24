use std::sync::Arc;

use async_trait::async_trait;
use orka_core::{
    Error, ErrorCategory, Result, SkillInput, SkillOutput, SkillSchema, traits::Skill,
};

use crate::{config::PermissionLevel, events::emit_executed, guard::PermissionGuard};

fn categorize_daemon_spawn_error(daemon: &str, e: std::io::Error) -> Error {
    let category = match e.kind() {
        std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied => {
            ErrorCategory::Environmental
        }
        _ => ErrorCategory::Unknown,
    };
    Error::SkillCategorized {
        message: format!("{} failed: {}", daemon, e),
        category,
    }
}

// ── service_status ──

/// Skill that returns the current status of a systemd service unit.
pub struct ServiceStatusSkill {
    guard: Arc<PermissionGuard>,
}

impl ServiceStatusSkill {
    /// Create a new `service_status` skill with the given permission guard.
    pub fn new(guard: Arc<PermissionGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
impl Skill for ServiceStatusSkill {
    fn name(&self) -> &str {
        "service_status"
    }

    fn category(&self) -> &str {
        "systemd"
    }

    fn description(&self) -> &str {
        "Get the status of a systemd service."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "unit": { "type": "string", "description": "Service unit name (e.g. 'sshd.service')" }
            },
            "required": ["unit"]
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        self.guard.check_permission(PermissionLevel::ReadOnly)?;

        let unit = input
            .args
            .get("unit")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::SkillCategorized {
                message: "missing 'unit' argument".into(),
                category: ErrorCategory::Input,
            })?;

        let output = tokio::process::Command::new("systemctl")
            .args(["status", unit, "--no-pager"])
            .output()
            .await
            .map_err(|e| categorize_daemon_spawn_error("systemctl", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();

        Ok(SkillOutput::new(serde_json::json!({
            "unit": unit,
            "status": stdout,
            "exit_code": output.status.code(),
        })))
    }
}

// ── service_list ──

/// Skill that lists systemd service units and their active/load state.
pub struct ServiceListSkill {
    guard: Arc<PermissionGuard>,
}

impl ServiceListSkill {
    /// Create a new `service_list` skill with the given permission guard.
    pub fn new(guard: Arc<PermissionGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
impl Skill for ServiceListSkill {
    fn name(&self) -> &str {
        "service_list"
    }

    fn category(&self) -> &str {
        "systemd"
    }

    fn description(&self) -> &str {
        "List systemd service units."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "state": {
                    "type": "string",
                    "description": "Filter by state (running, failed, etc.)"
                }
            },
            "required": []
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        self.guard.check_permission(PermissionLevel::ReadOnly)?;

        let state = input.args.get("state").and_then(|v| v.as_str());

        let mut cmd = tokio::process::Command::new("systemctl");
        cmd.args(["list-units", "--type=service", "--no-pager", "--plain"]);
        if let Some(s) = state {
            cmd.arg(format!("--state={}", s));
        }

        let output = cmd
            .output()
            .await
            .map_err(|e| categorize_daemon_spawn_error("systemctl", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();

        Ok(SkillOutput::new(serde_json::json!({
            "services": stdout,
            "success": output.status.success(),
        })))
    }
}

// ── journal_read ──

/// Skill that reads systemd journal log entries, with optional unit and time
/// filtering.
pub struct JournalReadSkill {
    guard: Arc<PermissionGuard>,
}

impl JournalReadSkill {
    /// Create a new `journal_read` skill with the given permission guard.
    pub fn new(guard: Arc<PermissionGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
impl Skill for JournalReadSkill {
    fn name(&self) -> &str {
        "journal_read"
    }

    fn category(&self) -> &str {
        "systemd"
    }

    fn description(&self) -> &str {
        "Read systemd journal logs. Can filter by unit, time range, and priority."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
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
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        self.guard.check_permission(PermissionLevel::ReadOnly)?;

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
            .map_err(|e| categorize_daemon_spawn_error("journalctl", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();

        Ok(SkillOutput::new(serde_json::json!({
            "logs": stdout,
            "success": output.status.success(),
        })))
    }
}

// ── service_control ──

/// Skill that starts, stops, or restarts systemd services via sudo.
pub struct ServiceControlSkill {
    guard: Arc<PermissionGuard>,
}

impl ServiceControlSkill {
    /// Create a new `service_control` skill from a permission guard.
    pub fn new(guard: Arc<PermissionGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
impl Skill for ServiceControlSkill {
    fn name(&self) -> &str {
        "service_control"
    }

    fn category(&self) -> &str {
        "systemd"
    }

    fn description(&self) -> &str {
        "Start, stop, or restart a systemd service (requires sudo)."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "unit": { "type": "string", "description": "Service unit name (e.g. 'nginx.service')" },
                "action": {
                    "type": "string",
                    "enum": ["start", "stop", "restart"],
                    "description": "Action to perform on the service"
                }
            },
            "required": ["unit", "action"]
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        self.guard.check_permission(PermissionLevel::Admin)?;

        let unit = input
            .args
            .get("unit")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::SkillCategorized {
                message: "missing 'unit' argument".into(),
                category: ErrorCategory::Input,
            })?;
        let action = input
            .args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::SkillCategorized {
                message: "missing 'action' argument".into(),
                category: ErrorCategory::Input,
            })?;

        if !matches!(action, "start" | "stop" | "restart") {
            return Err(Error::SkillCategorized {
                message: format!(
                    "invalid action '{}': must be start, stop, or restart",
                    action
                ),
                category: ErrorCategory::Input,
            });
        }

        // Check sudo allowlist
        self.guard
            .check_sudo_command("systemctl", &[action, unit])?;

        // Sudo execution proceeds without interactive approval
        // (approval should be handled externally via sudoers configuration)

        let start = std::time::Instant::now();
        let output = tokio::process::Command::new(self.guard.sudo_path())
            .args(["-n", "systemctl", action, unit])
            .output()
            .await
            .map_err(|e| Error::SkillCategorized {
                message: format!("systemctl failed: {}", e),
                category: ErrorCategory::Environmental,
            })?;
        let duration_ms = start.elapsed().as_millis() as u64;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        emit_executed(
            &input,
            "systemctl",
            &[action, unit],
            output.status.code(),
            output.status.success(),
            duration_ms,
        )
        .await;

        if !output.status.success() {
            return Err(Error::SkillCategorized {
                message: format!(
                    "systemctl {} {} failed (exit {}): {}",
                    action,
                    unit,
                    output.status.code().unwrap_or(-1),
                    stderr.trim()
                ),
                category: ErrorCategory::Environmental,
            });
        }

        Ok(SkillOutput::new(serde_json::json!({
            "unit": unit,
            "action": action,
            "stdout": stdout,
            "stderr": stderr,
        })))
    }
}

#[cfg(test)]
mod tests {
    use orka_core::config::{OsConfig, primitives::OsPermissionLevel};

    use super::*;

    fn make_guard() -> Arc<PermissionGuard> {
        let mut config = OsConfig::default();
        config.permission_level = OsPermissionLevel::Admin;
        Arc::new(PermissionGuard::new(&config))
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

    #[test]
    fn service_control_schema_valid() {
        let skill = ServiceControlSkill::new(make_guard());
        let schema = skill.schema();
        assert_eq!(schema.parameters["required"][0], "unit");
        assert_eq!(schema.parameters["required"][1], "action");
    }

    #[tokio::test]
    async fn service_control_requires_admin() {
        use orka_core::config::SudoConfig;
        let mut config = OsConfig::default();
        config.permission_level = OsPermissionLevel::Execute;
        let mut sudo = SudoConfig::default();
        sudo.allowed = true;
        sudo.allowed_commands = vec!["systemctl restart".into()];
        config.sudo = sudo;
        let guard = Arc::new(PermissionGuard::new(&config));
        let skill = ServiceControlSkill::new(guard);
        let mut args = std::collections::HashMap::new();
        args.insert("unit".into(), serde_json::json!("nginx.service"));
        args.insert("action".into(), serde_json::json!("restart"));
        assert!(skill.execute(SkillInput::new(args)).await.is_err());
    }

    #[tokio::test]
    async fn service_status_allowed_at_read_only() {
        let mut config = OsConfig::default();
        config.permission_level = OsPermissionLevel::ReadOnly;
        let guard = Arc::new(PermissionGuard::new(&config));
        let skill = ServiceStatusSkill::new(guard);
        let mut args = std::collections::HashMap::new();
        args.insert("unit".into(), serde_json::json!("sshd.service"));
        // Permission check passes; any failure is from the missing systemctl binary.
        let result = skill.execute(SkillInput::new(args)).await;
        assert!(
            result.is_ok()
                || !result
                    .unwrap_err()
                    .to_string()
                    .contains("permission denied"),
            "should not fail with a permission error at read-only level"
        );
    }
}
