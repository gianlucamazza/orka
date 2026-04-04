use std::time::Duration;

use async_trait::async_trait;
use base64::Engine as _;

use crate::{
    ArtifactId, Conversation, ConversationArtifact, ConversationId, ConversationMessage,
    DomainEvent, Envelope, MemoryEntry, MessageId, MessageSink, MessageStream, OutboundMessage,
    Result, SecretValue, Session, SessionId, SkillInput, SkillOutput, SkillSchema,
};

/// Opaque pagination cursor for message lists.
///
/// Encodes `(created_at_ms, message_id)` as a base64-URL-no-pad string so that
/// callers never need to know the internal representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageCursor {
    /// Creation timestamp of the referenced message, in milliseconds since
    /// epoch.
    pub created_at_ms: i64,
    /// UUID of the referenced message (tiebreaker within the same millisecond).
    pub message_id: uuid::Uuid,
}

impl MessageCursor {
    /// Build a cursor from a [`ConversationMessage`].
    pub fn from_message(msg: &ConversationMessage) -> Self {
        Self {
            created_at_ms: msg.created_at.timestamp_millis(),
            message_id: msg.id.as_uuid(),
        }
    }

    /// Encode the cursor to an opaque base64-URL-no-pad string.
    pub fn encode(&self) -> String {
        let mut buf = [0u8; 24]; // 8 bytes i64 + 16 bytes uuid
        buf[..8].copy_from_slice(&self.created_at_ms.to_be_bytes());
        buf[8..].copy_from_slice(self.message_id.as_bytes());
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
    }

    /// Decode a cursor from an opaque base64-URL-no-pad string.
    ///
    /// Returns `None` if the string is malformed.
    pub fn decode(s: &str) -> Option<Self> {
        let buf = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(s)
            .ok()?;
        if buf.len() != 24 {
            return None;
        }
        let created_at_ms = i64::from_be_bytes(buf[..8].try_into().ok()?);
        let message_id = uuid::Uuid::from_bytes(buf[8..].try_into().ok()?);
        Some(Self {
            created_at_ms,
            message_id,
        })
    }
}

/// Filter and paginate a fully-loaded, chronologically-sorted message list.
///
/// - `after`: return messages strictly after this cursor (exclusive)
/// - `before`: return messages strictly before this cursor (exclusive)
/// - `limit`: maximum number of messages to return
///
/// When `after` is `None` and `limit < usize::MAX`, the **last** `limit`
/// messages are returned (tail-of-history semantics), which is what a chat
/// client wants when opening a conversation for the first time.
pub fn apply_message_cursors(
    all: Vec<ConversationMessage>,
    after: Option<&MessageCursor>,
    before: Option<&MessageCursor>,
    limit: usize,
) -> Vec<ConversationMessage> {
    let mut messages = all;

    if let Some(after) = after {
        messages.retain(|m| {
            let ms = m.created_at.timestamp_millis();
            ms > after.created_at_ms
                || (ms == after.created_at_ms && m.id.as_uuid() > after.message_id)
        });
    }

    if let Some(before) = before {
        messages.retain(|m| {
            let ms = m.created_at.timestamp_millis();
            ms < before.created_at_ms
                || (ms == before.created_at_ms && m.id.as_uuid() < before.message_id)
        });
    }

    if limit == usize::MAX {
        return messages;
    }

    if after.is_none() {
        // No lower bound — return the tail (most recent).
        let skip = messages.len().saturating_sub(limit);
        messages.split_off(skip)
    } else {
        messages.truncate(limit);
        messages
    }
}

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

/// Persistent conversation storage for product-facing chat clients.
#[async_trait]
pub trait ConversationStore: Send + Sync + 'static {
    /// Store or update conversation metadata.
    async fn put_conversation(&self, conversation: &Conversation) -> Result<()>;

    /// Retrieve a conversation by ID.
    async fn get_conversation(&self, id: &ConversationId) -> Result<Option<Conversation>>;

    /// List recent conversations for a user.
    ///
    /// When `include_archived` is `false` (the default for callers), archived
    /// conversations are excluded from the result.
    async fn list_conversations(
        &self,
        user_id: &str,
        limit: usize,
        offset: usize,
        include_archived: bool,
    ) -> Result<Vec<Conversation>>;

    /// Permanently delete a conversation and its transcript.
    async fn delete_conversation(&self, id: &ConversationId) -> Result<()>;

    /// Append a user-facing message to a conversation transcript.
    async fn append_message(&self, message: &ConversationMessage) -> Result<()>;

    /// Insert or replace a user-facing message in the transcript.
    async fn upsert_message(&self, message: &ConversationMessage) -> Result<()>;

    /// Retrieve one transcript message by ID.
    async fn get_message(
        &self,
        conversation_id: &ConversationId,
        message_id: &MessageId,
    ) -> Result<Option<ConversationMessage>>;

    /// List transcript messages for a conversation in ascending order.
    ///
    /// `after` and `before` are opaque cursors produced by
    /// [`MessageCursor::encode`]. Pass `limit = usize::MAX` to fetch all
    /// messages without truncation.
    async fn list_messages(
        &self,
        conversation_id: &ConversationId,
        after: Option<&MessageCursor>,
        before: Option<&MessageCursor>,
        limit: usize,
    ) -> Result<Vec<ConversationMessage>>;

    /// Delete a single message from the conversation transcript.
    ///
    /// Returns `Ok(())` whether or not the message existed (idempotent).
    async fn delete_message(
        &self,
        conversation_id: &ConversationId,
        message_id: &MessageId,
    ) -> Result<()>;

    /// Advance the read watermark for `user_id` in `conversation_id`.
    ///
    /// The watermark never moves backward: if the stored cursor is already
    /// newer than `cursor`, the call is a no-op.
    async fn set_read_watermark(
        &self,
        user_id: &str,
        conversation_id: &ConversationId,
        cursor: &MessageCursor,
    ) -> Result<()>;

    /// Return the current read watermark for `user_id` in `conversation_id`,
    /// or `None` if the user has never marked the conversation as read.
    async fn get_read_watermark(
        &self,
        user_id: &str,
        conversation_id: &ConversationId,
    ) -> Result<Option<MessageCursor>>;
}

/// Persistent artifact metadata and content storage for product-facing clients.
#[async_trait]
pub trait ArtifactStore: Send + Sync + 'static {
    /// Store or replace one artifact and its raw bytes.
    async fn put_artifact(&self, artifact: &ConversationArtifact, bytes: &[u8]) -> Result<()>;

    /// Update artifact metadata without replacing the stored bytes.
    async fn update_artifact(&self, artifact: &ConversationArtifact) -> Result<()>;

    /// Retrieve artifact metadata by ID.
    async fn get_artifact(&self, artifact_id: &ArtifactId) -> Result<Option<ConversationArtifact>>;

    /// Retrieve artifact raw bytes by ID.
    async fn get_artifact_bytes(&self, artifact_id: &ArtifactId) -> Result<Option<Vec<u8>>>;

    /// Delete artifact metadata and raw bytes by ID.
    async fn delete_artifact(&self, artifact_id: &ArtifactId) -> Result<()>;

    /// List all artifacts attached to a conversation, ordered by creation time.
    async fn list_artifacts_by_conversation(
        &self,
        conversation_id: &crate::ConversationId,
    ) -> Result<Vec<ConversationArtifact>>;
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

    /// List entries, optionally filtering by key prefix.
    async fn list(&self, prefix: Option<&str>, limit: usize) -> Result<Vec<MemoryEntry>>;

    /// Delete a value by key. Returns `true` when an entry was removed.
    async fn delete(&self, key: &str) -> Result<bool>;

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

    /// Extend the TTL of an existing lock by `ttl_ms` milliseconds.
    ///
    /// Used by the watchdog pattern to renew a lock during long-running
    /// dispatches. Returns `true` if the lock was extended, `false` if it
    /// had already expired or was released.
    ///
    /// Default: no-op returning `true` (safe for in-memory test backends).
    async fn extend(&self, _session_id: &str, _ttl_ms: u64) -> bool {
        true
    }
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use chrono::{TimeZone, Utc};

    use super::*;
    use crate::{
        ConversationId, ConversationMessage, MessageId, SessionId,
        testing::InMemoryConversationStore,
    };

    fn make_message_in(
        conv_id: ConversationId,
        created_at_ms: i64,
        id: uuid::Uuid,
    ) -> ConversationMessage {
        let mut msg = ConversationMessage::new(
            MessageId::from(id),
            conv_id,
            SessionId::new(),
            crate::ConversationMessageRole::User,
            "test",
        );
        msg.created_at = Utc.timestamp_millis_opt(created_at_ms).unwrap();
        msg
    }

    fn make_message(created_at_ms: i64, id: uuid::Uuid) -> ConversationMessage {
        make_message_in(ConversationId::new(), created_at_ms, id)
    }

    // --- MessageCursor ---

    #[test]
    fn cursor_round_trip() {
        let id = uuid::Uuid::now_v7();
        let c = MessageCursor {
            created_at_ms: 1_700_000_000_000,
            message_id: id,
        };
        let encoded = c.encode();
        let decoded = MessageCursor::decode(&encoded).unwrap();
        assert_eq!(c, decoded);
    }

    #[test]
    fn cursor_decode_invalid_returns_none() {
        assert!(MessageCursor::decode("not-valid-base64!!!").is_none());
        assert!(MessageCursor::decode("dG9vc2hvcnQ").is_none()); // valid base64 but wrong length
    }

    // --- apply_message_cursors ---

    fn msgs() -> Vec<ConversationMessage> {
        vec![
            make_message(1000, uuid::Uuid::now_v7()),
            make_message(2000, uuid::Uuid::now_v7()),
            make_message(3000, uuid::Uuid::now_v7()),
            make_message(4000, uuid::Uuid::now_v7()),
            make_message(5000, uuid::Uuid::now_v7()),
        ]
    }

    #[test]
    fn no_cursors_no_limit_returns_all() {
        let result = apply_message_cursors(msgs(), None, None, usize::MAX);
        assert_eq!(result.len(), 5);
    }

    #[test]
    fn no_cursors_with_limit_returns_tail() {
        let all = msgs();
        let result = apply_message_cursors(all.clone(), None, None, 3);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].created_at, all[2].created_at);
    }

    #[test]
    fn after_cursor_returns_newer_messages() {
        let all = msgs();
        let after = MessageCursor::from_message(&all[1]);
        let result = apply_message_cursors(all.clone(), Some(&after), None, usize::MAX);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].created_at, all[2].created_at);
    }

    #[test]
    fn before_cursor_returns_older_messages() {
        let all = msgs();
        let before = MessageCursor::from_message(&all[3]);
        let result = apply_message_cursors(all.clone(), None, Some(&before), usize::MAX);
        assert_eq!(result.len(), 3);
        assert_eq!(result[2].created_at, all[2].created_at);
    }

    #[test]
    fn after_and_limit_truncates() {
        let all = msgs();
        let after = MessageCursor::from_message(&all[0]);
        let result = apply_message_cursors(all.clone(), Some(&after), None, 2);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].created_at, all[1].created_at);
    }

    // --- InMemoryConversationStore: read watermarks ---

    #[tokio::test]
    async fn watermark_advances() {
        let store = InMemoryConversationStore::new();
        let conv_id = ConversationId::new();
        let all: Vec<_> = [1000i64, 2000, 3000, 4000, 5000]
            .into_iter()
            .map(|ms| make_message_in(conv_id, ms, uuid::Uuid::now_v7()))
            .collect();
        let c1 = MessageCursor::from_message(&all[1]);
        let c3 = MessageCursor::from_message(&all[3]);

        store
            .set_read_watermark("user1", &conv_id, &c1)
            .await
            .unwrap();
        store
            .set_read_watermark("user1", &conv_id, &c3)
            .await
            .unwrap();

        let got = store
            .get_read_watermark("user1", &conv_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got, c3);
    }

    #[tokio::test]
    async fn watermark_does_not_go_backward() {
        let store = InMemoryConversationStore::new();
        let conv_id = ConversationId::new();
        let all: Vec<_> = [1000i64, 2000, 3000, 4000, 5000]
            .into_iter()
            .map(|ms| make_message_in(conv_id, ms, uuid::Uuid::now_v7()))
            .collect();
        let c3 = MessageCursor::from_message(&all[3]);
        let c1 = MessageCursor::from_message(&all[1]);

        store
            .set_read_watermark("user1", &conv_id, &c3)
            .await
            .unwrap();
        store
            .set_read_watermark("user1", &conv_id, &c1)
            .await
            .unwrap(); // older — should be ignored

        let got = store
            .get_read_watermark("user1", &conv_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got, c3);
    }

    #[tokio::test]
    async fn watermark_none_when_unset() {
        let store = InMemoryConversationStore::new();
        let conv_id = ConversationId::new();
        assert!(
            store
                .get_read_watermark("user1", &conv_id)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn unread_count_via_list_messages() {
        let store = InMemoryConversationStore::new();
        let conv_id = ConversationId::new();
        let all: Vec<_> = [1000i64, 2000, 3000, 4000, 5000]
            .into_iter()
            .map(|ms| make_message_in(conv_id, ms, uuid::Uuid::now_v7()))
            .collect();

        for m in &all {
            store.upsert_message(m).await.unwrap();
        }

        // Mark first 2 as read.
        let watermark = MessageCursor::from_message(&all[1]);
        store
            .set_read_watermark("user1", &conv_id, &watermark)
            .await
            .unwrap();

        let wm = store.get_read_watermark("user1", &conv_id).await.unwrap();
        let unread = store
            .list_messages(&conv_id, wm.as_ref(), None, usize::MAX)
            .await
            .unwrap();
        assert_eq!(unread.len(), 3);
    }
}
