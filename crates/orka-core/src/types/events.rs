use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::ids::{ErrorCategory, EventId, MessageId, SessionId};

/// A domain-level event for observability.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[non_exhaustive]
pub struct DomainEvent {
    /// Unique identifier for this event.
    pub id: EventId,
    /// Time at which the event was recorded.
    pub timestamp: DateTime<Utc>,
    /// Discriminant describing what happened.
    pub kind: DomainEventKind,
    /// Arbitrary key-value annotations attached to the event.
    pub metadata: HashMap<String, serde_json::Value>,
}

impl DomainEvent {
    /// Create a new domain event with the given kind.
    pub fn new(kind: DomainEventKind) -> Self {
        Self {
            id: EventId::new(),
            timestamp: Utc::now(),
            kind,
            metadata: HashMap::new(),
        }
    }
}

/// The kind of domain event that occurred.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[non_exhaustive]
#[serde(tag = "type")]
pub enum DomainEventKind {
    /// Emitted when a new inbound message is accepted from a channel adapter.
    MessageReceived {
        /// ID of the inbound message.
        message_id: MessageId,
        /// Channel the message arrived on.
        channel: String,
        /// Session this message belongs to.
        session_id: SessionId,
    },
    /// Emitted when a new session is opened for a channel/user pair.
    SessionCreated {
        /// ID of the newly created session.
        session_id: SessionId,
        /// Channel the session is associated with.
        channel: String,
    },
    /// Emitted when a worker picks up a message and begins processing.
    HandlerInvoked {
        /// ID of the message being processed.
        message_id: MessageId,
        /// Session the message belongs to.
        session_id: SessionId,
    },
    /// Emitted when a handler finishes processing, with timing and reply count.
    HandlerCompleted {
        /// ID of the processed message.
        message_id: MessageId,
        /// Session the message belonged to.
        session_id: SessionId,
        /// Wall-clock processing time in milliseconds.
        duration_ms: u64,
        /// Number of outbound replies produced.
        reply_count: usize,
    },
    /// Emitted when an agent invokes a named skill.
    SkillInvoked {
        /// Name of the skill that was invoked.
        skill_name: String,
        /// ID of the message that triggered the invocation.
        message_id: MessageId,
        /// Serialized input arguments (for audit trail).
        #[serde(default, skip_serializing_if = "HashMap::is_empty")]
        input_args: HashMap<String, serde_json::Value>,
        /// Optional caller identity (agent ID, session, etc.).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        caller_id: Option<String>,
    },
    /// Emitted when a skill returns, with timing and success flag.
    SkillCompleted {
        /// Name of the skill that completed.
        skill_name: String,
        /// ID of the message that triggered the invocation.
        message_id: MessageId,
        /// Wall-clock execution time in milliseconds.
        duration_ms: u64,
        /// Whether the skill returned successfully.
        success: bool,
        /// Error category, if the skill failed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error_category: Option<ErrorCategory>,
        /// Truncated preview of the output (max 1024 chars), for audit trail.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output_preview: Option<String>,
        /// Error message if the skill failed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error_message: Option<String>,
    },
    /// Emitted when a skill is structurally disabled (circuit open or
    /// experience feedback).
    SkillDisabled {
        /// Name of the skill that was disabled.
        skill_name: String,
        /// Human-readable reason for disabling.
        reason: String,
        /// Source of the disable action: "`circuit_breaker`" or
        /// "`experience_feedback`".
        source: String,
    },
    /// Emitted before each LLM call with request parameters.
    LlmRequest {
        /// ID of the message that triggered the LLM call.
        message_id: MessageId,
        /// Model identifier to be used.
        model: String,
        /// LLM provider system (e.g. `"anthropic"`, `"openai"`).
        provider: String,
        /// Agent loop iteration number.
        iteration: usize,
    },
    /// Emitted after each LLM call with token usage and latency.
    LlmCompleted {
        /// ID of the message that triggered the LLM call.
        message_id: MessageId,
        /// Model identifier used for the completion.
        model: String,
        /// LLM provider system (e.g. `"anthropic"`, `"openai"`).
        #[serde(default)]
        provider: String,
        /// Number of tokens in the prompt.
        input_tokens: u32,
        /// Number of tokens in the response.
        output_tokens: u32,
        /// Number of tokens consumed by extended thinking / reasoning.
        #[serde(default)]
        reasoning_tokens: u32,
        /// Wall-clock time for the LLM call in milliseconds.
        duration_ms: u64,
        /// Estimated cost in USD (if cost-per-token config is available).
        #[serde(default)]
        estimated_cost_usd: Option<f64>,
    },
    /// Emitted when an error is encountered during processing.
    ErrorOccurred {
        /// Subsystem or component that raised the error.
        source: String,
        /// Human-readable error description.
        message: String,
    },
    /// Emitted after each LLM response when reasoning text is extracted.
    AgentReasoning {
        /// ID of the message being processed.
        message_id: MessageId,
        /// Agent loop iteration number (1-based).
        iteration: usize,
        /// Extracted reasoning/thinking text from the model.
        reasoning_text: String,
    },
    /// Emitted at the end of each agent loop iteration with summary metrics.
    AgentIteration {
        /// ID of the message being processed.
        message_id: MessageId,
        /// Agent loop iteration number (1-based).
        iteration: usize,
        /// Number of tool calls made in this iteration.
        tool_count: usize,
        /// Cumulative tokens used so far in this agent loop.
        tokens_used: u64,
        /// Wall-clock time elapsed since the loop started, in milliseconds.
        elapsed_ms: u64,
    },
    /// Emitted after a privileged shell command runs (approved or not).
    PrivilegedCommandExecuted {
        /// ID of the message that triggered the command.
        message_id: MessageId,
        /// Session the command ran in.
        session_id: SessionId,
        /// The command binary that was executed.
        command: String,
        /// Arguments passed to the command.
        args: Vec<String>,
        /// ID of the approval record, if the command required approval.
        approval_id: Option<Uuid>,
        /// Identity of the approver, if approval was granted.
        approved_by: Option<String>,
        /// Process exit code, if available.
        exit_code: Option<i32>,
        /// Whether the command completed without error.
        success: bool,
        /// Wall-clock execution time in milliseconds.
        duration_ms: u64,
    },
    /// Emitted when a privileged command is rejected before execution.
    PrivilegedCommandDenied {
        /// ID of the message that attempted the command.
        message_id: MessageId,
        /// Session the attempt occurred in.
        session_id: SessionId,
        /// The command that was denied.
        command: String,
        /// Arguments that were passed.
        args: Vec<String>,
        /// Reason the command was rejected.
        reason: String,
    },
    /// Principles were retrieved and injected into the system prompt.
    PrinciplesInjected {
        /// Session the principles were injected into.
        session_id: SessionId,
        /// Number of principles injected.
        count: usize,
    },
    /// Post-task reflection completed, producing new or updated principles.
    ReflectionCompleted {
        /// Session the reflection was performed for.
        session_id: SessionId,
        /// Number of principles created or updated.
        principles_created: usize,
        /// ID of the trajectory that was reflected upon.
        trajectory_id: String,
    },
    /// A trajectory was persisted for future offline distillation.
    TrajectoryRecorded {
        /// Session the trajectory was captured from.
        session_id: SessionId,
        /// Unique identifier for the stored trajectory.
        trajectory_id: String,
    },
    /// Offline distillation completed, synthesizing cross-trajectory patterns.
    DistillationCompleted {
        /// Workspace the distillation ran for.
        workspace: String,
        /// Number of principles created or updated by this distillation run.
        principles_created: usize,
    },
    /// Periodic liveness signal from the observe subsystem.
    Heartbeat,
    /// Emitted when an agent hands off or delegates to another agent.
    AgentDelegated {
        /// Graph run identifier.
        run_id: String,
        /// Agent that initiated the delegation.
        source_agent: String,
        /// Agent receiving the delegation.
        target_agent: String,
        /// Delegation mode (e.g. `"handoff"`, `"parallel"`).
        mode: String,
        /// Human-readable reason for the delegation.
        reason: String,
    },
    /// Emitted when a single agent node completes within a graph execution.
    AgentCompleted {
        /// Graph run identifier.
        run_id: String,
        /// ID of the agent that completed.
        agent_id: String,
        /// Number of agentic loop iterations taken.
        iterations: usize,
        /// Total tokens consumed.
        tokens: u64,
        /// Wall-clock duration in milliseconds.
        duration_ms: u64,
        /// Whether the agent completed successfully.
        success: bool,
    },
    /// Emitted when a graph run is paused pending human approval.
    RunInterrupted {
        /// Graph run identifier.
        run_id: String,
        /// ID of the agent that triggered the interrupt.
        agent_id: String,
        /// Name of the tool awaiting approval.
        tool_name: String,
    },
    /// Emitted when the entire graph execution completes.
    GraphCompleted {
        /// Graph run identifier.
        run_id: String,
        /// ID of the graph definition.
        graph_id: String,
        /// IDs of all agents that ran.
        agents_executed: Vec<String>,
        /// Sum of all agent iterations.
        total_iterations: usize,
        /// Sum of all tokens consumed.
        total_tokens: u64,
        /// Total wall-clock duration in milliseconds.
        duration_ms: u64,
    },
    /// Emitted when a new workspace is created via the API.
    WorkspaceCreated {
        /// Name of the workspace.
        name: String,
        /// Agent name configured in SOUL.md, if any.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent_name: Option<String>,
    },
    /// Emitted when a workspace's configuration is updated via the API.
    WorkspaceUpdated {
        /// Name of the workspace.
        name: String,
        /// Top-level fields that were modified (e.g. `"agent_name"`,
        /// `"soul_body"`, `"tools_body"`).
        changed_fields: Vec<String>,
    },
    /// Emitted when a workspace is archived/removed via the API.
    WorkspaceRemoved {
        /// Name of the workspace that was archived.
        name: String,
    },
    /// Emitted when the scheduler fires a due task.
    ScheduleTriggered {
        /// Name of the schedule entry that fired.
        schedule_name: String,
        /// Workspace the schedule belongs to (if any).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace: Option<String>,
        /// Name of the skill invoked, if this is a skill-based schedule.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        skill_name: Option<String>,
    },
}
