use std::sync::Arc;
use std::time::Duration;

use orka_core::testing::{InMemoryBus, InMemoryEventSink, InMemoryQueue, InMemorySessionStore};
use orka_core::traits::{MessageBus, PriorityQueue, SessionStore};
use orka_core::{Envelope, SessionId};
use orka_gateway::Gateway;
use orka_workspace::WorkspaceLoader;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn gateway_creates_session_and_enqueues() {
    let bus = Arc::new(InMemoryBus::new());
    let sessions = Arc::new(InMemorySessionStore::new());
    let queue = Arc::new(InMemoryQueue::new());
    let workspace = Arc::new(WorkspaceLoader::new("/tmp/orka-test-gw"));

    let event_sink = Arc::new(InMemoryEventSink::new());

    let gateway = Gateway::new(
        bus.clone(),
        sessions.clone(),
        queue.clone(),
        workspace,
        event_sink,
        None,
        60,
        3600,
    );

    let cancel = CancellationToken::new();
    let cancel2 = cancel.clone();

    let env = Envelope::text("custom", SessionId::new(), "hello gateway");
    let session_id = env.session_id.clone();
    let env_id = env.id.clone();

    // Spawn gateway first so it subscribes before we publish
    let handle = tokio::spawn(async move {
        gateway.run(cancel2).await
    });

    // Small delay to ensure the gateway has subscribed
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Publish after gateway is subscribed
    bus.publish("inbound", &env).await.unwrap();

    // Give gateway time to process
    tokio::time::sleep(Duration::from_millis(100)).await;
    cancel.cancel();
    handle.await.unwrap().unwrap();

    // Verify session was created
    let session = sessions.get(&session_id).await.unwrap();
    assert!(session.is_some());

    // Verify message was enqueued
    let popped = queue.pop(Duration::from_millis(10)).await.unwrap();
    assert!(popped.is_some());
    assert_eq!(popped.unwrap().id, env_id);
}
