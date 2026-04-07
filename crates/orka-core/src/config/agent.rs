//! Agent and multi-agent graph configuration.

use std::collections::HashMap;

use serde::Deserialize;

use crate::config::{
    defaults,
    primitives::{GraphExecutionMode, HistoryFilter, NodeKindDef, ThinkingEffort},
};

/// Per-agent runtime configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct AgentConfig {
    /// Human-readable agent name.
    #[serde(default = "defaults::default_agent_name")]
    pub name: String,
    /// System prompt/instructions for the agent.
    #[serde(default)]
    pub system_prompt: String,
    /// Model identifier to use.
    #[serde(default = "defaults::default_model")]
    pub model: String,
    /// Temperature for generation.
    /// When `None`, inherits from `[llm].default_temperature`, then falls back
    /// to `0.7`. Some models (e.g. gpt-5-mini, o-series) only accept the
    /// default temperature (1.0) and reject custom values.
    #[serde(default)]
    pub temperature: Option<f32>,
    /// Maximum tokens per response.
    #[serde(default = "defaults::default_max_tokens")]
    pub max_tokens: u32,
    /// Thinking/reasoning effort level.
    ///
    /// Enables Anthropic adaptive thinking (Claude 4.6+) or maps to `OpenAI`
    /// `reasoning_effort` depending on the provider. `"max"` is only available
    /// on Claude Opus 4.6+. Omit to disable thinking entirely.
    #[serde(default)]
    pub thinking: Option<ThinkingEffort>,
    /// Maximum tool-use turns before the agent stops.
    #[serde(default = "defaults::default_max_turns")]
    pub max_turns: usize,
    /// Maximum characters for tool results.
    #[serde(default = "defaults::default_tool_result_max_chars")]
    pub tool_result_max_chars: usize,
    /// Allowed tools for this agent.
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Denied tools (takes precedence).
    #[serde(default)]
    pub denied_tools: Vec<String>,
    /// How to filter conversation history when this agent receives a handoff.
    ///
    /// `"full"` (default) passes the entire history. `"last_n"` passes the
    /// last `history_filter_n` messages. `"none"` starts with an empty
    /// context.
    #[serde(default)]
    pub history_filter: HistoryFilter,
    /// Number of messages to keep when `history_filter = "last_n"`.
    #[serde(default)]
    pub history_filter_n: Option<usize>,
    /// Planning mode: `"none"` (default), `"adaptive"`, or `"always"`.
    ///
    /// - `"adaptive"`: inject `create_plan` / `update_plan_step` tools; model
    ///   decides when to plan.
    /// - `"always"`: generate a plan automatically before the first iteration.
    #[serde(default)]
    pub planning_mode: Option<String>,
    /// History strategy: `"truncate"` (default), `"summarize"`, or
    /// `"rolling_window:<n>"` where `<n>` is the number of turns to retain.
    #[serde(default)]
    pub history_strategy: Option<String>,
    /// List of tool names that require human approval before execution.
    ///
    /// When the LLM requests a tool in this list the executor pauses and saves
    /// an `Interrupted` checkpoint instead of running the tool. Resume the run
    /// via `POST /api/v1/runs/{run_id}/approve`.
    #[serde(default)]
    pub interrupt_before_tools: Vec<String>,
    /// Per-skill execution timeout in seconds (default: 120).
    ///
    /// Skills that exceed this wall-clock limit are cancelled and an error is
    /// returned to the LLM.
    #[serde(default = "defaults::default_skill_timeout_secs")]
    pub skill_timeout_secs: u64,
    /// Per-LLM-call timeout in seconds (default: 120).
    ///
    /// If a single LLM call (including streaming) exceeds this limit the agent
    /// stops immediately and returns an error message to the user.
    #[serde(default = "defaults::default_llm_call_timeout_secs")]
    pub llm_call_timeout_secs: u64,
    /// Maximum wall-clock run time in seconds (default: no limit).
    ///
    /// If the entire agent run (across all iterations) exceeds this limit the
    /// agent stops and returns an error message to the user.
    #[serde(default)]
    pub max_run_secs: Option<u64>,
    /// Maximum concurrent skill invocations (reserved for future use).
    #[serde(default)]
    pub max_concurrent_skills: Option<usize>,
    /// Maximum budget extensions granted when the agent completes plan steps
    /// while under budget pressure (default: 2). Each extension adds
    /// `budget_extension_size` turns to the effective limit.
    #[serde(default = "defaults::default_max_budget_extensions")]
    pub max_budget_extensions: usize,
    /// Turns added per budget extension (default: 5).
    #[serde(default = "defaults::default_budget_extension_size")]
    pub budget_extension_size: usize,
    /// Steps between self-reflection checkpoints injected into LLM context
    /// (default: `None` — disabled). When set to e.g. `7`, a progress hint is
    /// injected every 7 consumed steps prompting the agent to reassess.
    #[serde(default)]
    pub reflection_interval: Option<usize>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            name: defaults::default_agent_name(),
            system_prompt: String::new(),
            model: defaults::default_model(),
            temperature: None,
            max_tokens: defaults::default_max_tokens(),
            max_turns: defaults::default_max_turns(),
            tool_result_max_chars: defaults::default_tool_result_max_chars(),
            allowed_tools: Vec::new(),
            denied_tools: Vec::new(),
            thinking: None,
            history_filter: HistoryFilter::default(),
            history_filter_n: None,
            planning_mode: None,
            history_strategy: None,
            interrupt_before_tools: Vec::new(),
            skill_timeout_secs: defaults::default_skill_timeout_secs(),
            llm_call_timeout_secs: defaults::default_llm_call_timeout_secs(),
            max_run_secs: None,
            max_concurrent_skills: None,
            max_budget_extensions: defaults::default_max_budget_extensions(),
            budget_extension_size: defaults::default_budget_extension_size(),
            reflection_interval: None,
        }
    }
}

/// Multi-agent definition for graph-based execution.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct AgentDef {
    /// Agent identifier.
    pub id: String,
    /// Node kind in the graph (default: `agent`).
    ///
    /// Valid values: `"agent"`, `"router"`, `"fan_out"`, `"fan_in"`.
    #[serde(default)]
    pub kind: NodeKindDef,
    /// Agent configuration.
    #[serde(flatten)]
    pub config: AgentConfig,
}

/// Graph topology for multi-agent execution.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct GraphDef {
    /// Explicit entry-point agent ID.
    ///
    /// When set, execution begins at this agent instead of the first entry in
    /// `[[agents]]`.
    #[serde(default)]
    pub entry: Option<String>,
    /// Execution mode for the graph.
    #[serde(default)]
    pub execution_mode: GraphExecutionMode,
    /// Maximum number of hops in the graph.
    #[serde(default = "defaults::default_max_hops")]
    pub max_hops: usize,
    /// Edges connecting agents.
    #[serde(default)]
    pub edges: Vec<EdgeDef>,
    /// State-slot reducer strategies, keyed by `"namespace::slot"`.
    ///
    /// Valid values: `"last_write_wins"` (default), `"append"`,
    /// `"merge_object"`, `"sum"`, `"max"`, `"min"`.
    ///
    /// ```toml
    /// [graph.reducers]
    /// "__shared::results" = "append"
    /// "__shared::score"   = "sum"
    /// ```
    #[serde(default)]
    pub reducers: HashMap<String, String>,
}

impl Default for GraphDef {
    fn default() -> Self {
        Self {
            entry: None,
            execution_mode: GraphExecutionMode::default(),
            max_hops: defaults::default_max_hops(),
            edges: Vec::new(),
            reducers: HashMap::new(),
        }
    }
}

/// Edge definition for agent graph.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct EdgeDef {
    /// Source agent ID.
    pub from: String,
    /// Target agent ID.
    pub to: String,
    /// Condition for traversing this edge (optional).
    pub condition: Option<String>,
    /// Edge weight for routing decisions.
    #[serde(default = "default_edge_weight")]
    pub weight: f32,
}

const fn default_edge_weight() -> f32 {
    1.0
}

impl AgentDef {
    /// Create a new agent definition with the given ID and default config.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            kind: NodeKindDef::default(),
            config: AgentConfig::default(),
        }
    }
}

impl EdgeDef {
    /// Create a new edge from `from` to `to` with default weight and no
    /// condition.
    pub fn new(from: impl Into<String>, to: impl Into<String>) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
            condition: None,
            weight: default_edge_weight(),
        }
    }
}
