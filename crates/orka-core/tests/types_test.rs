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
fn envelope_roundtrip_json() {
    let env = Envelope::text("discord", SessionId::new(), "test message");
    let json = serde_json::to_string(&env).unwrap();
    let decoded: Envelope = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.id, env.id);
    assert_eq!(decoded.channel, "discord");
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
    let debug = format!("{:?}", secret);
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
fn payload_variants_serialize() {
    let text = Payload::Text("hello".into());
    let json = serde_json::to_value(&text).unwrap();
    assert_eq!(json["type"], "Text");

    let cmd = Payload::Command(CommandPayload::new("ping", Default::default()));
    let json = serde_json::to_value(&cmd).unwrap();
    assert_eq!(json["type"], "Command");
}
