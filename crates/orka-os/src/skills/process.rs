use std::sync::Arc;

use async_trait::async_trait;
use orka_core::traits::Skill;
use orka_core::{Error, ErrorCategory, Result, SkillInput, SkillOutput, SkillSchema};
use sysinfo::System;

use crate::config::PermissionLevel;
use crate::guard::PermissionGuard;

// ── process_list ──

/// Skill that lists running processes with CPU and memory usage.
pub struct ProcessListSkill {
    _guard: Arc<PermissionGuard>,
}

impl ProcessListSkill {
    /// Create a new `process_list` skill with the given permission guard.
    pub fn new(guard: Arc<PermissionGuard>) -> Self {
        Self { _guard: guard }
    }
}

#[async_trait]
impl Skill for ProcessListSkill {
    fn name(&self) -> &str {
        "process_list"
    }

    fn description(&self) -> &str {
        "List running processes with CPU and memory usage."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "filter": { "type": "string", "description": "Filter by process name (substring match)" },
                "sort_by": {
                    "type": "string",
                    "enum": ["cpu", "memory", "pid", "name"],
                    "default": "memory"
                },
                "limit": { "type": "integer", "default": 20, "maximum": 100 }
            },
            "required": []
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let filter = input.args.get("filter").and_then(|v| v.as_str());
        let sort_by = input
            .args
            .get("sort_by")
            .and_then(|v| v.as_str())
            .unwrap_or("memory");
        let limit = input
            .args
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(20) as usize;

        let mut sys = System::new();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

        let mut procs: Vec<serde_json::Value> = sys
            .processes()
            .values()
            .filter(|p| {
                if let Some(f) = filter {
                    p.name().to_string_lossy().to_lowercase().contains(&f.to_lowercase())
                } else {
                    true
                }
            })
            .map(|p| {
                serde_json::json!({
                    "pid": p.pid().as_u32(),
                    "name": p.name().to_string_lossy(),
                    "cpu_percent": p.cpu_usage(),
                    "memory_bytes": p.memory(),
                    "status": format!("{:?}", p.status()),
                    "command": p.cmd().iter().map(|s| s.to_string_lossy().to_string()).collect::<Vec<_>>().join(" "),
                })
            })
            .collect();

        match sort_by {
            "cpu" => procs.sort_by(|a, b| {
                b["cpu_percent"]
                    .as_f64()
                    .unwrap_or(0.0)
                    .partial_cmp(&a["cpu_percent"].as_f64().unwrap_or(0.0))
                    .unwrap_or(std::cmp::Ordering::Equal)
            }),
            "pid" => procs.sort_by(|a, b| {
                a["pid"]
                    .as_u64()
                    .unwrap_or(0)
                    .cmp(&b["pid"].as_u64().unwrap_or(0))
            }),
            "name" => procs.sort_by(|a, b| {
                a["name"]
                    .as_str()
                    .unwrap_or("")
                    .cmp(b["name"].as_str().unwrap_or(""))
            }),
            _ => procs.sort_by(|a, b| {
                b["memory_bytes"]
                    .as_u64()
                    .unwrap_or(0)
                    .cmp(&a["memory_bytes"].as_u64().unwrap_or(0))
            }),
        }

        procs.truncate(limit.min(100));

        Ok(SkillOutput::new(serde_json::json!({
            "processes": procs,
            "total": sys.processes().len(),
            "shown": procs.len(),
        })))
    }
}

// ── process_info ──

/// Skill that returns detailed information about a single process by PID.
pub struct ProcessInfoSkill {
    _guard: Arc<PermissionGuard>,
}

impl ProcessInfoSkill {
    /// Create a new `process_info` skill with the given permission guard.
    pub fn new(guard: Arc<PermissionGuard>) -> Self {
        Self { _guard: guard }
    }
}

#[async_trait]
impl Skill for ProcessInfoSkill {
    fn name(&self) -> &str {
        "process_info"
    }

    fn description(&self) -> &str {
        "Get detailed information about a specific process by PID."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "pid": { "type": "integer", "description": "Process ID" }
            },
            "required": ["pid"]
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let pid = input
            .args
            .get("pid")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| Error::Skill("missing 'pid' argument".into()))? as u32;

        let mut sys = System::new();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

        let process =
            sys.process(sysinfo::Pid::from_u32(pid))
                .ok_or_else(|| Error::SkillCategorized {
                    message: format!("process {} not found", pid),
                    category: ErrorCategory::Input,
                })?;

        Ok(SkillOutput::new(serde_json::json!({
            "pid": process.pid().as_u32(),
            "name": process.name().to_string_lossy(),
            "cpu_percent": process.cpu_usage(),
            "memory_bytes": process.memory(),
            "virtual_memory_bytes": process.virtual_memory(),
            "status": format!("{:?}", process.status()),
            "command": process.cmd().iter().map(|s| s.to_string_lossy().to_string()).collect::<Vec<_>>(),
            "cwd": process.cwd().map(|p| p.to_string_lossy().to_string()),
            "root": process.root().map(|p| p.to_string_lossy().to_string()),
            "parent_pid": process.parent().map(|p| p.as_u32()),
            "start_time": process.start_time(),
            "run_time": process.run_time(),
        })))
    }
}

// ── process_signal ──

/// Skill that sends a UNIX signal to a process by PID.
pub struct ProcessSignalSkill {
    guard: Arc<PermissionGuard>,
}

impl ProcessSignalSkill {
    /// Create a new `process_signal` skill with the given permission guard.
    pub fn new(guard: Arc<PermissionGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
impl Skill for ProcessSignalSkill {
    fn name(&self) -> &str {
        "process_signal"
    }

    fn description(&self) -> &str {
        "Send a signal to a process (SIGTERM, SIGKILL, SIGHUP, SIGUSR1, SIGUSR2)."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "pid": { "type": "integer", "description": "Process ID" },
                "signal": {
                    "type": "string",
                    "enum": ["SIGTERM", "SIGKILL", "SIGHUP", "SIGUSR1", "SIGUSR2"],
                    "default": "SIGTERM"
                }
            },
            "required": ["pid"]
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        self.guard.check_permission(PermissionLevel::Execute)?;

        let pid = input
            .args
            .get("pid")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| Error::Skill("missing 'pid' argument".into()))? as i32;
        let signal_name = input
            .args
            .get("signal")
            .and_then(|v| v.as_str())
            .unwrap_or("SIGTERM");

        let signal = match signal_name {
            "SIGTERM" => nix::sys::signal::Signal::SIGTERM,
            "SIGKILL" => nix::sys::signal::Signal::SIGKILL,
            "SIGHUP" => nix::sys::signal::Signal::SIGHUP,
            "SIGUSR1" => nix::sys::signal::Signal::SIGUSR1,
            "SIGUSR2" => nix::sys::signal::Signal::SIGUSR2,
            _ => return Err(Error::Skill(format!("unsupported signal: {}", signal_name))),
        };

        nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), signal).map_err(|e| {
            let category = match e {
                nix::errno::Errno::EPERM => ErrorCategory::Environmental,
                nix::errno::Errno::ESRCH => ErrorCategory::Input,
                _ => ErrorCategory::Unknown,
            };
            Error::SkillCategorized {
                message: format!("failed to send {} to pid {}: {}", signal_name, pid, e),
                category,
            }
        })?;

        Ok(SkillOutput::new(serde_json::json!({
            "pid": pid,
            "signal": signal_name,
            "success": true,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_guard() -> Arc<PermissionGuard> {
        use orka_core::config::OsConfig;
        Arc::new(PermissionGuard::new(&OsConfig::default()))
    }

    #[test]
    fn process_list_schema_valid() {
        let skill = ProcessListSkill::new(make_guard());
        let schema = skill.schema();
        assert!(schema.parameters["properties"]["filter"].is_object());
    }

    #[tokio::test]
    async fn process_list_returns_data() {
        let skill = ProcessListSkill::new(make_guard());
        let input = SkillInput::new(HashMap::new());
        let output = skill.execute(input).await.unwrap();
        assert!(output.data["total"].as_u64().unwrap() > 0);
    }

    #[tokio::test]
    async fn process_info_current() {
        let skill = ProcessInfoSkill::new(make_guard());
        let pid = std::process::id();
        let mut args = HashMap::new();
        args.insert("pid".into(), serde_json::json!(pid));
        let output = skill.execute(SkillInput::new(args)).await.unwrap();
        assert_eq!(output.data["pid"], pid);
    }

    #[tokio::test]
    async fn process_signal_requires_execute() {
        let guard = {
            use orka_core::config::OsConfig;
            Arc::new(PermissionGuard::new(&OsConfig {
                permission_level: "read-only".into(),
                ..OsConfig::default()
            }))
        };
        let skill = ProcessSignalSkill::new(guard);
        let mut args = HashMap::new();
        args.insert("pid".into(), serde_json::json!(1));
        let result = skill.execute(SkillInput::new(args)).await;
        assert!(result.is_err());
    }

    #[test]
    fn process_info_schema_valid() {
        let skill = ProcessInfoSkill::new(make_guard());
        let schema = skill.schema();
        assert_eq!(schema.parameters["required"][0], "pid");
    }
}
