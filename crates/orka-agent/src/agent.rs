//! Agent definition and related identifier, scope, and LLM-config types.

use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

use orka_core::config::HistoryFilter;
use orka_llm::ThinkingConfig;

use crate::planner::PlanningMode;

/// How to manage conversation history when it exceeds the context window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum HistoryStrategy {
    /// Drop oldest turns (current behaviour, default).
    #[default]
    Truncate,
    /// Summarize dropped turns with a cheap LLM call and prepend the summary to
    /// the system prompt so key context is not silently discarded.
    Summarize,
    /// Keep only the most recent `recent_turns` conversation turns. Dropped
    /// turns are summarized incrementally and prepended to the system prompt.
    RollingWindow {
        /// Maximum number of conversation turns to retain.
        recent_turns: usize,
    },
}
use orka_prompts::{
    pipeline::{BuildContext, PipelineConfig, SystemPromptPipeline},
    template::TemplateRegistry,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Opaque, cheaply-cloneable agent identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AgentId(Arc<str>);

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

    /// Return the identifier as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
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
    /// Build the full prompt using the configurable pipeline.
    ///
    /// This method uses the template-based pipeline for maximum flexibility.
    #[allow(clippy::too_many_arguments)]
    pub async fn build(
        &self,
        agent_name: &str,
        workspace_name: &str,
        available_workspaces: Vec<String>,
        cwd: Option<String>,
        principles: Vec<serde_json::Value>,
        conversation_summary: Option<String>,
        template_registry: Option<Arc<TemplateRegistry>>,
        config: &PipelineConfig,
    ) -> orka_core::Result<String> {
        let pipeline = SystemPromptPipeline::from_config(config);

        let ctx = BuildContext::new(agent_name)
            .with_persona(&self.persona)
            .with_tool_instructions(&self.tool_instructions)
            .with_workspace(workspace_name, available_workspaces)
            .with_principles(principles)
            .with_config(config.clone());

        let ctx = if let Some(cwd) = cwd {
            ctx.with_cwd(cwd)
        } else {
            ctx
        };

        let ctx = if let Some(summary) = conversation_summary {
            ctx.with_summary(summary)
        } else {
            ctx
        };

        let ctx = if let Some(registry) = template_registry {
            ctx.with_templates(registry)
        } else {
            ctx
        };

        // Add dynamic sections
        let ctx = self
            .dynamic_sections
            .iter()
            .fold(ctx, |ctx, (name, content)| {
                ctx.with_dynamic_section(name, content)
            });

        pipeline.build(&ctx).await
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
    /// Maximum tool-use turns before the agent stops.
    pub max_turns: usize,
    /// Per-skill execution timeout in seconds.
    pub skill_timeout_secs: u64,
    /// Maximum output size in bytes per skill invocation (None = no limit).
    pub skill_max_output_bytes: Option<usize>,
    /// Maximum duration in milliseconds per skill invocation (None = no limit).
    pub skill_max_duration_ms: Option<u64>,
    /// Enable progressive tool disclosure (start with synthetic discovery tools
    /// only).
    pub progressive_disclosure: bool,
    /// Strategy for filtering conversation history when this agent receives a
    /// handoff.
    pub history_filter: HistoryFilter,
    /// Number of messages to keep when `history_filter` is `LastN`.
    pub history_filter_n: Option<usize>,
    /// Whether to inject planning tools into the LLM tool list.
    pub planning_mode: PlanningMode,
    /// How to handle conversation history when it exceeds the context window.
    pub history_strategy: HistoryStrategy,
    /// Tool names that require human approval before execution.
    ///
    /// When the LLM requests a tool in this set the executor saves an
    /// `Interrupted` checkpoint and pauses, waiting for `approve` or `reject`.
    pub interrupt_before_tools: HashSet<String>,
    /// Maximum characters kept per tool result before truncation for LLM
    /// context.
    pub tool_result_max_chars: usize,
    /// Per-LLM-call timeout in seconds. If a single LLM call exceeds this
    /// limit the agent stops and returns an error message to the user.
    pub llm_call_timeout_secs: u64,
    /// Maximum wall-clock run time in seconds across all iterations.
    /// `None` means no limit beyond `max_turns`.
    pub max_run_secs: Option<u64>,
    /// Maximum budget extensions granted for plan progress (default: 2).
    pub max_budget_extensions: usize,
    /// Turns added per budget extension (default: 5).
    pub budget_extension_size: usize,
    /// Steps between self-reflection checkpoints (default: None = disabled).
    pub reflection_interval: Option<usize>,
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
            max_turns: 15,
            skill_timeout_secs: 120,
            skill_max_output_bytes: None,
            skill_max_duration_ms: None,
            progressive_disclosure: false,
            history_filter: HistoryFilter::default(),
            history_filter_n: None,
            planning_mode: PlanningMode::None,
            history_strategy: HistoryStrategy::Truncate,
            interrupt_before_tools: HashSet::new(),
            tool_result_max_chars: 50_000,
            llm_call_timeout_secs: 120,
            max_run_secs: None,
            max_budget_extensions: 2,
            budget_extension_size: 5,
            reflection_interval: None,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
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
                .map(std::string::ToString::to_string)
                .collect(),
        );
        assert!(scope.allows("web_search"));
        assert!(scope.allows("echo"));
        assert!(!scope.allows("run_code"));
    }

    #[test]
    fn tool_scope_deny_list() {
        let scope = ToolScope::Deny(
            ["run_code"]
                .iter()
                .map(std::string::ToString::to_string)
                .collect(),
        );
        assert!(scope.allows("web_search"));
        assert!(!scope.allows("run_code"));
    }

    #[tokio::test]
    async fn system_prompt_build_uses_display_name() {
        let sp = SystemPrompt::default();
        let config = PipelineConfig::default();
        let built = sp
            .build("Aria", "default", vec![], None, vec![], None, None, &config)
            .await
            .unwrap();
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
        assert_eq!(id.as_str(), "test");
    }

    #[test]
    fn agent_id_serde_roundtrip() {
        let id = AgentId::new("researcher");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"researcher\"");
        let back: AgentId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }

    #[tokio::test]
    async fn system_prompt_build_empty_persona() {
        let sp = SystemPrompt::default();
        let config = PipelineConfig {
            sections: vec!["persona".to_string()],
            ..Default::default()
        };
        let built = sp
            .build("Bot", "default", vec![], None, vec![], None, None, &config)
            .await
            .unwrap();
        assert_eq!(built, "You are Bot.");
    }

    #[tokio::test]
    async fn system_prompt_build_with_persona() {
        let sp = SystemPrompt {
            persona: "I am helpful.".into(),
            ..Default::default()
        };
        let config = PipelineConfig {
            sections: vec!["persona".to_string()],
            ..Default::default()
        };
        let built = sp
            .build("Bot", "default", vec![], None, vec![], None, None, &config)
            .await
            .unwrap();
        assert!(built.contains("You are Bot."));
        assert!(built.contains("I am helpful."));
    }

    #[tokio::test]
    async fn system_prompt_build_with_tools() {
        let sp = SystemPrompt {
            persona: "I am helpful.".into(),
            tool_instructions: "Use tools wisely.".into(),
            ..Default::default()
        };
        let config = PipelineConfig {
            sections: vec!["persona".to_string(), "tools".to_string()],
            ..Default::default()
        };
        let built = sp
            .build("Bot", "default", vec![], None, vec![], None, None, &config)
            .await
            .unwrap();
        assert!(built.contains("I am helpful."));
        assert!(built.contains("Use tools wisely."));
    }
}
