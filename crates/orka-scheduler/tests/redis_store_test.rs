#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

use orka_scheduler::{RedisScheduleStore, store::ScheduleStore};
use orka_test_support::RedisService;

fn make_schedule(id: &str, name: &str, next_run: i64) -> orka_scheduler::types::Schedule {
    orka_scheduler::types::Schedule {
        id: id.to_string(),
        name: name.to_string(),
        cron: None,
        run_at: None,
        timezone: None,
        skill: None,
        args: None,
        message: Some("test".to_string()),
        next_run,
        created_at: "2025-01-01T00:00:00Z".to_string(),
        completed: false,
    }
}

#[serial_test::serial]
#[tokio::test]
#[ignore = "requires Redis"]
async fn add_and_list_schedules() {
    let redis = RedisService::discover().await.unwrap();
    let store = RedisScheduleStore::new(redis.url()).unwrap();

    let id = format!("test-add-{}", uuid::Uuid::new_v4().simple());
    let s = make_schedule(&id, "test-add", 9_999_999_999);
    store.add(&s).await.unwrap();

    let all = store.list(false).await.unwrap();
    assert!(all.iter().any(|x| x.id == id));

    // cleanup
    store.remove(&id).await.unwrap();
}

#[serial_test::serial]
#[tokio::test]
#[ignore = "requires Redis"]
async fn remove_schedule_returns_true_if_existed() {
    let redis = RedisService::discover().await.unwrap();
    let store = RedisScheduleStore::new(redis.url()).unwrap();

    let id = format!("test-remove-{}", uuid::Uuid::new_v4().simple());
    let s = make_schedule(&id, "test-remove", 9_999_999_999);
    store.add(&s).await.unwrap();

    let removed = store.remove(&id).await.unwrap();
    assert!(removed);

    // removing again returns false
    let removed_again = store.remove(&id).await.unwrap();
    assert!(!removed_again);
}

#[serial_test::serial]
#[tokio::test]
#[ignore = "requires Redis"]
async fn get_due_returns_past_schedules_only() {
    let redis = RedisService::discover().await.unwrap();
    let store = RedisScheduleStore::new(redis.url()).unwrap();

    let past_id = format!("test-due-past-{}", uuid::Uuid::new_v4().simple());
    let future_id = format!("test-due-future-{}", uuid::Uuid::new_v4().simple());

    let past = make_schedule(&past_id, "past", 1_000_000); // far in the past
    let future = make_schedule(&future_id, "future", 9_999_999_999); // far in the future

    store.add(&past).await.unwrap();
    store.add(&future).await.unwrap();

    let now = chrono::Utc::now().timestamp();
    let due = store.get_due(now).await.unwrap();

    assert!(due.iter().any(|s| s.id == past_id));
    assert!(!due.iter().any(|s| s.id == future_id));

    // cleanup
    store.remove(&past_id).await.unwrap();
    store.remove(&future_id).await.unwrap();
}

#[serial_test::serial]
#[tokio::test]
#[ignore = "requires Redis"]
async fn find_by_name_returns_schedule() {
    let redis = RedisService::discover().await.unwrap();
    let store = RedisScheduleStore::new(redis.url()).unwrap();

    let id = format!("test-find-{}", uuid::Uuid::new_v4().simple());
    let name = format!("find-by-name-{}", uuid::Uuid::new_v4().simple());
    let s = make_schedule(&id, &name, 9_999_999_999);
    store.add(&s).await.unwrap();

    let found = store.find_by_name(&name).await.unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().id, id);

    store.remove(&id).await.unwrap();
}

#[serial_test::serial]
#[tokio::test]
#[ignore = "requires Redis"]
async fn try_lock_exclusive_only_first_acquires() {
    let redis = RedisService::discover().await.unwrap();
    let store = RedisScheduleStore::new(redis.url()).unwrap();

    let id = format!("lock-{}", uuid::Uuid::new_v4().simple());
    let run_at = 1_700_000_000_i64;

    let first = store.try_lock_execution(&id, run_at, 30).await.unwrap();
    let second = store.try_lock_execution(&id, run_at, 30).await.unwrap();

    assert!(first, "first caller should acquire the lock");
    assert!(!second, "second caller should not acquire the lock");

    store.release_execution_lock(&id, run_at).await.unwrap();
}

#[serial_test::serial]
#[tokio::test]
#[ignore = "requires Redis"]
async fn release_lock_allows_reacquisition() {
    let redis = RedisService::discover().await.unwrap();
    let store = RedisScheduleStore::new(redis.url()).unwrap();

    let id = format!("relock-{}", uuid::Uuid::new_v4().simple());
    let run_at = 1_700_000_001_i64;

    assert!(store.try_lock_execution(&id, run_at, 30).await.unwrap());
    store.release_execution_lock(&id, run_at).await.unwrap();
    // After release, another instance can acquire
    assert!(store.try_lock_execution(&id, run_at, 30).await.unwrap());

    store.release_execution_lock(&id, run_at).await.unwrap();
}
