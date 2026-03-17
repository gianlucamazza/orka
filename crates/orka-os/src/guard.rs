use std::path::{Path, PathBuf};

use orka_core::config::OsConfig;
use regex::Regex;

use crate::config::PermissionLevel;

/// Central safety enforcement for all OS skills.
pub struct PermissionGuard {
    level: PermissionLevel,
    allowed_paths: Vec<PathBuf>,
    blocked_paths: Vec<String>,
    blocked_commands: Vec<String>,
    allowed_commands: Vec<String>,
    max_file_size_bytes: u64,
    sensitive_env_patterns: Vec<String>,
    sudo_enabled: bool,
    sudo_allowed_commands: Vec<String>,
    sudo_path: String,
}

impl PermissionGuard {
    pub fn new(config: &OsConfig) -> Self {
        let level = PermissionLevel::from_str_config(&config.permission_level);
        let allowed_paths = config
            .allowed_paths
            .iter()
            .map(|p| {
                let expanded = shellexpand(p);
                PathBuf::from(expanded)
            })
            .collect();
        Self {
            level,
            allowed_paths,
            blocked_paths: config.blocked_paths.clone(),
            blocked_commands: config.blocked_commands.clone(),
            allowed_commands: config.allowed_commands.clone(),
            max_file_size_bytes: config.max_file_size_bytes,
            sensitive_env_patterns: config.sensitive_env_patterns.clone(),
            sudo_enabled: config.sudo.enabled,
            sudo_allowed_commands: config.sudo.allowed_commands.clone(),
            sudo_path: config.sudo.sudo_path.clone(),
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

    /// Validate a command against block/allow lists.
    pub fn check_command(&self, cmd: &str, args: &[&str]) -> orka_core::Result<()> {
        let full = if args.is_empty() {
            cmd.to_string()
        } else {
            format!("{} {}", cmd, args.join(" "))
        };

        // Check blocked commands (substring match)
        for blocked in &self.blocked_commands {
            if full.contains(blocked.as_str()) || cmd == blocked.as_str() {
                return Err(orka_core::Error::Skill(format!(
                    "command blocked: matches blocked pattern '{}'",
                    blocked,
                )));
            }
            // Try as regex
            if let Ok(re) = Regex::new(blocked)
                && re.is_match(&full)
            {
                return Err(orka_core::Error::Skill(format!(
                    "command blocked: matches blocked pattern '{}'",
                    blocked,
                )));
            }
        }

        // Check allowed commands (if non-empty, only those are permitted)
        if !self.allowed_commands.is_empty()
            && !self.allowed_commands.iter().any(|a| cmd == a.as_str())
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

    /// Check if an env var name matches a sensitive pattern (for masking in lists).
    pub fn is_sensitive_env(&self, name: &str) -> bool {
        let upper = name.to_uppercase();
        self.sensitive_env_patterns
            .iter()
            .any(|p| glob_match(p, &upper))
    }

    pub fn level(&self) -> PermissionLevel {
        self.level
    }

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
    /// 3. command matches an entry in `sudo.allowed_commands` (prefix match at word boundary)
    /// 4. command is NOT in the block list (block list takes absolute precedence)
    pub fn check_sudo_command(&self, cmd: &str, args: &[&str]) -> orka_core::Result<()> {
        if !self.sudo_enabled {
            return Err(orka_core::Error::Skill(
                "sudo is not enabled in configuration".into(),
            ));
        }

        self.check_permission(PermissionLevel::Admin)?;

        // Block list has absolute precedence
        let full = if args.is_empty() {
            cmd.to_string()
        } else {
            format!("{} {}", cmd, args.join(" "))
        };
        for blocked in &self.blocked_commands {
            if full.contains(blocked.as_str()) || cmd == blocked.as_str() {
                return Err(orka_core::Error::Skill(format!(
                    "privileged command blocked: matches blocked pattern '{}'",
                    blocked,
                )));
            }
            if let Ok(re) = Regex::new(blocked)
                && re.is_match(&full)
            {
                return Err(orka_core::Error::Skill(format!(
                    "privileged command blocked: matches blocked pattern '{}'",
                    blocked,
                )));
            }
        }

        // Check sudo allowed commands (prefix match at word boundary)
        let allowed = self.sudo_allowed_commands.iter().any(|allowed_cmd| {
            full == *allowed_cmd || full.starts_with(&format!("{} ", allowed_cmd))
        });
        if !allowed {
            return Err(orka_core::Error::Skill(format!(
                "command '{}' not in sudo allowed list",
                full,
            )));
        }

        Ok(())
    }

    fn validate_canonical_path(&self, canonical: &Path) -> orka_core::Result<()> {
        let path_str = canonical.to_string_lossy();

        // Check blocked paths
        for blocked in &self.blocked_paths {
            let expanded = shellexpand(blocked);
            if path_str.starts_with(&expanded) || glob_match(&expanded, &path_str) {
                return Err(orka_core::Error::Skill(format!(
                    "path '{}' is blocked",
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
fn shellexpand(s: &str) -> String {
    if let Some(rest) = s.strip_prefix("~/")
        && let Ok(home) = std::env::var("HOME")
    {
        return format!("{}/{}", home, rest);
    }
    if s == "~"
        && let Ok(home) = std::env::var("HOME")
    {
        return home;
    }
    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> OsConfig {
        OsConfig {
            enabled: true,
            permission_level: "read-only".into(),
            allowed_paths: vec!["/tmp".into()],
            blocked_paths: vec!["/tmp/secret".into()],
            blocked_commands: vec!["rm -rf /".into(), "dd".into()],
            allowed_commands: vec![],
            max_file_size_bytes: 1024,
            shell_timeout_secs: 30,
            max_output_bytes: 1024,
            max_list_entries: 100,
            sensitive_env_patterns: vec![
                "*_KEY".into(),
                "*_SECRET".into(),
                "*_TOKEN".into(),
                "*_PASSWORD".into(),
            ],
            sudo: orka_core::config::SudoConfig::default(),
        }
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
    fn blocked_path_rejected() {
        let guard = PermissionGuard::new(&test_config());
        // Create a file in blocked path
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
    fn blocked_command_rejected() {
        let guard = PermissionGuard::new(&test_config());
        assert!(guard.check_command("dd", &["if=/dev/zero"]).is_err());
        assert!(guard.check_command("rm", &["-rf", "/"]).is_err());
    }

    #[test]
    fn unblocked_command_allowed() {
        let guard = PermissionGuard::new(&test_config());
        assert!(guard.check_command("ls", &["-la"]).is_ok());
    }

    #[test]
    fn file_size_check() {
        let guard = PermissionGuard::new(&test_config());
        assert!(guard.check_file_size(512).is_ok());
        assert!(guard.check_file_size(2048).is_err());
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
        OsConfig {
            enabled: true,
            permission_level: "admin".into(),
            allowed_paths: vec!["/tmp".into()],
            blocked_paths: vec![],
            blocked_commands: vec!["rm -rf /".into(), "dd".into()],
            allowed_commands: vec![],
            max_file_size_bytes: 1024,
            shell_timeout_secs: 30,
            max_output_bytes: 1024,
            max_list_entries: 100,
            sensitive_env_patterns: vec![],
            sudo: orka_core::config::SudoConfig {
                enabled: true,
                allowed_commands: vec![
                    "systemctl restart".into(),
                    "systemctl stop".into(),
                    "pacman -S".into(),
                ],
                ..orka_core::config::SudoConfig::default()
            },
        }
    }

    #[test]
    fn sudo_allowed_command_accepted() {
        let guard = PermissionGuard::new(&sudo_config());
        assert!(guard
            .check_sudo_command("systemctl", &["restart", "nginx"])
            .is_ok());
    }

    #[test]
    fn sudo_command_not_in_allowlist_rejected() {
        let guard = PermissionGuard::new(&sudo_config());
        assert!(guard
            .check_sudo_command("systemctl", &["start", "nginx"])
            .is_err());
    }

    #[test]
    fn sudo_blocked_command_rejected_even_if_allowed() {
        let mut config = sudo_config();
        config.sudo.allowed_commands.push("dd".into());
        let guard = PermissionGuard::new(&config);
        // dd is in blocked_commands, so it must be rejected
        assert!(guard
            .check_sudo_command("dd", &["if=/dev/zero"])
            .is_err());
    }

    #[test]
    fn sudo_disabled_rejects_all() {
        let mut config = sudo_config();
        config.sudo.enabled = false;
        let guard = PermissionGuard::new(&config);
        assert!(guard
            .check_sudo_command("systemctl", &["restart", "nginx"])
            .is_err());
    }

    #[test]
    fn sudo_requires_admin_level() {
        let mut config = sudo_config();
        config.permission_level = "execute".into();
        let guard = PermissionGuard::new(&config);
        assert!(guard
            .check_sudo_command("systemctl", &["restart", "nginx"])
            .is_err());
    }

    #[test]
    fn sudo_enabled_accessor() {
        let guard = PermissionGuard::new(&sudo_config());
        assert!(guard.sudo_enabled());
    }
}
