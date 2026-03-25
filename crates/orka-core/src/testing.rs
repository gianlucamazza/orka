use std::{collections::HashMap, sync::Arc, time::Duration};

use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::{
    DomainEvent, Envelope, Error, MemoryEntry, MessageId, MessageStream, Priority, Result,
    SecretValue, Session, SessionId, SkillInput, SkillOutput, SkillSchema,
    traits::{
        DeadLetterQueue, EventSink, MemoryStore, MessageBus, PriorityQueue, SecretManager,
        SessionLock, SessionStore, Skill,
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
                entry.key.contains(query) || entry.tags.iter().any(|t| t.contains(query))
            })
            .take(limit)
            .map(|(entry, _)| entry.clone())
            .collect();
        Ok(results)
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
// EchoSkill
// ---------------------------------------------------------------------------

/// Test [`Skill`] that echoes its input arguments back as JSON output.
pub struct EchoSkill;

#[async_trait]
impl Skill for EchoSkill {
    fn name(&self) -> &'static str {
        "echo"
    }

    fn description(&self) -> &'static str {
        "Echoes back the input arguments"
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema {
            parameters: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": true
            }),
        }
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        Ok(SkillOutput {
            data: serde_json::to_value(input.args).map_err(|e| Error::Skill(e.to_string()))?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn in_memory_bus_pubsub() {
        let bus = InMemoryBus::new();
        let mut rx = bus.subscribe("test").await.unwrap();
        let env = Envelope::text("ch", SessionId::new(), "hello");
        bus.publish("test", &env).await.unwrap();
        let received = rx.recv().await.unwrap();
        assert_eq!(received.id, env.id);
    }

    #[tokio::test]
    async fn in_memory_session_crud() {
        let store = InMemorySessionStore::new();
        let session = Session::new("ch", "user1");
        store.put(&session).await.unwrap();
        let got = store.get(&session.id).await.unwrap().unwrap();
        assert_eq!(got.user_id, "user1");
        store.delete(&session.id).await.unwrap();
        assert!(store.get(&session.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn in_memory_queue_priority_order() {
        let queue = InMemoryQueue::new();
        let mut bg = Envelope::text("ch", SessionId::new(), "bg");
        bg.priority = Priority::Background;
        let mut urgent = Envelope::text("ch", SessionId::new(), "urgent");
        urgent.priority = Priority::Urgent;
        let normal = Envelope::text("ch", SessionId::new(), "normal");

        PriorityQueue::push(&queue, &bg).await.unwrap();
        PriorityQueue::push(&queue, &normal).await.unwrap();
        PriorityQueue::push(&queue, &urgent).await.unwrap();

        let first = queue.pop(Duration::from_millis(10)).await.unwrap().unwrap();
        assert_eq!(first.priority, Priority::Urgent);
        let second = queue.pop(Duration::from_millis(10)).await.unwrap().unwrap();
        assert_eq!(second.priority, Priority::Normal);
        let third = queue.pop(Duration::from_millis(10)).await.unwrap().unwrap();
        assert_eq!(third.priority, Priority::Background);
    }

    #[tokio::test]
    async fn in_memory_queue_empty_timeout() {
        let queue = InMemoryQueue::new();
        let result = queue.pop(Duration::from_millis(50)).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn in_memory_memory_store_recall() {
        let store = InMemoryMemoryStore::new();
        let entry = MemoryEntry {
            key: "test-key".into(),
            value: serde_json::json!({"data": "hello"}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            tags: vec!["tag1".into()],
        };
        store.store("test-key", entry.clone(), None).await.unwrap();
        let recalled = store.recall("test-key").await.unwrap().unwrap();
        assert_eq!(recalled.key, "test-key");
        assert_eq!(recalled.value, serde_json::json!({"data": "hello"}));
    }

    #[tokio::test]
    async fn in_memory_memory_store_ttl_expiry() {
        let store = InMemoryMemoryStore::new();
        let entry = MemoryEntry {
            key: "ephemeral".into(),
            value: serde_json::json!("gone soon"),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            tags: vec![],
        };
        store
            .store("ephemeral", entry, Some(Duration::from_millis(50)))
            .await
            .unwrap();
        assert!(store.recall("ephemeral").await.unwrap().is_some());
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(store.recall("ephemeral").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn in_memory_memory_store_search() {
        let store = InMemoryMemoryStore::new();
        let entry1 = MemoryEntry {
            key: "user-profile".into(),
            value: serde_json::json!({}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            tags: vec!["user".into()],
        };
        let entry2 = MemoryEntry {
            key: "system-config".into(),
            value: serde_json::json!({}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            tags: vec!["system".into()],
        };
        store.store("user-profile", entry1, None).await.unwrap();
        store.store("system-config", entry2, None).await.unwrap();

        let results = store.search("user", 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].key, "user-profile");
    }

    #[tokio::test]
    async fn in_memory_secret_manager_crud() {
        let mgr = InMemorySecretManager::new();
        let secret = SecretValue::new(b"super-secret".to_vec());
        mgr.set_secret("api/key", &secret).await.unwrap();

        let retrieved = mgr.get_secret("api/key").await.unwrap();
        assert_eq!(retrieved.expose(), b"super-secret");
    }

    #[tokio::test]
    async fn in_memory_secret_manager_not_found() {
        let mgr = InMemorySecretManager::new();
        let result = mgr.get_secret("nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn in_memory_secret_manager_delete() {
        let mgr = InMemorySecretManager::new();
        let secret = SecretValue::new(b"to-delete".to_vec());
        mgr.set_secret("temp/key", &secret).await.unwrap();
        assert!(mgr.get_secret("temp/key").await.is_ok());

        mgr.delete_secret("temp/key").await.unwrap();
        assert!(mgr.get_secret("temp/key").await.is_err());
    }

    #[tokio::test]
    async fn in_memory_secret_manager_delete_not_found() {
        let mgr = InMemorySecretManager::new();
        assert!(mgr.delete_secret("nonexistent").await.is_err());
    }

    #[tokio::test]
    async fn in_memory_secret_manager_list() {
        let mgr = InMemorySecretManager::new();
        assert!(mgr.list_secrets().await.unwrap().is_empty());

        mgr.set_secret("a", &SecretValue::new(b"1".to_vec()))
            .await
            .unwrap();
        mgr.set_secret("b", &SecretValue::new(b"2".to_vec()))
            .await
            .unwrap();

        let mut keys = mgr.list_secrets().await.unwrap();
        keys.sort();
        assert_eq!(keys, vec!["a", "b"]);
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
            metadata: Default::default(),
        };
        sink.emit(event.clone()).await;

        let events = sink.events().await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, event.id);

        sink.clear().await;
        assert!(sink.events().await.is_empty());
    }

    #[tokio::test]
    async fn echo_skill_execute() {
        let skill = EchoSkill;
        assert_eq!(skill.name(), "echo");
        assert!(!skill.description().is_empty());

        let input = SkillInput {
            args: [("greeting".to_string(), serde_json::json!("hello"))]
                .into_iter()
                .collect(),
            context: None,
        };
        let output = skill.execute(input).await.unwrap();
        assert_eq!(output.data, serde_json::json!({"greeting": "hello"}));
    }
}
