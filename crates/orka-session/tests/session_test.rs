#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

use orka_core::{Session, traits::SessionStore};
use orka_session::RedisSessionStore;
use orka_test_support::RedisService;

#[tokio::test]
#[ignore = "requires Redis"]
async fn put_get_roundtrip() {
    let redis = RedisService::discover().await.unwrap();
    let store = RedisSessionStore::new(redis.url(), 86400).expect("create store");

    let session = Session::new("telegram", "user-42");
    let id = session.id;

    store.put(&session).await.expect("put session");

    let retrieved = store.get(&id).await.expect("get session");
    let retrieved = retrieved.expect("session should exist");
    assert_eq!(retrieved.id, id);
    assert_eq!(retrieved.channel, "telegram");
    assert_eq!(retrieved.user_id, "user-42");
}

#[tokio::test]
#[ignore = "requires Redis"]
async fn delete_then_get_returns_none() {
    let redis = RedisService::discover().await.unwrap();
    let store = RedisSessionStore::new(redis.url(), 86400).expect("create store");

    let session = Session::new("discord", "user-99");
    let id = session.id;

    store.put(&session).await.expect("put session");
    store.delete(&id).await.expect("delete session");

    let result = store.get(&id).await.expect("get session");
    assert!(result.is_none(), "session should be gone after delete");
}

#[tokio::test]
#[ignore = "requires Redis"]
async fn full_crud_cycle() {
    let redis = RedisService::discover().await.unwrap();
    let store = RedisSessionStore::new(redis.url(), 86400).expect("create store");

    // Create
    let mut session = Session::new("slack", "user-7");
    let id = session.id;

    // Read before create returns None
    let result = store.get(&id).await.expect("get before put");
    assert!(result.is_none());

    // Put
    store.put(&session).await.expect("put session");

    // Read
    let retrieved = store
        .get(&id)
        .await
        .expect("get session")
        .expect("should exist");
    assert_eq!(retrieved.channel, "slack");

    // Update state and put again
    session
        .state
        .insert("mood".to_string(), serde_json::json!("happy"));
    store.put(&session).await.expect("update session");

    let updated = store
        .get(&id)
        .await
        .expect("get updated")
        .expect("should exist");
    assert_eq!(
        updated.state.get("mood").unwrap(),
        &serde_json::json!("happy")
    );
    assert!(updated.updated_at >= retrieved.updated_at);

    // Delete
    store.delete(&id).await.expect("delete session");
    let gone = store.get(&id).await.expect("get after delete");
    assert!(gone.is_none());
}
