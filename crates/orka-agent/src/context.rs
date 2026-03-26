//! Shared execution context that flows through the agent graph during a run.

use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use chrono::Utc;
use orka_checkpoint::{
    Checkpoint, CheckpointId, RunStatus, SerializableSlotKey, SerializableStateChange,
};
use orka_core::{Envelope, SessionId};
use orka_llm::client::ChatMessage;
use serde_json::Value;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::{
    agent::AgentId,
    reducer::{ReducerStrategy, apply_reducer},
};

/// Unique identifier for a single graph execution run.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RunId(pub Uuid);

impl RunId {
    /// Generate a new time-ordered run identifier.
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl Default for RunId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for RunId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A typed key for values stored in the execution context.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SlotKey {
    /// Agent namespace, or "__shared" for cross-agent values.
    pub namespace: String,
    /// Key name within the namespace.
    pub name: String,
}

impl SlotKey {
    /// Create a slot key scoped to a specific agent.
    pub fn agent(agent_id: &AgentId, name: impl Into<String>) -> Self {
        Self {
            namespace: agent_id.0.to_string(),
            name: name.into(),
        }
    }

    /// Create a slot key in the shared cross-agent namespace.
    pub fn shared(name: impl Into<String>) -> Self {
        Self {
            namespace: "__shared".into(),
            name: name.into(),
        }
    }
}

/// A single state mutation recorded for observability.
#[derive(Debug, Clone)]
pub struct StateChange {
    /// UTC wall-clock time when the change occurred.
    ///
    /// Using `DateTime<Utc>` rather than `std::time::Instant` ensures the
    /// changelog can be serialized into checkpoints without loss of meaning.
    pub timestamp: chrono::DateTime<Utc>,
    /// The slot that was modified.
    pub key: SlotKey,
    /// The agent that performed the write.
    pub agent: AgentId,
    /// Previous value, or `None` if this was an insert.
    pub old_value: Option<Value>,
    /// New value written to the slot.
    pub new_value: Value,
}

/// Shared, typed execution state that flows through the graph.
///
/// All access to `state` and `messages` is protected by `RwLock` so
/// fan-out nodes can read concurrently and write to separate namespaces.
#[derive(Clone)]
pub struct ExecutionContext {
    /// Unique identifier for this graph execution run.
    pub run_id: RunId,
    /// Session this run belongs to.
    pub session_id: SessionId,
    /// The inbound envelope that triggered this run.
    pub trigger: Envelope,
    /// Wall-clock time when the run started.
    pub started_at: Instant,
    state: Arc<RwLock<std::collections::HashMap<SlotKey, Value>>>,
    messages: Arc<RwLock<Vec<ChatMessage>>>,
    changelog: Arc<RwLock<VecDeque<StateChange>>>,
    usage: Arc<AtomicU64>,
    /// Accumulated summary of conversation history dropped by
    /// [`HistoryStrategy::Summarize`].
    summary: Arc<RwLock<Option<String>>>,
    /// Per-slot reducer strategies shared across all clones of this context.
    ///
    /// Set once at run start via [`set_reducers`](Self::set_reducers);
    /// effectively immutable during execution. Key format: `"namespace::name"`.
    reducers: Arc<RwLock<HashMap<String, ReducerStrategy>>>,
}

impl ExecutionContext {
    /// Create a new context for the given trigger envelope.
    pub fn new(trigger: Envelope) -> Self {
        let session_id = trigger.session_id;
        Self {
            run_id: RunId::new(),
            session_id,
            trigger,
            started_at: Instant::now(),
            state: Arc::new(RwLock::new(std::collections::HashMap::new())),
            messages: Arc::new(RwLock::new(Vec::new())),
            changelog: Arc::new(RwLock::new(VecDeque::new())),
            usage: Arc::new(AtomicU64::new(0)),
            summary: Arc::new(RwLock::new(None)),
            reducers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Read a value from the state map.
    pub async fn get(&self, key: &SlotKey) -> Option<Value> {
        self.state.read().await.get(key).cloned()
    }

    /// Read and deserialize a typed value.
    pub async fn get_typed<T: serde::de::DeserializeOwned>(&self, key: &SlotKey) -> Option<T> {
        let v = self.get(key).await?;
        serde_json::from_value(v).ok()
    }

    /// Write a value to the state map, recording the change.
    ///
    /// If a [`ReducerStrategy`] is registered for this slot (via
    /// [`set_reducers`](Self::set_reducers)) it is applied before storing the
    /// value so that concurrent fan-out writes merge deterministically.
    pub async fn set(&self, agent: &AgentId, key: SlotKey, value: Value) {
        // Build the "namespace::name" key used by the reducer registry.
        let reducer_key = format!("{}::{}", key.namespace, key.name);
        let strategy = self
            .reducers
            .read()
            .await
            .get(&reducer_key)
            .copied()
            .unwrap_or_default();

        let old_value = {
            let mut state = self.state.write().await;
            let old = state.get(&key).cloned();
            let merged = apply_reducer(strategy, old.as_ref(), &value);
            state.insert(key.clone(), merged);
            old
        };

        let change = StateChange {
            timestamp: Utc::now(),
            key: key.clone(),
            agent: agent.clone(),
            old_value,
            new_value: value,
        };

        tracing::info!(
            namespace = %key.namespace,
            name = %key.name,
            agent = %agent,
            "execution_context.state_change"
        );

        self.changelog.write().await.push_back(change);
    }

    /// Get the current conversation messages.
    pub async fn messages(&self) -> Vec<ChatMessage> {
        self.messages.read().await.clone()
    }

    /// Append a message to the conversation.
    pub async fn push_message(&self, msg: ChatMessage) {
        self.messages.write().await.push(msg);
    }

    /// Replace the conversation messages (e.g., after history truncation).
    pub async fn set_messages(&self, msgs: Vec<ChatMessage>) {
        *self.messages.write().await = msgs;
    }

    /// Accumulate token usage.
    pub fn add_tokens(&self, n: u64) {
        self.usage.fetch_add(n, Ordering::Relaxed);
    }

    /// Get total tokens consumed.
    pub fn total_tokens(&self) -> u64 {
        self.usage.load(Ordering::Relaxed)
    }

    /// Get the full changelog for observability.
    pub async fn changelog(&self) -> Vec<StateChange> {
        self.changelog.read().await.iter().cloned().collect()
    }

    /// Get the current conversation summary (set by
    /// `HistoryStrategy::Summarize`).
    pub async fn conversation_summary(&self) -> Option<String> {
        self.summary.read().await.clone()
    }

    /// Store a conversation summary produced by history summarization.
    pub async fn set_conversation_summary(&self, s: String) {
        *self.summary.write().await = Some(s);
    }

    /// Elapsed time since this run started.
    pub fn elapsed_ms(&self) -> u64 {
        self.started_at.elapsed().as_millis() as u64
    }

    /// Snapshot the current execution state into a [`Checkpoint`].
    ///
    /// Called by the executor after each node completes and the next node has
    /// been determined. `completed_node` is the node that just finished;
    /// `resume_node` is the node the executor will start from on resume
    /// (`None` when the run has reached a terminal state).
    pub async fn to_checkpoint(
        &self,
        graph_id: &str,
        completed_node: &str,
        resume_node: Option<&str>,
        total_iterations: usize,
        agents_executed: Vec<String>,
        status: RunStatus,
    ) -> Checkpoint {
        // Serialize state map: SlotKey → "namespace::name" string keys.
        let state: HashMap<String, Value> = self
            .state
            .read()
            .await
            .iter()
            .map(|(k, v)| {
                let key = SerializableSlotKey {
                    namespace: k.namespace.clone(),
                    name: k.name.clone(),
                };
                (key.to_map_key(), v.clone())
            })
            .collect();

        let messages = self.messages.read().await.clone();

        let changelog: Vec<SerializableStateChange> = self
            .changelog
            .read()
            .await
            .iter()
            .map(|c| SerializableStateChange {
                timestamp: c.timestamp,
                slot: SerializableSlotKey {
                    namespace: c.key.namespace.clone(),
                    name: c.key.name.clone(),
                }
                .to_map_key(),
                agent_id: c.agent.to_string(),
                old_value: c.old_value.clone(),
                new_value: c.new_value.clone(),
            })
            .collect();

        Checkpoint {
            id: CheckpointId::new(),
            run_id: self.run_id.to_string(),
            session_id: self.session_id,
            graph_id: graph_id.to_string(),
            trigger: self.trigger.clone(),
            completed_node: completed_node.to_string(),
            resume_node: resume_node.map(std::string::ToString::to_string),
            state,
            messages,
            total_tokens: self.total_tokens(),
            total_iterations,
            agents_executed,
            changelog,
            status,
            created_at: Utc::now(),
        }
    }

    /// Reconstruct an `ExecutionContext` from a persisted [`Checkpoint`].
    ///
    /// Used by the executor's `resume` path to restore execution state after
    /// a crash or HITL interruption.
    pub async fn from_checkpoint(checkpoint: &Checkpoint) -> Self {
        let session_id = checkpoint.session_id;
        let trigger = checkpoint.trigger.clone();

        // Deserialize state map.
        let state: HashMap<SlotKey, Value> = checkpoint
            .state
            .iter()
            .filter_map(|(key_str, value): (&String, &Value)| {
                let parsed = SerializableSlotKey::from_map_key(key_str)?;
                let slot = SlotKey {
                    namespace: parsed.namespace,
                    name: parsed.name,
                };
                Some((slot, value.clone()))
            })
            .collect();

        // Deserialize changelog.
        let changelog: VecDeque<StateChange> = checkpoint
            .changelog
            .iter()
            .filter_map(|c| {
                let parsed = SerializableSlotKey::from_map_key(&c.slot)?;
                Some(StateChange {
                    timestamp: c.timestamp,
                    key: SlotKey {
                        namespace: parsed.namespace,
                        name: parsed.name,
                    },
                    agent: AgentId::new(c.agent_id.as_str()),
                    old_value: c.old_value.clone(),
                    new_value: c.new_value.clone(),
                })
            })
            .collect();

        Self {
            run_id: RunId(
                checkpoint
                    .run_id
                    .parse()
                    .unwrap_or_else(|_| uuid::Uuid::now_v7()),
            ),
            session_id,
            trigger,
            started_at: Instant::now(),
            state: Arc::new(RwLock::new(state)),
            messages: Arc::new(RwLock::new(checkpoint.messages.clone())),
            changelog: Arc::new(RwLock::new(changelog)),
            usage: Arc::new(AtomicU64::new(checkpoint.total_tokens)),
            summary: Arc::new(RwLock::new(None)),
            reducers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register reducer strategies for this run.
    ///
    /// Must be called before any fan-out agent begins writing.  The reducers
    /// are shared across all clones of this context (all branches see the same
    /// map since it is `Arc`-wrapped).
    pub async fn set_reducers(&self, reducers: HashMap<String, ReducerStrategy>) {
        *self.reducers.write().await = reducers;
    }
}

#[cfg(test)]
mod tests {
    use orka_core::{Envelope, SessionId};

    use super::*;

    fn make_context() -> ExecutionContext {
        let env = Envelope::text("test-channel", SessionId::new(), "hello");
        ExecutionContext::new(env)
    }

    #[tokio::test]
    async fn get_returns_none_for_missing_key() {
        let ctx = make_context();
        let key = SlotKey::shared("missing");
        assert!(ctx.get(&key).await.is_none());
    }

    #[tokio::test]
    async fn set_then_get_roundtrip() {
        let ctx = make_context();
        let agent = AgentId::new("a1");
        let key = SlotKey::agent(&agent, "score");
        ctx.set(&agent, key.clone(), serde_json::json!(42)).await;
        assert_eq!(ctx.get(&key).await, Some(serde_json::json!(42)));
    }

    #[tokio::test]
    async fn set_records_changelog() {
        let ctx = make_context();
        let agent = AgentId::new("a1");
        let key = SlotKey::shared("status");
        ctx.set(&agent, key.clone(), serde_json::json!("pending"))
            .await;
        ctx.set(&agent, key.clone(), serde_json::json!("done"))
            .await;

        let log = ctx.changelog().await;
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].new_value, serde_json::json!("pending"));
        assert_eq!(log[0].old_value, None);
        assert_eq!(log[1].new_value, serde_json::json!("done"));
        assert_eq!(log[1].old_value, Some(serde_json::json!("pending")));
    }

    #[tokio::test]
    async fn token_accounting() {
        let ctx = make_context();
        assert_eq!(ctx.total_tokens(), 0);
        ctx.add_tokens(100);
        ctx.add_tokens(50);
        assert_eq!(ctx.total_tokens(), 150);
    }

    #[tokio::test]
    async fn messages_push_and_read() {
        let ctx = make_context();
        assert!(ctx.messages().await.is_empty());
        ctx.push_message(orka_llm::client::ChatMessage::user("hi"))
            .await;
        assert_eq!(ctx.messages().await.len(), 1);
    }

    #[test]
    fn slot_key_agent_factory() {
        let agent = AgentId::new("a1");
        let key = SlotKey::agent(&agent, "score");
        assert_eq!(key.namespace, "a1");
        assert_eq!(key.name, "score");
    }

    #[test]
    fn slot_key_shared_factory() {
        let key = SlotKey::shared("counter");
        assert_eq!(key.namespace, "__shared");
        assert_eq!(key.name, "counter");
    }

    #[tokio::test]
    async fn get_typed_deserializes() {
        let ctx = make_context();
        let agent = AgentId::new("a1");
        let key = SlotKey::shared("num");
        ctx.set(&agent, key.clone(), serde_json::json!(42)).await;
        let val: Option<i64> = ctx.get_typed(&key).await;
        assert_eq!(val, Some(42));
    }

    #[tokio::test]
    async fn set_messages_replaces_all() {
        let ctx = make_context();
        ctx.push_message(orka_llm::client::ChatMessage::user("a"))
            .await;
        ctx.push_message(orka_llm::client::ChatMessage::user("b"))
            .await;
        assert_eq!(ctx.messages().await.len(), 2);

        ctx.set_messages(vec![orka_llm::client::ChatMessage::user("only")])
            .await;
        assert_eq!(ctx.messages().await.len(), 1);
    }

    #[test]
    fn elapsed_ms_is_positive() {
        let ctx = make_context();
        // Even immediately, elapsed should be >= 0 (it's u64, so always true)
        // But we really just want to verify it doesn't panic
        let _ms = ctx.elapsed_ms();
    }

    #[tokio::test]
    async fn slot_key_namespacing() {
        let ctx = make_context();
        let a1 = AgentId::new("agent1");
        let a2 = AgentId::new("agent2");
        let k1 = SlotKey::agent(&a1, "x");
        let k2 = SlotKey::agent(&a2, "x");
        ctx.set(&a1, k1.clone(), serde_json::json!(1)).await;
        ctx.set(&a2, k2.clone(), serde_json::json!(2)).await;
        assert_eq!(ctx.get(&k1).await, Some(serde_json::json!(1)));
        assert_eq!(ctx.get(&k2).await, Some(serde_json::json!(2)));
    }

    #[tokio::test]
    async fn reducer_append_collects_values_from_multiple_agents() {
        use std::collections::HashMap;

        use crate::reducer::ReducerStrategy;

        let ctx = make_context();
        let key = SlotKey::shared("results");

        // Register Append reducer for this slot
        let mut reducers = HashMap::new();
        reducers.insert("__shared::results".to_string(), ReducerStrategy::Append);
        ctx.set_reducers(reducers).await;

        let a1 = AgentId::new("worker_a");
        let a2 = AgentId::new("worker_b");
        ctx.set(&a1, key.clone(), serde_json::json!("a_result"))
            .await;
        ctx.set(&a2, key.clone(), serde_json::json!("b_result"))
            .await;

        let value = ctx.get(&key).await.unwrap();
        let arr = value
            .as_array()
            .expect("expected array after Append reducer");
        assert_eq!(arr.len(), 2, "both results should be accumulated");
        assert!(arr.contains(&serde_json::json!("a_result")));
        assert!(arr.contains(&serde_json::json!("b_result")));
    }

    #[tokio::test]
    async fn reducer_sum_accumulates_numeric_values() {
        use std::collections::HashMap;

        use crate::reducer::ReducerStrategy;

        let ctx = make_context();
        let key = SlotKey::shared("score");

        let mut reducers = HashMap::new();
        reducers.insert("__shared::score".to_string(), ReducerStrategy::Sum);
        ctx.set_reducers(reducers).await;

        let a1 = AgentId::new("w1");
        let a2 = AgentId::new("w2");
        ctx.set(&a1, key.clone(), serde_json::json!(10.0)).await;
        ctx.set(&a2, key.clone(), serde_json::json!(5.0)).await;

        let value = ctx.get(&key).await.unwrap();
        assert_eq!(value.as_f64(), Some(15.0));
    }

    #[tokio::test]
    async fn reducer_last_write_wins_without_registration() {
        // Without a reducer registered, default is LastWriteWins
        let ctx = make_context();
        let a1 = AgentId::new("w1");
        let a2 = AgentId::new("w2");
        let key = SlotKey::shared("val");
        ctx.set(&a1, key.clone(), serde_json::json!("first")).await;
        ctx.set(&a2, key.clone(), serde_json::json!("second")).await;
        assert_eq!(ctx.get(&key).await, Some(serde_json::json!("second")));
    }
}
