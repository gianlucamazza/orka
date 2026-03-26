//! Operating system interaction skills with permission guards.
//!
//! Provides file, shell, process, and network skills gated by configurable
//! [`PermissionLevel`].

#![warn(missing_docs)]

/// [`PermissionLevel`] enum for OS skill access control.
pub mod config;
/// Shared domain-event helpers for privileged OS skills.
pub mod events;
/// [`PermissionGuard`] — central safety enforcement for all OS skills.
pub mod guard;
/// Runtime capability probing for startup validation.
pub mod probe;
/// All OS skills grouped by capability area.
pub mod skills;

use std::sync::Arc;

use orka_core::{Result, config::OsConfig, traits::Skill};
pub use probe::{EnvironmentCapabilities, PackageUpdateMethod};
use tracing::{info, warn};

/// Check whether the current process has `NoNewPrivileges` set.
///
/// On Linux this uses `prctl(PR_GET_NO_NEW_PRIVS)`. Returns `false` on
/// non-Linux platforms or if the check cannot be performed.
#[allow(unsafe_code)]
pub fn has_no_new_privileges() -> bool {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: PR_GET_NO_NEW_PRIVS (39) takes no pointer arguments and
        // cannot cause undefined behaviour — it simply returns 0 or 1.
        unsafe { libc::prctl(libc::PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0) == 1 }
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

use config::PermissionLevel;
use guard::PermissionGuard;

/// Create OS skills from config, filtered by permission level and feature
/// flags.
///
/// Pass `caps` from [`EnvironmentCapabilities::probe`] to conditionally exclude
/// skills that are not functional in the current environment.
pub fn create_os_skills(
    config: &OsConfig,
    caps: Option<&EnvironmentCapabilities>,
) -> Result<Vec<Arc<dyn Skill>>> {
    let guard = Arc::new(PermissionGuard::new(config));
    let level = guard.level();

    // ReadOnly skills — always included
    let mut result: Vec<Arc<dyn Skill>> = vec![
        Arc::new(skills::system_info::SystemInfoSkill::new(guard.clone())),
        Arc::new(skills::fs::FsReadSkill::new(guard.clone())),
        Arc::new(skills::fs::FsListSkill::new(guard.clone())),
        Arc::new(skills::fs::FsInfoSkill::new(guard.clone())),
        Arc::new(skills::fs::FsSearchSkill::new(guard.clone())),
        Arc::new(skills::process::ProcessListSkill::new(guard.clone())),
        Arc::new(skills::process::ProcessInfoSkill::new(guard.clone())),
        Arc::new(skills::env::EnvGetSkill::new(guard.clone())),
        Arc::new(skills::env::EnvListSkill::new(guard.clone())),
        Arc::new(skills::network::NetworkInfoSkill::new(guard.clone())),
        Arc::new(skills::network::NetworkCheckSkill::new(guard.clone())),
        Arc::new(skills::package::PackageSearchSkill::new(guard.clone())),
        Arc::new(skills::package::PackageInfoSkill::new(guard.clone())),
        Arc::new(skills::package::PackageListSkill::new(guard.clone())),
    ];

    // package_updates: only if probe says it's available (or no probe was done)
    let update_available = caps.map(|c| c.package_updates.available).unwrap_or(true);
    if update_available {
        let method = caps.and_then(|c| c.update_method);
        result.push(Arc::new(skills::package::PackageUpdatesSkill::new(
            guard.clone(),
            method,
        )));
    } else {
        warn!("package_updates skill disabled: not functional in current environment");
    }

    #[cfg(feature = "systemd")]
    {
        let systemctl_ok = caps.map(|c| c.systemctl.available).unwrap_or(true);
        let journalctl_ok = caps.map(|c| c.journalctl.available).unwrap_or(true);
        if systemctl_ok {
            result.push(Arc::new(skills::systemd::ServiceStatusSkill::new(
                guard.clone(),
            )));
            result.push(Arc::new(skills::systemd::ServiceListSkill::new(
                guard.clone(),
            )));
        }
        if journalctl_ok {
            result.push(Arc::new(skills::systemd::JournalReadSkill::new(
                guard.clone(),
            )));
        }
    }

    // Interact skills — clipboard and desktop notifications
    if level >= PermissionLevel::Interact {
        #[cfg(feature = "clipboard")]
        {
            result.push(Arc::new(skills::clipboard::ClipboardReadSkill::new(
                guard.clone(),
            )));
            result.push(Arc::new(skills::clipboard::ClipboardWriteSkill::new(
                guard.clone(),
            )));
        }

        #[cfg(feature = "desktop")]
        {
            result.push(Arc::new(skills::notify::NotifySendSkill::new(
                guard.clone(),
            )));
        }
    }

    // Write skills
    if level >= PermissionLevel::Write {
        result.push(Arc::new(skills::fs::FsWriteSkill::new(guard.clone())));
        result.push(Arc::new(skills::fs::FsEditSkill::new(guard.clone())));
    }

    // Execute skills
    if level >= PermissionLevel::Execute {
        result.push(Arc::new(skills::shell::ShellExecSkill::new(guard.clone())));
        result.push(Arc::new(skills::process::ProcessSignalSkill::new(
            guard.clone(),
        )));
        result.push(Arc::new(skills::fs::FsWatchSkill::new(guard.clone())));

        #[cfg(feature = "desktop")]
        {
            result.push(Arc::new(skills::desktop::DesktopOpenSkill::new(
                guard.clone(),
            )));
            result.push(Arc::new(skills::desktop::DesktopScreenshotSkill::new(
                guard.clone(),
            )));
        }
    }

    // Admin skills — sudo-only operations
    if level >= PermissionLevel::Admin {
        if guard.sudo_enabled() {
            result.push(Arc::new(skills::package::PackageInstallSkill::new(
                guard.clone(),
            )));
        }

        #[cfg(feature = "systemd")]
        {
            if guard.sudo_enabled() {
                result.push(Arc::new(skills::systemd::ServiceControlSkill::new(
                    guard.clone(),
                )));
            }
        }
    }

    // Coding delegation skills — routing entrypoint plus explicit backends.
    let claude_enabled = config.coding.providers.claude_code.enabled
        && caps.map(|c| c.claude_code.available).unwrap_or(true);
    let codex_enabled =
        config.coding.providers.codex.enabled && caps.map(|c| c.codex.available).unwrap_or(true);
    let opencode_enabled = config.coding.providers.opencode.enabled
        && caps.map(|c| c.opencode.available).unwrap_or(true);

    if config.coding.providers.claude_code.enabled && !claude_enabled {
        warn!("coding provider claude_code disabled: CLI not functional in current environment");
    }
    if config.coding.providers.codex.enabled && !codex_enabled {
        warn!("coding provider codex disabled: CLI not functional in current environment");
    }
    if config.coding.providers.opencode.enabled && !opencode_enabled {
        warn!("coding provider opencode disabled: CLI not functional in current environment");
    }

    if config.coding.enabled && (claude_enabled || codex_enabled || opencode_enabled) {
        result.push(Arc::new(skills::coding_delegate::CodingDelegateSkill::new(
            config,
        )));
    }
    if config.coding.enabled {
        info!(
            claude_code = claude_enabled,
            codex = codex_enabled,
            opencode = opencode_enabled,
            "coding_delegate skill routing initialized"
        );
    }

    info!(
        permission_level = %level,
        skill_count = result.len(),
        "OS skills initialized"
    );

    Ok(result)
}

#[cfg(test)]
mod tests {
    use orka_core::config::{OsConfig, SudoConfig, primitives::OsPermissionLevel};

    use super::*;

    fn config_with_level(level: OsPermissionLevel) -> OsConfig {
        let mut config = OsConfig::default();
        config.permission_level = level;
        config
    }

    #[test]
    fn read_only_skill_count() {
        let config = config_with_level(OsPermissionLevel::ReadOnly);
        let skills = create_os_skills(&config, None).unwrap();
        // 11 base + 4 package read skills + 0–3 systemd read skills (feature-gated)
        let count = skills.len();
        assert!(
            count >= 15,
            "expected at least 15 read-only skills, got {count}"
        );
        assert!(
            count <= 18,
            "expected at most 18 read-only skills (15 + 3 systemd), got {count}"
        );
    }

    #[test]
    fn interact_level_has_more_skills() {
        let config = config_with_level(OsPermissionLevel::Interact);
        let skills = create_os_skills(&config, None).unwrap();
        assert!(skills.len() >= 15);
    }

    #[test]
    fn write_level_has_more_skills() {
        let config = config_with_level(OsPermissionLevel::Write);
        let skills = create_os_skills(&config, None).unwrap();
        assert!(skills.len() > 15);
    }

    #[test]
    fn execute_level_has_more_skills() {
        let config = config_with_level(OsPermissionLevel::Execute);
        let skills = create_os_skills(&config, None).unwrap();
        // Should include read-only + write + execute skills
        let write_config = config_with_level(OsPermissionLevel::Write);
        let write_skills = create_os_skills(&write_config, None).unwrap();
        assert!(skills.len() > write_skills.len());
    }

    #[test]
    fn admin_level_has_all_skills() {
        let mut sudo = SudoConfig::default();
        sudo.allowed = true;
        sudo.allowed_commands = vec![
            "pacman -S".into(),
            "apt install".into(),
            "dnf install".into(),
            "systemctl restart".into(),
            "systemctl start".into(),
            "systemctl stop".into(),
        ];
        let mut config = config_with_level(OsPermissionLevel::Admin);
        config.sudo = sudo.clone();
        let skills = create_os_skills(&config, None).unwrap();
        let mut exec_config = config_with_level(OsPermissionLevel::Execute);
        exec_config.sudo = sudo;
        let exec_skills = create_os_skills(&exec_config, None).unwrap();
        assert!(
            skills.len() > exec_skills.len(),
            "admin ({}) should have more skills than execute ({})",
            skills.len(),
            exec_skills.len()
        );
    }

    #[test]
    fn all_skills_have_valid_schemas() {
        let config = config_with_level(OsPermissionLevel::Admin);
        let skills = create_os_skills(&config, None).unwrap();
        for skill in &skills {
            let schema = skill.schema();
            assert!(
                schema.parameters["type"] == "object",
                "skill '{}' has invalid schema",
                skill.name()
            );
        }
    }

    #[test]
    fn all_skills_have_unique_names() {
        let config = config_with_level(OsPermissionLevel::Admin);
        let skills = create_os_skills(&config, None).unwrap();
        let mut names: Vec<&str> = skills.iter().map(|s| s.name()).collect();
        let total = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), total, "duplicate skill names found");
    }
}
