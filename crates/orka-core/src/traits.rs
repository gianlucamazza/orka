use async_trait::async_trait;
use std::time::Duration;

use crate::{
    Envelope, MemoryEntry, MessageId, MessageSink, MessageStream, OutboundMessage, Result,
    SecretValue, Session, SessionId,
};

/// Adapter for an external messaging channel (Telegram, Discord, etc.).
#[async_trait]
pub trait ChannelAdapter: Send + Sync + 'static {
    /// Returns the unique identifier for this channel (e.g. "telegram", "discord").
    fn channel_id(&self) -> &str;

    /// Start receiving messages, forwarding them into the provided sink.
    async fn start(&self, sink: MessageSink) -> Result<()>;

    /// Send an outbound message to this channel.
    async fn send(&self, msg: OutboundMessage) -> Result<()>;

    /// Gracefully shut down the adapter.
    async fn shutdown(&self) -> Result<()>;
}

/// Publish/subscribe message bus.
#[async_trait]
pub trait MessageBus: Send + Sync + 'static {
    /// Publish an envelope to a topic.
    async fn publish(&self, topic: &str, msg: &Envelope) -> Result<()>;

    /// Subscribe to a topic, returning a stream of envelopes.
    async fn subscribe(&self, topic: &str) -> Result<MessageStream>;

    /// Acknowledge processing of a message.
    async fn ack(&self, id: &MessageId) -> Result<()>;
}

/// Persistent session storage.
#[async_trait]
pub trait SessionStore: Send + Sync + 'static {
    /// Retrieve a session by ID.
    async fn get(&self, id: &SessionId) -> Result<Option<Session>>;

    /// Store or update a session.
    async fn put(&self, session: &Session) -> Result<()>;

    /// Delete a session.
    async fn delete(&self, id: &SessionId) -> Result<()>;
}

/// Key-value memory store with TTL and search.
#[async_trait]
pub trait MemoryStore: Send + Sync + 'static {
    /// Store a value with an optional TTL.
    async fn store(&self, key: &str, value: MemoryEntry, ttl: Option<Duration>) -> Result<()>;

    /// Recall a value by key.
    async fn recall(&self, key: &str) -> Result<Option<MemoryEntry>>;

    /// Search entries by query string.
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryEntry>>;

    /// Compact expired or low-priority entries. Returns number of entries removed.
    async fn compact(&self) -> Result<usize>;
}

/// Secure secret retrieval and storage.
#[async_trait]
pub trait SecretManager: Send + Sync + 'static {
    /// Get a secret by path.
    async fn get_secret(&self, path: &str) -> Result<SecretValue>;

    /// Set a secret at a path.
    async fn set_secret(&self, path: &str, value: &SecretValue) -> Result<()>;
}
