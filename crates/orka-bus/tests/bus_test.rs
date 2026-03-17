use orka_bus::RedisBus;
use orka_core::traits::MessageBus;
use orka_core::{Envelope, MessageId, SessionId};

#[test]
fn stream_key_format() {
    assert_eq!(RedisBus::stream_key("foo"), "orka:bus:foo");
    assert_eq!(RedisBus::stream_key("events.user"), "orka:bus:events.user");
}

#[test]
fn envelope_json_roundtrip() {
    let env = Envelope::text("telegram", SessionId::new(), "hello world");
    let json = serde_json::to_string(&env).expect("serialize");
    let deserialized: Envelope = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(deserialized.id, env.id);
    assert_eq!(deserialized.channel, env.channel);
    assert_eq!(deserialized.session_id, env.session_id);
}

#[tokio::test]
#[ignore] // requires Redis
async fn publish_subscribe_ack_roundtrip() {
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::redis::Redis;

    let container = Redis::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(6379).await.unwrap();
    let url = format!("redis://127.0.0.1:{port}");

    let bus = RedisBus::new(&url, &orka_core::config::BusConfig::default()).expect("create bus");
    let topic = "test-topic";

    // Subscribe first
    let mut stream = bus.subscribe(topic).await.expect("subscribe");

    // Publish an envelope
    let envelope = Envelope::text("test-channel", SessionId::new(), "integration test");
    let expected_id = envelope.id;
    bus.publish(topic, &envelope).await.expect("publish");

    // Receive it
    let received = tokio::time::timeout(std::time::Duration::from_secs(10), stream.recv())
        .await
        .expect("timeout waiting for message")
        .expect("stream closed unexpectedly");

    assert_eq!(received.id, expected_id);
    assert_eq!(received.channel, "test-channel");

    // Ack it
    bus.ack(&received.id).await.expect("ack");

    // Verify pending map is now empty (ack removed the entry)
    // A second ack with the same ID should fail
    let result = bus.ack(&received.id).await;
    assert!(result.is_err(), "acking same message twice should error");
}

#[tokio::test]
#[ignore] // requires Redis
async fn ack_unknown_message_errors() {
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::redis::Redis;

    let container = Redis::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(6379).await.unwrap();
    let url = format!("redis://127.0.0.1:{port}");

    let bus = RedisBus::new(&url, &orka_core::config::BusConfig::default()).expect("create bus");
    let random_id = MessageId::new();
    let result = bus.ack(&random_id).await;
    assert!(result.is_err(), "acking unknown message should error");
}
