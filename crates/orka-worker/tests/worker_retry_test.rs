#![allow(missing_docs)]

use std::{
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use orka_core::{
    Envelope, Error, OutboundMessage, Session,
    testing::{InMemoryBus, InMemoryEventSink, InMemoryQueue, InMemorySessionStore},
    traits::{MessageBus, PriorityQueue, SessionStore},
};
use orka_worker::{AgentHandler, HandlerDispatcher, WorkerPool};
use tokio_util::sync::CancellationToken;

/// Handler that always fails.
struct AlwaysFailHandler;

#[async_trait]
impl AgentHandler for AlwaysFailHandler {
    async fn handle(&self, _: &Envelope, _: &Session) -> orka_core::Result<Vec<OutboundMessage>> {
        Err(Error::worker_msg("intentional failure"))
    }
}

/// Handler that fails for the first N calls, then succeeds.
struct FailNTimesHandler {
    fail_count: u32,
    calls: AtomicU32,
}

impl FailNTimesHandler {
    fn new(fail_count: u32) -> Self {
        Self {
            fail_count,
            calls: AtomicU32::new(0),
        }
    }
}

#[async_trait]
impl AgentHandler for FailNTimesHandler {
    async fn handle(
        &self,
        envelope: &Envelope,
        _: &Session,
    ) -> orka_core::Result<Vec<OutboundMessage>> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        if n < self.fail_count {
            Err(Error::worker_msg(format!("failure #{}", n + 1)))
        } else {
            Ok(vec![OutboundMessage::text(
                envelope.channel.clone(),
                envelope.session_id,
                "ok",
                Some(envelope.id),
            )])
        }
    }
}

#[tokio::test]
async fn handler_ok_no_retry() {
    let queue = Arc::new(InMemoryQueue::new());
    let sessions = Arc::new(InMemorySessionStore::new());
    let bus = Arc::new(InMemoryBus::new());
    let event_sink = Arc::new(InMemoryEventSink::new());

    let session = Session::new("ch", "u1");
    sessions.put(&session).await.unwrap();
    let envelope = Envelope::text("ch", session.id, "hello");
    PriorityQueue::push(&*queue, &envelope).await.unwrap();

    let mut outbound_rx = bus.subscribe("outbound").await.unwrap();

    let handler = Arc::new(HandlerDispatcher::new(Arc::new(FailNTimesHandler::new(0)))); // never fails
    let pool = WorkerPool::new(queue.clone(), sessions, bus, handler, event_sink, 1, 3);
    let cancel = CancellationToken::new();
    let c2 = cancel.clone();
    let h = tokio::spawn(async move { pool.run(c2).await });

    let msg = tokio::time::timeout(Duration::from_secs(5), outbound_rx.recv())
        .await
        .expect("timeout")
        .expect("closed");
    assert!(matches!(&msg.payload, orka_core::Payload::Text(t) if t == "ok"));

    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(queue.len().await.unwrap(), 0);
    assert!(queue.dlq_items().await.is_empty());

    cancel.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(2), h).await;
}

#[tokio::test]
async fn handler_failure_retries_then_dlq() {
    let queue = Arc::new(InMemoryQueue::new());
    let sessions = Arc::new(InMemorySessionStore::new());
    let bus = Arc::new(InMemoryBus::new());
    let event_sink = Arc::new(InMemoryEventSink::new());

    let session = Session::new("ch", "u1");
    sessions.put(&session).await.unwrap();
    let envelope = Envelope::text("ch", session.id, "hello");
    PriorityQueue::push(&*queue, &envelope).await.unwrap();

    let max_retries = 3;
    let handler = Arc::new(HandlerDispatcher::new(Arc::new(AlwaysFailHandler)));
    // Use 10ms base delay for fast tests (10ms, 30ms, 90ms)
    let pool = WorkerPool::new(
        queue.clone(),
        sessions,
        bus,
        handler,
        event_sink,
        1,
        max_retries,
    )
    .with_retry_delay(10)
    .with_dlq(queue.clone());
    let cancel = CancellationToken::new();
    let c2 = cancel.clone();
    let h = tokio::spawn(async move { pool.run(c2).await });

    // Wait for all retries + DLQ push (total delay: ~130ms + processing)
    tokio::time::sleep(Duration::from_secs(2)).await;

    assert_eq!(queue.len().await.unwrap(), 0);
    let dlq = queue.dlq_items().await;
    assert_eq!(dlq.len(), 1);
    assert_eq!(dlq[0].id, envelope.id);

    cancel.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(2), h).await;
}

#[tokio::test]
async fn handler_fails_then_succeeds_with_retry() {
    let queue = Arc::new(InMemoryQueue::new());
    let sessions = Arc::new(InMemorySessionStore::new());
    let bus = Arc::new(InMemoryBus::new());
    let event_sink = Arc::new(InMemoryEventSink::new());

    let session = Session::new("ch", "u1");
    sessions.put(&session).await.unwrap();
    let envelope = Envelope::text("ch", session.id, "hello");
    PriorityQueue::push(&*queue, &envelope).await.unwrap();

    let mut outbound_rx = bus.subscribe("outbound").await.unwrap();

    // Fails twice, then succeeds on 3rd attempt
    let handler = Arc::new(HandlerDispatcher::new(Arc::new(FailNTimesHandler::new(2))));
    // Use 10ms base delay for fast tests
    let pool = WorkerPool::new(queue.clone(), sessions, bus, handler, event_sink, 1, 3)
        .with_retry_delay(10);
    let cancel = CancellationToken::new();
    let c2 = cancel.clone();
    let h = tokio::spawn(async move { pool.run(c2).await });

    let msg = tokio::time::timeout(Duration::from_secs(5), outbound_rx.recv())
        .await
        .expect("timeout")
        .expect("closed");
    assert!(matches!(&msg.payload, orka_core::Payload::Text(t) if t == "ok"));

    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(queue.dlq_items().await.is_empty());

    cancel.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(2), h).await;
}
