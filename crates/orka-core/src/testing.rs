use std::{collections::HashMap, sync::Arc, time::Duration};

use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::{
    ArtifactId, Conversation, ConversationArtifact, ConversationId, ConversationMessage,
    DomainEvent, Envelope, Error, MemoryEntry, MessageId, MessageStream, Priority, Result,
    SecretValue, Session, SessionId,
    traits::{
        ArtifactStore, ConversationStore, DeadLetterQueue, EventSink, Guardrail, GuardrailDecision,
        MemoryStore, MessageBus, MessageCursor, PriorityQueue, SecretManager, SessionLock,
        SessionStore, apply_message_cursors,
    },
};

// ---------------------------------------------------------------------------
// InMemoryBus
// ---------------------------------------------------------------------------

/// In-memory [`MessageBus`] implementation for use in tests.
pub struct InMemoryBus {
    topics: Arc<Mutex<HashMap<String, Vec<tokio::sync::mpsc::Sender<Envelope>>>>>,
}

impl InMemoryBus {
    /// Create a new empty bus.
    pub fn new() -> Self {
        Self {
            topics: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Default for InMemoryBus {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MessageBus for InMemoryBus {
    async fn publish(&self, topic: &str, msg: &Envelope) -> Result<()> {
        let topics = self.topics.lock().await;
        if let Some(senders) = topics.get(topic) {
            for tx in senders {
                let _ = tx.send(msg.clone()).await;
            }
        }
        Ok(())
    }

    async fn subscribe(&self, topic: &str) -> Result<MessageStream> {
        let (tx, rx) = tokio::sync::mpsc::channel(256);
        let mut topics = self.topics.lock().await;
        topics.entry(topic.to_string()).or_default().push(tx);
        Ok(rx)
    }

    async fn ack(&self, _id: &MessageId) -> Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// InMemorySessionStore
// ---------------------------------------------------------------------------

/// In-memory [`SessionStore`] implementation for use in tests.
pub struct InMemorySessionStore {
    sessions: Arc<Mutex<HashMap<SessionId, Session>>>,
}

impl InMemorySessionStore {
    /// Create a new empty session store.
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Default for InMemorySessionStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SessionStore for InMemorySessionStore {
    async fn get(&self, id: &SessionId) -> Result<Option<Session>> {
        let sessions = self.sessions.lock().await;
        Ok(sessions.get(id).cloned())
    }

    async fn put(&self, session: &Session) -> Result<()> {
        let mut sessions = self.sessions.lock().await;
        sessions.insert(session.id, session.clone());
        Ok(())
    }

    async fn delete(&self, id: &SessionId) -> Result<()> {
        let mut sessions = self.sessions.lock().await;
        sessions.remove(id);
        Ok(())
    }

    async fn list(&self, limit: usize) -> Result<Vec<Session>> {
        let sessions = self.sessions.lock().await;
        let mut result: Vec<Session> = sessions.values().cloned().collect();
        result.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        result.truncate(limit);
        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// InMemoryConversationStore
// ---------------------------------------------------------------------------

/// In-memory [`ConversationStore`] implementation for use in tests.
pub struct InMemoryConversationStore {
    conversations: Arc<Mutex<HashMap<ConversationId, Conversation>>>,
    messages: Arc<Mutex<HashMap<ConversationId, Vec<ConversationMessage>>>>,
    read_watermarks: Arc<Mutex<HashMap<(String, ConversationId), MessageCursor>>>,
}

impl InMemoryConversationStore {
    /// Create a new empty conversation store.
    pub fn new() -> Self {
        Self {
            conversations: Arc::new(Mutex::new(HashMap::new())),
            messages: Arc::new(Mutex::new(HashMap::new())),
            read_watermarks: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Default for InMemoryConversationStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ConversationStore for InMemoryConversationStore {
    async fn put_conversation(&self, conversation: &Conversation) -> Result<()> {
        let mut conversations = self.conversations.lock().await;
        conversations.insert(conversation.id, conversation.clone());
        Ok(())
    }

    async fn get_conversation(&self, id: &ConversationId) -> Result<Option<Conversation>> {
        let conversations = self.conversations.lock().await;
        Ok(conversations.get(id).cloned())
    }

    async fn list_conversations(
        &self,
        user_id: &str,
        limit: usize,
        offset: usize,
        include_archived: bool,
        workspace: Option<&str>,
    ) -> Result<Vec<Conversation>> {
        let conversations = self.conversations.lock().await;
        let mut result: Vec<_> = conversations
            .values()
            .filter(|c| {
                c.user_id == user_id
                    && (include_archived || c.archived_at.is_none())
                    && workspace.is_none_or(|ws| c.workspace.as_deref() == Some(ws))
            })
            .cloned()
            .collect();
        result.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        let start = offset.min(result.len());
        let end = start.saturating_add(limit).min(result.len());
        result = result[start..end].to_vec();
        Ok(result)
    }

    async fn delete_conversation(&self, id: &ConversationId) -> Result<()> {
        let mut conversations = self.conversations.lock().await;
        conversations.remove(id);
        let mut messages = self.messages.lock().await;
        messages.remove(id);
        Ok(())
    }

    async fn append_message(&self, message: &ConversationMessage) -> Result<()> {
        self.upsert_message(message).await
    }

    async fn upsert_message(&self, message: &ConversationMessage) -> Result<()> {
        let mut messages = self.messages.lock().await;
        let items = messages.entry(message.conversation_id).or_default();
        if let Some(existing) = items.iter_mut().find(|item| item.id == message.id) {
            *existing = message.clone();
        } else {
            items.push(message.clone());
        }
        Ok(())
    }

    async fn get_message(
        &self,
        conversation_id: &ConversationId,
        message_id: &MessageId,
    ) -> Result<Option<ConversationMessage>> {
        let messages = self.messages.lock().await;
        Ok(messages
            .get(conversation_id)
            .and_then(|items| items.iter().find(|item| &item.id == message_id).cloned()))
    }

    async fn list_messages(
        &self,
        conversation_id: &ConversationId,
        after: Option<&MessageCursor>,
        before: Option<&MessageCursor>,
        limit: usize,
    ) -> Result<Vec<ConversationMessage>> {
        let messages = self.messages.lock().await;
        let mut all = messages.get(conversation_id).cloned().unwrap_or_default();
        all.sort_by_key(|m| (m.created_at, m.id.as_uuid()));
        Ok(apply_message_cursors(all, after, before, limit))
    }

    async fn delete_message(
        &self,
        conversation_id: &ConversationId,
        message_id: &MessageId,
    ) -> Result<()> {
        let mut messages = self.messages.lock().await;
        if let Some(items) = messages.get_mut(conversation_id) {
            items.retain(|item| &item.id != message_id);
        }
        Ok(())
    }

    async fn set_read_watermark(
        &self,
        user_id: &str,
        conversation_id: &ConversationId,
        cursor: &MessageCursor,
    ) -> Result<()> {
        let mut watermarks = self.read_watermarks.lock().await;
        let key = (user_id.to_string(), *conversation_id);
        let should_update = watermarks.get(&key).is_none_or(|existing| {
            cursor.created_at_ms > existing.created_at_ms
                || (cursor.created_at_ms == existing.created_at_ms
                    && cursor.message_id > existing.message_id)
        });
        if should_update {
            watermarks.insert(key, cursor.clone());
        }
        Ok(())
    }

    async fn get_read_watermark(
        &self,
        user_id: &str,
        conversation_id: &ConversationId,
    ) -> Result<Option<MessageCursor>> {
        let watermarks = self.read_watermarks.lock().await;
        Ok(watermarks
            .get(&(user_id.to_string(), *conversation_id))
            .cloned())
    }
}

// ---------------------------------------------------------------------------
// InMemoryArtifactStore
// ---------------------------------------------------------------------------

type ArtifactMap = Arc<Mutex<HashMap<ArtifactId, (ConversationArtifact, Vec<u8>)>>>;

/// In-memory [`ArtifactStore`] implementation for use in tests.
pub struct InMemoryArtifactStore {
    artifacts: ArtifactMap,
}

impl InMemoryArtifactStore {
    /// Create a new empty artifact store.
    pub fn new() -> Self {
        Self {
            artifacts: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Default for InMemoryArtifactStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ArtifactStore for InMemoryArtifactStore {
    async fn put_artifact(&self, artifact: &ConversationArtifact, bytes: &[u8]) -> Result<()> {
        let mut artifacts = self.artifacts.lock().await;
        artifacts.insert(artifact.id, (artifact.clone(), bytes.to_vec()));
        Ok(())
    }

    async fn update_artifact(&self, artifact: &ConversationArtifact) -> Result<()> {
        let mut artifacts = self.artifacts.lock().await;
        if let Some((stored, _)) = artifacts.get_mut(&artifact.id) {
            *stored = artifact.clone();
        }
        Ok(())
    }

    async fn get_artifact(&self, artifact_id: &ArtifactId) -> Result<Option<ConversationArtifact>> {
        let artifacts = self.artifacts.lock().await;
        Ok(artifacts
            .get(artifact_id)
            .map(|(artifact, _)| artifact.clone()))
    }

    async fn get_artifact_bytes(&self, artifact_id: &ArtifactId) -> Result<Option<Vec<u8>>> {
        let artifacts = self.artifacts.lock().await;
        Ok(artifacts.get(artifact_id).map(|(_, bytes)| bytes.clone()))
    }

    async fn delete_artifact(&self, artifact_id: &ArtifactId) -> Result<()> {
        let mut artifacts = self.artifacts.lock().await;
        artifacts.remove(artifact_id);
        Ok(())
    }

    async fn list_artifacts_by_conversation(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<Vec<ConversationArtifact>> {
        let artifacts = self.artifacts.lock().await;
        let mut result: Vec<ConversationArtifact> = artifacts
            .values()
            .filter(|(a, _)| a.conversation_id.as_ref() == Some(conversation_id))
            .map(|(a, _)| a.clone())
            .collect();
        result.sort_by_key(|a| a.created_at);
        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// InMemoryQueue
// ---------------------------------------------------------------------------

/// In-memory [`PriorityQueue`] implementation for use in tests.
pub struct InMemoryQueue {
    items: Arc<Mutex<Vec<Envelope>>>,
    dlq: Arc<Mutex<Vec<Envelope>>>,
    notify: Arc<tokio::sync::Notify>,
}

impl InMemoryQueue {
    /// Create a new empty queue.
    pub fn new() -> Self {
        Self {
            items: Arc::new(Mutex::new(Vec::new())),
            dlq: Arc::new(Mutex::new(Vec::new())),
            notify: Arc::new(tokio::sync::Notify::new()),
        }
    }

    /// Return all envelopes currently in the dead-letter queue.
    pub async fn dlq_items(&self) -> Vec<Envelope> {
        self.dlq.lock().await.clone()
    }

    fn priority_bucket(p: Priority) -> u8 {
        match p {
            Priority::Urgent => 0,
            Priority::Normal => 1,
            Priority::Background => 2,
        }
    }
}

impl Default for InMemoryQueue {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PriorityQueue for InMemoryQueue {
    async fn push(&self, envelope: &Envelope) -> Result<()> {
        let mut items = self.items.lock().await;
        items.push(envelope.clone());
        // Sort: lower bucket = higher priority, then by timestamp (earlier first)
        items.sort_by(|a, b| {
            let ba = Self::priority_bucket(a.priority);
            let bb = Self::priority_bucket(b.priority);
            ba.cmp(&bb).then_with(|| a.timestamp.cmp(&b.timestamp))
        });
        self.notify.notify_one();
        Ok(())
    }

    async fn pop(&self, timeout: Duration) -> Result<Option<Envelope>> {
        // Try immediate pop
        {
            let mut items = self.items.lock().await;
            if !items.is_empty() {
                return Ok(Some(items.remove(0)));
            }
        }
        // Wait for notification or timeout
        match tokio::time::timeout(timeout, self.notify.notified()).await {
            Ok(()) => {
                let mut items = self.items.lock().await;
                if items.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(items.remove(0)))
                }
            }
            Err(_) => Ok(None),
        }
    }

    async fn len(&self) -> Result<usize> {
        let items = self.items.lock().await;
        Ok(items.len())
    }
}

#[async_trait]
impl DeadLetterQueue for InMemoryQueue {
    async fn push(&self, envelope: &Envelope) -> Result<()> {
        let mut dlq = self.dlq.lock().await;
        dlq.push(envelope.clone());
        Ok(())
    }

    async fn list(&self) -> Result<Vec<Envelope>> {
        Ok(self.dlq.lock().await.clone())
    }

    async fn purge(&self) -> Result<usize> {
        let mut dlq = self.dlq.lock().await;
        let count = dlq.len();
        dlq.clear();
        Ok(count)
    }

    async fn replay(&self, id: &MessageId) -> Result<bool> {
        let mut dlq = self.dlq.lock().await;
        if let Some(pos) = dlq.iter().position(|e| &e.id == id) {
            let mut envelope = dlq.remove(pos);
            drop(dlq);
            envelope.metadata.remove("retry_count");
            envelope.priority = Priority::Normal;
            PriorityQueue::push(self, &envelope).await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

// ---------------------------------------------------------------------------
// InMemoryMemoryStore
// ---------------------------------------------------------------------------

/// In-memory [`MemoryStore`] implementation for use in tests.
pub struct InMemoryMemoryStore {
    #[allow(clippy::type_complexity)]
    entries: Arc<Mutex<HashMap<String, (MemoryEntry, Option<tokio::time::Instant>)>>>,
}

impl InMemoryMemoryStore {
    /// Create a new empty memory store.
    pub fn new() -> Self {
        Self {
            entries: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Default for InMemoryMemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MemoryStore for InMemoryMemoryStore {
    async fn store(&self, key: &str, value: MemoryEntry, ttl: Option<Duration>) -> Result<()> {
        let deadline = ttl.map(|d| tokio::time::Instant::now() + d);
        let mut entries = self.entries.lock().await;
        entries.insert(key.to_string(), (value, deadline));
        Ok(())
    }

    async fn recall(&self, key: &str) -> Result<Option<MemoryEntry>> {
        let mut entries = self.entries.lock().await;
        if let Some((entry, deadline)) = entries.get(key) {
            if let Some(dl) = deadline
                && tokio::time::Instant::now() >= *dl
            {
                entries.remove(key);
                return Ok(None);
            }
            Ok(Some(entry.clone()))
        } else {
            Ok(None)
        }
    }

    async fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryEntry>> {
        let now = tokio::time::Instant::now();
        let entries = self.entries.lock().await;
        let results: Vec<MemoryEntry> = entries
            .values()
            .filter(|(_, deadline)| deadline.is_none_or(|dl| now < dl))
            .filter(|(entry, _)| {
                entry.key.contains(query)
                    || entry.tags.iter().any(|t| t.contains(query))
                    || entry.source.contains(query)
                    || entry
                        .metadata
                        .iter()
                        .any(|(k, v)| k.contains(query) || v.contains(query))
            })
            .take(limit)
            .map(|(entry, _)| entry.clone())
            .collect();
        Ok(results)
    }

    async fn list(&self, prefix: Option<&str>, limit: usize) -> Result<Vec<MemoryEntry>> {
        let now = tokio::time::Instant::now();
        let entries = self.entries.lock().await;
        let mut results: Vec<MemoryEntry> = entries
            .iter()
            .filter(|(_, (_, deadline))| deadline.is_none_or(|dl| now < dl))
            .filter(|(key, _)| prefix.is_none_or(|p| key.starts_with(p)))
            .map(|(_, (entry, _))| entry.clone())
            .collect();
        results.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        results.truncate(limit);
        Ok(results)
    }

    async fn delete(&self, key: &str) -> Result<bool> {
        let mut entries = self.entries.lock().await;
        Ok(entries.remove(key).is_some())
    }

    async fn compact(&self) -> Result<usize> {
        let now = tokio::time::Instant::now();
        let mut entries = self.entries.lock().await;
        let before = entries.len();
        entries.retain(|_, (_, deadline)| deadline.is_none_or(|dl| now < dl));
        Ok(before - entries.len())
    }
}

#[async_trait]
impl SessionLock for InMemoryMemoryStore {
    async fn try_acquire(&self, _session_id: &str, _ttl_ms: u64) -> bool {
        true
    }

    async fn release(&self, _session_id: &str) {}
}

// ---------------------------------------------------------------------------
// InMemorySecretManager
// ---------------------------------------------------------------------------

/// In-memory [`SecretManager`] implementation for use in tests.
pub struct InMemorySecretManager {
    secrets: Arc<Mutex<HashMap<String, Vec<u8>>>>,
}

impl InMemorySecretManager {
    /// Create a new empty secret manager.
    pub fn new() -> Self {
        Self {
            secrets: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Default for InMemorySecretManager {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SecretManager for InMemorySecretManager {
    async fn get_secret(&self, path: &str) -> Result<SecretValue> {
        let secrets = self.secrets.lock().await;
        secrets
            .get(path)
            .map(|v| SecretValue::new(v.clone()))
            .ok_or_else(|| Error::secret(format!("not found: {path}")))
    }

    async fn set_secret(&self, path: &str, value: &SecretValue) -> Result<()> {
        let mut secrets = self.secrets.lock().await;
        secrets.insert(path.to_string(), value.expose().to_vec());
        Ok(())
    }

    async fn delete_secret(&self, path: &str) -> Result<()> {
        let mut secrets = self.secrets.lock().await;
        secrets
            .remove(path)
            .map(|_| ())
            .ok_or_else(|| Error::secret(format!("not found: {path}")))
    }

    async fn list_secrets(&self) -> Result<Vec<String>> {
        let secrets = self.secrets.lock().await;
        Ok(secrets.keys().cloned().collect())
    }
}

// ---------------------------------------------------------------------------
// InMemoryEventSink
// ---------------------------------------------------------------------------

/// In-memory [`EventSink`] implementation for use in tests.
pub struct InMemoryEventSink {
    events: Arc<Mutex<Vec<DomainEvent>>>,
}

impl InMemoryEventSink {
    /// Create a new empty event sink.
    pub fn new() -> Self {
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Return all events that have been emitted so far.
    pub async fn events(&self) -> Vec<DomainEvent> {
        self.events.lock().await.clone()
    }

    /// Clear all recorded events.
    pub async fn clear(&self) {
        self.events.lock().await.clear();
    }
}

impl Default for InMemoryEventSink {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EventSink for InMemoryEventSink {
    async fn emit(&self, event: DomainEvent) {
        self.events.lock().await.push(event);
    }
}

// ---------------------------------------------------------------------------
// Guardrail test doubles
// ---------------------------------------------------------------------------

/// [`Guardrail`] that always allows all input and output content.
///
/// Use in tests where guardrails must be present but should never block.
pub struct AlwaysPassGuardrail;

#[async_trait]
impl Guardrail for AlwaysPassGuardrail {
    async fn check_input(&self, _input: &str, _session: &Session) -> Result<GuardrailDecision> {
        Ok(GuardrailDecision::Allow)
    }

    async fn check_output(&self, _output: &str, _session: &Session) -> Result<GuardrailDecision> {
        Ok(GuardrailDecision::Allow)
    }
}

/// [`Guardrail`] that always blocks all content with a fixed reason.
///
/// Use in tests that verify behavior when a guardrail fires.
pub struct AlwaysBlockGuardrail;

#[async_trait]
impl Guardrail for AlwaysBlockGuardrail {
    async fn check_input(&self, _input: &str, _session: &Session) -> Result<GuardrailDecision> {
        Ok(GuardrailDecision::Block("blocked by test guardrail".into()))
    }

    async fn check_output(&self, _output: &str, _session: &Session) -> Result<GuardrailDecision> {
        Ok(GuardrailDecision::Block("blocked by test guardrail".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn in_memory_bus_pubsub() -> Result<()> {
        let bus = InMemoryBus::new();
        let mut rx = bus.subscribe("test").await?;
        let env = Envelope::text("ch", SessionId::new(), "hello");
        bus.publish("test", &env).await?;
        let Some(received) = rx.recv().await else {
            panic!("expected published message")
        };
        assert_eq!(received.id, env.id);
        Ok(())
    }

    #[tokio::test]
    async fn in_memory_session_crud() -> Result<()> {
        let store = InMemorySessionStore::new();
        let session = Session::new("ch", "user1");
        store.put(&session).await?;
        let Some(got) = store.get(&session.id).await? else {
            panic!("expected stored session")
        };
        assert_eq!(got.user_id, "user1");
        store.delete(&session.id).await?;
        assert!(store.get(&session.id).await?.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn in_memory_queue_priority_order() -> Result<()> {
        let queue = InMemoryQueue::new();
        let mut bg = Envelope::text("ch", SessionId::new(), "bg");
        bg.priority = Priority::Background;
        let mut urgent = Envelope::text("ch", SessionId::new(), "urgent");
        urgent.priority = Priority::Urgent;
        let normal = Envelope::text("ch", SessionId::new(), "normal");

        PriorityQueue::push(&queue, &bg).await?;
        PriorityQueue::push(&queue, &normal).await?;
        PriorityQueue::push(&queue, &urgent).await?;

        let Some(first) = queue.pop(Duration::from_millis(10)).await? else {
            panic!("expected urgent item")
        };
        assert_eq!(first.priority, Priority::Urgent);
        let Some(second) = queue.pop(Duration::from_millis(10)).await? else {
            panic!("expected normal item")
        };
        assert_eq!(second.priority, Priority::Normal);
        let Some(third) = queue.pop(Duration::from_millis(10)).await? else {
            panic!("expected background item")
        };
        assert_eq!(third.priority, Priority::Background);
        Ok(())
    }

    #[tokio::test]
    async fn in_memory_queue_empty_timeout() -> Result<()> {
        let queue = InMemoryQueue::new();
        let result = queue.pop(Duration::from_millis(50)).await?;
        assert!(result.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn in_memory_memory_store_recall() -> Result<()> {
        let store = InMemoryMemoryStore::new();
        let entry = MemoryEntry {
            key: "test-key".into(),
            kind: crate::MemoryKind::Working,
            scope: crate::MemoryScope::Session,
            source: "system".into(),
            value: serde_json::json!({"data": "hello"}),
            metadata: HashMap::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            tags: vec!["tag1".into()],
        };
        store.store("test-key", entry.clone(), None).await?;
        let Some(recalled) = store.recall("test-key").await? else {
            panic!("expected stored memory entry")
        };
        assert_eq!(recalled.key, "test-key");
        assert_eq!(recalled.value, serde_json::json!({"data": "hello"}));
        Ok(())
    }

    #[tokio::test]
    async fn in_memory_memory_store_ttl_expiry() -> Result<()> {
        let store = InMemoryMemoryStore::new();
        let entry = MemoryEntry {
            key: "ephemeral".into(),
            kind: crate::MemoryKind::Working,
            scope: crate::MemoryScope::Session,
            source: "system".into(),
            value: serde_json::json!("gone soon"),
            metadata: HashMap::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            tags: vec![],
        };
        store
            .store("ephemeral", entry, Some(Duration::from_millis(50)))
            .await?;
        assert!(store.recall("ephemeral").await?.is_some());
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(store.recall("ephemeral").await?.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn in_memory_memory_store_search() -> Result<()> {
        let store = InMemoryMemoryStore::new();
        let entry1 = MemoryEntry {
            key: "user-profile".into(),
            kind: crate::MemoryKind::Semantic,
            scope: crate::MemoryScope::User,
            source: "user".into(),
            value: serde_json::json!({}),
            metadata: HashMap::from([("workspace".into(), "main".into())]),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            tags: vec!["user".into()],
        };
        let entry2 = MemoryEntry {
            key: "system-config".into(),
            kind: crate::MemoryKind::Working,
            scope: crate::MemoryScope::Global,
            source: "system".into(),
            value: serde_json::json!({}),
            metadata: HashMap::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            tags: vec!["system".into()],
        };
        store.store("user-profile", entry1, None).await?;
        store.store("system-config", entry2, None).await?;

        let results = store.search("user", 10).await?;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].key, "user-profile");
        assert_eq!(store.search("main", 10).await?.len(), 1);
        assert_eq!(store.list(Some("system"), 10).await?.len(), 1);
        assert!(store.delete("system-config").await?);
        Ok(())
    }

    #[tokio::test]
    async fn in_memory_secret_manager_crud() -> Result<()> {
        let mgr = InMemorySecretManager::new();
        let secret = SecretValue::new(b"super-secret".to_vec());
        mgr.set_secret("api/key", &secret).await?;

        let retrieved = mgr.get_secret("api/key").await?;
        assert_eq!(retrieved.expose(), b"super-secret");
        Ok(())
    }

    #[tokio::test]
    async fn in_memory_secret_manager_not_found() {
        let mgr = InMemorySecretManager::new();
        let result = mgr.get_secret("nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn in_memory_secret_manager_delete() -> Result<()> {
        let mgr = InMemorySecretManager::new();
        let secret = SecretValue::new(b"to-delete".to_vec());
        mgr.set_secret("temp/key", &secret).await?;
        assert!(mgr.get_secret("temp/key").await.is_ok());

        mgr.delete_secret("temp/key").await?;
        assert!(mgr.get_secret("temp/key").await.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn in_memory_secret_manager_delete_not_found() {
        let mgr = InMemorySecretManager::new();
        assert!(mgr.delete_secret("nonexistent").await.is_err());
    }

    #[tokio::test]
    async fn in_memory_secret_manager_list() -> Result<()> {
        let mgr = InMemorySecretManager::new();
        assert!(mgr.list_secrets().await?.is_empty());

        mgr.set_secret("a", &SecretValue::new(b"1".to_vec()))
            .await?;
        mgr.set_secret("b", &SecretValue::new(b"2".to_vec()))
            .await?;

        let mut keys = mgr.list_secrets().await?;
        keys.sort();
        assert_eq!(keys, vec!["a", "b"]);
        Ok(())
    }

    #[tokio::test]
    async fn in_memory_event_sink_emit_and_retrieve() {
        use crate::{DomainEventKind, EventId};

        let sink = InMemoryEventSink::new();
        let event = DomainEvent {
            id: EventId::new(),
            timestamp: chrono::Utc::now(),
            kind: DomainEventKind::ErrorOccurred {
                source: "test".into(),
                message: "something went wrong".into(),
            },
            metadata: std::collections::HashMap::default(),
        };
        sink.emit(event.clone()).await;

        let events = sink.events().await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, event.id);

        sink.clear().await;
        assert!(sink.events().await.is_empty());
    }

    #[tokio::test]
    async fn in_memory_artifact_store_put_get() -> Result<()> {
        let store = InMemoryArtifactStore::new();
        let artifact = ConversationArtifact::new(
            "user1",
            crate::ConversationArtifactOrigin::UserUpload,
            "image/png",
            "photo.png",
        );
        let bytes = b"fake-png-data";
        store.put_artifact(&artifact, bytes).await?;

        let got = store.get_artifact(&artifact.id).await?;
        assert!(got.is_some());
        let got = got.ok_or_else(|| Error::Other("artifact must exist".into()))?;
        assert_eq!(got.filename, "photo.png");

        let got_bytes = store.get_artifact_bytes(&artifact.id).await?;
        let got_bytes =
            got_bytes.ok_or_else(|| Error::Other("artifact bytes must exist".into()))?;
        assert_eq!(got_bytes, bytes);
        Ok(())
    }

    #[tokio::test]
    async fn in_memory_artifact_store_update() -> Result<()> {
        let store = InMemoryArtifactStore::new();
        let mut artifact = ConversationArtifact::new(
            "user1",
            crate::ConversationArtifactOrigin::UserUpload,
            "image/png",
            "photo.png",
        );
        store.put_artifact(&artifact, b"bytes").await?;

        artifact.caption = Some("A caption".to_string());
        store.update_artifact(&artifact).await?;

        let got = store
            .get_artifact(&artifact.id)
            .await?
            .ok_or_else(|| Error::Other("artifact must exist after update".into()))?;
        assert_eq!(got.caption.as_deref(), Some("A caption"));
        Ok(())
    }

    #[tokio::test]
    async fn in_memory_artifact_store_delete() -> Result<()> {
        let store = InMemoryArtifactStore::new();
        let artifact = ConversationArtifact::new(
            "user1",
            crate::ConversationArtifactOrigin::UserUpload,
            "image/png",
            "photo.png",
        );
        store.put_artifact(&artifact, b"bytes").await?;
        store.delete_artifact(&artifact.id).await?;
        assert!(store.get_artifact(&artifact.id).await?.is_none());
        assert!(store.get_artifact_bytes(&artifact.id).await?.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn in_memory_artifact_store_list_by_conversation() -> Result<()> {
        let store = InMemoryArtifactStore::new();
        let conv_id = ConversationId::new();

        let mut a1 = ConversationArtifact::new(
            "user1",
            crate::ConversationArtifactOrigin::UserUpload,
            "image/png",
            "a1.png",
        );
        a1.conversation_id = Some(conv_id);
        store.put_artifact(&a1, b"bytes1").await?;

        let mut a2 = ConversationArtifact::new(
            "user1",
            crate::ConversationArtifactOrigin::UserUpload,
            "image/jpeg",
            "a2.jpg",
        );
        a2.conversation_id = Some(conv_id);
        store.put_artifact(&a2, b"bytes2").await?;

        // Artifact belonging to another conversation — must not appear
        let a3 = ConversationArtifact::new(
            "user1",
            crate::ConversationArtifactOrigin::UserUpload,
            "text/plain",
            "other.txt",
        );
        store.put_artifact(&a3, b"bytes3").await?;

        let listed = store.list_artifacts_by_conversation(&conv_id).await?;
        assert_eq!(listed.len(), 2);
        assert!(listed.iter().any(|a| a.filename == "a1.png"));
        assert!(listed.iter().any(|a| a.filename == "a2.jpg"));
        Ok(())
    }
}
