use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use orka_core::config::OsConfig;
use orka_core::traits::Skill;
use orka_core::{Error, ErrorCategory, Result, SkillInput, SkillOutput, SkillSchema};
use uuid::Uuid;

use crate::approval::{ApprovalChannel, ApprovalDecision, ApprovalRequest};
use crate::config::PermissionLevel;
use crate::events::{emit_denied, emit_executed};
use crate::guard::PermissionGuard;
use crate::probe::PackageUpdateMethod;

#[derive(Debug, Clone, Copy)]
enum PackageManager {
    Pacman,
    Apt,
    Dnf,
}

fn spawn_error(context: &str, e: std::io::Error) -> Error {
    let category = match e.kind() {
        std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied => {
            ErrorCategory::Environmental
        }
        _ => ErrorCategory::Unknown,
    };
    Error::SkillCategorized {
        message: format!("{}: {}", context, e),
        category,
    }
}

fn no_package_manager_error() -> Error {
    Error::SkillCategorized {
        message: "no supported package manager found".into(),
        category: ErrorCategory::Environmental,
    }
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

    fn category(&self) -> &str {
        "package"
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
            .ok_or_else(|| Error::SkillCategorized {
                message: "missing 'query' argument".into(),
                category: ErrorCategory::Input,
            })?;

        let pm = detect_package_manager().ok_or_else(no_package_manager_error)?;

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
        .map_err(|e| spawn_error("package search failed", e))?;

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

    fn category(&self) -> &str {
        "package"
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
            .ok_or_else(|| Error::SkillCategorized {
                message: "missing 'name' argument".into(),
                category: ErrorCategory::Input,
            })?;

        let pm = detect_package_manager().ok_or_else(no_package_manager_error)?;

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
        .map_err(|e| spawn_error("package info failed", e))?;

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

    fn category(&self) -> &str {
        "package"
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

        let pm = detect_package_manager().ok_or_else(no_package_manager_error)?;

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
        .map_err(|e| spawn_error("package list failed", e))?;

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
    /// Pre-determined update method from startup probe. If `None`, runtime detection is used.
    method: Option<PackageUpdateMethod>,
}

impl PackageUpdatesSkill {
    /// Create a new `package_updates` skill.
    ///
    /// Pass `method` from [`crate::probe::EnvironmentCapabilities`] to use a pre-validated
    /// method, avoiding runtime crashes (e.g. `checkupdates` under `NoNewPrivileges`).
    pub fn new(guard: Arc<PermissionGuard>, method: Option<PackageUpdateMethod>) -> Self {
        Self { guard, method }
    }
}

#[async_trait]
impl Skill for PackageUpdatesSkill {
    fn name(&self) -> &str {
        "package_updates"
    }

    fn category(&self) -> &str {
        "package"
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

        // Use the pre-probed method if available, otherwise fall back to runtime detection
        let effective_method = self.method.or_else(|| {
            let pm = detect_package_manager()?;
            Some(match pm {
                PackageManager::Pacman => {
                    if std::path::Path::new("/usr/bin/checkupdates").exists() {
                        crate::probe::PackageUpdateMethod::CheckUpdates
                    } else {
                        crate::probe::PackageUpdateMethod::PacmanQu
                    }
                }
                PackageManager::Apt => crate::probe::PackageUpdateMethod::AptListUpgradable,
                PackageManager::Dnf => crate::probe::PackageUpdateMethod::DnfCheckUpdate,
            })
        });

        let Some(effective_method) = effective_method else {
            return Err(no_package_manager_error());
        };

        let (output, method_str, using_fallback) = match effective_method {
            crate::probe::PackageUpdateMethod::CheckUpdates => {
                let out = tokio::process::Command::new("checkupdates")
                    .output()
                    .await
                    .map_err(|e| spawn_error("checkupdates failed", e))?;
                (out, "checkupdates", false)
            }
            crate::probe::PackageUpdateMethod::PacmanQu => {
                let out = tokio::process::Command::new("pacman")
                    .args(["-Qu"])
                    .output()
                    .await
                    .map_err(|e| spawn_error("pacman -Qu failed", e))?;
                (out, "pacman -Qu", true)
            }
            crate::probe::PackageUpdateMethod::AptListUpgradable => {
                let out = tokio::process::Command::new("apt")
                    .args(["list", "--upgradable"])
                    .output()
                    .await
                    .map_err(|e| spawn_error("apt list --upgradable failed", e))?;
                (out, "apt list --upgradable", false)
            }
            crate::probe::PackageUpdateMethod::DnfCheckUpdate => {
                let out = tokio::process::Command::new("dnf")
                    .args(["check-update"])
                    .output()
                    .await
                    .map_err(|e| spawn_error("dnf check-update failed", e))?;
                (out, "dnf check-update", false)
            }
        };

        // Map method_str back to the PackageManager for exit code interpretation
        let pm = match effective_method {
            crate::probe::PackageUpdateMethod::CheckUpdates
            | crate::probe::PackageUpdateMethod::PacmanQu => PackageManager::Pacman,
            crate::probe::PackageUpdateMethod::AptListUpgradable => PackageManager::Apt,
            crate::probe::PackageUpdateMethod::DnfCheckUpdate => PackageManager::Dnf,
        };
        let method = method_str;

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

    fn category(&self) -> &str {
        "package"
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
            .ok_or_else(|| Error::SkillCategorized {
                message: "missing 'name' argument".into(),
                category: ErrorCategory::Input,
            })?;

        let pm = detect_package_manager().ok_or_else(no_package_manager_error)?;

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
                    return Err(Error::SkillCategorized {
                        message: format!("package install denied: {}", reason),
                        category: ErrorCategory::Input,
                    });
                }
                ApprovalDecision::Expired => {
                    emit_denied(
                        &input,
                        cmd,
                        &install_args,
                        "package install approval expired",
                    )
                    .await;
                    return Err(Error::SkillCategorized {
                        message: "package install approval expired".into(),
                        category: ErrorCategory::Timeout,
                    });
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
            .map_err(|e| spawn_error("package install failed", e))?;
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
            return Err(Error::SkillCategorized {
                message: format!(
                    "package install failed (exit {}): {}",
                    output.status.code().unwrap_or(-1),
                    stderr.trim()
                ),
                category: ErrorCategory::Environmental,
            });
        }

        Ok(SkillOutput::new(serde_json::json!({
            "stdout": stdout,
            "stderr": stderr,
            "package_manager": format!("{:?}", pm).to_lowercase(),
        })))
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
        let skill = PackageUpdatesSkill::new(make_guard(), None);
        let schema = skill.schema();
        let required = schema.parameters.get("required");
        // required should be empty array
        assert!(
            required.is_none_or(|r| r.as_array().is_none_or(|a| a.is_empty())),
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
        let skill = PackageUpdatesSkill::new(guard, None);
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
