//! Primitive configuration types shared across modules.

use serde::Deserialize;

/// Configuration schema version.
pub const CURRENT_CONFIG_VERSION: u32 = 4;

/// Execution mode for agent graphs.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum GraphExecutionMode {
    /// Execute agents sequentially in dependency order.
    #[default]
    Sequential,
    /// Execute agents in parallel where possible.
    Parallel,
    /// Agent-driven execution (agents decide when to hand off).
    Autonomous,
}

impl GraphExecutionMode {
    /// Returns true if this mode supports parallel execution.
    #[must_use]
    pub const fn is_parallel(&self) -> bool {
        matches!(self, Self::Parallel)
    }
}

/// Node behaviour in a multi-agent graph.
///
/// Serialized as: `"agent"`, `"router"`, `"fan_out"`, `"fan_in"`.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NodeKindDef {
    /// Standard agent: runs the LLM tool loop, can hand off to other agents.
    #[default]
    Agent,
    /// Evaluates outgoing edge conditions without calling the LLM.
    Router,
    /// Dispatches to all successors in parallel.
    FanOut,
    /// Waits for predecessors to complete, then synthesizes results via LLM.
    FanIn,
}

/// Strategy for filtering conversation history when an agent receives a
/// handoff.
///
/// For `last_n` use the companion `history_filter_n` field to set the count.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HistoryFilter {
    /// Pass the full conversation history to the receiving agent (default).
    #[default]
    Full,
    /// Pass only the last N messages (set `history_filter_n` to N).
    LastN,
    /// Start with an empty history — the receiving agent gets a fresh context.
    None,
}

/// Thinking/reasoning effort level for LLM extended reasoning.
///
/// Maps to Anthropic adaptive thinking and `OpenAI` reasoning effort.
/// Omit `thinking` in agent config to disable thinking entirely.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingEffort {
    /// Minimal thinking — fastest, for simple queries.
    Low,
    /// Moderate thinking — balanced default.
    Medium,
    /// Deep thinking — for complex tasks.
    High,
    /// Maximum depth — only available on Claude Opus 4.6+.
    Max,
}

impl ThinkingEffort {
    /// Return the canonical string value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Max => "max",
        }
    }
}
