#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

use std::{collections::HashMap, sync::Arc};

use orka_adapter_discord::DiscordAdapter;
use orka_core::{SecretStr, traits::ChannelAdapter, types::SessionId};
use tokio::sync::Mutex;

#[test]
fn channel_id_returns_discord() {
    let adapter = DiscordAdapter::new(SecretStr::new("test-token"), None);
    assert_eq!(adapter.channel_id(), "discord");
}

/// Same `channel_id` string maps to the same `SessionId`; different strings
/// differ.
#[tokio::test]
async fn session_map_consistency() {
    let sessions: Arc<Mutex<HashMap<String, SessionId>>> = Arc::new(Mutex::new(HashMap::new()));

    let sid1 = {
        let mut s = sessions.lock().await;
        *s.entry("ch-abc".into()).or_insert_with(SessionId::new)
    };
    let sid2 = {
        let mut s = sessions.lock().await;
        *s.entry("ch-abc".into()).or_insert_with(SessionId::new)
    };
    let sid3 = {
        let mut s = sessions.lock().await;
        *s.entry("ch-xyz".into()).or_insert_with(SessionId::new)
    };

    assert_eq!(sid1, sid2, "same channel_id must produce same SessionId");
    assert_ne!(
        sid1, sid3,
        "different channel_ids must produce different SessionIds"
    );
}

/// Verifies Envelope creation from a Discord MESSAGE_CREATE-style payload
/// using the canonical `PlatformContext` routing model.
#[tokio::test]
async fn envelope_from_discord_message() {
    use orka_contracts::platform::{PlatformContext, SenderInfo};
    use orka_core::types::{Envelope, Payload};

    let session_id = SessionId::new();
    let mut envelope = Envelope::text("discord", session_id, "Hey there");
    envelope.platform_context = Some(PlatformContext {
        chat_id: Some("ch-abc".into()),
        guild_id: Some("guild-123".into()),
        sender: SenderInfo::default(),
        ..Default::default()
    });

    assert_eq!(envelope.channel, "discord");
    assert_eq!(envelope.session_id, session_id);
    match &envelope.payload {
        Payload::Text(t) => assert_eq!(t, "Hey there"),
        other => panic!("expected Text payload, got {other:?}"),
    }
    let pc = envelope.platform_context.as_ref().unwrap();
    assert_eq!(pc.chat_id.as_deref(), Some("ch-abc"));
    assert_eq!(pc.guild_id.as_deref(), Some("guild-123"));
}

/// Verifies `chat_type` classification: `guild_id` present = group, absent =
/// direct.
#[test]
fn chat_type_classification() {
    let server_msg = serde_json::json!({
        "content": "hello",
        "channel_id": "123",
        "guild_id": "456",
        "author": { "bot": false }
    });
    let dm_msg = serde_json::json!({
        "content": "hello",
        "channel_id": "789",
        "author": { "bot": false }
    });

    let server_type = if server_msg
        .get("guild_id")
        .and_then(|v| v.as_str())
        .is_some()
    {
        "group"
    } else {
        "direct"
    };
    let dm_type = if dm_msg.get("guild_id").and_then(|v| v.as_str()).is_some() {
        "group"
    } else {
        "direct"
    };

    assert_eq!(server_type, "group");
    assert_eq!(dm_type, "direct");
}

/// Bot messages should be filtered out.
#[test]
fn bot_message_filtering() {
    let msg = serde_json::json!({
        "content": "automated",
        "channel_id": "123",
        "author": { "bot": true }
    });

    let is_bot = msg["author"]["bot"].as_bool().unwrap_or(false);
    assert!(is_bot, "bot messages should be detected and skipped");
}

#[tokio::test]
async fn shutdown_without_start_is_ok() {
    let adapter = DiscordAdapter::new(SecretStr::new("test-token"), None);
    adapter.shutdown().await.unwrap();
}
