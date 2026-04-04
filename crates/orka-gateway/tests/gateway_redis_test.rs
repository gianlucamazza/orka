#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use orka_core::{
    Envelope, SessionId,
    testing::{InMemoryBus, InMemoryEventSink, InMemoryQueue, InMemorySessionStore},
    traits::PriorityQueue,
};
use orka_gateway::{Gateway, GatewayConfig, GatewayDeps};
use orka_test_support::RedisService;
use orka_workspace::WorkspaceLoader;

fn make_gateway(redis_url: &str, rate_limit: u32) -> (Gateway, Arc<InMemoryQueue>) {
    let bus = Arc::new(InMemoryBus::new());
    let sessions = Arc::new(InMemorySessionStore::new());
    let queue = Arc::new(InMemoryQueue::new());
    let workspace = Arc::new(WorkspaceLoader::new("/tmp/orka-test-gateway-redis"));
    let event_sink = Arc::new(InMemoryEventSink::new());

    let gw = Gateway::new(
        GatewayDeps {
            bus,
            sessions,
            queue: queue.clone(),
            workspace,
            event_sink,
        },
        GatewayConfig {
            redis_url: Some(redis_url.to_string()),
            rate_limit,
            dedup_ttl_secs: 60,
            dedup_enabled: true,
        },
    );
    (gw, queue)
}

#[serial_test::serial]
#[tokio::test]
#[ignore = "requires Redis"]
async fn redis_dedup_second_duplicate_dropped() {
    let redis = RedisService::discover().await.unwrap();
    let (gw, queue) = make_gateway(redis.url(), 0);

    let env = Envelope::text("test", SessionId::new(), "hello");

    gw.process(env.clone()).await.unwrap();
    gw.process(env).await.unwrap();

    // Only the first should be enqueued; duplicate is dropped
    assert_eq!(queue.len().await.unwrap(), 1);
}

#[serial_test::serial]
#[tokio::test]
#[ignore = "requires Redis"]
async fn redis_dedup_different_message_ids_both_pass() {
    let redis = RedisService::discover().await.unwrap();
    let (gw, queue) = make_gateway(redis.url(), 0);

    let env1 = Envelope::text("test", SessionId::new(), "first");
    let env2 = Envelope::text("test", SessionId::new(), "second");

    gw.process(env1).await.unwrap();
    gw.process(env2).await.unwrap();

    assert_eq!(queue.len().await.unwrap(), 2);
}

#[serial_test::serial]
#[tokio::test]
#[ignore = "requires Redis"]
async fn redis_rate_limit_drops_excess() {
    let redis = RedisService::discover().await.unwrap();
    let (gw, queue) = make_gateway(redis.url(), 2);

    let sid = SessionId::new();
    for i in 0..4u32 {
        let env = Envelope::text("test", sid, format!("msg{i}"));
        let _ = gw.process(env).await;
    }

    // Only 2 per window allowed
    assert_eq!(queue.len().await.unwrap(), 2);
}

#[serial_test::serial]
#[tokio::test]
#[ignore = "requires Redis"]
async fn redis_rate_limit_zero_means_unlimited() {
    let redis = RedisService::discover().await.unwrap();
    let (gw, queue) = make_gateway(redis.url(), 0);

    let sid = SessionId::new();
    for i in 0..5u32 {
        let env = Envelope::text("test", sid, format!("msg{i}"));
        gw.process(env).await.unwrap();
    }

    assert_eq!(queue.len().await.unwrap(), 5);
}
