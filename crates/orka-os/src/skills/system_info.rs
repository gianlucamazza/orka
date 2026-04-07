use std::sync::Arc;

use async_trait::async_trait;
use orka_core::{Result, SkillInput, SkillOutput, SkillSchema, traits::Skill};
use sysinfo::{Networks, System};

use crate::guard::PermissionGuard;

/// Skill that returns CPU, memory, disk, and network usage for the local host.
pub struct SystemInfoSkill {
    _guard: Arc<PermissionGuard>,
}

impl SystemInfoSkill {
    /// Create a new `system_info` skill with the given permission guard.
    pub fn new(guard: Arc<PermissionGuard>) -> Self {
        Self { _guard: guard }
    }
}

#[async_trait]
impl Skill for SystemInfoSkill {
    fn name(&self) -> &'static str {
        "system_info"
    }

    fn category(&self) -> &'static str {
        "system"
    }

    fn description(&self) -> &'static str {
        "Get system information: CPU, memory, disk, network interfaces, OS details, and uptime."
    }

    fn budget_cost(&self) -> f32 {
        0.5
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "category": {
                    "type": "string",
                    "description": "Category of info to retrieve",
                    "enum": ["all", "cpu", "memory", "disk", "network", "os", "processes"],
                    "default": "all"
                }
            },
            "required": []
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let category = input
            .args
            .get("category")
            .and_then(|v| v.as_str())
            .unwrap_or("all");

        let mut sys = System::new();

        let data = match category {
            "cpu" => {
                sys.refresh_cpu_all();
                cpu_info(&sys)
            }
            "memory" => {
                sys.refresh_memory();
                memory_info(&sys)
            }
            "disk" => disk_info(),
            "network" => network_info(),
            "os" => os_info(),
            "processes" => {
                sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
                serde_json::json!({
                    "total_processes": sys.processes().len(),
                })
            }
            _ => {
                sys.refresh_all();
                serde_json::json!({
                    "cpu": cpu_info(&sys),
                    "memory": memory_info(&sys),
                    "disk": disk_info(),
                    "network": network_info(),
                    "os": os_info(),
                    "processes": {
                        "total": sys.processes().len(),
                    },
                })
            }
        };

        Ok(SkillOutput::new(data))
    }
}

fn cpu_info(sys: &System) -> serde_json::Value {
    let cpus = sys.cpus();
    serde_json::json!({
        "count": cpus.len(),
        "brand": cpus.first().map(|c| c.brand().to_string()).unwrap_or_default(),
        "usage_percent": sys.global_cpu_usage(),
    })
}

fn memory_info(sys: &System) -> serde_json::Value {
    serde_json::json!({
        "total_bytes": sys.total_memory(),
        "used_bytes": sys.used_memory(),
        "available_bytes": sys.available_memory(),
        "swap_total_bytes": sys.total_swap(),
        "swap_used_bytes": sys.used_swap(),
    })
}

fn disk_info() -> serde_json::Value {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let disk_list: Vec<serde_json::Value> = disks
        .iter()
        .map(|d| {
            serde_json::json!({
                "name": d.name().to_string_lossy(),
                "mount_point": d.mount_point().to_string_lossy(),
                "total_bytes": d.total_space(),
                "available_bytes": d.available_space(),
                "file_system": String::from_utf8_lossy(d.file_system().as_encoded_bytes()),
            })
        })
        .collect();
    serde_json::json!(disk_list)
}

fn network_info() -> serde_json::Value {
    let nets = Networks::new_with_refreshed_list();
    let net_list: Vec<serde_json::Value> = nets
        .iter()
        .map(|(name, data)| {
            serde_json::json!({
                "name": name,
                "received_bytes": data.total_received(),
                "transmitted_bytes": data.total_transmitted(),
                "mac_address": data.mac_address().to_string(),
            })
        })
        .collect();
    serde_json::json!(net_list)
}

/// Parse NAME and VERSION from os-release file content.
fn parse_os_release(content: &str) -> Option<(String, String)> {
    let mut name = None;
    let mut version = None;
    for line in content.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("NAME=") {
            name = Some(val.trim_matches('"').to_string());
        } else if let Some(val) = line.strip_prefix("VERSION=") {
            version = Some(val.trim_matches('"').to_string());
        }
    }
    Some((name?, version.unwrap_or_default()))
}

/// Read host OS info from the bind-mounted `/host/os-release` file.
fn host_os_release() -> Option<(String, String)> {
    let content = std::fs::read_to_string("/host/os-release").ok()?;
    parse_os_release(&content)
}

fn os_info() -> serde_json::Value {
    let (os_name, os_version) = host_os_release().unwrap_or_else(|| {
        (
            System::name().unwrap_or_default(),
            System::os_version().unwrap_or_default(),
        )
    });

    let host_name = std::env::var("ORKA_HOST_HOSTNAME")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| System::host_name().unwrap_or_default());

    serde_json::json!({
        "name": os_name,
        "kernel_version": System::kernel_version().unwrap_or_default(),
        "os_version": os_version,
        "host_name": host_name,
        "uptime_secs": System::uptime(),
        "boot_time": System::boot_time(),
        "load_avg": {
            "one": System::load_average().one,
            "five": System::load_average().five,
            "fifteen": System::load_average().fifteen,
        },
    })
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::field_reassign_with_default,
    clippy::default_trait_access,
    clippy::needless_pass_by_value,
    clippy::stable_sort_primitive
)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn make_skill() -> SystemInfoSkill {
        use crate::{config::OsConfig, guard::PermissionGuard};
        let config = OsConfig::default();
        SystemInfoSkill::new(Arc::new(PermissionGuard::new(&config)))
    }

    #[test]
    fn schema_is_valid() {
        let skill = make_skill();
        let schema = skill.schema();
        assert!(schema.parameters["properties"]["category"].is_object());
    }

    #[tokio::test]
    async fn system_info_all() {
        let skill = make_skill();
        let input = SkillInput::new(HashMap::new());
        let output = skill.execute(input).await.unwrap();
        assert!(output.data["os"]["host_name"].is_string());
        assert!(output.data["memory"]["total_bytes"].is_number());
    }

    #[tokio::test]
    async fn system_info_cpu() {
        let skill = make_skill();
        let mut args = HashMap::new();
        args.insert("category".into(), serde_json::json!("cpu"));
        let input = SkillInput::new(args);
        let output = skill.execute(input).await.unwrap();
        assert!(output.data["count"].is_number());
    }

    #[test]
    fn parse_os_release_arch() {
        let content = r#"NAME="Arch Linux"
PRETTY_NAME="Arch Linux"
ID=arch
BUILD_ID=rolling
VERSION_ID=TEMPLATE_VERSION_ID
ANSI_COLOR="38;2;23;147;209"
HOME_URL="https://archlinux.org/"
"#;
        let (name, version) = parse_os_release(content).unwrap();
        assert_eq!(name, "Arch Linux");
        assert_eq!(version, ""); // Arch has no VERSION field
    }

    #[test]
    fn parse_os_release_debian() {
        let content = r#"PRETTY_NAME="Debian GNU/Linux 12 (bookworm)"
NAME="Debian GNU/Linux"
VERSION_ID="12"
VERSION="12 (bookworm)"
ID=debian
"#;
        let (name, version) = parse_os_release(content).unwrap();
        assert_eq!(name, "Debian GNU/Linux");
        assert_eq!(version, "12 (bookworm)");
    }

    #[test]
    fn parse_os_release_missing_name() {
        let content = "VERSION=\"12\"\nID=debian\n";
        assert!(parse_os_release(content).is_none());
    }

    #[tokio::test]
    async fn system_info_os() {
        let skill = make_skill();
        let mut args = HashMap::new();
        args.insert("category".into(), serde_json::json!("os"));
        let input = SkillInput::new(args);
        let output = skill.execute(input).await.unwrap();
        assert!(output.data["uptime_secs"].is_number());
    }
}
