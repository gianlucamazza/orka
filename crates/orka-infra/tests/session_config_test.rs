#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

use orka_core::{Session, SessionId, testing::InMemorySessionStore, traits::SessionStore};
use orka_infra::SessionConfig;

// ── SessionConfig validation
// ──────────────────────────────────────────────────

#[test]
fn config_default_ttl_is_positive() {
    let config = SessionConfig::default();
    assert!(config.ttl_secs > 0);
    assert!(config.validate().is_ok());
}

#[test]
fn config_validate_rejects_zero_ttl() {
    let mut config = SessionConfig::default();
    config.ttl_secs = 0;
    assert!(config.validate().is_err());
}

#[test]
fn config_validate_accepts_nonzero_ttl() {
    let mut config = SessionConfig::default();
    config.ttl_secs = 3600;
    assert!(config.validate().is_ok());
}

// ── InMemorySessionStore behaviour (no Redis)
// ─────────────────────────────────

#[tokio::test]
async fn in_memory_put_get_roundtrip() {
    let store = InMemorySessionStore::new();
    let session = Session::new("telegram", "user-1");
    let id = session.id;
    store.put(&session).await.unwrap();
    let got = store.get(&id).await.unwrap().unwrap();
    assert_eq!(got.channel, "telegram");
    assert_eq!(got.user_id, "user-1");
}

#[tokio::test]
async fn in_memory_get_missing_returns_none() {
    let store = InMemorySessionStore::new();
    let result = store.get(&SessionId::new()).await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn in_memory_delete_removes_session() {
    let store = InMemorySessionStore::new();
    let session = Session::new("discord", "user-2");
    let id = session.id;
    store.put(&session).await.unwrap();
    store.delete(&id).await.unwrap();
    assert!(store.get(&id).await.unwrap().is_none());
}

#[tokio::test]
async fn in_memory_put_updates_existing_session() {
    let store = InMemorySessionStore::new();
    let mut session = Session::new("slack", "user-3");
    let id = session.id;
    store.put(&session).await.unwrap();

    session
        .state
        .insert("key".to_string(), serde_json::json!("value"));
    store.put(&session).await.unwrap();

    let got = store.get(&id).await.unwrap().unwrap();
    assert_eq!(got.state.get("key").unwrap(), &serde_json::json!("value"));
}

#[tokio::test]
async fn in_memory_list_returns_stored_sessions() {
    let store = InMemorySessionStore::new();
    store.put(&Session::new("ch", "u1")).await.unwrap();
    store.put(&Session::new("ch", "u2")).await.unwrap();
    let sessions = store.list(10).await.unwrap();
    assert_eq!(sessions.len(), 2);
}

#[tokio::test]
async fn in_memory_list_respects_limit() {
    let store = InMemorySessionStore::new();
    for i in 0..5 {
        store
            .put(&Session::new("ch", format!("u{i}").as_str()))
            .await
            .unwrap();
    }
    let sessions = store.list(3).await.unwrap();
    assert_eq!(sessions.len(), 3);
}

#[tokio::test]
async fn in_memory_multiple_sessions_independent() {
    let store = InMemorySessionStore::new();
    let s1 = Session::new("ch", "alice");
    let s2 = Session::new("ch", "bob");
    let id1 = s1.id;
    let id2 = s2.id;
    store.put(&s1).await.unwrap();
    store.put(&s2).await.unwrap();

    store.delete(&id1).await.unwrap();
    assert!(store.get(&id1).await.unwrap().is_none());
    assert!(store.get(&id2).await.unwrap().is_some());
}
