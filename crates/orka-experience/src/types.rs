use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A structured trajectory record aggregated from domain events during a single handler invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trajectory {
    /// Unique trajectory identifier (UUID v7).
    pub id: String,
    /// Session that produced this trajectory.
    pub session_id: String,
    /// Workspace in which the interaction took place.
    pub workspace: String,
    /// When the interaction completed.
    pub timestamp: DateTime<Utc>,
    /// The user's original message.
    pub user_message: String,
    /// The agent's final response.
    pub agent_response: String,
    /// Skills invoked during this interaction.
    pub skills_used: Vec<SkillTrace>,
    /// Total LLM iterations in the agent loop.
    pub iterations: usize,
    /// Total tokens consumed.
    pub total_tokens: u64,
    /// Whether the overall interaction succeeded (no unrecovered errors).
    pub success: bool,
    /// Total duration in milliseconds.
    pub duration_ms: u64,
    /// Error messages encountered, if any.
    pub errors: Vec<String>,
}

/// A record of a single skill invocation within a trajectory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillTrace {
    /// Skill name.
    pub name: String,
    /// How long the skill took to execute, in milliseconds.
    pub duration_ms: u64,
    /// Whether the skill invocation succeeded.
    pub success: bool,
}

/// A principle extracted from trajectory reflection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Principle {
    /// Unique principle identifier (UUID v7).
    pub id: String,
    /// Concise, actionable principle text.
    pub text: String,
    /// The category of principle: "do" (positive) or "avoid" (negative).
    pub kind: PrincipleKind,
    /// The workspace this principle applies to, or "global".
    pub scope: String,
    /// When this principle was first created.
    pub created_at: DateTime<Utc>,
    /// How many times this principle has been reinforced by subsequent reflections.
    pub reinforcement_count: u32,
    /// Relevance score from the last retrieval (transient, not stored).
    #[serde(skip)]
    pub relevance_score: f32,
}

/// Whether a principle is something to do or avoid.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PrincipleKind {
    /// A positive pattern: something the agent should do.
    Do,
    /// A negative pattern: something the agent should avoid.
    Avoid,
}

/// Outcome signal for a completed interaction, used to decide whether to reflect.
#[derive(Debug, Clone)]
pub enum OutcomeSignal {
    /// At least one skill or the overall handler failed.
    Failure,
    /// All skills succeeded and a response was produced.
    Success,
}

/// A trace of a single agent's execution within a graph run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTrace {
    /// Agent identifier.
    pub agent_id: String,
    /// The final response produced by this agent, if any.
    pub response: Option<String>,
    /// Number of LLM iterations consumed.
    pub iterations: usize,
    /// Total tokens consumed by this agent.
    pub tokens: u64,
    /// Whether this agent handed off to another.
    pub did_handoff: bool,
    /// Skills invoked during this agent's execution.
    pub skills_used: Vec<SkillTrace>,
}

/// A record of a handoff between agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoffTrace {
    /// Source agent ID.
    pub from_agent: String,
    /// Target agent ID.
    pub to_agent: String,
    /// Transfer or Delegate.
    pub mode: String,
    /// Why the handoff occurred.
    pub reason: String,
}

/// A trajectory record aggregated from a full multi-agent graph execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphTrajectory {
    /// Unique trajectory identifier (UUID v7).
    pub id: String,
    /// Graph identifier.
    pub graph_id: String,
    /// Run identifier for this execution.
    pub run_id: String,
    /// Session that produced this trajectory.
    pub session_id: String,
    /// When the graph execution completed.
    pub timestamp: DateTime<Utc>,
    /// The user's original message.
    pub user_message: String,
    /// The final agent response.
    pub final_response: String,
    /// Per-agent execution traces.
    pub agent_traces: Vec<AgentTrace>,
    /// Handoffs that occurred during execution.
    pub handoffs: Vec<HandoffTrace>,
    /// Total LLM iterations across all agents.
    pub total_iterations: usize,
    /// Total tokens consumed across all agents.
    pub total_tokens: u64,
    /// Whether the overall execution succeeded.
    pub success: bool,
    /// Total duration in milliseconds.
    pub duration_ms: u64,
}
