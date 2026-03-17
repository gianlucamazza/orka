pub mod config;
pub mod guard;
pub mod skills;

use std::sync::Arc;

use orka_core::config::OsConfig;
use orka_core::traits::Skill;
use orka_core::Result;
use tracing::info;

use config::PermissionLevel;
use guard::PermissionGuard;

/// Create OS skills from config, filtered by permission level and feature flags.
pub fn create_os_skills(config: &OsConfig) -> Result<Vec<Arc<dyn Skill>>> {
    let guard = Arc::new(PermissionGuard::new(config));
    let level = guard.level();
    let mut result: Vec<Arc<dyn Skill>> = vec![
        Arc::new(skills::system_info::SystemInfoSkill::new(guard.clone())),
        Arc::new(skills::fs::FsReadSkill::new(guard.clone(), config)),
        Arc::new(skills::fs::FsListSkill::new(guard.clone(), config)),
        Arc::new(skills::fs::FsInfoSkill::new(guard.clone())),
        Arc::new(skills::fs::FsSearchSkill::new(guard.clone(), config)),
        Arc::new(skills::process::ProcessListSkill::new(guard.clone())),
        Arc::new(skills::process::ProcessInfoSkill::new(guard.clone())),
        Arc::new(skills::env::EnvGetSkill::new(guard.clone())),
        Arc::new(skills::env::EnvListSkill::new(guard.clone())),
        Arc::new(skills::network::NetworkInfoSkill::new(guard.clone())),
        Arc::new(skills::network::NetworkCheckSkill::new(guard.clone())),
    ];

    // Write skills
    if level >= PermissionLevel::Write {
        result.push(Arc::new(skills::fs::FsWriteSkill::new(guard.clone())));

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

    // Execute skills
    if level >= PermissionLevel::Execute {
        result.push(Arc::new(skills::shell::ShellExecSkill::new(
            guard.clone(),
            config,
        )));
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

    // Admin skills
    if level >= PermissionLevel::Admin {
        result.push(Arc::new(skills::package::PackageSearchSkill::new(
            guard.clone(),
        )));
        result.push(Arc::new(skills::package::PackageInfoSkill::new(
            guard.clone(),
        )));
        result.push(Arc::new(skills::package::PackageListSkill::new(
            guard.clone(),
        )));

        #[cfg(feature = "systemd")]
        {
            result.push(Arc::new(skills::systemd::ServiceStatusSkill::new(
                guard.clone(),
            )));
            result.push(Arc::new(skills::systemd::ServiceListSkill::new(
                guard.clone(),
            )));
            result.push(Arc::new(skills::systemd::JournalReadSkill::new(
                guard.clone(),
            )));
        }
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
    use super::*;

    #[test]
    fn read_only_skill_count() {
        let config = OsConfig {
            permission_level: "read-only".into(),
            ..OsConfig::default()
        };
        let skills = create_os_skills(&config).unwrap();
        assert_eq!(skills.len(), 11); // 11 read-only skills
    }

    #[test]
    fn write_level_has_more_skills() {
        let config = OsConfig {
            permission_level: "write".into(),
            ..OsConfig::default()
        };
        let skills = create_os_skills(&config).unwrap();
        assert!(skills.len() > 11);
    }

    #[test]
    fn execute_level_has_more_skills() {
        let config = OsConfig {
            permission_level: "execute".into(),
            ..OsConfig::default()
        };
        let skills = create_os_skills(&config).unwrap();
        // Should include read-only + write + execute skills
        let write_config = OsConfig {
            permission_level: "write".into(),
            ..OsConfig::default()
        };
        let write_skills = create_os_skills(&write_config).unwrap();
        assert!(skills.len() > write_skills.len());
    }

    #[test]
    fn admin_level_has_all_skills() {
        let config = OsConfig {
            permission_level: "admin".into(),
            ..OsConfig::default()
        };
        let skills = create_os_skills(&config).unwrap();
        let exec_config = OsConfig {
            permission_level: "execute".into(),
            ..OsConfig::default()
        };
        let exec_skills = create_os_skills(&exec_config).unwrap();
        assert!(skills.len() > exec_skills.len());
    }

    #[test]
    fn all_skills_have_valid_schemas() {
        let config = OsConfig {
            permission_level: "admin".into(),
            ..OsConfig::default()
        };
        let skills = create_os_skills(&config).unwrap();
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
        let config = OsConfig {
            permission_level: "admin".into(),
            ..OsConfig::default()
        };
        let skills = create_os_skills(&config).unwrap();
        let mut names: Vec<&str> = skills.iter().map(|s| s.name()).collect();
        let total = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), total, "duplicate skill names found");
    }
}
