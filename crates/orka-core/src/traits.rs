use std::time::Duration;

use async_trait::async_trait;

use crate::{
    DomainEvent, Envelope, MemoryEntry, MessageId, MessageSink, MessageStream, OutboundMessage,
    Result, SecretValue, Session, SessionId, SkillInput, SkillOutput, SkillSchema,
};

/// Adapter for an external messaging channel (Telegram, Discord, etc.).
#[async_trait]
pub trait ChannelAdapter: Send + Sync + 'static {
    /// Returns the unique identifier for this channel (e.g. "telegram",
    /// "discord").
    fn channel_id(&self) -> &str;

    /// Start receiving messages, forwarding them into the provided sink.
    async fn start(&self, sink: MessageSink) -> Result<()>;

    /// Send an outbound message to this channel.
    async fn send(&self, msg: OutboundMessage) -> Result<()>;

    /// Gracefully shut down the adapter.
    async fn shutdown(&self) -> Result<()>;

    /// Register slash commands with the platform (e.g. Telegram command menu).
    /// Default: no-op.
    async fn register_commands(&self, _commands: &[(&str, &str)]) -> Result<()> {
        Ok(())
    }
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

    /// List recent sessions, up to `limit`. Default: returns empty list.
    async fn list(&self, _limit: usize) -> Result<Vec<Session>> {
        Ok(Vec::new())
    }
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

    /// Compact expired or low-priority entries. Returns number of entries
    /// removed.
    async fn compact(&self) -> Result<usize>;
}

/// Distributed per-session lock for preventing concurrent history corruption.
///
/// Decoupled from [`MemoryStore`] so that the lock backend can be swapped
/// independently (e.g. Redis for locking, in-memory for history in tests).
#[async_trait]
pub trait SessionLock: Send + Sync + 'static {
    /// Try to acquire a lock for `session_id`.
    ///
    /// Returns `true` if the lock was acquired, `false` if another worker
    /// already holds it.  The lock expires automatically after `ttl_ms`
    /// milliseconds.
    async fn try_acquire(&self, session_id: &str, ttl_ms: u64) -> bool;

    /// Release the lock previously acquired for `session_id`.
    async fn release(&self, session_id: &str);
}

/// Priority queue for ordered message processing.
#[async_trait]
pub trait PriorityQueue: Send + Sync + 'static {
    /// Push an envelope into the queue.
    async fn push(&self, envelope: &Envelope) -> Result<()>;

    /// Pop the highest-priority envelope, blocking up to `timeout`.
    async fn pop(&self, timeout: Duration) -> Result<Option<Envelope>>;

    /// Return the number of envelopes currently in the queue.
    async fn len(&self) -> Result<usize>;

    /// Returns true if the queue is empty.
    async fn is_empty(&self) -> Result<bool> {
        Ok(self.len().await? == 0)
    }
}

/// Dead-letter queue for messages that exhausted all retry attempts.
///
/// Decoupled from [`PriorityQueue`] so that API routes and observability tools
/// can depend only on DLQ operations without access to the full queue.
#[async_trait]
pub trait DeadLetterQueue: Send + Sync + 'static {
    /// Push a failed envelope to the dead-letter queue.
    async fn push(&self, envelope: &Envelope) -> Result<()>;

    /// List all envelopes in the dead-letter queue.
    async fn list(&self) -> Result<Vec<Envelope>>;

    /// Remove all envelopes from the dead-letter queue. Returns the number
    /// removed.
    async fn purge(&self) -> Result<usize>;

    /// Remove a single envelope from the DLQ by ID and re-enqueue it for
    /// processing.
    async fn replay(&self, id: &MessageId) -> Result<bool>;
}

/// Fire-and-forget event sink for domain-level observability.
#[async_trait]
pub trait EventSink: Send + Sync + 'static {
    /// Emit a domain event for observability.
    async fn emit(&self, event: DomainEvent);
}

/// A no-op [`EventSink`] that discards all events.
///
/// Useful as a default when no observability is configured.
pub struct NoopEventSink;

#[async_trait]
impl EventSink for NoopEventSink {
    async fn emit(&self, _event: DomainEvent) {}
}

/// A named, schema-described skill that an agent can invoke.
#[async_trait]
pub trait Skill: Send + Sync + 'static {
    /// The unique name of this skill (e.g. "`web_search`").
    fn name(&self) -> &str;
    /// A human-readable description for LLM tool-use prompts.
    fn description(&self) -> &str;
    /// JSON Schema describing the skill's input parameters.
    fn schema(&self) -> SkillSchema;
    /// Execute the skill with the given input.
    async fn execute(&self, input: SkillInput) -> Result<SkillOutput>;

    /// Called when the skill is registered. Default: no-op.
    async fn init(&self) -> Result<()> {
        Ok(())
    }

    /// Called on shutdown. Default: no-op.
    async fn cleanup(&self) -> Result<()> {
        Ok(())
    }

    /// Skill category for progressive disclosure grouping.
    ///
    /// Used to group skills by domain when building the LLM tool list.
    /// Override this method to assign a non-default category.
    fn category(&self) -> &'static str {
        "general"
    }

    /// Validate the output produced by [`execute`].
    ///
    /// Called automatically by [`SkillRegistry::invoke`] after a successful
    /// execution. Return `Err` if the output is semantically invalid
    /// (hallucinated, wrong schema, etc.). A Semantic failure increments
    /// the quality circuit-breaker counter.
    ///
    /// Default implementation accepts all outputs.
    fn validate_output(&self, _output: &crate::SkillOutput) -> Result<()> {
        Ok(())
    }
}

/// Secure secret retrieval and storage.
#[async_trait]
pub trait SecretManager: Send + Sync + 'static {
    /// Get a secret by path.
    async fn get_secret(&self, path: &str) -> Result<SecretValue>;

    /// Set a secret at a path.
    async fn set_secret(&self, path: &str, value: &SecretValue) -> Result<()>;

    /// Delete a secret by path.
    async fn delete_secret(&self, path: &str) -> Result<()> {
        let _ = path;
        Err(crate::Error::secret("delete not supported"))
    }

    /// List all secret paths.
    async fn list_secrets(&self) -> Result<Vec<String>> {
        Err(crate::Error::secret("list not supported"))
    }
}

/// Decision from a guardrail check.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum GuardrailDecision {
    /// Allow the content through unchanged.
    Allow,
    /// Block the content with a reason.
    Block(String),
    /// Modify the content (e.g., redact PII).
    Modify(String),
}

/// Input/output guardrail for content safety filtering.
#[async_trait]
pub trait Guardrail: Send + Sync + 'static {
    /// Check input content before sending to LLM.
    async fn check_input(&self, input: &str, session: &Session) -> Result<GuardrailDecision>;
    /// Check output content before returning to user.
    async fn check_output(&self, output: &str, session: &Session) -> Result<GuardrailDecision>;
}
