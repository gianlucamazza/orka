use std::{collections::HashMap, sync::Arc};

use chrono::{DateTime, Utc};
use orka_contracts::{
    CommandContent, EventContent, InboundInteraction, InteractionContent, MediaAttachment,
    PlatformContext, RichInput, TraceContext,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::traits::{EventSink, SecretManager};

/// Category of error for skill invocations, used by the circuit breaker and
/// self-learning system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    /// Permanent environment error: permissions, missing binary, sandbox,
    /// blocked syscall.
    Environmental,
    /// Invalid input provided by the caller (LLM).
    Input,
    /// Execution timeout.
    Timeout,
    /// Transient error: network, service temporarily unavailable.
    Transient,
    /// Skill output failed semantic validation (hallucinated or schema-invalid
    /// result).
    Semantic,
    /// Skill invocation was blocked by a budget constraint (cost or duration
    /// ceiling).
    Budget,
    /// Category cannot be determined.
    Unknown,
}

/// Unique identifier for a message flowing through the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, utoipa::ToSchema)]
pub struct MessageId(Uuid);

impl MessageId {
    /// Create a new unique message ID (UUID v7).
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    /// Return the underlying UUID.
    pub fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for MessageId {
    fn default() -> Self {
        Self::new()
    }
}

impl From<Uuid> for MessageId {
    fn from(id: Uuid) -> Self {
        Self(id)
    }
}

impl std::fmt::Display for MessageId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for a user session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SessionId(Uuid);

impl SessionId {
    /// Create a new unique session ID (UUID v7).
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    /// Return the underlying UUID.
    pub fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl From<Uuid> for SessionId {
    fn from(id: Uuid) -> Self {
        Self(id)
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for a user-facing conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ConversationId(Uuid);

impl ConversationId {
    /// Create a new unique conversation ID (UUID v7).
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    /// Return the underlying UUID.
    pub fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for ConversationId {
    fn default() -> Self {
        Self::new()
    }
}

impl From<Uuid> for ConversationId {
    fn from(id: Uuid) -> Self {
        Self(id)
    }
}

impl From<ConversationId> for SessionId {
    fn from(id: ConversationId) -> Self {
        Self(id.0)
    }
}

impl From<SessionId> for ConversationId {
    fn from(id: SessionId) -> Self {
        Self(id.0)
    }
}

impl std::fmt::Display for ConversationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for a product-facing conversation artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ArtifactId(Uuid);

impl ArtifactId {
    /// Create a new unique artifact ID (UUID v7).
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    /// Return the underlying UUID.
    pub fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for ArtifactId {
    fn default() -> Self {
        Self::new()
    }
}

impl From<Uuid> for ArtifactId {
    fn from(id: Uuid) -> Self {
        Self(id)
    }
}

impl std::fmt::Display for ArtifactId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for a graph execution run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RunId(Uuid);

impl RunId {
    /// Create a new unique run ID (UUID v7).
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    /// Return the underlying UUID.
    pub fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for RunId {
    fn default() -> Self {
        Self::new()
    }
}

impl From<Uuid> for RunId {
    fn from(id: Uuid) -> Self {
        Self(id)
    }
}

impl std::fmt::Display for RunId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for a domain event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, utoipa::ToSchema)]
pub struct EventId(Uuid);

impl EventId {
    /// Create a new unique event ID (UUID v7).
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    /// Return the underlying UUID.
    pub fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for EventId {
    fn default() -> Self {
        Self::new()
    }
}

impl From<Uuid> for EventId {
    fn from(id: Uuid) -> Self {
        Self(id)
    }
}

impl std::fmt::Display for EventId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

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

/// Context available to skills during execution.
/// Per-invocation budget constraints for a skill execution.
///
/// Fields are all optional — `None` means "no limit". Enforced by the
/// registry before and after `execute()`.
#[derive(Debug, Clone, Default)]
pub struct SkillBudget {
    /// Maximum wall-clock execution time in milliseconds.
    ///
    /// If the skill's measured duration exceeds this value the registry returns
    /// a [`ErrorCategory::Budget`] error after execution (not a hard timeout —
    /// use `AgentConfig::skill_timeout_secs` for hard cancellation).
    pub max_duration_ms: Option<u64>,
    /// Maximum allowed output size in bytes (serialized JSON).
    ///
    /// Prevents oversized skill outputs from flooding the LLM context window.
    pub max_output_bytes: Option<usize>,
}

/// Runtime context provided to a skill during execution.
#[derive(Clone)]
#[non_exhaustive]
pub struct SkillContext {
    /// Provides access to named secrets during skill execution.
    pub secrets: Arc<dyn SecretManager>,
    /// Optional sink for emitting domain events from within a skill.
    pub event_sink: Option<Arc<dyn EventSink>>,
    /// Optional per-invocation budget constraints.
    pub budget: Option<SkillBudget>,
    /// The user's working directory on the client machine, sent via
    /// `workspace:cwd` metadata. OS skills (e.g. `shell_exec`) should use
    /// this as their default CWD when the LLM does not explicitly supply
    /// one, so that commands run in the user's directory rather than the
    /// server process's working directory.
    pub user_cwd: Option<String>,
    /// Active git worktree path for this agent turn. When set, skills that
    /// operate on files or run commands (`shell_exec`, `coding_delegate`,
    /// `git_*`) should prefer this over `user_cwd` so the agent works
    /// inside the isolated worktree automatically, without needing to pass
    /// an explicit `path`/`cwd`/`working_dir` argument on every call.
    ///
    /// Set by the node runner when a `git_worktree_create` call succeeds;
    /// cleared when `git_worktree_remove` is called.
    pub worktree_cwd: Option<String>,
    /// Channel for streaming progress events from long-running skills.
    ///
    /// Used by `coding_delegate` to emit real-time [`DelegateEvent`]s.
    /// The payload is `serde_json::Value` to keep `orka-core` decoupled from
    /// skill-specific types.
    pub progress_tx: Option<tokio::sync::mpsc::UnboundedSender<serde_json::Value>>,
    /// Token checked by skills to support cooperative cancellation.
    pub cancellation_token: Option<tokio_util::sync::CancellationToken>,
}

impl std::fmt::Debug for SkillContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SkillContext").finish()
    }
}

/// Input passed to a skill invocation.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[non_exhaustive]
pub struct SkillInput {
    /// Named arguments passed to the skill, keyed by parameter name.
    pub args: HashMap<String, serde_json::Value>,
    /// Runtime context injected by the worker before invocation.
    #[serde(skip)]
    #[schema(ignore)]
    pub context: Option<SkillContext>,
}

impl SkillInput {
    /// Get a required string argument, returning a `Skill` error if missing or
    /// not a string.
    pub fn get_string(&self, key: &str) -> crate::Result<&str> {
        self.args
            .get(key)
            .and_then(|v| v.as_str())
            .ok_or_else(|| crate::Error::Skill(format!("{key} is required")))
    }

    /// Get an optional string argument.
    pub fn get_optional_string(&self, key: &str) -> Option<&str> {
        self.args.get(key).and_then(|v| v.as_str())
    }

    /// Get a required i64 argument.
    pub fn get_i64(&self, key: &str) -> crate::Result<i64> {
        self.args
            .get(key)
            .and_then(serde_json::Value::as_i64)
            .ok_or_else(|| crate::Error::Skill(format!("{key} is required")))
    }

    /// Get a required bool argument.
    pub fn get_bool(&self, key: &str) -> crate::Result<bool> {
        self.args
            .get(key)
            .and_then(serde_json::Value::as_bool)
            .ok_or_else(|| crate::Error::Skill(format!("{key} is required")))
    }

    /// Resolve a path string against the user's CWD from context.
    ///
    /// - Relative paths are joined onto `user_cwd` when set.
    /// - Paths starting with `~/` or equal to `~` are treated as relative to
    ///   `user_cwd` (not the server process home), so LLM-generated tilde paths
    ///   land in the user's working directory rather than the server's `$HOME`.
    /// - Absolute paths without a tilde are returned as-is.
    pub fn resolve_path(&self, path: &str) -> std::path::PathBuf {
        let cwd = self.context.as_ref().and_then(|c| c.user_cwd.as_deref());

        // Expand leading `~` relative to user_cwd when available.
        let tilde_rest = path
            .strip_prefix("~/")
            .or_else(|| if path == "~" { Some("") } else { None });
        if let (Some(rest), Some(dir)) = (tilde_rest, cwd) {
            return std::path::PathBuf::from(dir).join(rest);
        }

        let p = std::path::Path::new(path);
        if p.is_relative()
            && let Some(dir) = cwd
        {
            return std::path::PathBuf::from(dir).join(p);
        }
        p.to_path_buf()
    }
}

/// Output returned from a skill invocation.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[non_exhaustive]
pub struct SkillOutput {
    /// Structured result value produced by the skill.
    pub data: serde_json::Value,
    /// Media attachments produced alongside the text result (e.g. generated
    /// charts). These are forwarded as separate `Payload::Media` messages
    /// to the channel adapter.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<MediaPayload>,
}

/// JSON Schema describing a skill's parameters.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[non_exhaustive]
pub struct SkillSchema {
    /// JSON Schema object describing the skill's accepted parameters.
    pub parameters: serde_json::Value,
}

/// Product-facing rich input payload combining text with media attachments.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RichInputPayload {
    /// Optional user-authored text accompanying the attachments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Media attachments submitted in the same turn.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<MediaPayload>,
}

/// Message priority for queue routing.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    utoipa::ToSchema,
)]
#[non_exhaustive]
pub enum Priority {
    /// Lowest priority; processed after Normal and Urgent messages.
    Background = 0,
    /// Default priority for standard messages.
    #[default]
    Normal = 1,
    /// Highest priority, used for direct messages and time-sensitive work.
    Urgent = 2,
}

/// Message payload variants.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[non_exhaustive]
#[serde(tag = "type", content = "data")]
pub enum Payload {
    /// Plain text message content.
    Text(String),
    /// Rich user input combining text and attachments in a single turn.
    RichInput(RichInputPayload),
    /// File or media attachment.
    Media(MediaPayload),
    /// Structured slash command from a user or internal system.
    Command(CommandPayload),
    /// Internal system or lifecycle event.
    Event(EventPayload),
}

/// Media attachment info.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[non_exhaustive]
pub struct MediaPayload {
    /// MIME type of the media content (e.g. `image/png`, `audio/ogg`).
    pub mime_type: String,
    /// URL or path where the media can be retrieved. Empty for inline payloads.
    pub url: String,
    /// Suggested filename when materialized as a file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    /// Optional human-readable description of the media.
    pub caption: Option<String>,
    /// File size in bytes, if known.
    pub size_bytes: Option<u64>,
    /// Inline media data encoded as standard base64. When present, adapters use
    /// this directly (multipart upload) instead of fetching from `url`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_base64: Option<String>,
}

/// Structured command from a channel or internal system.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[non_exhaustive]
pub struct CommandPayload {
    /// The command name (without the leading slash).
    pub name: String,
    /// Named parameters parsed from the command invocation.
    pub args: HashMap<String, serde_json::Value>,
}

/// Unified command arguments produced by any adapter.
///
/// Both structured adapters (Discord slash commands with typed options) and
/// text-based adapters (Telegram `bot_command` entity) normalise their input
/// into this type.  This eliminates the round-trip `CommandPayload → text →
/// re-parse` that previously happened in the worker.
///
/// Constructed via [`From<CommandPayload>`] or [`From<crate::ParsedCommand>`].
#[derive(Debug, Clone, Default)]
pub struct CommandArgs {
    /// Positional tokens: everything that is *not* a `key=value` pair.
    positional: Vec<String>,
    /// Named parameters parsed from `key=value` tokens.
    named: HashMap<String, serde_json::Value>,
    /// The raw text argument string, if available (used by [`Self::text`]).
    raw: Option<String>,
}

impl CommandArgs {
    /// All positional argument tokens.
    pub fn positional_args(&self) -> &[String] {
        &self.positional
    }

    /// The n-th positional argument, or `None` if out of range.
    pub fn positional(&self, i: usize) -> Option<&str> {
        self.positional.get(i).map(String::as_str)
    }

    /// The raw text following the command name, or `None` if there were no
    /// arguments.
    ///
    /// Equivalent to all positional tokens joined by a single space when no raw
    /// string was preserved.
    pub fn text(&self) -> Option<&str> {
        if self.positional.is_empty() && self.named.is_empty() {
            return None;
        }
        self.raw.as_deref()
    }

    /// A named argument value, or `None` if not present.
    pub fn named(&self, key: &str) -> Option<&serde_json::Value> {
        self.named.get(key)
    }

    /// Iterate over all named `(key, value)` pairs.
    pub fn named_iter(&self) -> impl Iterator<Item = (&str, &serde_json::Value)> {
        self.named.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// `true` if there are no positional or named arguments.
    pub fn is_empty(&self) -> bool {
        self.positional.is_empty() && self.named.is_empty()
    }
}

/// System or lifecycle event.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[non_exhaustive]
pub struct EventPayload {
    /// Short string identifier for the event type.
    pub kind: String,
    /// Arbitrary structured payload for the event.
    pub data: serde_json::Value,
}

/// Universal message envelope that flows through the entire system.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[non_exhaustive]
pub struct Envelope {
    /// Unique message ID (UUID v7).
    pub id: MessageId,
    /// Source/destination channel identifier.
    pub channel: String,
    /// Session this message belongs to.
    pub session_id: SessionId,
    /// When the message was created.
    pub timestamp: DateTime<Utc>,
    /// Routing priority for the message queue.
    pub priority: Priority,
    /// The message content.
    pub payload: Payload,
    /// Adapter-specific and routing metadata.
    pub metadata: HashMap<String, serde_json::Value>,
    /// Distributed tracing propagation headers.
    pub trace_context: TraceContext,
    /// Canonical platform context produced by the adapter.
    ///
    /// Replaces the scattered platform-specific metadata keys
    /// (`telegram_chat_id`, `slack_channel`, etc.) with a typed, two-level
    /// model. Only the originating adapter writes `extensions`; shared code
    /// reads only the canonical fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_context: Option<PlatformContext>,
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

impl SkillContext {
    /// Create a new skill context without a budget constraint.
    pub fn new(secrets: Arc<dyn SecretManager>, event_sink: Option<Arc<dyn EventSink>>) -> Self {
        Self {
            secrets,
            event_sink,
            budget: None,
            user_cwd: None,
            worktree_cwd: None,
            progress_tx: None,
            cancellation_token: None,
        }
    }

    /// Attach a [`SkillBudget`] to this context.
    #[must_use]
    pub fn with_budget(mut self, budget: SkillBudget) -> Self {
        self.budget = Some(budget);
        self
    }

    /// Set the user's working directory (from `workspace:cwd` envelope
    /// metadata).
    #[must_use]
    pub fn with_user_cwd(mut self, cwd: Option<String>) -> Self {
        self.user_cwd = cwd;
        self
    }

    /// Set the active git worktree path. Skills that operate on files or run
    /// commands will prefer this over `user_cwd` when set.
    #[must_use]
    pub fn with_worktree_cwd(mut self, cwd: Option<String>) -> Self {
        self.worktree_cwd = cwd;
        self
    }

    /// Attach a progress channel for streaming delegate events.
    #[must_use]
    pub fn with_progress(
        mut self,
        tx: tokio::sync::mpsc::UnboundedSender<serde_json::Value>,
    ) -> Self {
        self.progress_tx = Some(tx);
        self
    }

    /// Attach a cancellation token for cooperative cancellation.
    #[must_use]
    pub fn with_cancellation(mut self, token: tokio_util::sync::CancellationToken) -> Self {
        self.cancellation_token = Some(token);
        self
    }
}

impl SkillOutput {
    /// Create a new skill output.
    pub fn new(data: serde_json::Value) -> Self {
        Self {
            data,
            attachments: Vec::new(),
        }
    }

    /// Attach media payloads to be forwarded alongside the text response.
    #[must_use]
    pub fn with_attachments(mut self, attachments: Vec<MediaPayload>) -> Self {
        self.attachments = attachments;
        self
    }
}

impl SkillSchema {
    /// Create a new skill schema.
    pub fn new(parameters: serde_json::Value) -> Self {
        Self { parameters }
    }
}

impl SkillInput {
    /// Create a new skill input with the given arguments.
    pub fn new(args: HashMap<String, serde_json::Value>) -> Self {
        Self {
            args,
            context: None,
        }
    }

    /// Set the skill context.
    #[must_use]
    pub fn with_context(mut self, context: SkillContext) -> Self {
        self.context = Some(context);
        self
    }
}

impl MediaPayload {
    /// Create a new media payload referencing an external URL.
    pub fn new(mime_type: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            mime_type: mime_type.into(),
            url: url.into(),
            filename: None,
            caption: None,
            size_bytes: None,
            data_base64: None,
        }
    }

    /// Create an inline media payload from raw bytes.
    ///
    /// The data is base64-encoded and stored in `data_base64`; `url` is left
    /// empty. Adapters that support multipart upload use the inline data
    /// directly without making an outbound HTTP request.
    pub fn inline(
        mime_type: impl Into<String>,
        data: Vec<u8>,
        caption: impl Into<Option<String>>,
    ) -> Self {
        use base64::Engine as _;
        let size = data.len() as u64;
        Self {
            mime_type: mime_type.into(),
            url: String::new(),
            filename: None,
            caption: caption.into(),
            size_bytes: Some(size),
            data_base64: Some(base64::engine::general_purpose::STANDARD.encode(data)),
        }
    }

    /// Decode the inline base64 data, if present.
    pub fn decode_data(&self) -> Option<Vec<u8>> {
        use base64::Engine as _;
        self.data_base64
            .as_deref()
            .and_then(|s| base64::engine::general_purpose::STANDARD.decode(s).ok())
    }

    /// Set the suggested filename for this payload.
    #[must_use]
    pub fn with_filename(mut self, filename: impl Into<String>) -> Self {
        self.filename = Some(filename.into());
        self
    }
}

impl CommandPayload {
    /// Create a new command payload.
    pub fn new(name: impl Into<String>, args: HashMap<String, serde_json::Value>) -> Self {
        Self {
            name: name.into(),
            args,
        }
    }
}

impl EventPayload {
    /// Create a new event payload.
    pub fn new(kind: impl Into<String>, data: serde_json::Value) -> Self {
        Self {
            kind: kind.into(),
            data,
        }
    }
}

impl OutboundMessage {
    /// Create a new text outbound message.
    pub fn text(
        channel: impl Into<String>,
        session_id: SessionId,
        text: impl Into<String>,
        reply_to: Option<MessageId>,
    ) -> Self {
        Self {
            channel: channel.into(),
            session_id,
            payload: Payload::Text(text.into()),
            reply_to,
            metadata: HashMap::new(),
            platform_context: None,
        }
    }

    /// Create a new outbound message with the given payload.
    pub fn new(
        channel: impl Into<String>,
        session_id: SessionId,
        payload: Payload,
        reply_to: Option<MessageId>,
    ) -> Self {
        Self {
            channel: channel.into(),
            session_id,
            payload,
            reply_to,
            metadata: HashMap::new(),
            platform_context: None,
        }
    }
}

impl MemoryEntry {
    /// Create a new memory entry.
    pub fn new(key: impl Into<String>, value: serde_json::Value) -> Self {
        let now = Utc::now();
        Self {
            key: key.into(),
            kind: MemoryKind::Working,
            scope: MemoryScope::Session,
            source: "system".into(),
            value,
            metadata: HashMap::new(),
            created_at: now,
            updated_at: now,
            tags: Vec::new(),
        }
    }

    /// Create a working-memory entry.
    pub fn working(key: impl Into<String>, value: serde_json::Value) -> Self {
        Self::new(key, value).with_kind(MemoryKind::Working)
    }

    /// Create an episodic-memory entry.
    pub fn episodic(key: impl Into<String>, value: serde_json::Value) -> Self {
        Self::new(key, value).with_kind(MemoryKind::Episodic)
    }

    /// Create a semantic-memory entry.
    pub fn semantic(key: impl Into<String>, value: serde_json::Value) -> Self {
        Self::new(key, value).with_kind(MemoryKind::Semantic)
    }

    /// Create a procedural-memory entry.
    pub fn procedural(key: impl Into<String>, value: serde_json::Value) -> Self {
        Self::new(key, value).with_kind(MemoryKind::Procedural)
    }

    /// Set the memory kind for this entry.
    #[must_use]
    pub fn with_kind(mut self, kind: MemoryKind) -> Self {
        self.kind = kind;
        self
    }

    /// Set the memory scope for this entry.
    #[must_use]
    pub fn with_scope(mut self, scope: MemoryScope) -> Self {
        self.scope = scope;
        self
    }

    /// Set the source marker for this entry.
    #[must_use]
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = source.into();
        self
    }

    /// Replace the metadata map for this entry.
    #[must_use]
    pub fn with_metadata(mut self, metadata: HashMap<String, String>) -> Self {
        self.metadata = metadata;
        self
    }

    /// Set tags on this entry.
    #[must_use]
    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }
}

impl Envelope {
    /// Insert a metadata key-value pair.
    pub fn insert_meta(&mut self, key: impl Into<String>, value: impl Into<serde_json::Value>) {
        self.metadata.insert(key.into(), value.into());
    }

    /// Create a text envelope with default priority and no metadata.
    pub fn text(
        channel: impl Into<String>,
        session_id: SessionId,
        text: impl Into<String>,
    ) -> Self {
        Self {
            id: MessageId::new(),
            channel: channel.into(),
            session_id,
            timestamp: Utc::now(),
            priority: Priority::default(),
            payload: Payload::Text(text.into()),
            metadata: HashMap::new(),
            trace_context: TraceContext::default(),
            platform_context: None,
        }
    }

    /// Create an envelope with an arbitrary payload, preserving priority and
    /// trace context from a source envelope.
    pub fn with_payload(
        channel: impl Into<String>,
        session_id: SessionId,
        payload: Payload,
        source: &Envelope,
    ) -> Self {
        Self {
            id: MessageId::new(),
            channel: channel.into(),
            session_id,
            timestamp: Utc::now(),
            priority: source.priority,
            payload,
            metadata: HashMap::new(),
            trace_context: source.trace_context.clone(),
            platform_context: source.platform_context.clone(),
        }
    }
}

/// Convert an [`InboundInteraction`] into an [`Envelope`] for the message bus.
///
/// This is the single conversion boundary between the public adapter contract
/// and the internal wire format. Called by the bridge task in `orka-server`.
impl From<InboundInteraction> for Envelope {
    fn from(interaction: InboundInteraction) -> Self {
        let payload = match interaction.content {
            InteractionContent::Text(text) => Payload::Text(text),
            InteractionContent::RichInput(RichInput { text, attachments }) => {
                Payload::RichInput(RichInputPayload {
                    text,
                    attachments: attachments
                        .into_iter()
                        .map(|a| MediaPayload {
                            mime_type: a.mime_type,
                            url: a.url,
                            filename: a.filename,
                            caption: a.caption,
                            size_bytes: a.size_bytes,
                            data_base64: a.data_base64,
                        })
                        .collect(),
                })
            }
            InteractionContent::Media(MediaAttachment {
                mime_type,
                url,
                filename,
                caption,
                size_bytes,
                data_base64,
            }) => Payload::Media(MediaPayload {
                mime_type,
                url,
                filename,
                caption,
                size_bytes,
                data_base64,
            }),
            InteractionContent::Command(CommandContent { name, args }) => {
                Payload::Command(CommandPayload { name, args })
            }
            InteractionContent::Event(EventContent { kind, data }) => {
                Payload::Event(EventPayload { kind, data })
            }
            // `InteractionContent` is `#[non_exhaustive]`; future variants fall
            // back to an empty text payload so the envelope is never silently
            // dropped.
            _ => Payload::Text(String::new()),
        };

        Self {
            id: MessageId::from(interaction.id),
            channel: interaction.source_channel,
            session_id: SessionId::from(interaction.session_id),
            timestamp: interaction.timestamp,
            priority: Priority::default(),
            payload,
            metadata: HashMap::new(),
            trace_context: interaction.trace,
            platform_context: Some(interaction.context),
        }
    }
}

/// Outbound message sent back to a channel.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[non_exhaustive]
pub struct OutboundMessage {
    /// Destination channel to deliver the message to.
    pub channel: String,
    /// Session this reply belongs to.
    pub session_id: SessionId,
    /// The outbound message content.
    pub payload: Payload,
    /// Optional ID of the inbound message being replied to.
    pub reply_to: Option<MessageId>,
    /// Adapter-specific delivery metadata (legacy; prefer `platform_context`).
    pub metadata: HashMap<String, serde_json::Value>,
    /// Canonical platform context for routing.
    ///
    /// Set by the worker from the inbound envelope's `platform_context`.
    /// Adapters should read routing information from here first and fall back
    /// to `metadata` for backward compatibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_context: Option<PlatformContext>,
}

impl OutboundMessage {
    /// Return the canonical chat / channel identifier from `platform_context`.
    ///
    /// This is the authoritative routing key for all adapters.  Every outbound
    /// message that travels through the standard worker→adapter pipeline will
    /// have `platform_context` populated from the originating inbound envelope,
    /// so adapters should call this instead of reading the legacy metadata bag.
    pub fn chat_id(&self) -> crate::Result<&str> {
        self.platform_context
            .as_ref()
            .and_then(|pc| pc.chat_id.as_deref())
            .ok_or_else(|| crate::Error::Other("missing platform_context.chat_id".into()))
    }

    /// Return a platform-specific extension value as `i64`.
    ///
    /// Extensions use the `{platform}_{field}` naming convention, e.g.
    /// `telegram_message_id`.  Returns `None` if the key is absent or is not
    /// an integer.
    pub fn extension_i64(&self, key: &str) -> Option<i64> {
        self.platform_context
            .as_ref()
            .and_then(|pc| pc.extensions.get(key))
            .and_then(serde_json::Value::as_i64)
    }

    /// Return a platform-specific extension value as `&str`.
    ///
    /// Returns `None` if the key is absent or is not a string.
    pub fn extension_str(&self, key: &str) -> Option<&str> {
        self.platform_context
            .as_ref()
            .and_then(|pc| pc.extensions.get(key))
            .and_then(serde_json::Value::as_str)
    }

    /// Set `source_channel` in metadata and return self (builder-style).
    #[must_use]
    pub fn with_source_channel(mut self, channel: &str) -> Self {
        self.metadata.insert(
            "source_channel".into(),
            serde_json::Value::String(channel.into()),
        );
        self
    }
}

/// A stored session with associated state.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[non_exhaustive]
pub struct Session {
    /// Unique session identifier.
    pub id: SessionId,
    /// Channel this session is associated with.
    pub channel: String,
    /// Platform-specific user identifier.
    pub user_id: String,
    /// When the session was first opened.
    pub created_at: DateTime<Utc>,
    /// When the session was last modified.
    pub updated_at: DateTime<Utc>,
    /// Arbitrary key-value scratchpad for handler and skill state.
    pub state: HashMap<String, serde_json::Value>,
}

impl Session {
    /// Create a new session for the given channel and user.
    pub fn new(channel: impl Into<String>, user_id: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: SessionId::new(),
            channel: channel.into(),
            user_id: user_id.into(),
            created_at: now,
            updated_at: now,
            state: HashMap::new(),
        }
    }

    /// Read a value from the shared scratchpad.
    pub fn scratchpad_get(&self, key: &str) -> Option<&serde_json::Value> {
        self.state.get("scratchpad").and_then(|sp| sp.get(key))
    }

    /// Write a value to the shared scratchpad.
    pub fn scratchpad_set(&mut self, key: impl Into<String>, value: serde_json::Value) {
        let scratchpad = self
            .state
            .entry("scratchpad".to_string())
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        if let serde_json::Value::Object(map) = scratchpad {
            map.insert(key.into(), value);
        }
    }
}

/// Lifecycle state for a product-facing conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConversationStatus {
    /// Conversation is active and can accept more messages.
    #[default]
    Active,
    /// Conversation is paused waiting for a human or external decision.
    Interrupted,
    /// Conversation completed with an error state.
    Failed,
}

/// Product-facing conversation metadata used by mobile clients.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[non_exhaustive]
pub struct Conversation {
    /// Stable conversation identifier.
    pub id: ConversationId,
    /// Runtime session backing this conversation.
    pub session_id: SessionId,
    /// Owning authenticated user.
    pub user_id: String,
    /// Human-readable conversation title.
    pub title: String,
    /// Preview of the most recent user-facing message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message_preview: Option<String>,
    /// Lifecycle state shown to clients.
    pub status: ConversationStatus,
    /// When this conversation was archived, or `None` if active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<DateTime<Utc>>,
    /// Whether the conversation is pinned by the user.
    #[serde(default)]
    pub pinned: bool,
    /// User-defined labels attached to this conversation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Workspace this conversation is bound to, or `None` for the server
    /// default workspace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// Conversation creation time.
    pub created_at: DateTime<Utc>,
    /// Last update time.
    pub updated_at: DateTime<Utc>,
}

/// Source of a conversation artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConversationArtifactOrigin {
    /// Uploaded by the end user from the client device.
    UserUpload,
    /// Produced by Orka during assistant execution.
    AssistantOutput,
}

/// Product-facing artifact metadata associated with a conversation.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[non_exhaustive]
pub struct ConversationArtifact {
    /// Stable artifact identifier.
    pub id: ArtifactId,
    /// Owning authenticated user.
    pub owner_user_id: String,
    /// Owning conversation, if already attached to one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<ConversationId>,
    /// Owning message, if already attached to one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<MessageId>,
    /// Where the artifact originated.
    pub origin: ConversationArtifactOrigin,
    /// MIME type of the content.
    pub mime_type: String,
    /// Suggested filename for downloads and previews.
    pub filename: String,
    /// Optional caption or user-provided note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    /// Byte size, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    /// Width in pixels for visual media, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    /// Height in pixels for visual media, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// Duration in milliseconds for timed media, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Artifact creation time.
    pub created_at: DateTime<Utc>,
}

impl ConversationArtifact {
    /// Create a new conversation artifact metadata record.
    pub fn new(
        owner_user_id: impl Into<String>,
        origin: ConversationArtifactOrigin,
        mime_type: impl Into<String>,
        filename: impl Into<String>,
    ) -> Self {
        Self {
            id: ArtifactId::new(),
            owner_user_id: owner_user_id.into(),
            conversation_id: None,
            message_id: None,
            origin,
            mime_type: mime_type.into(),
            filename: filename.into(),
            caption: None,
            size_bytes: None,
            width: None,
            height: None,
            duration_ms: None,
            created_at: Utc::now(),
        }
    }
}

impl Conversation {
    /// Create a new active conversation.
    pub fn new(
        id: ConversationId,
        session_id: SessionId,
        user_id: impl Into<String>,
        title: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id,
            session_id,
            user_id: user_id.into(),
            title: title.into(),
            last_message_preview: None,
            status: ConversationStatus::Active,
            archived_at: None,
            pinned: false,
            tags: Vec::new(),
            workspace: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Bind this conversation to a named workspace.
    #[must_use]
    pub fn with_workspace(mut self, workspace: impl Into<String>) -> Self {
        self.workspace = Some(workspace.into());
        self
    }
}

/// Role of a transcript message in a product-facing conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConversationMessageRole {
    /// End-user authored message.
    User,
    /// Assistant authored message.
    Assistant,
}

/// Delivery state of a transcript message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConversationMessageStatus {
    /// Message has been accepted but not finalized yet.
    Pending,
    /// Message has been fully committed.
    Completed,
    /// Message generation failed.
    Failed,
}

/// Product-facing transcript message for mobile clients.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[non_exhaustive]
pub struct ConversationMessage {
    /// Stable message identifier.
    pub id: MessageId,
    /// Owning conversation.
    pub conversation_id: ConversationId,
    /// Runtime session backing this message.
    pub session_id: SessionId,
    /// User-visible role.
    pub role: ConversationMessageRole,
    /// Message text content.
    pub text: String,
    /// Associated artifacts for this message.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ConversationArtifact>,
    /// Persistence / delivery state.
    pub status: ConversationMessageStatus,
    /// Message creation time.
    pub created_at: DateTime<Utc>,
    /// Finalization time when completed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finalized_at: Option<DateTime<Utc>>,
}

impl ConversationMessage {
    /// Create a completed user-facing message.
    pub fn new(
        id: MessageId,
        conversation_id: ConversationId,
        session_id: SessionId,
        role: ConversationMessageRole,
        text: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id,
            conversation_id,
            session_id,
            role,
            text: text.into(),
            artifacts: Vec::new(),
            status: ConversationMessageStatus::Completed,
            created_at: now,
            finalized_at: Some(now),
        }
    }
}

/// Functional class of a memory record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    /// Active thread/session state and rolling conversation context.
    Working,
    /// Summaries, handoffs, and decisions distilled from completed turns.
    Episodic,
    /// Durable facts retrievable by semantic relevance.
    Semantic,
    /// Learned heuristics and principles.
    Procedural,
}

impl std::fmt::Display for MemoryKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Working => "working",
            Self::Episodic => "episodic",
            Self::Semantic => "semantic",
            Self::Procedural => "procedural",
        };
        f.write_str(s)
    }
}

/// Visibility and retention scope for a memory record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MemoryScope {
    /// Only valid for the current session/thread.
    Session,
    /// Shared across a workspace.
    Workspace,
    /// Shared across sessions for a single user.
    User,
    /// Globally applicable.
    Global,
}

impl std::fmt::Display for MemoryScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Session => "session",
            Self::Workspace => "workspace",
            Self::User => "user",
            Self::Global => "global",
        };
        f.write_str(s)
    }
}

impl std::str::FromStr for MemoryScope {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "session" => Ok(Self::Session),
            "workspace" => Ok(Self::Workspace),
            "user" => Ok(Self::User),
            "global" => Ok(Self::Global),
            _ => Err("invalid memory scope"),
        }
    }
}

/// An entry in the memory store.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[non_exhaustive]
pub struct MemoryEntry {
    /// Lookup key for this entry within the memory store.
    pub key: String,
    /// Functional class of this memory record.
    pub kind: MemoryKind,
    /// Visibility/retention scope for this record.
    pub scope: MemoryScope,
    /// Origin of the memory record (e.g. `system`, `user`, `experience`).
    pub source: String,
    /// Stored value as a JSON document.
    pub value: serde_json::Value,
    /// Structured metadata for filtering and introspection.
    pub metadata: HashMap<String, String>,
    /// When this entry was first written.
    pub created_at: DateTime<Utc>,
    /// When this entry was last modified.
    pub updated_at: DateTime<Utc>,
    /// Optional labels for grouping and filtering entries.
    pub tags: Vec<String>,
}

/// Opaque secret value, securely zeroized on drop.
///
/// Intentionally not `Clone` to prevent accidental copies of secrets
/// scattered across the heap. Use [`SecretValue::to_owned_secret`] for
/// explicit, deliberate copies.
pub struct SecretValue(zeroize::Zeroizing<Vec<u8>>);

impl SecretValue {
    /// Wrap raw bytes as a secret value.
    pub fn new(value: impl Into<Vec<u8>>) -> Self {
        Self(zeroize::Zeroizing::new(value.into()))
    }

    /// Access the raw secret bytes.
    pub fn expose(&self) -> &[u8] {
        &self.0
    }

    /// Access the secret as a UTF-8 string, if valid.
    pub fn expose_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.0).ok()
    }

    /// Create an explicit copy of the secret. Prefer passing references
    /// instead of cloning to minimize secret copies in memory.
    #[must_use]
    pub fn to_owned_secret(&self) -> Self {
        Self(zeroize::Zeroizing::new(self.0.to_vec()))
    }
}

impl std::fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[REDACTED]")
    }
}

/// A zeroize-on-drop wrapper for secret strings (API keys, tokens, passwords).
///
/// Intentionally not `Clone` to prevent accidental copies of secrets
/// scattered across the heap. Use [`SecretStr::to_owned_secret`] for
/// explicit, deliberate copies.
pub struct SecretStr(zeroize::Zeroizing<String>);

impl SecretStr {
    /// Wrap a string as a secret.
    pub fn new(value: impl Into<String>) -> Self {
        Self(zeroize::Zeroizing::new(value.into()))
    }

    /// Access the secret string.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Create an explicit copy of the secret. Prefer passing references
    /// instead of cloning to minimize secret copies in memory.
    #[must_use]
    pub fn to_owned_secret(&self) -> Self {
        Self(self.0.clone())
    }
}

impl std::fmt::Debug for SecretStr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[REDACTED]")
    }
}

/// Type alias for the message sink passed to channel adapters.
///
/// Deprecated by [`InteractionSink`]; kept for internal use by the bus/worker.
pub type MessageSink = tokio::sync::mpsc::Sender<Envelope>;

/// Type alias for the interaction sink passed to channel adapters.
///
/// Adapters produce [`orka_contracts::InboundInteraction`] and send it to this
/// sink. The bridge in `orka-server` converts to [`Envelope`] for the bus.
pub type InteractionSink = tokio::sync::mpsc::Sender<orka_contracts::InboundInteraction>;

/// Type alias for the message stream returned by the bus.
pub type MessageStream = tokio::sync::mpsc::Receiver<Envelope>;

/// Shared map from session ID to active generation cancellation token.
///
/// The worker registers a token before each dispatch; the `/cancel` endpoint
/// uses it to abort an in-progress generation without stopping the worker.
pub type SessionCancelTokens = std::sync::Arc<
    std::sync::Mutex<std::collections::HashMap<SessionId, tokio_util::sync::CancellationToken>>,
>;

/// Exponential backoff delay with full jitter, capped at `max_secs`.
///
/// Computes a ceiling of `base_secs * 2^attempt` (capped at `max_secs`), then
/// returns a duration in `[0, ceiling]` using subsecond system-clock entropy.
/// This prevents thundering-herd retry storms when multiple workers fail
/// simultaneously, without requiring an external PRNG dependency.
pub fn backoff_delay(attempt: u32, base_secs: u64, max_secs: u64) -> std::time::Duration {
    let secs = base_secs.saturating_mul(1u64.checked_shl(attempt).unwrap_or(u64::MAX));
    let ceiling = secs.min(max_secs);
    let jittered = if ceiling > 0 {
        let nanos = u64::from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos(),
        );
        nanos % (ceiling + 1)
    } else {
        0
    };
    std::time::Duration::from_secs(jittered)
}

// --- CommandArgs conversions ---

fn split_args(tokens: Vec<String>) -> (Vec<String>, HashMap<String, serde_json::Value>) {
    let mut positional = Vec::new();
    let mut named = HashMap::new();
    for token in tokens {
        if let Some((k, v)) = token.split_once('=') {
            // Try to parse as JSON first (handles numbers, booleans, null); fall back to
            // string.
            let value = serde_json::from_str(v)
                .unwrap_or_else(|_| serde_json::Value::String(v.to_string()));
            named.insert(k.to_string(), value);
        } else {
            positional.push(token);
        }
    }
    (positional, named)
}

impl From<CommandPayload> for CommandArgs {
    fn from(cmd: CommandPayload) -> Self {
        // Telegram (and similar text-based adapters) puts the raw trailing text in
        // args["text"]. Discord and structured adapters put typed values
        // directly into the args map under their parameter names.
        if let Some(raw_text) = cmd.args.get("text").and_then(|v| v.as_str()) {
            let raw = raw_text.to_string();
            let tokens = crate::slash_command::tokenize(raw_text);
            let (positional, named) = split_args(tokens);
            Self {
                positional,
                named,
                raw: Some(raw),
            }
        } else {
            // Structured adapter (Discord): all args are named.
            Self {
                positional: Vec::new(),
                named: cmd.args,
                raw: None,
            }
        }
    }
}

impl From<crate::ParsedCommand> for CommandArgs {
    fn from(cmd: crate::ParsedCommand) -> Self {
        // raw is everything after the command name in the original input.
        let raw_text: String = cmd
            .raw
            .trim_start_matches('/')
            .split_once(char::is_whitespace)
            .map(|(_, rest)| rest.trim().to_string())
            .unwrap_or_default();
        let raw = if raw_text.is_empty() {
            None
        } else {
            Some(raw_text)
        };
        let (positional, named) = split_args(cmd.args);
        Self {
            positional,
            named,
            raw,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, time::Duration};

    use serde_json::json;

    use super::*;

    // --- SkillInput accessors ---

    fn ok<T, E: std::fmt::Display>(result: Result<T, E>) -> T {
        match result {
            Ok(value) => value,
            Err(error) => panic!("expected Ok(..), got Err({error})"),
        }
    }

    fn some<'a, T>(value: Option<&'a T>, label: &str) -> &'a T {
        match value {
            Some(value) => value,
            None => panic!("expected {label} to be present"),
        }
    }

    fn make_input(args: serde_json::Value) -> SkillInput {
        let map: HashMap<String, serde_json::Value> =
            serde_json::from_value(args).unwrap_or_default();
        SkillInput::new(map)
    }

    #[test]
    fn skill_input_get_string_present() {
        let input = make_input(json!({"name": "alice"}));
        assert_eq!(ok(input.get_string("name")), "alice");
    }

    #[test]
    fn skill_input_get_string_missing() {
        let input = make_input(json!({}));
        assert!(input.get_string("name").is_err());
    }

    #[test]
    fn skill_input_get_string_wrong_type() {
        let input = make_input(json!({"name": 42}));
        assert!(input.get_string("name").is_err());
    }

    #[test]
    fn skill_input_get_optional_string_present() {
        let input = make_input(json!({"x": "hello"}));
        assert_eq!(input.get_optional_string("x"), Some("hello"));
    }

    #[test]
    fn skill_input_get_optional_string_missing() {
        let input = make_input(json!({}));
        assert_eq!(input.get_optional_string("x"), None);
    }

    #[test]
    fn skill_input_get_i64_and_get_bool() {
        let input = make_input(json!({"count": 7, "flag": true}));
        assert_eq!(ok(input.get_i64("count")), 7);
        assert!(ok(input.get_bool("flag")));
        assert!(input.get_i64("missing").is_err());
        assert!(input.get_bool("missing").is_err());
    }

    // --- OutboundMessage platform_context helpers ---

    fn make_outbound_with_chat_id(chat_id: &str) -> OutboundMessage {
        use orka_contracts::platform::{PlatformContext, SenderInfo};
        let mut msg = OutboundMessage::text("ch", SessionId::new(), "hi", None);
        msg.platform_context = Some(PlatformContext {
            chat_id: Some(chat_id.into()),
            sender: SenderInfo::default(),
            ..Default::default()
        });
        msg
    }

    #[test]
    fn chat_id_present() {
        let msg = make_outbound_with_chat_id("123");
        assert_eq!(ok(msg.chat_id()), "123");
    }

    #[test]
    fn chat_id_missing_errors() {
        let msg = OutboundMessage::text("ch", SessionId::new(), "hi", None);
        assert!(msg.chat_id().is_err());
    }

    #[test]
    fn extension_i64_present_and_missing() {
        use orka_contracts::platform::{PlatformContext, SenderInfo};
        let mut msg = OutboundMessage::text("ch", SessionId::new(), "hi", None);
        let mut pc = PlatformContext {
            sender: SenderInfo::default(),
            ..Default::default()
        };
        pc.extensions
            .insert("telegram_message_id".into(), json!(42));
        msg.platform_context = Some(pc);
        assert_eq!(msg.extension_i64("telegram_message_id"), Some(42));
        assert_eq!(msg.extension_i64("nope"), None);
    }

    #[test]
    fn with_source_channel_sets_metadata() {
        let msg = OutboundMessage::text("ch", SessionId::new(), "hi", None)
            .with_source_channel("telegram");
        assert_eq!(
            some(msg.metadata.get("source_channel"), "source_channel").as_str(),
            Some("telegram")
        );
    }

    #[test]
    fn with_source_channel_overwrites_existing() {
        let mut msg = OutboundMessage::text("ch", SessionId::new(), "hi", None);
        msg.metadata.insert("source_channel".into(), json!("old"));
        let msg = msg.with_source_channel("new");
        assert_eq!(
            some(msg.metadata.get("source_channel"), "source_channel").as_str(),
            Some("new")
        );
    }

    // --- Envelope and MemoryEntry builders ---

    #[test]
    fn envelope_text_creates_text_payload() {
        let sid = SessionId::new();
        let env = Envelope::text("telegram", sid, "hello world");
        assert_eq!(env.channel, "telegram");
        assert_eq!(env.session_id, sid);
        assert!(matches!(env.payload, Payload::Text(ref s) if s == "hello world"));
        assert_eq!(env.priority, Priority::Normal);
    }

    #[test]
    fn envelope_insert_meta() {
        let mut env = Envelope::text("ch", SessionId::new(), "x");
        env.insert_meta("key1", json!("val1"));
        env.insert_meta("key2", json!(42));
        assert_eq!(some(env.metadata.get("key1"), "key1"), &json!("val1"));
        assert_eq!(some(env.metadata.get("key2"), "key2"), &json!(42));
    }

    #[test]
    fn memory_entry_with_tags() {
        let entry = MemoryEntry::semantic("user:prefs", json!({"theme": "dark"}))
            .with_scope(MemoryScope::User)
            .with_source("user")
            .with_tags(vec!["user".into(), "settings".into()]);
        assert_eq!(entry.key, "user:prefs");
        assert_eq!(entry.kind, MemoryKind::Semantic);
        assert_eq!(entry.scope, MemoryScope::User);
        assert_eq!(entry.source, "user");
        assert_eq!(entry.tags, vec!["user", "settings"]);
        assert_eq!(entry.value, json!({"theme": "dark"}));
    }

    // --- SecretValue ---

    #[test]
    fn secret_value_expose_bytes() {
        let sv = SecretValue::new(b"key123".to_vec());
        assert_eq!(sv.expose(), b"key123");
    }

    #[test]
    fn secret_value_expose_str_valid_utf8() {
        let sv = SecretValue::new(b"hello".to_vec());
        assert_eq!(sv.expose_str(), Some("hello"));
    }

    #[test]
    fn secret_value_expose_str_invalid_utf8() {
        let sv = SecretValue::new(vec![0xFF, 0xFE]);
        assert_eq!(sv.expose_str(), None);
    }

    // --- backoff_delay ---

    #[test]
    fn backoff_delay_first_attempt() {
        // ceiling = 2 * 2^0 = 2; jitter in [0, 2]
        let d = backoff_delay(0, 2, 60);
        assert!(d <= Duration::from_secs(2), "expected <= 2s, got {d:?}");
    }

    #[test]
    fn backoff_delay_exponential() {
        // ceiling = 1 * 2^3 = 8; jitter in [0, 8]
        let d = backoff_delay(3, 1, 60);
        assert!(d <= Duration::from_secs(8), "expected <= 8s, got {d:?}");
    }

    #[test]
    fn backoff_delay_capped_at_max() {
        // ceiling = min(1 * 2^10, 30) = 30; jitter in [0, 30]
        let d = backoff_delay(10, 1, 30);
        assert!(d <= Duration::from_secs(30), "expected <= 30s, got {d:?}");
    }

    #[test]
    fn backoff_delay_zero_base() {
        // base=0 → ceiling=0 → always 0
        assert_eq!(backoff_delay(5, 0, 60), Duration::from_secs(0));
    }

    // --- resolve_path ---

    fn input_with_cwd(cwd: &str) -> SkillInput {
        use crate::{traits::SecretManager, types::SecretValue};
        struct NoopSecrets;
        #[async_trait::async_trait]
        impl SecretManager for NoopSecrets {
            async fn get_secret(&self, _: &str) -> crate::Result<SecretValue> {
                Err(crate::Error::secret("noop"))
            }
            async fn set_secret(&self, _: &str, _: &SecretValue) -> crate::Result<()> {
                Err(crate::Error::secret("noop"))
            }
        }
        let ctx = SkillContext {
            secrets: std::sync::Arc::new(NoopSecrets),
            event_sink: None,
            budget: None,
            user_cwd: Some(cwd.to_string()),
            worktree_cwd: None,
            progress_tx: None,
            cancellation_token: None,
        };
        SkillInput::new(std::collections::HashMap::new()).with_context(ctx)
    }

    #[test]
    fn resolve_path_relative_joins_cwd() {
        let input = input_with_cwd("/tmp");
        assert_eq!(
            input.resolve_path("foo.txt"),
            std::path::PathBuf::from("/tmp/foo.txt")
        );
    }

    #[test]
    fn resolve_path_absolute_unchanged() {
        let input = input_with_cwd("/tmp");
        assert_eq!(
            input.resolve_path("/var/lib/orka/file.txt"),
            std::path::PathBuf::from("/var/lib/orka/file.txt")
        );
    }

    #[test]
    fn resolve_path_tilde_slash_maps_to_cwd() {
        let input = input_with_cwd("/tmp");
        assert_eq!(
            input.resolve_path("~/foo.txt"),
            std::path::PathBuf::from("/tmp/foo.txt")
        );
    }

    #[test]
    fn resolve_path_tilde_alone_maps_to_cwd() {
        let input = input_with_cwd("/tmp");
        assert_eq!(input.resolve_path("~"), std::path::PathBuf::from("/tmp"));
    }

    #[test]
    fn resolve_path_no_cwd_returns_as_is() {
        let input = SkillInput::new(std::collections::HashMap::new());
        assert_eq!(
            input.resolve_path("foo.txt"),
            std::path::PathBuf::from("foo.txt")
        );
        assert_eq!(
            input.resolve_path("~/foo.txt"),
            std::path::PathBuf::from("~/foo.txt")
        );
    }

    // --- SecretStr ---

    #[test]
    fn secret_str_debug_redacted() {
        let s = SecretStr::new("my-api-key");
        assert_eq!(format!("{s:?}"), "[REDACTED]");
    }

    #[test]
    fn secret_str_expose_returns_value() {
        let s = SecretStr::new("my-api-key");
        assert_eq!(s.expose(), "my-api-key");
    }

    #[test]
    fn secret_str_to_owned_secret_copies_value() {
        let s = SecretStr::new("my-api-key");
        let copy = s.to_owned_secret();
        assert_eq!(copy.expose(), "my-api-key");
        assert_eq!(s.expose(), "my-api-key");
    }

    #[test]
    fn secret_str_empty_string() {
        let s = SecretStr::new("");
        assert_eq!(s.expose(), "");
    }
}
