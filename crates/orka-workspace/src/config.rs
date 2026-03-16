use std::collections::HashMap;

use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SoulFrontmatter {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub max_tokens_per_session: Option<u64>,
    #[serde(default)]
    pub context_window_tokens: Option<u32>,
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default)]
    pub max_agent_iterations: Option<usize>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ToolsFrontmatter {
    #[serde(default)]
    pub tools: Vec<ToolEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolEntry {
    pub name: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub config: HashMap<String, serde_json::Value>,
}

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct IdentityFrontmatter {
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct HeartbeatFrontmatter {
    #[serde(default = "default_interval")]
    pub interval_secs: u64,
    #[serde(default)]
    pub version: Option<String>,
}

fn default_interval() -> u64 {
    30
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MemoryFrontmatter {
    #[serde(default)]
    pub backend: Option<String>,
    #[serde(default)]
    pub max_entries: Option<usize>,
    #[serde(default)]
    pub summarization_model: Option<String>,
    #[serde(default = "default_summarization_threshold")]
    pub summarization_threshold: Option<usize>,
}

fn default_summarization_threshold() -> Option<usize> {
    None
}
