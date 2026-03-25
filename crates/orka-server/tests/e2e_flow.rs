/// End-to-end test: adapter → bus(inbound) → gateway → queue → worker →
/// bus(outbound)
///
/// Uses in-memory test doubles from orka-core::testing to verify the full
/// message flow without Redis or external dependencies.
use std::sync::Arc;
use std::time::Duration;

use orka_core::{
    DomainEventKind, Envelope, SessionId,
    testing::{InMemoryBus, InMemoryEventSink, InMemoryQueue, InMemorySessionStore},
    traits::{MessageBus, PriorityQueue},
};
use orka_gateway::Gateway;
use orka_worker::{EchoHandler, HandlerDispatcher, WorkerPool};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn full_message_flow_echo() {
    // 1. Create in-memory infrastructure
    let bus = Arc::new(InMemoryBus::new());
    let sessions = Arc::new(InMemorySessionStore::new());
    let queue = Arc::new(InMemoryQueue::new());
    let event_sink = Arc::new(InMemoryEventSink::new());

    let shutdown = CancellationToken::new();

    // 2. Subscribe to outbound topic before starting components
    let mut outbound_rx = bus.subscribe("outbound").await.unwrap();

    // 3. Create workspace loader (empty workspace is fine for echo)
    let workspace = Arc::new(orka_workspace::WorkspaceLoader::new("."));

    // 4. Start gateway (subscribes to "inbound" topic, processes → queue)
    let gateway = Gateway::new(
        bus.clone(),
        sessions.clone(),
        queue.clone(),
        workspace,
        event_sink.clone(),
        None, // no Redis for dedup in tests
        0,    // no rate limit
        3600, // dedup TTL
    );
    let gateway_cancel = shutdown.clone();
    let gateway_handle = tokio::spawn(async move {
        gateway.run(gateway_cancel).await.ok();
    });

    // 5. Start worker pool with EchoHandler (1 worker, 0 retries)
    let handler = Arc::new(HandlerDispatcher::new(Arc::new(EchoHandler)));
    let worker_pool = WorkerPool::new(
        queue.clone(),
        sessions.clone(),
        bus.clone(),
        handler,
        event_sink.clone(),
        1, // 1 worker
        0, // no retries
    );
    let worker_cancel = shutdown.clone();
    let worker_handle = tokio::spawn(async move {
        worker_pool.run(worker_cancel).await.ok();
    });

    // Give workers time to start
    tokio::time::sleep(Duration::from_millis(50)).await;

    // 6. Simulate adapter: publish inbound message
    let session_id = SessionId::new();
    let inbound = Envelope::text("custom", session_id, "hello orka");
    bus.publish("inbound", &inbound).await.unwrap();

    // 7. Wait for outbound message (the echo reply)
    let outbound = tokio::time::timeout(Duration::from_secs(5), outbound_rx.recv())
        .await
        .expect("timed out waiting for outbound message")
        .expect("outbound channel closed");

    // Verify the echo reply
    match &outbound.payload {
        orka_core::Payload::Text(t) => {
            assert!(
                t.contains("hello orka"),
                "expected echo reply to contain original text, got: {t}"
            );
        }
        other => panic!("expected text payload, got: {other:?}"),
    }

    // 8. Verify events were emitted
    let events = event_sink.events().await;
    let event_kinds: Vec<&str> = events
        .iter()
        .map(|e| match &e.kind {
            DomainEventKind::MessageReceived { .. } => "MessageReceived",
            DomainEventKind::SessionCreated { .. } => "SessionCreated",
            DomainEventKind::HandlerInvoked { .. } => "HandlerInvoked",
            DomainEventKind::HandlerCompleted { .. } => "HandlerCompleted",
            _ => "other",
        })
        .collect();

    assert!(
        event_kinds.contains(&"MessageReceived"),
        "missing MessageReceived event"
    );
    assert!(
        event_kinds.contains(&"SessionCreated"),
        "missing SessionCreated event"
    );
    assert!(
        event_kinds.contains(&"HandlerInvoked"),
        "missing HandlerInvoked event"
    );
    assert!(
        event_kinds.contains(&"HandlerCompleted"),
        "missing HandlerCompleted event"
    );

    // 9. Verify queue is drained
    assert_eq!(queue.len().await.unwrap(), 0, "queue should be empty");

    // 10. Clean shutdown
    shutdown.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(2), gateway_handle).await;
    let _ = tokio::time::timeout(Duration::from_secs(2), worker_handle).await;
}

#[tokio::test]
async fn rate_limited_messages_are_dropped() {
    let bus = Arc::new(InMemoryBus::new());
    let sessions = Arc::new(InMemorySessionStore::new());
    let queue = Arc::new(InMemoryQueue::new());
    let event_sink = Arc::new(InMemoryEventSink::new());
    let shutdown = CancellationToken::new();

    let workspace = Arc::new(orka_workspace::WorkspaceLoader::new("."));

    // Gateway with rate_limit=2 (2 messages per 60s window per session)
    let gateway = Gateway::new(
        bus.clone(),
        sessions.clone(),
        queue.clone(),
        workspace,
        event_sink.clone(),
        None,
        2, // rate limit: 2 per session
        3600,
    );
    let gateway_cancel = shutdown.clone();
    let gateway_handle = tokio::spawn(async move {
        gateway.run(gateway_cancel).await.ok();
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let session_id = SessionId::new();

    // Send 4 messages — only 2 should be enqueued
    for i in 0..4 {
        let msg = Envelope::text("custom", session_id, format!("msg {i}"));
        bus.publish("inbound", &msg).await.unwrap();
    }

    // Give gateway time to process
    tokio::time::sleep(Duration::from_millis(200)).await;

    let queue_len = queue.len().await.unwrap();
    assert_eq!(queue_len, 2, "only 2 messages should pass rate limit");

    shutdown.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(2), gateway_handle).await;
}
