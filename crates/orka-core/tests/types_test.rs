#![allow(missing_docs)]

use std::collections::HashMap;

use orka_core::*;

#[test]
fn message_id_is_unique() {
    let a = MessageId::new();
    let b = MessageId::new();
    assert_ne!(a, b);
}

#[test]
fn session_id_is_unique() {
    let a = SessionId::new();
    let b = SessionId::new();
    assert_ne!(a, b);
}

#[test]
fn priority_ordering() {
    assert!(Priority::Urgent > Priority::Normal);
    assert!(Priority::Normal > Priority::Background);
}

#[test]
fn envelope_text_constructor() {
    let sid = SessionId::new();
    let env = Envelope::text("telegram", sid, "hello");
    assert_eq!(env.channel, "telegram");
    assert_eq!(env.session_id, sid);
    assert!(matches!(env.payload, Payload::Text(ref s) if s == "hello"));
    assert_eq!(env.priority, Priority::Normal);
}

#[test]
fn envelope_roundtrip_json() -> serde_json::Result<()> {
    let env = Envelope::text("discord", SessionId::new(), "test message");
    let json = serde_json::to_string(&env)?;
    let decoded: Envelope = serde_json::from_str(&json)?;
    assert_eq!(decoded.id, env.id);
    assert_eq!(decoded.channel, "discord");
    Ok(())
}

#[test]
fn session_creation() {
    let session = Session::new("telegram", "user123");
    assert_eq!(session.channel, "telegram");
    assert_eq!(session.user_id, "user123");
    assert!(session.state.is_empty());
}

#[test]
fn secret_value_redacted_debug() {
    let secret = SecretValue::new(b"super-secret".to_vec());
    let debug = format!("{secret:?}");
    assert_eq!(debug, "[REDACTED]");
    assert_eq!(secret.expose_str(), Some("super-secret"));
}

#[test]
fn secret_value_zeroed_on_drop() {
    let data = b"secret-data".to_vec();
    let secret = SecretValue::new(data);
    assert_eq!(secret.expose(), b"secret-data");
    drop(secret);
}

#[test]
fn payload_variants_serialize() -> serde_json::Result<()> {
    let text = Payload::Text("hello".into());
    let json = serde_json::to_value(&text)?;
    assert_eq!(json["type"], "Text");

    let cmd = Payload::Command(CommandPayload::new("ping", HashMap::default()));
    let json = serde_json::to_value(&cmd)?;
    assert_eq!(json["type"], "Command");
    Ok(())
}

// ── Phase 1: orka-contracts integration ──────────────────────────────────────

#[test]
fn capability_set_serde_roundtrip() -> serde_json::Result<()> {
    use orka_core::{Capability, CapabilitySet};
    let mut caps = CapabilitySet::new();
    caps.insert(Capability::TextInbound);
    caps.insert(Capability::StreamingDeltas);
    caps.insert(Capability::MediaOutbound);

    let json = serde_json::to_string(&caps)?;
    let decoded: CapabilitySet = serde_json::from_str(&json)?;
    assert_eq!(caps, decoded);
    Ok(())
}

#[test]
fn inbound_interaction_converts_to_envelope() {
    use chrono::Utc;
    use orka_contracts::{
        InboundInteraction, InteractionContent, PlatformContext, SenderInfo, TraceContext,
    };
    use uuid::Uuid;

    let session_id = Uuid::now_v7();
    let interaction = InboundInteraction {
        id: Uuid::now_v7(),
        source_channel: "telegram".into(),
        session_id,
        timestamp: Utc::now(),
        content: InteractionContent::Text("hello world".into()),
        context: PlatformContext {
            sender: SenderInfo {
                user_id: Some("u42".into()),
                display_name: Some("Alice".into()),
                platform_user_id: Some("12345".into()),
            },
            chat_id: Some("chat-99".into()),
            interaction_kind: Some("direct".into()),
            ..Default::default()
        },
        trace: TraceContext::default(),
    };

    let envelope = Envelope::from(interaction);
    assert_eq!(envelope.channel, "telegram");
    assert_eq!(envelope.session_id.as_uuid(), session_id);
    assert!(matches!(envelope.payload, Payload::Text(ref s) if s == "hello world"));

    let Some(ctx) = envelope.platform_context else {
        panic!("platform_context should be set");
    };
    assert_eq!(ctx.chat_id.as_deref(), Some("chat-99"));
    assert_eq!(ctx.interaction_kind.as_deref(), Some("direct"));
    assert_eq!(ctx.sender.platform_user_id.as_deref(), Some("12345"));
}

#[test]
fn stream_chunk_kind_converts_to_realtime_event() {
    use orka_contracts::RealtimeEvent;
    use orka_core::StreamChunkKind;

    let delta = StreamChunkKind::Delta("hello".into());
    let event = RealtimeEvent::from(delta);
    assert!(matches!(event, RealtimeEvent::MessageDelta { delta } if delta == "hello"));

    let done = StreamChunkKind::Done;
    let event = RealtimeEvent::from(done);
    assert!(matches!(event, RealtimeEvent::StreamDone));
}

#[test]
fn platform_context_serde_roundtrip() -> serde_json::Result<()> {
    use orka_contracts::{PlatformContext, SenderInfo};

    let ctx = PlatformContext {
        sender: SenderInfo::default(),
        chat_id: Some("42".into()),
        thread_id: None,
        guild_id: Some("srv-1".into()),
        reply_target: None,
        interaction_kind: Some("group".into()),
        trust_level: None,
        extensions: [("telegram_message_thread_id".into(), serde_json::json!(7))]
            .into_iter()
            .collect(),
    };
    let json = serde_json::to_string(&ctx)?;
    let decoded: PlatformContext = serde_json::from_str(&json)?;
    assert_eq!(decoded.chat_id, ctx.chat_id);
    assert_eq!(decoded.guild_id, ctx.guild_id);
    assert_eq!(
        decoded.extensions.get("telegram_message_thread_id"),
        Some(&serde_json::json!(7))
    );
    Ok(())
}
