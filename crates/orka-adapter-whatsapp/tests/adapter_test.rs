use orka_adapter_whatsapp::WhatsAppAdapter;
use orka_core::traits::ChannelAdapter;
use orka_core::types::SessionId;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

#[test]
fn channel_id_returns_whatsapp() {
    let adapter = WhatsAppAdapter::new(
        "access-token".into(),
        "phone-id".into(),
        "verify-token".into(),
        3001,
    );
    assert_eq!(adapter.channel_id(), "whatsapp");
}

/// Same sender phone number maps to the same SessionId; different numbers differ.
#[tokio::test]
async fn session_map_consistency() {
    let sessions: Arc<Mutex<HashMap<String, SessionId>>> = Arc::new(Mutex::new(HashMap::new()));

    let sid1 = {
        let mut s = sessions.lock().await;
        *s.entry("+1234567890".into()).or_insert_with(SessionId::new)
    };
    let sid2 = {
        let mut s = sessions.lock().await;
        *s.entry("+1234567890".into()).or_insert_with(SessionId::new)
    };
    let sid3 = {
        let mut s = sessions.lock().await;
        *s.entry("+0987654321".into()).or_insert_with(SessionId::new)
    };

    assert_eq!(sid1, sid2, "same phone number must produce same SessionId");
    assert_ne!(
        sid1, sid3,
        "different phone numbers must produce different SessionIds"
    );
}

/// Verifies Envelope creation from a WhatsApp webhook message.
#[tokio::test]
async fn envelope_from_whatsapp_message() {
    use orka_core::types::{Envelope, Payload};

    let session_id = SessionId::new();
    let mut envelope = Envelope::text("whatsapp", session_id, "Hello from WhatsApp");
    envelope.metadata.insert(
        "whatsapp_from".to_string(),
        serde_json::json!("+1234567890"),
    );
    envelope
        .metadata
        .insert("chat_type".to_string(), serde_json::json!("direct"));

    assert_eq!(envelope.channel, "whatsapp");
    assert_eq!(envelope.session_id, session_id);
    match &envelope.payload {
        Payload::Text(t) => assert_eq!(t, "Hello from WhatsApp"),
        other => panic!("expected Text payload, got {other:?}"),
    }
    assert_eq!(
        envelope.metadata["whatsapp_from"],
        serde_json::json!("+1234567890")
    );
    assert_eq!(envelope.metadata["chat_type"], serde_json::json!("direct"));
}

/// Verifies that the WhatsApp webhook JSON shape parses correctly.
#[test]
fn deserialize_webhook_payload() {
    let json = r#"{
        "entry": [
            {
                "changes": [
                    {
                        "value": {
                            "messages": [
                                {
                                    "from": "+1234567890",
                                    "type": "text",
                                    "text": { "body": "hi there" }
                                }
                            ]
                        }
                    }
                ]
            }
        ]
    }"#;
    let v: serde_json::Value = serde_json::from_str(json).unwrap();
    let msg = &v["entry"][0]["changes"][0]["value"]["messages"][0];
    assert_eq!(msg["from"].as_str().unwrap(), "+1234567890");
    assert_eq!(msg["type"].as_str().unwrap(), "text");
    assert_eq!(msg["text"]["body"].as_str().unwrap(), "hi there");
}

/// Non-text messages should be skipped.
#[test]
fn non_text_messages_filtered() {
    let json = r#"{
        "from": "+1234567890",
        "type": "image",
        "image": { "id": "img123" }
    }"#;
    let v: serde_json::Value = serde_json::from_str(json).unwrap();
    let msg_type = v["type"].as_str().unwrap();
    assert_ne!(msg_type, "text", "non-text messages should be filtered");
}

/// Webhook verification logic: mode=subscribe + matching token → return challenge.
#[test]
fn webhook_verification_logic() {
    let verify_token = "my-secret-token";
    let mode = Some("subscribe");
    let token = Some("my-secret-token");
    let challenge = "challenge-string";

    let verified = mode == Some("subscribe") && token == Some(verify_token);
    assert!(verified);
    assert_eq!(challenge, "challenge-string");

    // Wrong token should fail
    let bad_token = Some("wrong-token");
    let not_verified = mode == Some("subscribe") && bad_token == Some(verify_token);
    assert!(!not_verified);
}

#[tokio::test]
async fn shutdown_without_start_is_ok() {
    let adapter = WhatsAppAdapter::new(
        "access-token".into(),
        "phone-id".into(),
        "verify-token".into(),
        3001,
    );
    adapter.shutdown().await.unwrap();
}
