#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

use std::{sync::Arc, time::Duration};

use orka_core::{
    Envelope, SessionId,
    testing::{InMemoryBus, InMemoryEventSink, InMemoryQueue, InMemorySessionStore},
    traits::{MessageBus, PriorityQueue},
};
use orka_gateway::{Gateway, GatewayConfig, GatewayDeps};
use orka_workspace::WorkspaceLoader;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn single_message_enqueued_once() {
    let bus = Arc::new(InMemoryBus::new());
    let sessions = Arc::new(InMemorySessionStore::new());
    let queue = Arc::new(InMemoryQueue::new());
    let workspace = Arc::new(WorkspaceLoader::new("/tmp/orka-test-dedup-1"));
    let event_sink = Arc::new(InMemoryEventSink::new());

    let gateway = Gateway::new(
        GatewayDeps {
            bus: bus.clone(),
            sessions: sessions.clone(),
            queue: queue.clone(),
            workspace,
            event_sink,
        },
        GatewayConfig {
            rate_limit: 60,
            dedup_ttl_secs: 3600, // no Redis → no dedup
            ..Default::default()
        },
    );

    let cancel = CancellationToken::new();
    let c2 = cancel.clone();
    let handle = tokio::spawn(async move { gateway.run(c2).await });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let env = Envelope::text("custom", SessionId::new(), "hello");
    bus.publish("inbound", &env).await.unwrap();

    tokio::time::sleep(Duration::from_millis(100)).await;
    cancel.cancel();
    handle.await.unwrap().unwrap();

    assert_eq!(queue.len().await.unwrap(), 1);
}

#[tokio::test]
async fn duplicate_without_redis_both_enqueued() {
    // Without Redis pool, dedup is disabled — both messages go through
    let bus = Arc::new(InMemoryBus::new());
    let sessions = Arc::new(InMemorySessionStore::new());
    let queue = Arc::new(InMemoryQueue::new());
    let workspace = Arc::new(WorkspaceLoader::new("/tmp/orka-test-dedup-2"));
    let event_sink = Arc::new(InMemoryEventSink::new());

    let gateway = Gateway::new(
        GatewayDeps {
            bus: bus.clone(),
            sessions: sessions.clone(),
            queue: queue.clone(),
            workspace,
            event_sink,
        },
        GatewayConfig {
            rate_limit: 60,
            dedup_ttl_secs: 3600,
            ..Default::default()
        },
    );

    let cancel = CancellationToken::new();
    let c2 = cancel.clone();
    let handle = tokio::spawn(async move { gateway.run(c2).await });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let env = Envelope::text("custom", SessionId::new(), "hello");
    // Send the same envelope twice
    bus.publish("inbound", &env).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    bus.publish("inbound", &env).await.unwrap();

    tokio::time::sleep(Duration::from_millis(100)).await;
    cancel.cancel();
    handle.await.unwrap().unwrap();

    // Without Redis dedup, both should be enqueued
    assert_eq!(queue.len().await.unwrap(), 2);
}

#[tokio::test]
async fn different_messages_both_enqueued() {
    let bus = Arc::new(InMemoryBus::new());
    let sessions = Arc::new(InMemorySessionStore::new());
    let queue = Arc::new(InMemoryQueue::new());
    let workspace = Arc::new(WorkspaceLoader::new("/tmp/orka-test-dedup-3"));
    let event_sink = Arc::new(InMemoryEventSink::new());

    let gateway = Gateway::new(
        GatewayDeps {
            bus: bus.clone(),
            sessions: sessions.clone(),
            queue: queue.clone(),
            workspace,
            event_sink,
        },
        GatewayConfig {
            rate_limit: 60,
            dedup_ttl_secs: 3600,
            ..Default::default()
        },
    );

    let cancel = CancellationToken::new();
    let c2 = cancel.clone();
    let handle = tokio::spawn(async move { gateway.run(c2).await });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let env1 = Envelope::text("custom", SessionId::new(), "msg1");
    let env2 = Envelope::text("custom", SessionId::new(), "msg2");
    bus.publish("inbound", &env1).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    bus.publish("inbound", &env2).await.unwrap();

    tokio::time::sleep(Duration::from_millis(100)).await;
    cancel.cancel();
    handle.await.unwrap().unwrap();

    assert_eq!(queue.len().await.unwrap(), 2);
}
