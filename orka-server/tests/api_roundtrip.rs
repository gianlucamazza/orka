//! End-to-end HTTP round-trip test: POST message → WebSocket reply.
//!
//! Tests the full message lifecycle at the HTTP level:
//!
//! ```text
//! POST /api/v1/message
//!   → adapter mpsc sink
//!   → bus "inbound"
//!   → Gateway (dedup, rate-limit, session)
//!   → PriorityQueue
//!   → WorkerPool (EchoHandler)
//!   → bus "outbound"
//!   → WsRegistry.send_to_session()
//!   → WebSocket client
//! ```

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use orka_adapter_custom::{routes::app_router, ws::WsRegistry};
use orka_core::testing::{InMemoryBus, InMemoryEventSink, InMemoryQueue, InMemorySessionStore};
use orka_core::{Payload, SessionId, StreamRegistry, traits::MessageBus};
use orka_gateway::Gateway;
use orka_worker::{EchoHandler, WorkerPool};
use orka_workspace::WorkspaceLoader;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

/// Start the inbound/outbound pipeline with in-memory backends.
///
/// Returns (bus, sessions, adapter address, ws_registry, shutdown token).
async fn start_pipeline() -> (
    Arc<InMemoryBus>,
    Arc<InMemorySessionStore>,
    std::net::SocketAddr,
    WsRegistry,
    CancellationToken,
) {
    let bus = Arc::new(InMemoryBus::new());
    let sessions = Arc::new(InMemorySessionStore::new());
    let queue = Arc::new(InMemoryQueue::new());
    let event_sink = Arc::new(InMemoryEventSink::new());
    let shutdown = CancellationToken::new();

    // 1. Start gateway
    let workspace = Arc::new(WorkspaceLoader::new("."));
    let gateway = Gateway::new(
        bus.clone(),
        sessions.clone(),
        queue.clone(),
        workspace,
        event_sink.clone(),
        None, // no Redis dedup
        0,    // no rate limit
        3600,
    );
    tokio::spawn({
        let cancel = shutdown.clone();
        async move {
            gateway.run(cancel).await.ok();
        }
    });

    // 2. Start worker pool with EchoHandler
    let handler: Arc<dyn orka_worker::AgentHandler> = Arc::new(EchoHandler);
    let worker_pool = WorkerPool::new(
        queue.clone(),
        sessions.clone(),
        bus.clone(),
        handler,
        event_sink.clone(),
        1,
        0,
    );
    tokio::spawn({
        let cancel = shutdown.clone();
        async move {
            worker_pool.run(cancel).await.ok();
        }
    });

    // 3. Start custom adapter (app_router bound to port 0)
    let (sink_tx, mut sink_rx) = tokio::sync::mpsc::channel(16);
    let ws_registry = WsRegistry::new();
    let ws_reg_for_bridge = ws_registry.clone();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let router = app_router(sink_tx, ws_registry.clone(), StreamRegistry::new(), None);
    tokio::spawn(async move {
        axum::serve(listener, router).await.ok();
    });

    // 4. Bridge adapter sink → bus "inbound"
    let bus_for_inbound = bus.clone();
    tokio::spawn(async move {
        while let Some(envelope) = sink_rx.recv().await {
            bus_for_inbound.publish("inbound", &envelope).await.ok();
        }
    });

    // 5. Bridge bus "outbound" → WsRegistry
    let mut outbound_rx = bus.subscribe("outbound").await.unwrap();
    tokio::spawn(async move {
        while let Some(envelope) = outbound_rx.recv().await {
            let text = match &envelope.payload {
                Payload::Text(t) => t.clone(),
                _ => "[non-text]".to_string(),
            };
            let outbound = orka_core::OutboundMessage::text(
                envelope.channel.clone(),
                envelope.session_id,
                text,
                None,
            );
            let json = serde_json::to_string(&outbound).unwrap();
            ws_reg_for_bridge
                .send_to_session(&envelope.session_id, &json)
                .await;
        }
    });

    // Give the pipeline a moment to start subscribing
    tokio::time::sleep(Duration::from_millis(50)).await;

    (bus, sessions, addr, ws_registry, shutdown)
}

#[tokio::test]
async fn message_roundtrip_via_ws() {
    let (_, _, addr, _, shutdown) = start_pipeline().await;

    // Generate a session_id so we can connect WebSocket BEFORE sending the message
    let session_id = SessionId::new();

    // 1. Connect WebSocket with the pre-generated session_id
    let ws_url = format!("ws://{addr}/api/v1/ws?session_id={session_id}");
    let (mut ws_stream, _) = tokio_tungstenite::connect_async(&ws_url).await.unwrap();

    // 2. POST a message to the adapter with the same session_id
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{addr}/api/v1/message"))
        .json(&serde_json::json!({
            "text": "hello orka",
            "session_id": session_id.to_string(),
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "POST /api/v1/message should return 200");

    // 3. Wait for the echo reply on the WebSocket
    let msg = tokio::time::timeout(Duration::from_secs(5), ws_stream.next())
        .await
        .expect("timed out waiting for WebSocket message")
        .unwrap()
        .unwrap();

    let received: orka_core::OutboundMessage =
        serde_json::from_str(msg.to_text().unwrap()).unwrap();
    assert_eq!(received.session_id, session_id);
    match &received.payload {
        Payload::Text(t) => assert!(
            t.contains("hello orka"),
            "echo reply should contain original text; got: {t}"
        ),
        other => panic!("expected Text payload, got: {other:?}"),
    }

    shutdown.cancel();
}

#[tokio::test]
async fn message_reuses_session() {
    let (_, _, addr, _, shutdown) = start_pipeline().await;

    let session_id = SessionId::new();

    // Connect WebSocket once for the whole session
    let ws_url = format!("ws://{addr}/api/v1/ws?session_id={session_id}");
    let (mut ws_stream, _) = tokio_tungstenite::connect_async(&ws_url).await.unwrap();

    let client = reqwest::Client::new();

    // Send two messages with the same session_id
    for text in ["first message", "second message"] {
        let resp = client
            .post(format!("http://{addr}/api/v1/message"))
            .json(&serde_json::json!({
                "text": text,
                "session_id": session_id.to_string(),
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
    }

    // Both replies should arrive on the same WebSocket
    for _ in 0..2 {
        let msg = tokio::time::timeout(Duration::from_secs(5), ws_stream.next())
            .await
            .expect("timed out waiting for second WebSocket message")
            .unwrap()
            .unwrap();
        let received: orka_core::OutboundMessage =
            serde_json::from_str(msg.to_text().unwrap()).unwrap();
        assert_eq!(received.session_id, session_id);
    }

    shutdown.cancel();
}
