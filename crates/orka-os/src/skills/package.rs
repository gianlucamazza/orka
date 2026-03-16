use std::sync::Arc;

use async_trait::async_trait;
use orka_core::traits::Skill;
use orka_core::{Error, Result, SkillInput, SkillOutput, SkillSchema};

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
        SkillSchema {
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Package name to search for" }
                },
                "required": ["query"]
            }),
        }
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

        Ok(SkillOutput {
            data: serde_json::json!({
                "results": stdout,
                "package_manager": format!("{:?}", pm).to_lowercase(),
                "success": output.status.success(),
            }),
        })
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
        SkillSchema {
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Package name" }
                },
                "required": ["name"]
            }),
        }
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

        Ok(SkillOutput {
            data: serde_json::json!({
                "info": stdout,
                "package_manager": format!("{:?}", pm).to_lowercase(),
                "success": output.status.success(),
            }),
        })
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
        SkillSchema {
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "filter": { "type": "string", "description": "Filter by name (grep)" }
                },
                "required": []
            }),
        }
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

        Ok(SkillOutput {
            data: serde_json::json!({
                "packages": lines,
                "count": lines.len(),
                "package_manager": format!("{:?}", pm).to_lowercase(),
            }),
        })
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
        assert!(skill
            .execute(SkillInput { args, context: None })
            .await
            .is_err());
    }
}
