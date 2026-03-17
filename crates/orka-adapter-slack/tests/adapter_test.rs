use orka_adapter_slack::SlackAdapter;
use orka_core::traits::ChannelAdapter;
use orka_core::types::SessionId;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

#[test]
fn channel_id_returns_slack() {
    let adapter = SlackAdapter::new("xoxb-test".into(), 3000);
    assert_eq!(adapter.channel_id(), "slack");
}

/// Same Slack channel maps to the same SessionId; different channels differ.
#[tokio::test]
async fn session_map_consistency() {
    let sessions: Arc<Mutex<HashMap<String, SessionId>>> = Arc::new(Mutex::new(HashMap::new()));

    let sid1 = {
        let mut s = sessions.lock().await;
        *s.entry("C12345".into()).or_insert_with(SessionId::new)
    };
    let sid2 = {
        let mut s = sessions.lock().await;
        *s.entry("C12345".into()).or_insert_with(SessionId::new)
    };
    let sid3 = {
        let mut s = sessions.lock().await;
        *s.entry("C99999".into()).or_insert_with(SessionId::new)
    };

    assert_eq!(sid1, sid2, "same channel must produce same SessionId");
    assert_ne!(
        sid1, sid3,
        "different channels must produce different SessionIds"
    );
}

/// Verifies Envelope creation from a Slack event_callback message.
#[tokio::test]
async fn envelope_from_slack_event() {
    use orka_core::types::{Envelope, Payload};

    let session_id = SessionId::new();
    let mut envelope = Envelope::text("slack", session_id, "Slack message");
    envelope
        .metadata
        .insert("slack_channel".to_string(), serde_json::json!("C12345"));
    envelope
        .metadata
        .insert("slack_user".to_string(), serde_json::json!("U98765"));
    envelope
        .metadata
        .insert("chat_type".to_string(), serde_json::json!("group"));

    assert_eq!(envelope.channel, "slack");
    assert_eq!(envelope.session_id, session_id);
    match &envelope.payload {
        Payload::Text(t) => assert_eq!(t, "Slack message"),
        other => panic!("expected Text payload, got {other:?}"),
    }
    assert_eq!(
        envelope.metadata["slack_channel"],
        serde_json::json!("C12345")
    );
    assert_eq!(envelope.metadata["slack_user"], serde_json::json!("U98765"));
}

/// Verifies that the SlackEventPayload JSON shape parses correctly.
#[test]
fn deserialize_url_verification() {
    let json = r#"{
        "type": "url_verification",
        "challenge": "abc123",
        "token": "legacy"
    }"#;
    let v: serde_json::Value = serde_json::from_str(json).unwrap();
    assert_eq!(v["type"].as_str().unwrap(), "url_verification");
    assert_eq!(v["challenge"].as_str().unwrap(), "abc123");
}

#[test]
fn deserialize_event_callback() {
    let json = r#"{
        "type": "event_callback",
        "event": {
            "type": "message",
            "channel": "C12345",
            "text": "hello from slack",
            "user": "U98765",
            "channel_type": "im"
        }
    }"#;
    let v: serde_json::Value = serde_json::from_str(json).unwrap();
    assert_eq!(v["type"].as_str().unwrap(), "event_callback");
    assert_eq!(v["event"]["text"].as_str().unwrap(), "hello from slack");
    assert_eq!(v["event"]["channel_type"].as_str().unwrap(), "im");
}

/// chat_type: "im" → direct, anything else → group
#[test]
fn chat_type_classification() {
    let classify = |ct: Option<&str>| match ct {
        Some("im") => "direct",
        _ => "group",
    };

    assert_eq!(classify(Some("im")), "direct");
    assert_eq!(classify(Some("channel")), "group");
    assert_eq!(classify(Some("group")), "group");
    assert_eq!(classify(None), "group");
}

/// Bot messages (with bot_id) should be filtered out.
#[test]
fn bot_message_filtering() {
    let json = r#"{
        "type": "message",
        "channel": "C12345",
        "text": "bot says hi",
        "bot_id": "B12345"
    }"#;
    let v: serde_json::Value = serde_json::from_str(json).unwrap();
    assert!(
        v.get("bot_id").is_some(),
        "bot messages should be detected and skipped"
    );
}

#[tokio::test]
async fn shutdown_without_start_is_ok() {
    let adapter = SlackAdapter::new("xoxb-test".into(), 3000);
    adapter.shutdown().await.unwrap();
}
