use std::fmt;

/// Permission levels for OS skills, ordered from least to most permissive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PermissionLevel {
    /// Read-only access: file reads, process listing, environment inspection,
    /// package queries, and systemd status/journal reads.
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
    pub fn from_str_config(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "read-only" | "readonly" => Ok(Self::ReadOnly),
            "interact" => Ok(Self::Interact),
            "write" => Ok(Self::Write),
            "execute" => Ok(Self::Execute),
            "admin" => Ok(Self::Admin),
            other => Err(format!(
                "unknown permission level '{}': must be one of read-only, interact, write, execute, admin",
                other
            )),
        }
    }
}

impl From<orka_core::config::primitives::OsPermissionLevel> for PermissionLevel {
    fn from(level: orka_core::config::primitives::OsPermissionLevel) -> Self {
        match level {
            orka_core::config::primitives::OsPermissionLevel::ReadOnly => Self::ReadOnly,
            orka_core::config::primitives::OsPermissionLevel::Interact => Self::Interact,
            orka_core::config::primitives::OsPermissionLevel::Write => Self::Write,
            orka_core::config::primitives::OsPermissionLevel::Execute => Self::Execute,
            orka_core::config::primitives::OsPermissionLevel::Admin => Self::Admin,
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

#[cfg(test)]
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
