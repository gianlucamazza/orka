//! Inbound message gateway with deduplication, rate limiting, and priority
//! routing.
//!
//! The [`Gateway`] subscribes to the message bus, resolves sessions, applies
//! rate limits and idempotency checks, then enqueues messages for worker
//! processing.

#![warn(missing_docs)]

use std::{collections::HashMap, sync::Arc};

use chrono::Utc;
use deadpool_redis::Pool;
use orka_core::{
    DomainEvent, DomainEventKind, Envelope, Session,
    traits::{EventSink, MessageBus, PriorityQueue, SessionStore},
};
use orka_workspace::WorkspaceLoader;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

const DEDUP_KEY_PREFIX: &str = "orka:dedup:";

/// Central message gateway that bridges adapters to the worker queue.
pub struct Gateway {
    bus: Arc<dyn MessageBus>,
    sessions: Arc<dyn SessionStore>,
    queue: Arc<dyn PriorityQueue>,
    _workspace: Arc<WorkspaceLoader>,
    event_sink: Arc<dyn EventSink>,
    redis_pool: Option<Pool>,
    rate_limit: u32,
    dedup_ttl_secs: u64,
    /// Tracks (count, window_start) per session for rate limiting
    rate_counters: Mutex<HashMap<String, (u32, chrono::DateTime<Utc>)>>,
}

impl Gateway {
    #[allow(clippy::too_many_arguments)]
    /// Create a new gateway with the given dependencies.
    pub fn new(
        bus: Arc<dyn MessageBus>,
        sessions: Arc<dyn SessionStore>,
        queue: Arc<dyn PriorityQueue>,
        workspace: Arc<WorkspaceLoader>,
        event_sink: Arc<dyn EventSink>,
        redis_url: Option<&str>,
        rate_limit: u32,
        dedup_ttl_secs: u64,
    ) -> Self {
        let redis_pool = redis_url.and_then(|url| {
            let cfg = deadpool_redis::Config::from_url(url);
            cfg.create_pool(Some(deadpool_redis::Runtime::Tokio1)).ok()
        });
        Self {
            bus,
            sessions,
            queue,
            _workspace: workspace,
            event_sink,
            redis_pool,
            rate_limit,
            dedup_ttl_secs,
            rate_counters: Mutex::new(HashMap::new()),
        }
    }

    /// Start the gateway loop, processing messages until `shutdown` is
    /// signalled.
    pub async fn run(&self, shutdown: CancellationToken) -> orka_core::Result<()> {
        info!("gateway starting");
        let mut rx = self.bus.subscribe("inbound").await?;

        loop {
            tokio::select! {
                _ = shutdown.cancelled() => {
                    info!("gateway shutting down");
                    break;
                }
                msg = rx.recv() => {
                    match msg {
                        Some(envelope) => {
                            if let Err(e) = self.process(envelope).await {
                                error!(%e, "gateway: failed to process envelope");
                            }
                        }
                        None => {
                            warn!("gateway: inbound stream closed");
                            break;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    async fn is_duplicate(&self, message_id: &orka_core::MessageId) -> bool {
        let pool = match &self.redis_pool {
            Some(p) => p,
            None => return false,
        };
        let mut conn = match pool.get().await {
            Ok(c) => c,
            Err(e) => {
                warn!(error = %e, message_id = %message_id, "dedup: Redis pool error, accepting message");
                return false;
            }
        };
        let key = format!("{DEDUP_KEY_PREFIX}{message_id}");
        // SET NX EX - returns true if key was set (not duplicate), false if already
        // exists
        let result: redis::RedisResult<bool> = redis::cmd("SET")
            .arg(&key)
            .arg("1")
            .arg("NX")
            .arg("EX")
            .arg(self.dedup_ttl_secs)
            .query_async(&mut *conn)
            .await;
        match result {
            Ok(was_set) => !was_set,
            Err(e) => {
                warn!(error = %e, message_id = %message_id, "dedup: Redis SET NX error, accepting message");
                false
            }
        }
    }

    /// Check if session is within rate limit. Returns true if allowed.
    ///
    /// Uses Redis sliding window counter when available (correct across
    /// multiple instances). Falls back to in-memory counter otherwise.
    async fn check_rate_limit(&self, session_id: &str) -> bool {
        if self.rate_limit == 0 {
            return true;
        }

        // Try Redis-based rate limit first
        if let Some(ref pool) = self.redis_pool {
            match pool.get().await {
                Ok(mut conn) => {
                    let key = format!("orka:ratelimit:{session_id}");
                    let result: redis::RedisResult<i64> =
                        redis::cmd("INCR").arg(&key).query_async(&mut *conn).await;
                    match result {
                        Ok(count) => {
                            if count == 1 {
                                // First request in window — set expiry
                                let expire_result: redis::RedisResult<()> = redis::cmd("EXPIRE")
                                    .arg(&key)
                                    .arg(60i64)
                                    .query_async(&mut *conn)
                                    .await;
                                if let Err(e) = expire_result {
                                    warn!(error = %e, %key, "rate_limit: failed to set EXPIRE, key may persist");
                                }
                            }
                            return count <= self.rate_limit as i64;
                        }
                        Err(e) => {
                            warn!(error = %e, "rate_limit: Redis INCR error, falling back to in-memory");
                        }
                    }
                }
                Err(e) => {
                    warn!(error = %e, "rate_limit: Redis pool error, falling back to in-memory");
                }
            }
        }

        // In-memory fallback
        let now = Utc::now();
        let mut counters = self.rate_counters.lock().await;

        // Periodic cleanup: remove entries older than 2 minutes
        if counters.len() > 10_000 {
            counters.retain(|_, (_, window_start)| {
                now.signed_duration_since(*window_start).num_seconds() < 120
            });
        }

        let entry = counters.entry(session_id.to_string()).or_insert((0, now));

        // Reset window if more than 60 seconds have passed
        let elapsed = now.signed_duration_since(entry.1);
        if elapsed.num_seconds() >= 60 {
            entry.0 = 0;
            entry.1 = now;
        }

        if entry.0 >= self.rate_limit {
            return false;
        }
        entry.0 += 1;
        true
    }

    /// Resolve priority based on chat type: DMs get Urgent, groups get Normal.
    async fn resolve_priority(&self, envelope: &Envelope) -> orka_core::Priority {
        match envelope.metadata.get("chat_type").and_then(|v| v.as_str()) {
            Some("direct") => orka_core::Priority::Urgent,
            Some("group") => orka_core::Priority::Normal,
            _ => envelope.priority,
        }
    }

    /// Process a single inbound envelope (public for testing).
    pub async fn process(&self, mut envelope: Envelope) -> orka_core::Result<()> {
        // Generate trace context if missing
        if envelope.trace_context.trace_id.is_none() {
            envelope.trace_context.trace_id = Some(uuid::Uuid::now_v7().to_string());
            envelope.trace_context.span_id =
                Some(uuid::Uuid::now_v7().simple().to_string()[..16].to_string());
            envelope.trace_context.trace_flags = Some(1);
        }

        // Idempotency check
        if self.is_duplicate(&envelope.id).await {
            debug!(message_id = %envelope.id, "duplicate message, skipping");
            self.bus.ack(&envelope.id).await?;
            return Ok(());
        }

        // Rate limiting
        let session_key = envelope.session_id.to_string();
        if !self.check_rate_limit(&session_key).await {
            warn!(
                session_id = %envelope.session_id,
                message_id = %envelope.id,
                "rate limit exceeded, dropping message"
            );
            self.bus.ack(&envelope.id).await?;
            return Ok(());
        }

        // Workspace routing: resolve priority
        envelope.priority = self.resolve_priority(&envelope).await;

        // Emit MessageReceived
        self.event_sink
            .emit(DomainEvent::new(DomainEventKind::MessageReceived {
                message_id: envelope.id,
                channel: envelope.channel.clone(),
                session_id: envelope.session_id,
            }))
            .await;

        // Session resolution: get or create
        let session = match self.sessions.get(&envelope.session_id).await? {
            Some(s) => s,
            None => {
                let mut s = Session::new(envelope.channel.clone(), resolve_user_id(&envelope));
                s.id = envelope.session_id;
                s.created_at = envelope.timestamp;
                s.updated_at = envelope.timestamp;
                self.sessions.put(&s).await?;
                info!(session_id = %s.id, "gateway: created new session");

                // Emit SessionCreated
                self.event_sink
                    .emit(DomainEvent::new(DomainEventKind::SessionCreated {
                        session_id: s.id,
                        channel: s.channel.clone(),
                    }))
                    .await;

                s
            }
        };

        // Enqueue
        self.queue.push(&envelope).await?;
        info!(
            message_id = %envelope.id,
            session_id = %session.id,
            "gateway: enqueued message"
        );

        // Ack
        self.bus.ack(&envelope.id).await?;
        Ok(())
    }
}

/// Extract a user identifier from the envelope metadata.
///
/// Tries well-known keys in priority order (human-readable names first),
/// handles both string and numeric values, falls back to "anonymous".
fn resolve_user_id(envelope: &Envelope) -> String {
    const KEYS: &[&str] = &[
        "telegram_username",
        "telegram_user_name",
        "telegram_user_id",
        "discord_username",
        "discord_user_id",
        "slack_user_id",
        "whatsapp_user_id",
        "user_id",
    ];
    for key in KEYS {
        if let Some(val) = envelope.metadata.get(*key) {
            if let Some(s) = val.as_str() {
                if !s.is_empty() {
                    return s.to_string();
                }
            } else if let Some(n) = val.as_i64() {
                return n.to_string();
            }
        }
    }
    "anonymous".to_string()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use orka_core::{
        DomainEventKind, SessionId,
        testing::{InMemoryBus, InMemoryEventSink, InMemoryQueue, InMemorySessionStore},
    };

    use super::*;

    fn test_gateway(
        rate_limit: u32,
    ) -> (
        Gateway,
        Arc<InMemoryQueue>,
        Arc<InMemoryEventSink>,
        Arc<InMemorySessionStore>,
    ) {
        let bus = Arc::new(InMemoryBus::new());
        let sessions = Arc::new(InMemorySessionStore::new());
        let queue = Arc::new(InMemoryQueue::new());
        let workspace = Arc::new(WorkspaceLoader::new("/tmp/test-workspace"));
        let event_sink = Arc::new(InMemoryEventSink::new());

        let gw = Gateway::new(
            bus,
            sessions.clone(),
            queue.clone(),
            workspace,
            event_sink.clone(),
            None, // no Redis
            rate_limit,
            60,
        );
        (gw, queue, event_sink, sessions)
    }

    #[tokio::test]
    async fn process_enqueues_message() {
        let (gw, queue, _, _) = test_gateway(0);
        let env = Envelope::text("telegram", SessionId::new(), "hello");
        gw.process(env).await.unwrap();
        assert_eq!(queue.len().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn process_creates_session_if_missing() {
        let (gw, _, _, sessions) = test_gateway(0);
        let sid = SessionId::new();
        let env = Envelope::text("telegram", sid, "hello");
        gw.process(env).await.unwrap();
        assert!(sessions.get(&sid).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn process_reuses_existing_session() {
        let (gw, _, _, sessions) = test_gateway(0);
        let sid = SessionId::new();
        let session = orka_core::Session::new("telegram", "user1");
        let mut s = session;
        s.id = sid;
        sessions.put(&s).await.unwrap();

        let env = Envelope::text("telegram", sid, "hello");
        gw.process(env).await.unwrap();

        // Should still be one session, not two
        let all = sessions.list(100).await.unwrap();
        assert_eq!(all.len(), 1);
    }

    #[tokio::test]
    async fn rate_limit_drops_excess() {
        let (gw, queue, _, _) = test_gateway(2);
        let sid = SessionId::new();

        for i in 0..3 {
            let env = Envelope::text("ch", sid, format!("msg{i}"));
            let _ = gw.process(env).await;
        }

        // Only 2 should be enqueued
        assert_eq!(queue.len().await.unwrap(), 2);
    }

    #[tokio::test]
    async fn rate_limit_zero_means_unlimited() {
        let (gw, queue, _, _) = test_gateway(0);
        let sid = SessionId::new();

        for i in 0..10 {
            let env = Envelope::text("ch", sid, format!("msg{i}"));
            gw.process(env).await.unwrap();
        }
        assert_eq!(queue.len().await.unwrap(), 10);
    }

    #[tokio::test]
    async fn resolve_priority_direct_is_urgent() {
        let (gw, queue, _, _) = test_gateway(0);
        let mut env = Envelope::text("ch", SessionId::new(), "dm");
        env.metadata
            .insert("chat_type".into(), serde_json::json!("direct"));
        gw.process(env).await.unwrap();

        let msg = queue.pop(Duration::from_millis(10)).await.unwrap().unwrap();
        assert_eq!(msg.priority, orka_core::Priority::Urgent);
    }

    #[tokio::test]
    async fn resolve_priority_group_is_normal() {
        let (gw, queue, _, _) = test_gateway(0);
        let mut env = Envelope::text("ch", SessionId::new(), "group msg");
        env.metadata
            .insert("chat_type".into(), serde_json::json!("group"));
        gw.process(env).await.unwrap();

        let msg = queue.pop(Duration::from_millis(10)).await.unwrap().unwrap();
        assert_eq!(msg.priority, orka_core::Priority::Normal);
    }

    #[tokio::test]
    async fn dedup_without_redis_accepts_all() {
        let (gw, queue, _, _) = test_gateway(0);
        let env = Envelope::text("ch", SessionId::new(), "msg");
        // Send same envelope twice — without Redis, dedup is disabled
        gw.process(env.clone()).await.unwrap();
        gw.process(env).await.unwrap();
        assert_eq!(queue.len().await.unwrap(), 2);
    }

    #[tokio::test]
    async fn event_sink_receives_message_received() {
        let (gw, _, event_sink, _) = test_gateway(0);
        let env = Envelope::text("telegram", SessionId::new(), "hello");
        gw.process(env).await.unwrap();

        let events = event_sink.events().await;
        assert!(events.iter().any(|e| matches!(
            &e.kind,
            DomainEventKind::MessageReceived { channel, .. } if channel == "telegram"
        )));
    }

    #[tokio::test]
    async fn event_sink_receives_session_created() {
        let (gw, _, event_sink, _) = test_gateway(0);
        let env = Envelope::text("telegram", SessionId::new(), "hello");
        gw.process(env).await.unwrap();

        let events = event_sink.events().await;
        assert!(
            events
                .iter()
                .any(|e| matches!(&e.kind, DomainEventKind::SessionCreated { .. }))
        );
    }

    #[tokio::test]
    async fn trace_context_generated_if_missing() {
        let (gw, queue, _, _) = test_gateway(0);
        let env = Envelope::text("ch", SessionId::new(), "hello");
        assert!(env.trace_context.trace_id.is_none());

        gw.process(env).await.unwrap();
        let msg = queue.pop(Duration::from_millis(10)).await.unwrap().unwrap();
        assert!(msg.trace_context.trace_id.is_some());
    }
}
