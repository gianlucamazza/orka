use std::fmt;

/// Permission levels for OS skills, ordered from least to most permissive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PermissionLevel {
    /// Read-only access: file reads, process listing, environment inspection.
    ReadOnly,
    /// Adds file writes and clipboard access.
    Write,
    /// Adds shell execution, process signalling, and file watching.
    Execute,
    /// Adds package management and systemd service control.
    Admin,
}

impl PermissionLevel {
    /// Parse a permission level from the TOML config string representation.
    pub fn from_str_config(s: &str) -> Self {
        match s {
            "write" => Self::Write,
            "execute" => Self::Execute,
            "admin" => Self::Admin,
            _ => Self::ReadOnly,
        }
    }
}

impl fmt::Display for PermissionLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReadOnly => write!(f, "read-only"),
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
        assert!(PermissionLevel::ReadOnly < PermissionLevel::Write);
        assert!(PermissionLevel::Write < PermissionLevel::Execute);
        assert!(PermissionLevel::Execute < PermissionLevel::Admin);
    }

    #[test]
    fn parse_from_string() {
        assert_eq!(
            PermissionLevel::from_str_config("read-only"),
            PermissionLevel::ReadOnly
        );
        assert_eq!(
            PermissionLevel::from_str_config("write"),
            PermissionLevel::Write
        );
        assert_eq!(
            PermissionLevel::from_str_config("execute"),
            PermissionLevel::Execute
        );
        assert_eq!(
            PermissionLevel::from_str_config("admin"),
            PermissionLevel::Admin
        );
        assert_eq!(
            PermissionLevel::from_str_config("unknown"),
            PermissionLevel::ReadOnly
        );
    }
}
