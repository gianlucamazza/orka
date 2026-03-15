use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Unique identifier for a message flowing through the system.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MessageId(pub Uuid);

impl MessageId {
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
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub Uuid);

impl SessionId {
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

/// Message priority for queue routing.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub enum Priority {
    Background = 0,
    #[default]
    Normal = 1,
    Urgent = 2,
}

/// Message payload variants.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum Payload {
    Text(String),
    Media(MediaPayload),
    Command(CommandPayload),
    Event(EventPayload),
}

/// Media attachment info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaPayload {
    pub mime_type: String,
    pub url: String,
    pub caption: Option<String>,
    pub size_bytes: Option<u64>,
}

/// Structured command from a channel or internal system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandPayload {
    pub name: String,
    pub args: HashMap<String, serde_json::Value>,
}

/// System or lifecycle event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventPayload {
    pub kind: String,
    pub data: serde_json::Value,
}

/// W3C Trace Context for distributed tracing propagation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TraceContext {
    pub trace_id: Option<String>,
    pub span_id: Option<String>,
    pub trace_flags: Option<u8>,
}

/// Universal message envelope that flows through the entire system.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundMessage {
    pub channel: String,
    pub session_id: SessionId,
    pub payload: Payload,
    pub reply_to: Option<MessageId>,
    pub metadata: HashMap<String, serde_json::Value>,
}

/// A stored session with associated state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    pub channel: String,
    pub user_id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub state: HashMap<String, serde_json::Value>,
}

impl Session {
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub key: String,
    pub value: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub tags: Vec<String>,
}

/// Opaque secret value, zeroized on drop.
#[derive(Clone)]
pub struct SecretValue(Vec<u8>);

impl SecretValue {
    pub fn new(value: impl Into<Vec<u8>>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &[u8] {
        &self.0
    }

    pub fn expose_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.0).ok()
    }
}

impl Drop for SecretValue {
    fn drop(&mut self) {
        self.0.iter_mut().for_each(|b| *b = 0);
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
