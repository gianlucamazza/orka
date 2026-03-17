use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

use crate::traits::SecretManager;

/// Unique identifier for a message flowing through the system.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, utoipa::ToSchema)]
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
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, utoipa::ToSchema)]
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
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, utoipa::ToSchema)]
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
#[allow(missing_docs)]
pub struct DomainEvent {
    pub id: EventId,
    pub timestamp: DateTime<Utc>,
    pub kind: DomainEventKind,
    pub metadata: HashMap<String, serde_json::Value>,
}

/// The kind of domain event that occurred.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(tag = "type")]
#[allow(missing_docs)]
pub enum DomainEventKind {
    MessageReceived {
        message_id: MessageId,
        channel: String,
        session_id: SessionId,
    },
    SessionCreated {
        session_id: SessionId,
        channel: String,
    },
    HandlerInvoked {
        message_id: MessageId,
        session_id: SessionId,
    },
    HandlerCompleted {
        message_id: MessageId,
        session_id: SessionId,
        duration_ms: u64,
        reply_count: usize,
    },
    SkillInvoked {
        skill_name: String,
        message_id: MessageId,
    },
    SkillCompleted {
        skill_name: String,
        message_id: MessageId,
        duration_ms: u64,
        success: bool,
    },
    LlmCompleted {
        message_id: MessageId,
        model: String,
        input_tokens: u32,
        output_tokens: u32,
        duration_ms: u64,
    },
    ErrorOccurred {
        source: String,
        message: String,
    },
    Heartbeat,
}

/// Context available to skills during execution.
#[derive(Clone)]
#[allow(missing_docs)]
pub struct SkillContext {
    pub secrets: Arc<dyn SecretManager>,
}

impl std::fmt::Debug for SkillContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SkillContext").finish()
    }
}

/// Input passed to a skill invocation.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[allow(missing_docs)]
pub struct SkillInput {
    pub args: HashMap<String, serde_json::Value>,
    #[serde(skip)]
    #[schema(ignore)]
    pub context: Option<SkillContext>,
}

/// Output returned from a skill invocation.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[allow(missing_docs)]
pub struct SkillOutput {
    pub data: serde_json::Value,
}

/// JSON Schema describing a skill's parameters.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[allow(missing_docs)]
pub struct SkillSchema {
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
#[allow(missing_docs)]
pub enum Priority {
    Background = 0,
    #[default]
    Normal = 1,
    Urgent = 2,
}

/// Message payload variants.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(tag = "type", content = "data")]
#[allow(missing_docs)]
pub enum Payload {
    Text(String),
    Media(MediaPayload),
    Command(CommandPayload),
    Event(EventPayload),
}

/// Media attachment info.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[allow(missing_docs)]
pub struct MediaPayload {
    pub mime_type: String,
    pub url: String,
    pub caption: Option<String>,
    pub size_bytes: Option<u64>,
}

/// Structured command from a channel or internal system.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[allow(missing_docs)]
pub struct CommandPayload {
    pub name: String,
    pub args: HashMap<String, serde_json::Value>,
}

/// System or lifecycle event.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[allow(missing_docs)]
pub struct EventPayload {
    pub kind: String,
    pub data: serde_json::Value,
}

/// W3C Trace Context for distributed tracing propagation.
#[derive(Debug, Clone, Default, Serialize, Deserialize, utoipa::ToSchema)]
#[allow(missing_docs)]
pub struct TraceContext {
    pub trace_id: Option<String>,
    pub span_id: Option<String>,
    pub trace_flags: Option<u8>,
}

/// Universal message envelope that flows through the entire system.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[allow(missing_docs)]
pub struct Envelope {
    pub id: MessageId,
    pub channel: String,
    pub session_id: SessionId,
    pub timestamp: DateTime<Utc>,
    pub priority: Priority,
    pub payload: Payload,
    pub metadata: HashMap<String, serde_json::Value>,
    pub trace_context: TraceContext,
}

impl Envelope {
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
#[allow(missing_docs)]
pub struct OutboundMessage {
    pub channel: String,
    pub session_id: SessionId,
    pub payload: Payload,
    pub reply_to: Option<MessageId>,
    pub metadata: HashMap<String, serde_json::Value>,
}

/// A stored session with associated state.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[allow(missing_docs)]
pub struct Session {
    pub id: SessionId,
    pub channel: String,
    pub user_id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
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
}

/// An entry in the memory store.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[allow(missing_docs)]
pub struct MemoryEntry {
    pub key: String,
    pub value: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
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
