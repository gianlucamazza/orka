#![allow(
    missing_docs,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::too_many_lines
)]

use std::collections::HashMap;

use orka_checkpoint::{
    Checkpoint, CheckpointId, CheckpointStore, RunStatus, SerializableSlotKey,
    store::in_memory::InMemoryCheckpointStore,
};
use orka_core::{Envelope, SessionId};
use orka_llm::client::ChatMessage;

fn make_checkpoint(run_id: &str, node: &str) -> Checkpoint {
    let session_id = SessionId::new();
    Checkpoint {
        id: CheckpointId::new(),
        run_id: run_id.to_string(),
        session_id,
        graph_id: "g1".to_string(),
        trigger: Envelope::text("ch", session_id, "hi"),
        completed_node: node.to_string(),
        resume_node: Some("next-node".to_string()),
        state: HashMap::new(),
        messages: vec![ChatMessage::user("hello")],
        total_tokens: 10,
        total_iterations: 1,
        agents_executed: vec!["agent-a".to_string()],
        changelog: Vec::new(),
        status: RunStatus::Running,
        created_at: chrono::Utc::now(),
    }
}

#[tokio::test]
async fn save_then_load_latest_returns_most_recent() {
    let store = InMemoryCheckpointStore::new();
    let c1 = make_checkpoint("run-1", "node-a");
    let c2 = make_checkpoint("run-1", "node-b");
    store.save(&c1).await.unwrap();
    store.save(&c2).await.unwrap();

    let latest = store.load_latest("run-1").await.unwrap().unwrap();
    assert_eq!(latest.id, c2.id);
    assert_eq!(latest.completed_node, "node-b");
}

#[tokio::test]
async fn load_latest_empty_run_returns_none() {
    let store = InMemoryCheckpointStore::new();
    assert!(store.load_latest("no-such-run").await.unwrap().is_none());
}

#[tokio::test]
async fn load_by_id_returns_correct_checkpoint() {
    let store = InMemoryCheckpointStore::new();
    let c1 = make_checkpoint("run-2", "node-a");
    let c2 = make_checkpoint("run-2", "node-b");
    store.save(&c1).await.unwrap();
    store.save(&c2).await.unwrap();

    let loaded = store.load("run-2", &c1.id).await.unwrap().unwrap();
    assert_eq!(loaded.id, c1.id);
    assert_eq!(loaded.completed_node, "node-a");
}

#[tokio::test]
async fn load_nonexistent_id_returns_none() {
    let store = InMemoryCheckpointStore::new();
    let c = make_checkpoint("run-3", "node-a");
    store.save(&c).await.unwrap();

    let missing_id = CheckpointId::new();
    assert!(store.load("run-3", &missing_id).await.unwrap().is_none());
}

#[tokio::test]
async fn list_returns_oldest_first() {
    let store = InMemoryCheckpointStore::new();
    let c1 = make_checkpoint("run-4", "node-a");
    let c2 = make_checkpoint("run-4", "node-b");
    let c3 = make_checkpoint("run-4", "node-c");
    store.save(&c1).await.unwrap();
    store.save(&c2).await.unwrap();
    store.save(&c3).await.unwrap();

    let ids = store.list("run-4").await.unwrap();
    assert_eq!(ids.len(), 3);
    assert_eq!(ids[0], c1.id);
    assert_eq!(ids[1], c2.id);
    assert_eq!(ids[2], c3.id);
}

#[tokio::test]
async fn list_empty_run_returns_empty_vec() {
    let store = InMemoryCheckpointStore::new();
    let ids = store.list("phantom-run").await.unwrap();
    assert!(ids.is_empty());
}

#[tokio::test]
async fn delete_run_removes_all_checkpoints() {
    let store = InMemoryCheckpointStore::new();
    store.save(&make_checkpoint("run-5", "a")).await.unwrap();
    store.save(&make_checkpoint("run-5", "b")).await.unwrap();
    store.delete_run("run-5").await.unwrap();

    assert!(store.load_latest("run-5").await.unwrap().is_none());
    assert!(store.list("run-5").await.unwrap().is_empty());
}

#[tokio::test]
async fn delete_run_does_not_affect_other_runs() {
    let store = InMemoryCheckpointStore::new();
    let c_a = make_checkpoint("run-6a", "node-a");
    let c_b = make_checkpoint("run-6b", "node-b");
    store.save(&c_a).await.unwrap();
    store.save(&c_b).await.unwrap();

    store.delete_run("run-6a").await.unwrap();

    assert!(store.load_latest("run-6a").await.unwrap().is_none());
    let loaded = store.load_latest("run-6b").await.unwrap().unwrap();
    assert_eq!(loaded.id, c_b.id);
}

#[tokio::test]
async fn save_same_id_updates_in_place() {
    let store = InMemoryCheckpointStore::new();
    let mut c = make_checkpoint("run-7", "node-a");
    store.save(&c).await.unwrap();

    c.completed_node = "node-a-updated".to_string();
    store.save(&c).await.unwrap();

    // Only one entry in the list after update
    let ids = store.list("run-7").await.unwrap();
    assert_eq!(ids.len(), 1);

    let loaded = store.load_latest("run-7").await.unwrap().unwrap();
    assert_eq!(loaded.completed_node, "node-a-updated");
}

#[tokio::test]
async fn checkpoint_state_round_trips_through_json() {
    let store = InMemoryCheckpointStore::new();
    let mut c = make_checkpoint("run-8", "node-a");
    c.state.insert("ns::key".to_string(), serde_json::json!({"val": 42}));
    c.total_tokens = 999;
    store.save(&c).await.unwrap();

    let loaded = store.load_latest("run-8").await.unwrap().unwrap();
    assert_eq!(loaded.total_tokens, 999);
    assert_eq!(loaded.state["ns::key"], serde_json::json!({"val": 42}));
}

// ── SerializableSlotKey unit tests ────────────────────────────────────────────

#[test]
fn slot_key_to_map_key_format() {
    let key = SerializableSlotKey {
        namespace: "agent1".to_string(),
        name: "result".to_string(),
    };
    assert_eq!(key.to_map_key(), "agent1::result");
}

#[test]
fn slot_key_from_map_key_round_trips() {
    let original = SerializableSlotKey {
        namespace: "shared".to_string(),
        name: "context".to_string(),
    };
    let serialized = original.to_map_key();
    let recovered = SerializableSlotKey::from_map_key(&serialized).unwrap();
    assert_eq!(recovered.namespace, "shared");
    assert_eq!(recovered.name, "context");
}

#[test]
fn slot_key_from_map_key_rejects_malformed() {
    assert!(SerializableSlotKey::from_map_key("no-separator").is_none());
    assert!(SerializableSlotKey::from_map_key("").is_none());
}

#[test]
fn slot_key_from_map_key_handles_double_colon_in_name() {
    // split_once stops at the first "::", so "a::b::c" parses as namespace="a", name="b::c"
    let key = SerializableSlotKey::from_map_key("a::b::c").unwrap();
    assert_eq!(key.namespace, "a");
    assert_eq!(key.name, "b::c");
}
