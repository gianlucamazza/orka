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

#[derive(Debug, Clone, Copy)]
enum PackageManager {
    Pacman,
    Apt,
    Dnf,
}

fn detect_package_manager() -> Option<PackageManager> {
    if std::path::Path::new("/usr/bin/pacman").exists() {
        Some(PackageManager::Pacman)
    } else if std::path::Path::new("/usr/bin/apt").exists() {
        Some(PackageManager::Apt)
    } else if std::path::Path::new("/usr/bin/dnf").exists() {
        Some(PackageManager::Dnf)
    } else {
        None
    }
}

// ── package_search ──

pub struct PackageSearchSkill {
    guard: Arc<PermissionGuard>,
}

impl PackageSearchSkill {
    pub fn new(guard: Arc<PermissionGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
impl Skill for PackageSearchSkill {
    fn name(&self) -> &str {
        "package_search"
    }

    fn description(&self) -> &str {
        "Search for packages in the system package manager."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Package name to search for" }
            },
            "required": ["query"]
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        self.guard.check_permission(PermissionLevel::Admin)?;

        let query = input
            .args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Skill("missing 'query' argument".into()))?;

        let pm = detect_package_manager()
            .ok_or_else(|| Error::Skill("no supported package manager found".into()))?;

        let output = match pm {
            PackageManager::Pacman => {
                tokio::process::Command::new("pacman")
                    .args(["-Ss", query])
                    .output()
                    .await
            }
            PackageManager::Apt => {
                tokio::process::Command::new("apt")
                    .args(["search", query])
                    .output()
                    .await
            }
            PackageManager::Dnf => {
                tokio::process::Command::new("dnf")
                    .args(["search", query])
                    .output()
                    .await
            }
        }
        .map_err(|e| Error::Skill(format!("package search failed: {}", e)))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();

        Ok(SkillOutput::new(serde_json::json!({
            "results": stdout,
            "package_manager": format!("{:?}", pm).to_lowercase(),
            "success": output.status.success(),
        })))
    }
}

// ── package_info ──

pub struct PackageInfoSkill {
    guard: Arc<PermissionGuard>,
}

impl PackageInfoSkill {
    pub fn new(guard: Arc<PermissionGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
impl Skill for PackageInfoSkill {
    fn name(&self) -> &str {
        "package_info"
    }

    fn description(&self) -> &str {
        "Get detailed information about an installed or available package."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Package name" }
            },
            "required": ["name"]
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        self.guard.check_permission(PermissionLevel::Admin)?;

        let name = input
            .args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Skill("missing 'name' argument".into()))?;

        let pm = detect_package_manager()
            .ok_or_else(|| Error::Skill("no supported package manager found".into()))?;

        let output = match pm {
            PackageManager::Pacman => {
                tokio::process::Command::new("pacman")
                    .args(["-Si", name])
                    .output()
                    .await
            }
            PackageManager::Apt => {
                tokio::process::Command::new("apt")
                    .args(["show", name])
                    .output()
                    .await
            }
            PackageManager::Dnf => {
                tokio::process::Command::new("dnf")
                    .args(["info", name])
                    .output()
                    .await
            }
        }
        .map_err(|e| Error::Skill(format!("package info failed: {}", e)))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();

        Ok(SkillOutput::new(serde_json::json!({
            "info": stdout,
            "package_manager": format!("{:?}", pm).to_lowercase(),
            "success": output.status.success(),
        })))
    }
}

// ── package_list ──

pub struct PackageListSkill {
    guard: Arc<PermissionGuard>,
}

impl PackageListSkill {
    pub fn new(guard: Arc<PermissionGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
impl Skill for PackageListSkill {
    fn name(&self) -> &str {
        "package_list"
    }

    fn description(&self) -> &str {
        "List installed packages, optionally filtered."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "filter": { "type": "string", "description": "Filter by name (grep)" }
            },
            "required": []
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        self.guard.check_permission(PermissionLevel::Admin)?;

        let filter = input.args.get("filter").and_then(|v| v.as_str());

        let pm = detect_package_manager()
            .ok_or_else(|| Error::Skill("no supported package manager found".into()))?;

        let output = match pm {
            PackageManager::Pacman => {
                tokio::process::Command::new("pacman")
                    .args(["-Q"])
                    .output()
                    .await
            }
            PackageManager::Apt => {
                tokio::process::Command::new("apt")
                    .args(["list", "--installed"])
                    .output()
                    .await
            }
            PackageManager::Dnf => {
                tokio::process::Command::new("dnf")
                    .args(["list", "installed"])
                    .output()
                    .await
            }
        }
        .map_err(|e| Error::Skill(format!("package list failed: {}", e)))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = if let Some(f) = filter {
            let f_lower = f.to_lowercase();
            stdout
                .lines()
                .filter(|l| l.to_lowercase().contains(&f_lower))
                .collect()
        } else {
            stdout.lines().collect()
        };

        Ok(SkillOutput::new(serde_json::json!({
            "packages": lines,
            "count": lines.len(),
            "package_manager": format!("{:?}", pm).to_lowercase(),
        })))
    }
}

// ── package_install ──

pub struct PackageInstallSkill {
    guard: Arc<PermissionGuard>,
    require_confirmation: bool,
    confirmation_timeout_secs: u64,
    approval: Arc<dyn ApprovalChannel>,
}

impl PackageInstallSkill {
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
impl Skill for PackageInstallSkill {
    fn name(&self) -> &str {
        "package_install"
    }

    fn description(&self) -> &str {
        "Install a package using the system package manager (requires sudo)."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Package name to install" }
            },
            "required": ["name"]
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        self.guard.check_permission(PermissionLevel::Admin)?;

        let name = input
            .args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Skill("missing 'name' argument".into()))?;

        let pm = detect_package_manager()
            .ok_or_else(|| Error::Skill("no supported package manager found".into()))?;

        let (cmd, install_args) = match pm {
            PackageManager::Pacman => ("pacman", vec!["-S", "--noconfirm", name]),
            PackageManager::Apt => ("apt", vec!["install", "-y", name]),
            PackageManager::Dnf => ("dnf", vec!["install", "-y", name]),
        };

        // Check sudo allowlist
        self.guard.check_sudo_command(cmd, &install_args)?;

        // Request approval if needed
        if self.require_confirmation {
            let now = Utc::now();
            let req = ApprovalRequest {
                id: Uuid::now_v7(),
                command: cmd.to_string(),
                args: install_args.iter().map(|s| s.to_string()).collect(),
                reason: format!("install package: {}", name),
                session_id: orka_core::types::SessionId::new(),
                message_id: orka_core::types::MessageId::new(),
                requested_at: now,
                expires_at: now + chrono::Duration::seconds(self.confirmation_timeout_secs as i64),
            };
            match self.approval.request_approval(req).await? {
                ApprovalDecision::Approved => {}
                ApprovalDecision::Denied { reason } => {
                    emit_denied(
                        &input,
                        cmd,
                        &install_args,
                        &format!("package install denied: {reason}"),
                    )
                    .await;
                    return Err(Error::Skill(format!("package install denied: {}", reason)));
                }
                ApprovalDecision::Expired => {
                    emit_denied(
                        &input,
                        cmd,
                        &install_args,
                        "package install approval expired",
                    )
                    .await;
                    return Err(Error::Skill("package install approval expired".into()));
                }
            }
        }

        let start = std::time::Instant::now();
        let output = tokio::process::Command::new(self.guard.sudo_path())
            .arg("-n")
            .arg(cmd)
            .args(&install_args)
            .output()
            .await
            .map_err(|e| Error::Skill(format!("package install failed: {}", e)))?;
        let duration_ms = start.elapsed().as_millis() as u64;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        emit_executed(
            &input,
            cmd,
            &install_args,
            output.status.code(),
            output.status.success(),
            duration_ms,
        )
        .await;

        Ok(SkillOutput::new(serde_json::json!({
            "stdout": stdout,
            "stderr": stderr,
            "exit_code": output.status.code(),
            "package_manager": format!("{:?}", pm).to_lowercase(),
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
    fn package_search_schema_valid() {
        let skill = PackageSearchSkill::new(make_guard());
        let schema = skill.schema();
        assert_eq!(schema.parameters["required"][0], "query");
    }

    #[test]
    fn package_info_schema_valid() {
        let skill = PackageInfoSkill::new(make_guard());
        let schema = skill.schema();
        assert_eq!(schema.parameters["required"][0], "name");
    }

    #[tokio::test]
    async fn package_search_requires_admin() {
        use orka_core::config::OsConfig;
        let guard = Arc::new(PermissionGuard::new(&OsConfig {
            permission_level: "execute".into(),
            ..OsConfig::default()
        }));
        let skill = PackageSearchSkill::new(guard);
        let mut args = std::collections::HashMap::new();
        args.insert("query".into(), serde_json::json!("test"));
        assert!(skill.execute(SkillInput::new(args)).await.is_err());
    }
}
