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

/// Skill that searches the package manager database for packages matching a query.
pub struct PackageSearchSkill {
    guard: Arc<PermissionGuard>,
}

impl PackageSearchSkill {
    /// Create a new `package_search` skill with the given permission guard.
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
        self.guard.check_permission(PermissionLevel::ReadOnly)?;

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

/// Skill that retrieves detailed information about a specific package.
pub struct PackageInfoSkill {
    guard: Arc<PermissionGuard>,
}

impl PackageInfoSkill {
    /// Create a new `package_info` skill with the given permission guard.
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
        self.guard.check_permission(PermissionLevel::ReadOnly)?;

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

/// Skill that lists all installed packages, optionally filtered by name.
pub struct PackageListSkill {
    guard: Arc<PermissionGuard>,
}

impl PackageListSkill {
    /// Create a new `package_list` skill with the given permission guard.
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
        self.guard.check_permission(PermissionLevel::ReadOnly)?;

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

// ── package_updates ──

fn is_update_success(pm: PackageManager, exit_code: i32, using_fallback: bool) -> bool {
    match pm {
        PackageManager::Pacman => {
            if using_fallback {
                // pacman -Qu: exit 0 = updates, exit 1 = no updates
                exit_code == 0 || exit_code == 1
            } else {
                // checkupdates: exit 0 = updates, exit 2 = no updates
                exit_code == 0 || exit_code == 2
            }
        }
        PackageManager::Apt => exit_code == 0,
        PackageManager::Dnf => exit_code == 0 || exit_code == 100,
    }
}

/// Skill that checks for available package updates.
pub struct PackageUpdatesSkill {
    guard: Arc<PermissionGuard>,
}

impl PackageUpdatesSkill {
    /// Create a new `package_updates` skill with the given permission guard.
    pub fn new(guard: Arc<PermissionGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
impl Skill for PackageUpdatesSkill {
    fn name(&self) -> &str {
        "package_updates"
    }

    fn description(&self) -> &str {
        "Check for available package updates."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "filter": { "type": "string", "description": "Filter updates by name" }
            },
            "required": []
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        self.guard.check_permission(PermissionLevel::ReadOnly)?;

        let filter = input.args.get("filter").and_then(|v| v.as_str());

        let pm = detect_package_manager()
            .ok_or_else(|| Error::Skill("no supported package manager found".into()))?;

        let (output, method, using_fallback) = match pm {
            PackageManager::Pacman => {
                if std::path::Path::new("/usr/bin/checkupdates").exists() {
                    let out = tokio::process::Command::new("checkupdates")
                        .output()
                        .await
                        .map_err(|e| Error::Skill(format!("checkupdates failed: {}", e)))?;
                    (out, "checkupdates", false)
                } else {
                    let out = tokio::process::Command::new("pacman")
                        .args(["-Qu"])
                        .output()
                        .await
                        .map_err(|e| Error::Skill(format!("pacman -Qu failed: {}", e)))?;
                    (out, "pacman -Qu", true)
                }
            }
            PackageManager::Apt => {
                let out = tokio::process::Command::new("apt")
                    .args(["list", "--upgradable"])
                    .output()
                    .await
                    .map_err(|e| Error::Skill(format!("apt list --upgradable failed: {}", e)))?;
                (out, "apt list --upgradable", false)
            }
            PackageManager::Dnf => {
                let out = tokio::process::Command::new("dnf")
                    .args(["check-update"])
                    .output()
                    .await
                    .map_err(|e| Error::Skill(format!("dnf check-update failed: {}", e)))?;
                (out, "dnf check-update", false)
            }
        };

        let exit_code = output.status.code().unwrap_or(-1);
        if !is_update_success(pm, exit_code, using_fallback) {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(Error::Skill(format!(
                "{} failed with exit code {}: {}",
                method, exit_code, stderr
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let f_lower = filter.map(|f| f.to_lowercase());
        let lines: Vec<&str> = stdout
            .lines()
            .filter(|l| {
                // Skip apt's "Listing..." header line
                !l.starts_with("Listing...")
            })
            .filter(|l| !l.is_empty())
            .filter(|l| {
                if let Some(ref f) = f_lower {
                    l.to_lowercase().contains(f.as_str())
                } else {
                    true
                }
            })
            .collect();

        let mut result = serde_json::json!({
            "updates": lines,
            "count": lines.len(),
            "package_manager": format!("{:?}", pm).to_lowercase(),
            "method": method,
        });

        if using_fallback {
            result["stale_cache_warning"] = serde_json::json!(true);
        }

        Ok(SkillOutput::new(result))
    }
}

// ── package_install ──

/// Skill that installs a package via sudo, with optional approval gating.
pub struct PackageInstallSkill {
    guard: Arc<PermissionGuard>,
    require_confirmation: bool,
    confirmation_timeout_secs: u64,
    approval: Arc<dyn ApprovalChannel>,
}

impl PackageInstallSkill {
    /// Create a new `package_install` skill from config, a permission guard, and an approval channel.
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

        if !output.status.success() {
            return Err(Error::Skill(format!(
                "package install failed (exit {}): {}",
                output.status.code().unwrap_or(-1),
                stderr.trim()
            )));
        }

        Ok(SkillOutput::new(serde_json::json!({
            "stdout": stdout,
            "stderr": stderr,
            "package_manager": format!("{:?}", pm).to_lowercase(),
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

    #[test]
    fn package_updates_schema_valid() {
        let skill = PackageUpdatesSkill::new(make_guard());
        let schema = skill.schema();
        let required = schema.parameters.get("required");
        // required should be empty array
        assert!(
            required.map_or(true, |r| r.as_array().map_or(true, |a| a.is_empty())),
            "package_updates should have no required params"
        );
    }

    #[tokio::test]
    async fn package_search_allowed_at_read_only() {
        use orka_core::config::OsConfig;
        let guard = Arc::new(PermissionGuard::new(&OsConfig {
            permission_level: "read-only".into(),
            ..OsConfig::default()
        }));
        let skill = PackageSearchSkill::new(guard);
        let mut args = std::collections::HashMap::new();
        args.insert("query".into(), serde_json::json!("test"));
        // Permission check passes; any error here is from the missing package manager binary.
        let result = skill.execute(SkillInput::new(args)).await;
        assert!(
            result.is_ok()
                || result
                    .unwrap_err()
                    .to_string()
                    .contains("no supported package manager"),
            "should not fail with a permission error at read-only level"
        );
    }

    #[tokio::test]
    async fn package_updates_allowed_at_read_only() {
        use orka_core::config::OsConfig;
        let guard = Arc::new(PermissionGuard::new(&OsConfig {
            permission_level: "read-only".into(),
            ..OsConfig::default()
        }));
        let skill = PackageUpdatesSkill::new(guard);
        let result = skill.execute(SkillInput::new(Default::default())).await;
        assert!(
            result.is_ok()
                || result
                    .unwrap_err()
                    .to_string()
                    .contains("no supported package manager"),
            "should not fail with a permission error at read-only level"
        );
    }

    #[test]
    fn is_update_success_pacman_checkupdates() {
        // exit 0 = updates available
        assert!(is_update_success(PackageManager::Pacman, 0, false));
        // exit 2 = no updates
        assert!(is_update_success(PackageManager::Pacman, 2, false));
        // exit 1 = error
        assert!(!is_update_success(PackageManager::Pacman, 1, false));
    }

    #[test]
    fn is_update_success_pacman_fallback() {
        // exit 0 = updates available
        assert!(is_update_success(PackageManager::Pacman, 0, true));
        // exit 1 = no updates
        assert!(is_update_success(PackageManager::Pacman, 1, true));
        // other = error
        assert!(!is_update_success(PackageManager::Pacman, 2, true));
        assert!(!is_update_success(PackageManager::Pacman, -1, true));
    }

    #[test]
    fn is_update_success_apt() {
        assert!(is_update_success(PackageManager::Apt, 0, false));
        assert!(!is_update_success(PackageManager::Apt, 1, false));
        assert!(!is_update_success(PackageManager::Apt, 100, false));
    }

    #[test]
    fn is_update_success_dnf() {
        // exit 0 = no updates
        assert!(is_update_success(PackageManager::Dnf, 0, false));
        // exit 100 = updates available
        assert!(is_update_success(PackageManager::Dnf, 100, false));
        // exit 1 = error
        assert!(!is_update_success(PackageManager::Dnf, 1, false));
    }
}
