#![allow(missing_docs)]

use std::{sync::Arc, time::Duration};

use orka_core::{
    Envelope, Payload, Session,
    testing::{InMemoryBus, InMemoryEventSink, InMemoryQueue, InMemorySessionStore},
    traits::{MessageBus, PriorityQueue, SessionStore},
};
use orka_worker::{EchoHandler, HandlerDispatcher, WorkerPool};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn worker_pool_echo_handler() {
    let queue = Arc::new(InMemoryQueue::new());
    let sessions = Arc::new(InMemorySessionStore::new());
    let bus = Arc::new(InMemoryBus::new());

    // Create a session and a matching envelope
    let session = Session::new("test-channel", "user1");
    let session_id = session.id;
    sessions.put(&session).await.unwrap();

    let envelope = Envelope::text("test-channel", session_id, "hello world");

    // Subscribe to outbound BEFORE starting the worker
    let mut outbound_rx = bus.subscribe("outbound").await.unwrap();

    // Push the envelope to the queue
    queue.push(&envelope).await.unwrap();

    // Start the worker pool
    let handler = Arc::new(HandlerDispatcher::new(Arc::new(EchoHandler)));
    let event_sink = Arc::new(InMemoryEventSink::new());
    let pool = WorkerPool::new(
        queue.clone(),
        sessions.clone(),
        bus.clone(),
        handler,
        event_sink,
        1,
        3,
    );

    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();

    let pool_handle = tokio::spawn(async move {
        pool.run(cancel_clone).await.unwrap();
    });

    // Wait for the outbound message
    let received = tokio::time::timeout(Duration::from_secs(5), outbound_rx.recv())
        .await
        .expect("timed out waiting for outbound message")
        .expect("channel closed");

    // Verify the echo response
    match &received.payload {
        Payload::Text(t) => assert_eq!(t, "echo: hello world"),
        other => panic!("expected Text payload, got {:?}", other),
    }
    assert_eq!(received.channel, "test-channel");

    // Shut down
    cancel.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(2), pool_handle).await;
}
