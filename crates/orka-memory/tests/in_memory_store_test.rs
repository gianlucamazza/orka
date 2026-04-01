#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use orka_core::{MemoryEntry, testing::InMemoryMemoryStore, traits::MemoryStore};
use orka_memory::config::{MemoryBackend, MemoryConfig};

fn entry(key: &str) -> MemoryEntry {
    MemoryEntry::new(key, serde_json::json!({"data": key}))
}

fn tagged_entry(key: &str, tags: &[&str]) -> MemoryEntry {
    MemoryEntry::new(key, serde_json::json!({"data": key}))
        .with_tags(tags.iter().map(|s| (*s).to_string()).collect())
}

// ── store / recall ────────────────────────────────────────────────────────────

#[tokio::test]
async fn store_recall_roundtrip() {
    let store = InMemoryMemoryStore::new();
    let e = entry("greeting");
    store.store("greeting", e.clone(), None).await.unwrap();

    let recalled = store.recall("greeting").await.unwrap().unwrap();
    assert_eq!(recalled.key, "greeting");
    assert_eq!(recalled.value, serde_json::json!({"data": "greeting"}));
}

#[tokio::test]
async fn recall_nonexistent_returns_none() {
    let store = InMemoryMemoryStore::new();
    assert!(store.recall("ghost").await.unwrap().is_none());
}

#[tokio::test]
async fn store_overwrites_existing_key() {
    let store = InMemoryMemoryStore::new();
    store
        .store("k", MemoryEntry::new("k", serde_json::json!(1)), None)
        .await
        .unwrap();
    store
        .store("k", MemoryEntry::new("k", serde_json::json!(2)), None)
        .await
        .unwrap();
    let v = store.recall("k").await.unwrap().unwrap();
    assert_eq!(v.value, serde_json::json!(2));
}

// ── delete ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn delete_existing_returns_true() {
    let store = InMemoryMemoryStore::new();
    store.store("x", entry("x"), None).await.unwrap();
    assert!(store.delete("x").await.unwrap());
    assert!(store.recall("x").await.unwrap().is_none());
}

#[tokio::test]
async fn delete_nonexistent_returns_false() {
    let store = InMemoryMemoryStore::new();
    assert!(!store.delete("phantom").await.unwrap());
}

// ── search ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn search_by_key_substring() {
    let store = InMemoryMemoryStore::new();
    store.store("user-alice", entry("user-alice"), None).await.unwrap();
    store.store("user-bob", entry("user-bob"), None).await.unwrap();
    store.store("system-log", entry("system-log"), None).await.unwrap();

    let results = store.search("user", 10).await.unwrap();
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|e| e.key.contains("user")));
}

#[tokio::test]
async fn search_by_tag() {
    let store = InMemoryMemoryStore::new();
    store
        .store("a", tagged_entry("a", &["important"]), None)
        .await
        .unwrap();
    store
        .store("b", tagged_entry("b", &["trivial"]), None)
        .await
        .unwrap();

    let results = store.search("important", 10).await.unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].key, "a");
}

#[tokio::test]
async fn search_empty_store_returns_empty() {
    let store = InMemoryMemoryStore::new();
    assert!(store.search("anything", 10).await.unwrap().is_empty());
}

#[tokio::test]
async fn search_respects_limit() {
    let store = InMemoryMemoryStore::new();
    for i in 0..10 {
        store
            .store(&format!("item-{i}"), entry(&format!("item-{i}")), None)
            .await
            .unwrap();
    }
    let results = store.search("item", 3).await.unwrap();
    assert_eq!(results.len(), 3);
}

// ── list ──────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn list_no_prefix_returns_all() {
    let store = InMemoryMemoryStore::new();
    store.store("alpha", entry("alpha"), None).await.unwrap();
    store.store("beta", entry("beta"), None).await.unwrap();
    store.store("gamma", entry("gamma"), None).await.unwrap();

    let results = store.list(None, 100).await.unwrap();
    assert_eq!(results.len(), 3);
}

#[tokio::test]
async fn list_with_prefix_filters() {
    let store = InMemoryMemoryStore::new();
    store.store("conv:1", entry("conv:1"), None).await.unwrap();
    store.store("conv:2", entry("conv:2"), None).await.unwrap();
    store.store("meta:info", entry("meta:info"), None).await.unwrap();

    let results = store.list(Some("conv:"), 100).await.unwrap();
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|e| e.key.starts_with("conv:")));
}

#[tokio::test]
async fn list_respects_limit() {
    let store = InMemoryMemoryStore::new();
    for i in 0..5 {
        store
            .store(&format!("k{i}"), entry(&format!("k{i}")), None)
            .await
            .unwrap();
    }
    let results = store.list(None, 2).await.unwrap();
    assert_eq!(results.len(), 2);
}

#[tokio::test]
async fn list_empty_store_returns_empty() {
    let store = InMemoryMemoryStore::new();
    assert!(store.list(None, 100).await.unwrap().is_empty());
}

// ── TTL / compact ─────────────────────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn ttl_entry_not_visible_after_expiry() {
    let store = InMemoryMemoryStore::new();
    store
        .store("ephemeral", entry("ephemeral"), Some(Duration::from_secs(1)))
        .await
        .unwrap();

    // Still visible before expiry
    assert!(store.recall("ephemeral").await.unwrap().is_some());

    tokio::time::advance(Duration::from_secs(2)).await;

    // Gone after TTL
    assert!(store.recall("ephemeral").await.unwrap().is_none());
}

#[tokio::test(start_paused = true)]
async fn compact_removes_expired_entries() {
    let store = InMemoryMemoryStore::new();
    store
        .store("expire-me", entry("expire-me"), Some(Duration::from_secs(1)))
        .await
        .unwrap();
    store.store("keep-me", entry("keep-me"), None).await.unwrap();

    tokio::time::advance(Duration::from_secs(2)).await;

    let removed = store.compact().await.unwrap();
    assert_eq!(removed, 1);

    assert!(store.recall("keep-me").await.unwrap().is_some());
}

#[tokio::test]
async fn compact_on_empty_store_returns_zero() {
    let store = InMemoryMemoryStore::new();
    assert_eq!(store.compact().await.unwrap(), 0);
}

#[tokio::test]
async fn compact_with_no_expired_entries_returns_zero() {
    let store = InMemoryMemoryStore::new();
    store.store("a", entry("a"), None).await.unwrap();
    store.store("b", entry("b"), None).await.unwrap();
    assert_eq!(store.compact().await.unwrap(), 0);
}

// ── MemoryConfig validation ───────────────────────────────────────────────────

#[test]
fn config_validate_rejects_zero_max_entries() {
    let mut config = MemoryConfig::default();
    config.max_entries = 0;
    assert!(config.validate().is_err());
}

#[test]
fn config_validate_accepts_positive_max_entries() {
    let mut config = MemoryConfig::default();
    config.max_entries = 100;
    assert!(config.validate().is_ok());
}

#[test]
fn config_default_max_entries_is_positive() {
    let config = MemoryConfig::default();
    assert!(config.max_entries > 0);
    assert!(config.validate().is_ok());
}

#[test]
fn config_default_backend_is_auto() {
    let config = MemoryConfig::default();
    assert_eq!(config.backend, MemoryBackend::Auto);
}
