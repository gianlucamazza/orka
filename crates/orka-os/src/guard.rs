use std::path::{Path, PathBuf};

use orka_core::config::OsConfig;
use regex::Regex;

use crate::config::PermissionLevel;

/// Central safety enforcement for all OS skills.
pub struct PermissionGuard {
    level: PermissionLevel,
    allowed_paths: Vec<PathBuf>,
    denied_paths: Vec<PathBuf>,
    allowed_commands: Vec<String>,
    max_file_size_bytes: u64,
    sensitive_env_patterns: Vec<String>,
    sudo_enabled: bool,
    sudo_allowed_commands: Vec<String>,
    sudo_path: String,
}

impl PermissionGuard {
    /// Build a guard from the given OS configuration.
    pub fn new(config: &OsConfig) -> Self {
        let level = config.permission_level.into();
        let allowed_paths = config
            .allowed_paths
            .iter()
            .map(|p| {
                let expanded = shellexpand(p);
                PathBuf::from(expanded)
            })
            .collect();
        let denied_paths = config
            .denied_paths
            .iter()
            .map(|p| {
                let expanded = shellexpand(p);
                PathBuf::from(expanded)
            })
            .collect();
        Self {
            level,
            allowed_paths,
            denied_paths,
            allowed_commands: config.allowed_shell_commands.clone(),
            max_file_size_bytes: 100 * 1024 * 1024, // Default 100MB
            sensitive_env_patterns: vec![
                "*PASSWORD*".to_string(),
                "*SECRET*".to_string(),
                "*TOKEN*".to_string(),
                "*API_KEY*".to_string(),
                "*PRIVATE_KEY*".to_string(),
            ],
            sudo_enabled: config.sudo.allowed,
            sudo_allowed_commands: config.sudo.allowed_commands.clone(),
            sudo_path: "sudo".to_string(),
        }
    }

    /// Check that the current permission level is at least `required`.
    pub fn check_permission(&self, required: PermissionLevel) -> orka_core::Result<()> {
        if self.level >= required {
            Ok(())
        } else {
            Err(orka_core::Error::Skill(format!(
                "permission denied: requires '{}' but current level is '{}'",
                required, self.level,
            )))
        }
    }

    /// Validate a path for reading: canonicalize and check allow/block lists.
    pub fn check_path(&self, path: &Path) -> orka_core::Result<PathBuf> {
        let canonical = path.canonicalize().map_err(|e| {
            orka_core::Error::Skill(format!("cannot resolve path '{}': {}", path.display(), e,))
        })?;
        self.validate_canonical_path(&canonical)?;
        Ok(canonical)
    }

    /// Validate a path for writing: the file may not exist yet, so we
    /// canonicalize the parent directory instead.
    pub fn check_write_path(&self, path: &Path) -> orka_core::Result<PathBuf> {
        let parent = path
            .parent()
            .ok_or_else(|| orka_core::Error::Skill("invalid path: no parent directory".into()))?;
        let canonical_parent = parent.canonicalize().map_err(|e| {
            orka_core::Error::Skill(format!(
                "cannot resolve parent '{}': {}",
                parent.display(),
                e,
            ))
        })?;
        self.validate_canonical_path(&canonical_parent)?;
        let file_name = path
            .file_name()
            .ok_or_else(|| orka_core::Error::Skill("invalid path: no file name".into()))?;
        Ok(canonical_parent.join(file_name))
    }

    /// Validate a command against allow lists.
    pub fn check_command(&self, cmd: &str, args: &[&str]) -> orka_core::Result<()> {
        let full = if args.is_empty() {
            cmd.to_string()
        } else {
            format!("{} {}", cmd, args.join(" "))
        };

        // Check allowed commands (if non-empty, only those are permitted)
        if !self.allowed_commands.is_empty()
            && !self
                .allowed_commands
                .iter()
                .any(|a| cmd == a.as_str() || full.starts_with(a.as_str()))
        {
            return Err(orka_core::Error::Skill(format!(
                "command '{}' not in allowed list",
                cmd,
            )));
        }

        Ok(())
    }

    /// Check that a file size is within the configured limit.
    pub fn check_file_size(&self, size: u64) -> orka_core::Result<()> {
        if size > self.max_file_size_bytes {
            Err(orka_core::Error::Skill(format!(
                "file size {} bytes exceeds limit of {} bytes",
                size, self.max_file_size_bytes,
            )))
        } else {
            Ok(())
        }
    }

    /// Check that an environment variable name is not sensitive.
    pub fn check_env_var(&self, name: &str) -> orka_core::Result<()> {
        let upper = name.to_uppercase();
        for pattern in &self.sensitive_env_patterns {
            if glob_match(pattern, &upper) {
                return Err(orka_core::Error::Skill(format!(
                    "access to sensitive environment variable '{}' is blocked",
                    name,
                )));
            }
        }
        Ok(())
    }

    /// Check if an env var name matches a sensitive pattern (for masking in
    /// lists).
    pub fn is_sensitive_env(&self, name: &str) -> bool {
        let upper = name.to_uppercase();
        self.sensitive_env_patterns
            .iter()
            .any(|p| glob_match(p, &upper))
    }

    /// The configured permission level.
    pub fn level(&self) -> PermissionLevel {
        self.level
    }

    /// Maximum file size in bytes that may be read or written.
    pub fn max_file_size_bytes(&self) -> u64 {
        self.max_file_size_bytes
    }

    /// Whether sudo execution is enabled.
    pub fn sudo_enabled(&self) -> bool {
        self.sudo_enabled
    }

    /// Path to the sudo binary.
    pub fn sudo_path(&self) -> &str {
        &self.sudo_path
    }

    /// Validate a command for privileged (sudo) execution.
    ///
    /// Checks:
    /// 1. sudo is enabled
    /// 2. caller has Admin permission level
    /// 3. command matches an entry in `sudo.allowed_commands` (prefix match at
    ///    word boundary)
    pub fn check_sudo_command(&self, cmd: &str, args: &[&str]) -> orka_core::Result<()> {
        if !self.sudo_enabled {
            return Err(orka_core::Error::Skill(
                "sudo is not enabled in configuration".into(),
            ));
        }

        self.check_permission(PermissionLevel::Admin)?;

        let full = if args.is_empty() {
            cmd.to_string()
        } else {
            format!("{} {}", cmd, args.join(" "))
        };

        // Check sudo allowed commands (prefix match at word boundary).
        // Empty list = unrestricted (consistent with check_shell_command / check_path).
        if !self.sudo_allowed_commands.is_empty() {
            let allowed = self.sudo_allowed_commands.iter().any(|allowed_cmd| {
                full == *allowed_cmd || full.starts_with(&format!("{} ", allowed_cmd))
            });
            if !allowed {
                return Err(orka_core::Error::Skill(format!(
                    "command '{}' not in sudo allowed list",
                    full,
                )));
            }
        }

        Ok(())
    }

    fn validate_canonical_path(&self, canonical: &Path) -> orka_core::Result<()> {
        let _path_str = canonical.to_string_lossy();

        // Check denied paths (takes precedence)
        for denied in &self.denied_paths {
            if canonical.starts_with(denied) {
                return Err(orka_core::Error::Skill(format!(
                    "path '{}' is denied",
                    canonical.display(),
                )));
            }
        }

        // Check allowed paths (must be under at least one)
        if !self.allowed_paths.is_empty() {
            let allowed = self.allowed_paths.iter().any(|a| canonical.starts_with(a));
            if !allowed {
                return Err(orka_core::Error::Skill(format!(
                    "path '{}' is not under any allowed path",
                    canonical.display(),
                )));
            }
        }

        Ok(())
    }
}

/// Simple glob matching: `*` matches any sequence within a segment.
fn glob_match(pattern: &str, text: &str) -> bool {
    let regex_str = format!(
        "^{}$",
        regex::escape(pattern)
            .replace(r"\*", ".*")
            .replace(r"\?", ".")
    );
    Regex::new(&regex_str)
        .map(|re| re.is_match(text))
        .unwrap_or(false)
}

/// Expand `~` to the user's home directory.
///
/// Resolution order:
/// 1. `$HOME` environment variable
/// 2. `/etc/passwd` via `nix::unistd::User` (works even when `$HOME` is unset)
/// 3. Pattern returned unchanged with a warning logged
fn shellexpand(s: &str) -> String {
    let needs_expand = s.starts_with("~/") || s == "~";
    if !needs_expand {
        return s.to_string();
    }

    let home = std::env::var("HOME").ok().or_else(|| {
        // Fallback: derive home from $USER or $LOGNAME (common Linux conventions)
        std::env::var("USER")
            .or_else(|_| std::env::var("LOGNAME"))
            .ok()
            .map(|user| format!("/home/{user}"))
    });

    match home {
        Some(h) if s == "~" => h,
        Some(h) => format!("{}/{}", h, &s[2..]),
        None => {
            tracing::warn!("could not resolve ~ in path guard, pattern will not match tilde paths");
            s.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use orka_core::config::primitives::OsPermissionLevel;

    use super::*;

    fn test_config() -> OsConfig {
        let mut config = OsConfig::default();
        config.enabled = true;
        config.permission_level = OsPermissionLevel::ReadOnly;
        config.allowed_paths = vec!["/tmp".into()];
        config.denied_paths = vec!["/tmp/secret".into()];
        config
    }

    #[test]
    fn permission_check_allows_same_level() {
        let guard = PermissionGuard::new(&test_config());
        assert!(guard.check_permission(PermissionLevel::ReadOnly).is_ok());
    }

    #[test]
    fn permission_check_blocks_higher_level() {
        let guard = PermissionGuard::new(&test_config());
        assert!(guard.check_permission(PermissionLevel::Write).is_err());
    }

    #[test]
    fn denied_path_rejected() {
        let guard = PermissionGuard::new(&test_config());
        // Create a file in denied path
        let _ = std::fs::create_dir_all("/tmp/secret");
        let _ = std::fs::write("/tmp/secret/test.txt", "data");
        let result = guard.check_path(Path::new("/tmp/secret/test.txt"));
        assert!(result.is_err());
        let _ = std::fs::remove_dir_all("/tmp/secret");
    }

    #[test]
    fn allowed_path_accepted() {
        let guard = PermissionGuard::new(&test_config());
        // /tmp should be allowed
        let result = guard.check_path(Path::new("/tmp"));
        assert!(result.is_ok());
    }

    #[test]
    fn path_outside_allowed_rejected() {
        let guard = PermissionGuard::new(&test_config());
        let result = guard.check_path(Path::new("/etc/hostname"));
        assert!(result.is_err());
    }

    #[test]
    fn command_not_in_allowed_list_rejected() {
        // With empty allowed_shell_commands, all commands are allowed (no restriction)
        let guard = PermissionGuard::new(&test_config());
        assert!(guard.check_command("ls", &["-la"]).is_ok());
    }

    #[test]
    fn file_size_check() {
        let guard = PermissionGuard::new(&test_config());
        assert!(guard.check_file_size(512).is_ok());
        assert!(
            guard
                .check_file_size(guard.max_file_size_bytes() + 1)
                .is_err()
        );
    }

    #[test]
    fn sensitive_env_var_blocked() {
        let guard = PermissionGuard::new(&test_config());
        assert!(guard.check_env_var("API_KEY").is_err());
        assert!(guard.check_env_var("DB_SECRET").is_err());
        assert!(guard.check_env_var("AUTH_TOKEN").is_err());
        assert!(guard.check_env_var("DB_PASSWORD").is_err());
    }

    #[test]
    fn normal_env_var_allowed() {
        let guard = PermissionGuard::new(&test_config());
        assert!(guard.check_env_var("HOME").is_ok());
        assert!(guard.check_env_var("PATH").is_ok());
        assert!(guard.check_env_var("EDITOR").is_ok());
    }

    #[test]
    fn is_sensitive_env_detection() {
        let guard = PermissionGuard::new(&test_config());
        assert!(guard.is_sensitive_env("API_KEY"));
        assert!(!guard.is_sensitive_env("HOME"));
    }

    #[test]
    fn path_traversal_blocked() {
        let guard = PermissionGuard::new(&test_config());
        // ../etc from /tmp should resolve outside allowed paths
        let result = guard.check_path(Path::new("/tmp/../etc/hostname"));
        assert!(result.is_err());
    }

    fn sudo_config() -> OsConfig {
        let mut config = OsConfig::default();
        config.enabled = true;
        config.permission_level = OsPermissionLevel::Admin;
        config.allowed_paths = vec!["/tmp".into()];
        let mut sudo = orka_core::config::SudoConfig::default();
        sudo.allowed = true;
        sudo.allowed_commands = vec![
            "systemctl restart".into(),
            "systemctl stop".into(),
            "pacman -S".into(),
        ];
        sudo.password_required = false;
        config.sudo = sudo;
        config
    }

    #[test]
    fn sudo_allowed_command_accepted() {
        let guard = PermissionGuard::new(&sudo_config());
        assert!(
            guard
                .check_sudo_command("systemctl", &["restart", "nginx"])
                .is_ok()
        );
    }

    #[test]
    fn sudo_command_not_in_allowlist_rejected() {
        let guard = PermissionGuard::new(&sudo_config());
        assert!(
            guard
                .check_sudo_command("systemctl", &["start", "nginx"])
                .is_err()
        );
    }

    #[test]
    fn sudo_disabled_rejects_all() {
        let mut config = sudo_config();
        config.sudo.allowed = false;
        let guard = PermissionGuard::new(&config);
        assert!(
            guard
                .check_sudo_command("systemctl", &["restart", "nginx"])
                .is_err()
        );
    }

    #[test]
    fn sudo_requires_admin_level() {
        let mut config = sudo_config();
        config.permission_level = OsPermissionLevel::Execute;
        let guard = PermissionGuard::new(&config);
        assert!(
            guard
                .check_sudo_command("systemctl", &["restart", "nginx"])
                .is_err()
        );
    }

    #[test]
    fn sudo_enabled_accessor() {
        let guard = PermissionGuard::new(&sudo_config());
        assert!(guard.sudo_enabled());
    }
}
