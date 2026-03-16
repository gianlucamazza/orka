use std::sync::Arc;

use async_trait::async_trait;
use orka_core::traits::Skill;
use orka_core::{Result, SkillInput, SkillOutput, SkillSchema};
use sysinfo::{Networks, System};

use crate::guard::PermissionGuard;

pub struct SystemInfoSkill {
    _guard: Arc<PermissionGuard>,
}

impl SystemInfoSkill {
    pub fn new(guard: Arc<PermissionGuard>) -> Self {
        Self { _guard: guard }
    }
}

#[async_trait]
impl Skill for SystemInfoSkill {
    fn name(&self) -> &str {
        "system_info"
    }

    fn description(&self) -> &str {
        "Get system information: CPU, memory, disk, network interfaces, OS details, and uptime."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema {
            parameters: serde_json::json!({
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
            }),
        }
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
            "all" | _ => {
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

        Ok(SkillOutput { data })
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

fn os_info() -> serde_json::Value {
    serde_json::json!({
        "name": System::name().unwrap_or_default(),
        "kernel_version": System::kernel_version().unwrap_or_default(),
        "os_version": System::os_version().unwrap_or_default(),
        "host_name": System::host_name().unwrap_or_default(),
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
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_skill() -> SystemInfoSkill {
        use crate::guard::PermissionGuard;
        use orka_core::config::OsConfig;
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
        let input = SkillInput {
            args: HashMap::new(),
            context: None,
        };
        let output = skill.execute(input).await.unwrap();
        assert!(output.data["os"]["host_name"].is_string());
        assert!(output.data["memory"]["total_bytes"].is_number());
    }

    #[tokio::test]
    async fn system_info_cpu() {
        let skill = make_skill();
        let mut args = HashMap::new();
        args.insert("category".into(), serde_json::json!("cpu"));
        let input = SkillInput {
            args,
            context: None,
        };
        let output = skill.execute(input).await.unwrap();
        assert!(output.data["count"].is_number());
    }

    #[tokio::test]
    async fn system_info_os() {
        let skill = make_skill();
        let mut args = HashMap::new();
        args.insert("category".into(), serde_json::json!("os"));
        let input = SkillInput {
            args,
            context: None,
        };
        let output = skill.execute(input).await.unwrap();
        assert!(output.data["uptime_secs"].is_number());
    }
}
