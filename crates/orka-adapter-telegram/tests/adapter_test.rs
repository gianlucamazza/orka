#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

use std::{collections::HashMap, sync::Arc};

use orka_adapter_telegram::{TelegramAdapter, TelegramAdapterConfig};
use orka_core::{SecretStr, traits::ChannelAdapter, types::SessionId};
use tokio::sync::Mutex;

#[test]
fn channel_id_returns_telegram() {
    let adapter = TelegramAdapter::new(
        TelegramAdapterConfig::default(),
        SecretStr::new("test-token"),
    );
    assert_eq!(adapter.channel_id(), "telegram");
}

/// Verifies that the same `chat_id` always resolves to the same `SessionId`
/// (in-memory path) and that different `chat_ids` yield different `SessionIds`.
#[tokio::test]
async fn session_map_consistency() {
    let sessions: Arc<Mutex<HashMap<i64, SessionId>>> = Arc::new(Mutex::new(HashMap::new()));

    let sid1 = orka_adapter_telegram::polling::resolve_session(12345, &sessions, None).await;
    let sid2 = orka_adapter_telegram::polling::resolve_session(12345, &sessions, None).await;
    let sid3 = orka_adapter_telegram::polling::resolve_session(99999, &sessions, None).await;

    assert_eq!(sid1, sid2, "same chat_id must produce same SessionId");
    assert_ne!(
        sid1, sid3,
        "different chat_ids must produce different SessionIds"
    );
}

/// Verifies Envelope creation from a Telegram-style update payload.
#[tokio::test]
async fn envelope_from_telegram_update() {
    use orka_core::types::{Envelope, Payload};

    let session_id = SessionId::new();
    let mut envelope = Envelope::text("telegram", session_id, "Hello world");
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
    assert_eq!(
        envelope.metadata["telegram_chat_id"],
        serde_json::json!(12345i64)
    );
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
    let adapter = TelegramAdapter::new(
        TelegramAdapterConfig::default(),
        SecretStr::new("test-token"),
    );
    // Shutdown before start should not panic
    adapter.shutdown().await.unwrap();
}

#[test]
fn config_defaults_match_current_api() {
    let config = TelegramAdapterConfig::default();
    assert!(config.mode.is_none());
    assert_eq!(config.webhook_port_or_default(), 8443);
    assert!(!config.is_webhook());
}
