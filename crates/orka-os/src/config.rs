//! OS integration configuration types owned by `orka-os`.

use std::fmt;

use orka_core::Result;
use serde::Deserialize;

/// Permission levels for OS skills, ordered from least to most permissive.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PermissionLevel {
    /// Read-only access: file reads, process listing, environment inspection,
    /// package queries, and systemd status/journal reads.
    #[default]
    ReadOnly,
    /// Adds low-risk desktop side effects: clipboard read/write, notifications.
    Interact,
    /// Adds filesystem mutations (`fs_write`).
    Write,
    /// Adds shell execution, process signalling, file watching, and desktop
    /// open/screenshot.
    Execute,
    /// Adds sudo-only operations: package install, service control.
    Admin,
}

impl PermissionLevel {
    /// Parse a permission level from the TOML config string representation.
    ///
    /// Returns `Err` for unrecognised values. Parsing is case-insensitive.
    pub fn from_str_config(s: &str) -> std::result::Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "read-only" | "readonly" => Ok(Self::ReadOnly),
            "interact" => Ok(Self::Interact),
            "write" => Ok(Self::Write),
            "execute" => Ok(Self::Execute),
            "admin" => Ok(Self::Admin),
            other => Err(format!(
                "unknown permission level '{other}': must be one of read-only, interact, write, execute, admin",
            )),
        }
    }
}

impl fmt::Display for PermissionLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReadOnly => write!(f, "read-only"),
            Self::Interact => write!(f, "interact"),
            Self::Write => write!(f, "write"),
            Self::Execute => write!(f, "execute"),
            Self::Admin => write!(f, "admin"),
        }
    }
}

/// Linux OS integration configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct OsConfig {
    /// Enable OS integration.
    #[serde(default = "default_os_enabled")]
    pub enabled: bool,
    /// Permission level for OS operations.
    #[serde(default)]
    pub permission_level: PermissionLevel,
    /// Allowed paths for filesystem access.
    #[serde(default)]
    pub allowed_paths: Vec<String>,
    /// Denied paths (takes precedence).
    #[serde(default)]
    pub denied_paths: Vec<String>,
    /// Allowed shell commands.
    #[serde(default)]
    pub allowed_shell_commands: Vec<String>,
    /// Sudo configuration.
    #[serde(default)]
    pub sudo: SudoConfig,
}

impl Default for OsConfig {
    fn default() -> Self {
        Self {
            enabled: default_os_enabled(),
            permission_level: PermissionLevel::default(),
            allowed_paths: Vec::new(),
            denied_paths: Vec::new(),
            allowed_shell_commands: Vec::new(),
            sudo: SudoConfig::default(),
        }
    }
}

impl OsConfig {
    /// Validate OS-related configuration.
    pub fn validate(&self) -> Result<()> {
        Ok(())
    }
}

/// Sudo configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct SudoConfig {
    /// Allow sudo elevation.
    #[serde(default = "default_sudo_allowed")]
    pub allowed: bool,
    /// Allowed sudo commands (empty = all allowed if sudo enabled).
    #[serde(default)]
    pub allowed_commands: Vec<String>,
    /// Require password for sudo.
    #[serde(default = "default_password_required")]
    pub password_required: bool,
}

impl Default for SudoConfig {
    fn default() -> Self {
        Self {
            allowed: default_sudo_allowed(),
            allowed_commands: Vec::new(),
            password_required: default_password_required(),
        }
    }
}

// --- Private defaults ---

const fn default_os_enabled() -> bool {
    false
}

const fn default_sudo_allowed() -> bool {
    false
}

const fn default_password_required() -> bool {
    true
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::field_reassign_with_default
)]
mod tests {
    use super::*;

    #[test]
    fn permission_ordering() {
        assert!(PermissionLevel::ReadOnly < PermissionLevel::Interact);
        assert!(PermissionLevel::Interact < PermissionLevel::Write);
        assert!(PermissionLevel::Write < PermissionLevel::Execute);
        assert!(PermissionLevel::Execute < PermissionLevel::Admin);
    }

    #[test]
    fn parse_from_string() {
        assert_eq!(
            PermissionLevel::from_str_config("read-only").unwrap(),
            PermissionLevel::ReadOnly
        );
        assert_eq!(
            PermissionLevel::from_str_config("readonly").unwrap(),
            PermissionLevel::ReadOnly
        );
        assert_eq!(
            PermissionLevel::from_str_config("READ-ONLY").unwrap(),
            PermissionLevel::ReadOnly
        );
        assert_eq!(
            PermissionLevel::from_str_config("interact").unwrap(),
            PermissionLevel::Interact
        );
        assert_eq!(
            PermissionLevel::from_str_config("INTERACT").unwrap(),
            PermissionLevel::Interact
        );
        assert_eq!(
            PermissionLevel::from_str_config("write").unwrap(),
            PermissionLevel::Write
        );
        assert_eq!(
            PermissionLevel::from_str_config("execute").unwrap(),
            PermissionLevel::Execute
        );
        assert_eq!(
            PermissionLevel::from_str_config("admin").unwrap(),
            PermissionLevel::Admin
        );
        assert!(PermissionLevel::from_str_config("unknown").is_err());
    }
}
