use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

use crate::traits::{EventSink, SecretManager};

/// Well-known metadata keys used across adapters and the worker.
pub mod meta {
    /// The name of the workspace associated with this message.
    pub const WORKSPACE_NAME: &str = "workspace:name";
    /// Telegram chat identifier.
    pub const TELEGRAM_CHAT_ID: &str = "telegram_chat_id";
    /// Telegram user display name.
    pub const TELEGRAM_FROM_USERNAME: &str = "telegram_from_username";
    /// Slack team identifier.
    pub const SLACK_TEAM_ID: &str = "slack_team_id";
    /// Discord guild identifier.
    pub const DISCORD_GUILD_ID: &str = "discord_guild_id";
}

/// Unique identifier for a message flowing through the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, utoipa::ToSchema)]
pub struct MessageId(pub Uuid);

impl MessageId {
    /// Create a new unique message ID (UUID v7).
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl Default for MessageId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for MessageId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for a user session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SessionId(pub Uuid);

impl SessionId {
    /// Create a new unique session ID (UUID v7).
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for a domain event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, utoipa::ToSchema)]
pub struct EventId(pub Uuid);

impl EventId {
    /// Create a new unique event ID (UUID v7).
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl Default for EventId {
    fn default() -> Self {
        Self::new()
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
    },
    /// Emitted after each LLM call with token usage and latency.
    LlmCompleted {
        /// ID of the message that triggered the LLM call.
        message_id: MessageId,
        /// Model identifier used for the completion.
        model: String,
        /// Number of tokens in the prompt.
        input_tokens: u32,
        /// Number of tokens in the response.
        output_tokens: u32,
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
}

/// Context available to skills during execution.
#[derive(Clone)]
#[non_exhaustive]
pub struct SkillContext {
    /// Provides access to named secrets during skill execution.
    pub secrets: Arc<dyn SecretManager>,
    /// Optional sink for emitting domain events from within a skill.
    pub event_sink: Option<Arc<dyn EventSink>>,
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
    /// Get a required string argument, returning a `Skill` error if missing or not a string.
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
            .and_then(|v| v.as_i64())
            .ok_or_else(|| crate::Error::Skill(format!("{key} is required")))
    }

    /// Get a required bool argument.
    pub fn get_bool(&self, key: &str) -> crate::Result<bool> {
        self.args
            .get(key)
            .and_then(|v| v.as_bool())
            .ok_or_else(|| crate::Error::Skill(format!("{key} is required")))
    }
}

/// Output returned from a skill invocation.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[non_exhaustive]
pub struct SkillOutput {
    /// Structured result value produced by the skill.
    pub data: serde_json::Value,
}

/// JSON Schema describing a skill's parameters.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[non_exhaustive]
pub struct SkillSchema {
    /// JSON Schema object describing the skill's accepted parameters.
    pub parameters: serde_json::Value,
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
    /// URL or path where the media can be retrieved.
    pub url: String,
    /// Optional human-readable description of the media.
    pub caption: Option<String>,
    /// File size in bytes, if known.
    pub size_bytes: Option<u64>,
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

/// System or lifecycle event.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[non_exhaustive]
pub struct EventPayload {
    /// Short string identifier for the event type.
    pub kind: String,
    /// Arbitrary structured payload for the event.
    pub data: serde_json::Value,
}

/// W3C Trace Context for distributed tracing propagation.
#[derive(Debug, Clone, Default, Serialize, Deserialize, utoipa::ToSchema)]
#[non_exhaustive]
pub struct TraceContext {
    /// W3C traceparent `trace-id` component (32 lowercase hex characters).
    pub trace_id: Option<String>,
    /// W3C traceparent `parent-id` component (16 lowercase hex characters).
    pub span_id: Option<String>,
    /// W3C trace flags byte (`1` = sampled).
    pub trace_flags: Option<u8>,
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
    /// Create a new skill context.
    pub fn new(secrets: Arc<dyn SecretManager>, event_sink: Option<Arc<dyn EventSink>>) -> Self {
        Self {
            secrets,
            event_sink,
        }
    }
}

impl SkillOutput {
    /// Create a new skill output.
    pub fn new(data: serde_json::Value) -> Self {
        Self { data }
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
    pub fn with_context(mut self, context: SkillContext) -> Self {
        self.context = Some(context);
        self
    }
}

impl MediaPayload {
    /// Create a new media payload.
    pub fn new(mime_type: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            mime_type: mime_type.into(),
            url: url.into(),
            caption: None,
            size_bytes: None,
        }
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
        }
    }
}

impl MemoryEntry {
    /// Create a new memory entry.
    pub fn new(key: impl Into<String>, value: serde_json::Value) -> Self {
        let now = Utc::now();
        Self {
            key: key.into(),
            value,
            created_at: now,
            updated_at: now,
            tags: Vec::new(),
        }
    }

    /// Set tags on this entry.
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
    /// Adapter-specific delivery metadata.
    pub metadata: HashMap<String, serde_json::Value>,
}

impl OutboundMessage {
    /// Get a required string from metadata, returning an error if missing.
    pub fn require_meta_str(&self, key: &str) -> crate::Result<&str> {
        self.metadata
            .get(key)
            .and_then(|v| v.as_str())
            .ok_or_else(|| crate::Error::Other(format!("missing metadata key: {key}")))
    }

    /// Get a required i64 from metadata, returning an error if missing.
    pub fn require_meta_i64(&self, key: &str) -> crate::Result<i64> {
        self.metadata
            .get(key)
            .and_then(|v| v.as_i64())
            .ok_or_else(|| crate::Error::Other(format!("missing metadata key: {key}")))
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

/// An entry in the memory store.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[non_exhaustive]
pub struct MemoryEntry {
    /// Lookup key for this entry within the memory store.
    pub key: String,
    /// Stored value as a JSON document.
    pub value: serde_json::Value,
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
    pub fn to_owned_secret(&self) -> Self {
        Self(zeroize::Zeroizing::new(self.0.to_vec()))
    }
}

impl std::fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[REDACTED]")
    }
}

/// Type alias for the message sink passed to channel adapters.
pub type MessageSink = tokio::sync::mpsc::Sender<Envelope>;

/// Type alias for the message stream returned by the bus.
pub type MessageStream = tokio::sync::mpsc::Receiver<Envelope>;

/// Exponential backoff delay capped at `max_secs`.
/// Returns `base_secs * 2^attempt`, clamped to `max_secs`.
pub fn backoff_delay(attempt: u32, base_secs: u64, max_secs: u64) -> std::time::Duration {
    let secs = base_secs.saturating_mul(1u64.checked_shl(attempt).unwrap_or(u64::MAX));
    std::time::Duration::from_secs(secs.min(max_secs))
}
