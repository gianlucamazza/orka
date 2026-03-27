#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

use orka_core::{MemoryEntry, traits::MemoryStore};
use orka_memory::RedisMemoryStore;

async fn setup() -> (
    RedisMemoryStore,
    testcontainers::ContainerAsync<testcontainers_modules::redis::Redis>,
) {
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::redis::Redis;

    let container = Redis::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(6379).await.unwrap();
    let url = format!("redis://127.0.0.1:{port}");
    let store = RedisMemoryStore::new(&url, 10_000).expect("create store");
    (store, container)
}

fn make_entry(key: &str, tags: Vec<&str>) -> MemoryEntry {
    MemoryEntry::new(key, serde_json::json!({"data": key}))
        .with_tags(tags.into_iter().map(String::from).collect())
}

#[tokio::test]
#[ignore] // requires Redis
async fn store_recall_roundtrip() {
    let (store, _container) = setup().await;
    let entry = make_entry("test-key", vec!["tag1"]);

    store.store("test-key", entry, None).await.unwrap();

    let recalled = store.recall("test-key").await.unwrap().unwrap();
    assert_eq!(recalled.key, "test-key");
    assert_eq!(recalled.value, serde_json::json!({"data": "test-key"}));
    assert_eq!(recalled.tags, vec!["tag1"]);
}

#[tokio::test]
#[ignore] // requires Redis
async fn ttl_expiry() {
    use std::time::Duration;

    let (store, _container) = setup().await;
    let entry = make_entry("ephemeral", vec![]);

    store
        .store("ephemeral", entry, Some(Duration::from_secs(1)))
        .await
        .unwrap();

    assert!(store.recall("ephemeral").await.unwrap().is_some());

    tokio::time::sleep(Duration::from_secs(2)).await;

    assert!(store.recall("ephemeral").await.unwrap().is_none());
}

#[tokio::test]
#[ignore] // requires Redis
async fn search_by_key_pattern() {
    let (store, _container) = setup().await;

    store
        .store(
            "user-alice",
            make_entry("user-alice", vec!["profile"]),
            None,
        )
        .await
        .unwrap();
    store
        .store("user-bob", make_entry("user-bob", vec!["profile"]), None)
        .await
        .unwrap();
    store
        .store(
            "system-config",
            make_entry("system-config", vec!["system"]),
            None,
        )
        .await
        .unwrap();

    let results = store.search("user", 10).await.unwrap();
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|e| e.key.contains("user")));
}

#[tokio::test]
#[ignore] // requires Redis
async fn search_by_tag() {
    let (store, _container) = setup().await;

    store
        .store("item-a", make_entry("item-a", vec!["important"]), None)
        .await
        .unwrap();
    store
        .store("item-b", make_entry("item-b", vec!["trivial"]), None)
        .await
        .unwrap();

    let results = store.search("important", 10).await.unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].key, "item-a");
}

#[tokio::test]
#[ignore] // requires Redis
async fn compact_returns_zero() {
    let (store, _container) = setup().await;
    let count = store.compact().await.unwrap();
    assert_eq!(count, 0);
}
