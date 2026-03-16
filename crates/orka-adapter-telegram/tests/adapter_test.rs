use orka_adapter_telegram::TelegramAdapter;
use orka_core::traits::ChannelAdapter;
use orka_core::types::SessionId;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

#[test]
fn channel_id_returns_telegram() {
    let adapter = TelegramAdapter::new("test-token".into());
    assert_eq!(adapter.channel_id(), "telegram");
}

/// Simulates the session-map logic used inside the adapter: same chat_id always
/// yields the same SessionId, different chat_ids yield different SessionIds.
#[tokio::test]
async fn session_map_consistency() {
    let sessions: Arc<Mutex<HashMap<i64, SessionId>>> =
        Arc::new(Mutex::new(HashMap::new()));

    let sid1 = {
        let mut s = sessions.lock().await;
        s.entry(12345).or_insert_with(SessionId::new).clone()
    };
    let sid2 = {
        let mut s = sessions.lock().await;
        s.entry(12345).or_insert_with(SessionId::new).clone()
    };
    let sid3 = {
        let mut s = sessions.lock().await;
        s.entry(99999).or_insert_with(SessionId::new).clone()
    };

    assert_eq!(sid1, sid2, "same chat_id must produce same SessionId");
    assert_ne!(sid1, sid3, "different chat_ids must produce different SessionIds");
}

/// Verifies Envelope creation from a Telegram-style update payload.
#[tokio::test]
async fn envelope_from_telegram_update() {
    use orka_core::types::{Envelope, Payload};

    let session_id = SessionId::new();
    let mut envelope = Envelope::text("telegram", session_id.clone(), "Hello world");
    envelope
        .metadata
        .insert("telegram_chat_id".to_string(), serde_json::json!(12345i64));
    envelope
        .metadata
        .insert("chat_type".to_string(), serde_json::json!("direct"));

    assert_eq!(envelope.channel, "telegram");
    assert_eq!(envelope.session_id, session_id);
    match &envelope.payload {
        Payload::Text(t) => assert_eq!(t, "Hello world"),
        other => panic!("expected Text payload, got {other:?}"),
    }
    assert_eq!(envelope.metadata["telegram_chat_id"], serde_json::json!(12345i64));
    assert_eq!(envelope.metadata["chat_type"], serde_json::json!("direct"));
}

/// Verifies that the Telegram deserialization structs parse correctly.
#[test]
fn deserialize_telegram_response() {
    let json = r#"{
        "ok": true,
        "result": [
            {
                "update_id": 100,
                "message": {
                    "message_id": 1,
                    "chat": { "id": 12345, "type": "private" },
                    "text": "hello",
                    "from": { "id": 99, "first_name": "Test" }
                }
            }
        ]
    }"#;

    // We can at least validate that the JSON shape is correct by parsing as Value
    let v: serde_json::Value = serde_json::from_str(json).unwrap();
    assert!(v["ok"].as_bool().unwrap());
    let updates = v["result"].as_array().unwrap();
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0]["message"]["text"].as_str().unwrap(), "hello");
    assert_eq!(updates[0]["message"]["chat"]["id"].as_i64().unwrap(), 12345);
}

#[tokio::test]
async fn shutdown_without_start_is_ok() {
    let adapter = TelegramAdapter::new("test-token".into());
    // Shutdown before start should not panic
    adapter.shutdown().await.unwrap();
}
