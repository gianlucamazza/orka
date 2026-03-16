use orka_core::{
    traits::PriorityQueue,
    types::{Envelope, Priority, SessionId},
};
use orka_queue::RedisPriorityQueue;
use std::time::Duration;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::redis::Redis;

#[tokio::test]
#[ignore] // requires Redis
async fn push_pop_priority_order() {
    let container = Redis::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(6379).await.unwrap();
    let url = format!("redis://127.0.0.1:{port}");

    let queue = RedisPriorityQueue::new(&url).unwrap();

    let session = SessionId::new();

    // Create envelopes with different priorities.
    let mut bg = Envelope::text("test", session.clone(), "background");
    bg.priority = Priority::Background;

    let mut normal = Envelope::text("test", session.clone(), "normal");
    normal.priority = Priority::Normal;

    let mut urgent = Envelope::text("test", session.clone(), "urgent");
    urgent.priority = Priority::Urgent;

    // Push in non-priority order: background, normal, urgent.
    queue.push(&bg).await.unwrap();
    queue.push(&normal).await.unwrap();
    queue.push(&urgent).await.unwrap();

    assert_eq!(queue.len().await.unwrap(), 3);

    // Pop should return urgent first, then normal, then background.
    let e1 = queue.pop(Duration::from_secs(1)).await.unwrap().unwrap();
    assert_eq!(e1.priority, Priority::Urgent);

    let e2 = queue.pop(Duration::from_secs(1)).await.unwrap().unwrap();
    assert_eq!(e2.priority, Priority::Normal);

    let e3 = queue.pop(Duration::from_secs(1)).await.unwrap().unwrap();
    assert_eq!(e3.priority, Priority::Background);

    assert_eq!(queue.len().await.unwrap(), 0);
}

#[tokio::test]
#[ignore] // requires Redis
async fn pop_empty_returns_none() {
    let container = Redis::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(6379).await.unwrap();
    let url = format!("redis://127.0.0.1:{port}");

    let queue = RedisPriorityQueue::new(&url).unwrap();

    let result = queue.pop(Duration::from_millis(500)).await.unwrap();
    assert!(result.is_none());
}
