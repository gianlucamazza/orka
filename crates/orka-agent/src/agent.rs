//! Agent definition and related identifier, scope, and LLM-config types.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use orka_llm::ThinkingConfig;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Opaque, cheaply-cloneable agent identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AgentId(pub Arc<str>);

impl Serialize for AgentId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for AgentId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(AgentId(Arc::from(s.as_str())))
    }
}

impl AgentId {
    /// Create a new `AgentId` from any value that converts to `Arc<str>`.
    pub fn new(id: impl Into<Arc<str>>) -> Self {
        Self(id.into())
    }
}

impl std::fmt::Display for AgentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for AgentId {
    fn from(s: &str) -> Self {
        Self(Arc::from(s))
    }
}

impl From<String> for AgentId {
    fn from(s: String) -> Self {
        Self(Arc::from(s.as_str()))
    }
}

/// Which skills this agent can access from the registry.
#[derive(Debug, Clone)]
pub enum ToolScope {
    /// Access all registered skills.
    All,
    /// Only allow these skill names.
    Allow(HashSet<String>),
    /// Allow all except these skill names.
    Deny(HashSet<String>),
}

impl ToolScope {
    /// Returns true if the given skill name is allowed by this scope.
    pub fn allows(&self, name: &str) -> bool {
        match self {
            ToolScope::All => true,
            ToolScope::Allow(set) => set.contains(name),
            ToolScope::Deny(set) => !set.contains(name),
        }
    }
}

/// System prompt for an agent, split into composable sections.
#[derive(Debug, Clone, Default)]
pub struct SystemPrompt {
    /// The agent persona (from SOUL.md body).
    pub persona: String,
    /// Tool instructions (from TOOLS.md body).
    pub tool_instructions: String,
    /// Dynamic sections injected at runtime (principles, context, etc.).
    pub dynamic_sections: BTreeMap<String, String>,
}

impl SystemPrompt {
    /// Build the full prompt string for LLM submission.
    pub fn build(&self, agent_name: &str) -> String {
        let mut prompt = if self.persona.is_empty() {
            format!("You are {agent_name}.")
        } else {
            format!("You are {agent_name}.\n\n{}", self.persona)
        };

        if !self.tool_instructions.is_empty() {
            prompt.push_str("\n\n");
            prompt.push_str(&self.tool_instructions);
        }

        for section in self.dynamic_sections.values() {
            if !section.is_empty() {
                prompt.push_str("\n\n");
                prompt.push_str(section);
            }
        }

        prompt
    }
}

/// Per-agent LLM configuration overrides.
#[derive(Debug, Clone, Default)]
pub struct AgentLlmConfig {
    /// Override the model name (e.g. `"claude-sonnet-4-6"`).
    pub model: Option<String>,
    /// Override the maximum number of output tokens.
    pub max_tokens: Option<u32>,
    /// Override the context window size in tokens.
    pub context_window: Option<u32>,
    /// Override the sampling temperature (0.0–1.0).
    pub temperature: Option<f32>,
    /// Extended thinking / reasoning configuration.
    pub thinking: Option<ThinkingConfig>,
}

/// An agent definition — the core unit of execution in a graph.
///
/// Agents are structs rather than traits because the LLM loop is
/// uniform across all agents; variation is entirely in data.
#[derive(Debug, Clone)]
pub struct Agent {
    /// Unique identifier used in graph topology.
    pub id: AgentId,
    /// Human-readable name shown in logs and UIs.
    pub display_name: String,
    /// System prompt loaded from SOUL.md/TOOLS.md.
    pub system_prompt: SystemPrompt,
    /// Which skills this agent may use.
    pub tools: ToolScope,
    /// LLM configuration overrides.
    pub llm_config: AgentLlmConfig,
    /// IDs of agents this agent can hand off to at runtime.
    pub handoff_targets: Vec<AgentId>,
    /// Maximum LLM iterations before giving up.
    pub max_iterations: usize,
    /// Per-skill execution timeout in seconds.
    pub skill_timeout_secs: u64,
    /// Maximum output size in bytes per skill invocation (None = no limit).
    pub skill_max_output_bytes: Option<usize>,
    /// Maximum duration in milliseconds per skill invocation (None = no limit).
    pub skill_max_duration_ms: Option<u64>,
    /// Enable progressive tool disclosure (start with synthetic discovery tools only).
    pub progressive_disclosure: bool,
}

impl Agent {
    /// Create a new agent with default configuration.
    pub fn new(id: impl Into<AgentId>, display_name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            display_name: display_name.into(),
            system_prompt: SystemPrompt::default(),
            tools: ToolScope::All,
            llm_config: AgentLlmConfig::default(),
            handoff_targets: Vec::new(),
            max_iterations: 15,
            skill_timeout_secs: 120,
            skill_max_output_bytes: None,
            skill_max_duration_ms: None,
            progressive_disclosure: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_scope_all_allows_everything() {
        let scope = ToolScope::All;
        assert!(scope.allows("web_search"));
        assert!(scope.allows("echo"));
        assert!(scope.allows("anything"));
    }

    #[test]
    fn tool_scope_allow_list() {
        let scope = ToolScope::Allow(
            ["web_search", "echo"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        );
        assert!(scope.allows("web_search"));
        assert!(scope.allows("echo"));
        assert!(!scope.allows("run_code"));
    }

    #[test]
    fn tool_scope_deny_list() {
        let scope = ToolScope::Deny(["run_code"].iter().map(|s| s.to_string()).collect());
        assert!(scope.allows("web_search"));
        assert!(!scope.allows("run_code"));
    }

    #[test]
    fn system_prompt_build_uses_display_name() {
        let sp = SystemPrompt::default();
        let built = sp.build("Aria");
        assert!(built.contains("Aria"));
    }

    #[test]
    fn agent_id_display() {
        let id = AgentId::new("router");
        assert_eq!(id.to_string(), "router");
    }

    #[test]
    fn agent_id_from_string() {
        let id = AgentId::from("test".to_string());
        assert_eq!(id.0.as_ref(), "test");
    }

    #[test]
    fn agent_id_serde_roundtrip() {
        let id = AgentId::new("researcher");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"researcher\"");
        let back: AgentId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn system_prompt_build_empty_persona() {
        let sp = SystemPrompt::default();
        let built = sp.build("Bot");
        assert_eq!(built, "You are Bot.");
    }

    #[test]
    fn system_prompt_build_skips_empty_dynamic_sections() {
        let mut sp = SystemPrompt {
            persona: "Helpful.".into(),
            ..Default::default()
        };
        sp.dynamic_sections.insert("empty".into(), String::new());
        sp.dynamic_sections
            .insert("filled".into(), "Real content.".into());
        let built = sp.build("Bot");
        assert!(built.contains("Real content."));
        // No double blank lines from the empty section
        assert!(!built.contains("\n\n\n\n"));
    }

    #[test]
    fn system_prompt_build_includes_all_sections() {
        let mut sp = SystemPrompt {
            persona: "I am helpful.".into(),
            tool_instructions: "Use tools wisely.".into(),
            ..Default::default()
        };
        sp.dynamic_sections
            .insert("principles".into(), "Be honest.".into());
        let built = sp.build("Bot");
        assert!(built.contains("I am helpful."));
        assert!(built.contains("Use tools wisely."));
        assert!(built.contains("Be honest."));
    }
}
