//! Agent-to-Agent (A2A) protocol configuration.

use serde::Deserialize;

/// Agent-to-Agent (A2A) protocol configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct A2aConfig {
    /// Enable A2A discovery.
    #[serde(default = "default_a2a_discovery_enabled")]
    pub discovery_enabled: bool,
    /// Discovery interval in seconds.
    #[serde(default = "default_discovery_interval_secs")]
    pub discovery_interval_secs: u64,
    /// Known agent endpoints.
    #[serde(default)]
    pub known_agents: Vec<String>,
    /// Require authentication on the `POST /a2a` endpoint.
    #[serde(default)]
    pub auth_enabled: bool,
    /// Storage backend for A2A task and push-notification state.
    #[serde(default = "default_a2a_store_backend")]
    pub store_backend: String,
}

impl Default for A2aConfig {
    fn default() -> Self {
        Self {
            discovery_enabled: default_a2a_discovery_enabled(),
            discovery_interval_secs: default_discovery_interval_secs(),
            known_agents: Vec::new(),
            auth_enabled: false,
            store_backend: default_a2a_store_backend(),
        }
    }
}

const fn default_a2a_discovery_enabled() -> bool {
    false
}

const fn default_discovery_interval_secs() -> u64 {
    300
}

fn default_a2a_store_backend() -> String {
    "memory".to_string()
}
