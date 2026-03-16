use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use deadpool_redis::Pool;
use orka_core::traits::{EventSink, MessageBus, PriorityQueue, SessionStore};
use orka_core::{DomainEvent, DomainEventKind, Envelope, EventId, Session};
use orka_workspace::WorkspaceLoader;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

const DEDUP_KEY_PREFIX: &str = "orka:dedup:";

pub struct Gateway {
    bus: Arc<dyn MessageBus>,
    sessions: Arc<dyn SessionStore>,
    queue: Arc<dyn PriorityQueue>,
    workspace: Arc<WorkspaceLoader>,
    event_sink: Arc<dyn EventSink>,
    redis_pool: Option<Pool>,
    rate_limit: u32,
    dedup_ttl_secs: u64,
    /// Tracks (count, window_start) per session for rate limiting
    rate_counters: Mutex<HashMap<String, (u32, chrono::DateTime<Utc>)>>,
}

impl Gateway {
    #[allow(clippy::too_many_arguments)]
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
            workspace,
            event_sink,
            redis_pool,
            rate_limit,
            dedup_ttl_secs,
            rate_counters: Mutex::new(HashMap::new()),
        }
    }

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
            Err(_) => return false,
        };
        let key = format!("{DEDUP_KEY_PREFIX}{message_id}");
        // SET NX EX - returns true if key was set (not duplicate), false if already exists
        let result: redis::RedisResult<bool> = redis::cmd("SET")
            .arg(&key)
            .arg("1")
            .arg("NX")
            .arg("EX")
            .arg(self.dedup_ttl_secs)
            .query_async(&mut *conn)
            .await;
        // If SET NX returns false (or errors), it's a duplicate
        !result.unwrap_or(false)
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
            if let Ok(mut conn) = pool.get().await {
                let key = format!("orka:ratelimit:{session_id}");
                let result: redis::RedisResult<i64> = redis::cmd("INCR")
                    .arg(&key)
                    .query_async(&mut *conn)
                    .await;
                if let Ok(count) = result {
                    if count == 1 {
                        // First request in window — set expiry
                        let _: redis::RedisResult<()> = redis::cmd("EXPIRE")
                            .arg(&key)
                            .arg(60i64)
                            .query_async(&mut *conn)
                            .await;
                    }
                    return count <= self.rate_limit as i64;
                }
                // Redis error — fall through to in-memory
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

        let entry = counters
            .entry(session_id.to_string())
            .or_insert((0, now));

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

    /// Read workspace state for routing hints (e.g. priority by channel).
    async fn resolve_priority(&self, envelope: &Envelope) -> orka_core::Priority {
        let state = self.workspace.state();
        let state = state.read().await;
        if let Some(ref soul) = state.soul {
            // Check if soul frontmatter has channel-specific config
            // For now, default routing: urgent for direct messages, normal for groups
            if soul.frontmatter.name.is_some() {
                // Workspace is configured — use normal priority
                return orka_core::Priority::Normal;
            }
        }
        envelope.priority
    }

    async fn process(&self, mut envelope: Envelope) -> orka_core::Result<()> {
        // Generate trace context if missing
        if envelope.trace_context.trace_id.is_none() {
            envelope.trace_context.trace_id = Some(uuid::Uuid::now_v7().to_string());
            envelope.trace_context.span_id = Some(uuid::Uuid::now_v7().simple().to_string()[..16].to_string());
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
            .emit(DomainEvent {
                id: EventId::new(),
                timestamp: Utc::now(),
                kind: DomainEventKind::MessageReceived {
                    message_id: envelope.id.clone(),
                    channel: envelope.channel.clone(),
                    session_id: envelope.session_id.clone(),
                },
                metadata: Default::default(),
            })
            .await;

        // Session resolution: get or create
        let session = match self.sessions.get(&envelope.session_id).await? {
            Some(s) => s,
            None => {
                let s = Session {
                    id: envelope.session_id.clone(),
                    channel: envelope.channel.clone(),
                    user_id: "anonymous".into(),
                    created_at: envelope.timestamp,
                    updated_at: envelope.timestamp,
                    state: Default::default(),
                };
                self.sessions.put(&s).await?;
                info!(session_id = %s.id, "gateway: created new session");

                // Emit SessionCreated
                self.event_sink
                    .emit(DomainEvent {
                        id: EventId::new(),
                        timestamp: Utc::now(),
                        kind: DomainEventKind::SessionCreated {
                            session_id: s.id.clone(),
                            channel: s.channel.clone(),
                        },
                        metadata: Default::default(),
                    })
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
