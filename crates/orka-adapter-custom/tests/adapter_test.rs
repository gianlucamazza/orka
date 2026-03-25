#![allow(missing_docs)]

use orka_adapter_custom::{
    CustomAdapter, routes::app_router, types::InboundResponse, ws::WsRegistry,
};
use orka_core::{
    OutboundMessage, Payload, SessionId, StreamRegistry, config::CustomAdapterConfig,
    traits::ChannelAdapter,
};
use tokio::sync::mpsc;

#[tokio::test]
async fn post_message_arrives_on_sink() {
    let (tx, mut rx) = mpsc::channel(16);
    let ws_registry = WsRegistry::new();

    // Bind to port 0 to get a random available port.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let router = app_router(tx, ws_registry, StreamRegistry::new(), None);
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{addr}/api/v1/message"))
        .json(&serde_json::json!({
            "text": "Hello from test"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);

    let body: InboundResponse = resp.json().await.unwrap();
    assert!(!body.message_id.is_empty());
    assert!(!body.session_id.is_empty());

    let envelope = rx.try_recv().unwrap();
    assert_eq!(envelope.channel, "custom");
    match &envelope.payload {
        orka_core::Payload::Text(t) => assert_eq!(t, "Hello from test"),
        other => panic!("Expected Text payload, got {other:?}"),
    }
}

#[tokio::test]
async fn health_endpoint_returns_ok() {
    let (tx, _rx) = mpsc::channel(16);
    let ws_registry = WsRegistry::new();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let router = app_router(tx, ws_registry, StreamRegistry::new(), None);
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{addr}/api/v1/health"))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
async fn adapter_start_and_shutdown() {
    let adapter = CustomAdapter::new(
        {
            let mut cfg = CustomAdapterConfig::default();
            cfg.host = "127.0.0.1".into();
            cfg.port = 0;
            cfg.workspace = None;
            cfg
        },
        None,
        StreamRegistry::new(),
    );

    let (tx, _rx) = mpsc::channel(16);
    adapter.start(tx).await.unwrap();
    adapter.shutdown().await.unwrap();
}

#[tokio::test]
async fn ws_connect_and_receive_outbound() {
    let adapter = CustomAdapter::new(
        {
            let mut cfg = CustomAdapterConfig::default();
            cfg.host = "127.0.0.1".into();
            cfg.port = 0;
            cfg.workspace = None;
            cfg
        },
        None,
        StreamRegistry::new(),
    );

    let (tx, _rx) = mpsc::channel(16);
    adapter.start(tx).await.unwrap();

    // We need to find the actual port — start adapter and get the port from the
    // listener. Since the adapter binds to port 0, we need another approach.
    // Use app_router directly for controlled testing.
    adapter.shutdown().await.unwrap();

    // Test via app_router for deterministic port
    let (sink_tx, _sink_rx) = mpsc::channel(16);
    let ws_registry = WsRegistry::new();
    let registry_clone = ws_registry.clone();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let router = app_router(sink_tx, ws_registry, StreamRegistry::new(), None);
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let session_id = SessionId::new();
    let url = format!("ws://{addr}/api/v1/ws?session_id={session_id}");

    let (mut ws_stream, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

    // Send an outbound message via the registry
    let outbound = OutboundMessage::text("custom", session_id, "hello from server", None);

    let text = serde_json::to_string(&outbound).unwrap();
    let count = registry_clone.send_to_session(&session_id, &text).await;
    assert_eq!(count, 1);

    use futures_util::StreamExt;
    let msg = ws_stream.next().await.unwrap().unwrap();
    let received: OutboundMessage = serde_json::from_str(msg.to_text().unwrap()).unwrap();
    assert_eq!(received.session_id, session_id);
    match &received.payload {
        Payload::Text(t) => assert_eq!(t, "hello from server"),
        other => panic!("Expected Text payload, got {other:?}"),
    }
}
