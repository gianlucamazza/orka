use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use orka_core::config::OsConfig;
use orka_core::traits::Skill;
use orka_core::{
    DomainEvent, DomainEventKind, Error, Result, SkillInput, SkillOutput, SkillSchema,
};
use uuid::Uuid;

use crate::approval::{ApprovalChannel, ApprovalDecision, ApprovalRequest};
use crate::config::PermissionLevel;
use crate::guard::PermissionGuard;

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

        Ok(SkillOutput::new(serde_json::json!({
            "services": stdout,
            "success": output.status.success(),
        })))
    }
}

// ── journal_read ──

/// Skill that reads systemd journal log entries, with optional unit and time filtering.
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
    require_confirmation: bool,
    confirmation_timeout_secs: u64,
    approval: Arc<dyn ApprovalChannel>,
}

impl ServiceControlSkill {
    /// Create a new `service_control` skill from config, a permission guard, and an approval channel.
    pub fn new(
        guard: Arc<PermissionGuard>,
        config: &OsConfig,
        approval: Arc<dyn ApprovalChannel>,
    ) -> Self {
        Self {
            guard,
            require_confirmation: config.sudo.require_confirmation,
            confirmation_timeout_secs: config.sudo.confirmation_timeout_secs,
            approval,
        }
    }
}

#[async_trait]
impl Skill for ServiceControlSkill {
    fn name(&self) -> &str {
        "service_control"
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
            .ok_or_else(|| Error::Skill("missing 'unit' argument".into()))?;
        let action = input
            .args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Skill("missing 'action' argument".into()))?;

        if !matches!(action, "start" | "stop" | "restart") {
            return Err(Error::Skill(format!(
                "invalid action '{}': must be start, stop, or restart",
                action
            )));
        }

        // Check sudo allowlist
        self.guard
            .check_sudo_command("systemctl", &[action, unit])?;

        // Request approval if needed
        if self.require_confirmation {
            let now = Utc::now();
            let req = ApprovalRequest {
                id: Uuid::now_v7(),
                command: "systemctl".to_string(),
                args: vec![action.to_string(), unit.to_string()],
                reason: format!("systemctl {} {}", action, unit),
                session_id: orka_core::types::SessionId::new(),
                message_id: orka_core::types::MessageId::new(),
                requested_at: now,
                expires_at: now + chrono::Duration::seconds(self.confirmation_timeout_secs as i64),
            };
            match self.approval.request_approval(req).await? {
                ApprovalDecision::Approved => {}
                ApprovalDecision::Denied { reason } => {
                    let args = &[action, unit];
                    emit_denied(
                        &input,
                        "systemctl",
                        args,
                        &format!("service control denied: {reason}"),
                    )
                    .await;
                    return Err(Error::Skill(format!("service control denied: {}", reason)));
                }
                ApprovalDecision::Expired => {
                    let args = &[action, unit];
                    emit_denied(
                        &input,
                        "systemctl",
                        args,
                        "service control approval expired",
                    )
                    .await;
                    return Err(Error::Skill("service control approval expired".into()));
                }
            }
        }

        let start = std::time::Instant::now();
        let output = tokio::process::Command::new(self.guard.sudo_path())
            .args(["-n", "systemctl", action, unit])
            .output()
            .await
            .map_err(|e| Error::Skill(format!("systemctl failed: {}", e)))?;
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

        Ok(SkillOutput::new(serde_json::json!({
            "unit": unit,
            "action": action,
            "stdout": stdout,
            "stderr": stderr,
            "exit_code": output.status.code(),
            "success": output.status.success(),
        })))
    }
}

async fn emit_executed(
    input: &SkillInput,
    command: &str,
    args: &[&str],
    exit_code: Option<i32>,
    success: bool,
    duration_ms: u64,
) {
    if let Some(sink) = input.context.as_ref().and_then(|c| c.event_sink.as_ref()) {
        sink.emit(DomainEvent::new(
            DomainEventKind::PrivilegedCommandExecuted {
                message_id: orka_core::types::MessageId::new(),
                session_id: orka_core::types::SessionId::new(),
                command: command.to_string(),
                args: args.iter().map(|s| s.to_string()).collect(),
                approval_id: None,
                approved_by: None,
                exit_code,
                success,
                duration_ms,
            },
        ))
        .await;
    }
}

async fn emit_denied(input: &SkillInput, command: &str, args: &[&str], reason: &str) {
    if let Some(sink) = input.context.as_ref().and_then(|c| c.event_sink.as_ref()) {
        sink.emit(DomainEvent::new(DomainEventKind::PrivilegedCommandDenied {
            message_id: orka_core::types::MessageId::new(),
            session_id: orka_core::types::SessionId::new(),
            command: command.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            reason: reason.to_string(),
        }))
        .await;
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

    #[test]
    fn service_control_schema_valid() {
        use orka_core::config::{OsConfig, SudoConfig};
        let config = OsConfig {
            permission_level: "admin".into(),
            sudo: SudoConfig {
                enabled: true,
                ..SudoConfig::default()
            },
            ..OsConfig::default()
        };
        let skill = ServiceControlSkill::new(
            make_guard(),
            &config,
            Arc::new(crate::approval::AutoApproveChannel),
        );
        let schema = skill.schema();
        assert_eq!(schema.parameters["required"][0], "unit");
        assert_eq!(schema.parameters["required"][1], "action");
    }

    #[tokio::test]
    async fn service_control_requires_admin() {
        use orka_core::config::{OsConfig, SudoConfig};
        let config = OsConfig {
            permission_level: "execute".into(),
            sudo: SudoConfig {
                enabled: true,
                allowed_commands: vec!["systemctl restart".into()],
                ..SudoConfig::default()
            },
            ..OsConfig::default()
        };
        let guard = Arc::new(PermissionGuard::new(&config));
        let skill = ServiceControlSkill::new(
            guard,
            &config,
            Arc::new(crate::approval::AutoApproveChannel),
        );
        let mut args = std::collections::HashMap::new();
        args.insert("unit".into(), serde_json::json!("nginx.service"));
        args.insert("action".into(), serde_json::json!("restart"));
        assert!(skill.execute(SkillInput::new(args)).await.is_err());
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
        assert!(skill.execute(SkillInput::new(args)).await.is_err());
    }
}
