use std::sync::Arc;

use async_trait::async_trait;
use orka_core::traits::Skill;
use orka_core::{Result, SkillInput, SkillOutput, SkillSchema};
use sysinfo::Networks;

use crate::guard::PermissionGuard;

// ── network_info ──

/// Skill that lists network interfaces and their traffic statistics.
pub struct NetworkInfoSkill {
    _guard: Arc<PermissionGuard>,
}

impl NetworkInfoSkill {
    /// Create a new `network_info` skill with the given permission guard.
    pub fn new(guard: Arc<PermissionGuard>) -> Self {
        Self { _guard: guard }
    }
}

#[async_trait]
impl Skill for NetworkInfoSkill {
    fn name(&self) -> &str {
        "network_info"
    }

    fn category(&self) -> &str {
        "system"
    }

    fn description(&self) -> &str {
        "List network interfaces with their traffic statistics."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        }))
    }

    async fn execute(&self, _input: SkillInput) -> Result<SkillOutput> {
        let nets = Networks::new_with_refreshed_list();

        let interfaces: Vec<serde_json::Value> = nets
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

        Ok(SkillOutput::new(serde_json::json!({
            "interfaces": interfaces,
            "count": interfaces.len(),
        })))
    }
}

// ── network_check ──

/// Skill that checks TCP/HTTP connectivity to a given host and port.
pub struct NetworkCheckSkill {
    _guard: Arc<PermissionGuard>,
}

impl NetworkCheckSkill {
    /// Create a new `network_check` skill with the given permission guard.
    pub fn new(guard: Arc<PermissionGuard>) -> Self {
        Self { _guard: guard }
    }
}

#[async_trait]
impl Skill for NetworkCheckSkill {
    fn name(&self) -> &str {
        "network_check"
    }

    fn category(&self) -> &str {
        "system"
    }

    fn description(&self) -> &str {
        "Check network connectivity to a host by attempting a TCP connection."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "host": { "type": "string", "default": "1.1.1.1", "description": "Host to check" },
                "port": { "type": "integer", "default": 443, "description": "Port to connect to" }
            },
            "required": []
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let host = input
            .args
            .get("host")
            .and_then(|v| v.as_str())
            .unwrap_or("1.1.1.1");
        let port = input
            .args
            .get("port")
            .and_then(|v| v.as_u64())
            .unwrap_or(443) as u16;

        let addr = format!("{}:{}", host, port);
        let start = std::time::Instant::now();

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            tokio::net::TcpStream::connect(&addr),
        )
        .await;

        let latency_ms = start.elapsed().as_millis() as u64;

        let (reachable, error) = match result {
            Ok(Ok(_)) => (true, None),
            Ok(Err(e)) => (false, Some(e.to_string())),
            Err(_) => (false, Some("connection timed out".into())),
        };

        Ok(SkillOutput::new(serde_json::json!({
            "host": host,
            "port": port,
            "reachable": reachable,
            "latency_ms": if reachable { Some(latency_ms) } else { None },
            "error": error,
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
    fn network_info_schema_valid() {
        let skill = NetworkInfoSkill::new(make_guard());
        let _schema = skill.schema();
    }

    #[tokio::test]
    async fn network_info_returns_data() {
        let skill = NetworkInfoSkill::new(make_guard());
        let input = SkillInput::new(HashMap::new());
        let output = skill.execute(input).await.unwrap();
        assert!(output.data["count"].is_number());
    }

    #[test]
    fn network_check_schema_valid() {
        let skill = NetworkCheckSkill::new(make_guard());
        let _schema = skill.schema();
    }
}
