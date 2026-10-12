use async_trait::async_trait;
use deadpool_redis::{Pool, Runtime};
use orka_core::{DomainEvent, traits::EventSink};
use tokio::sync::mpsc;
use tracing::{debug, error, info};

const STREAM_KEY: &str = "orka:events";

/// Event sink that publishes domain events to a Redis Stream (`orka:events`).
///
/// Events are buffered in memory and flushed to Redis in batches, either when
/// the buffer reaches `batch_size` or after `flush_interval_ms` milliseconds,
/// whichever comes first.
pub struct RedisEventSink {
    tx: mpsc::Sender<DomainEvent>,
}

impl RedisEventSink {
    /// Create a new [`RedisEventSink`] connected to the given Redis URL.
    ///
    /// Spawns a background task that drains the event channel and writes
    /// batches to the `orka:events` Redis Stream.
    pub fn new(
        redis_url: &str,
        batch_size: usize,
        flush_interval_ms: u64,
    ) -> orka_core::Result<Self> {
        // redis 1.x defaults response_timeout to 500ms; disable it so flushes
        // are not aborted by the client while XADD is in flight.
        let conn_config = redis::AsyncConnectionConfig::new().set_response_timeout(None);
        let manager = deadpool_redis::Manager::new_with_config(redis_url, conn_config)
            .map_err(|e| orka_core::Error::observe(e, "failed to create Redis manager"))?;
        let pool = Pool::builder(manager)
            .runtime(Runtime::Tokio1)
            .build()
            .map_err(|e| orka_core::Error::observe(e, "failed to create Redis pool"))?;

        let (tx, rx) = mpsc::channel(1024);

        tokio::spawn(Self::batch_loop(pool, rx, batch_size, flush_interval_ms));

        Ok(Self { tx })
    }

    async fn batch_loop(
        pool: Pool,
        mut rx: mpsc::Receiver<DomainEvent>,
        batch_size: usize,
        flush_interval_ms: u64,
    ) {
        let mut buffer: Vec<String> = Vec::with_capacity(batch_size);
        // Skip the immediate first tick so a just-started sink does not flush a
        // partial batch when `select!` races interval against `recv`.
        let period = std::time::Duration::from_millis(flush_interval_ms.max(1));
        let mut interval = tokio::time::interval_at(tokio::time::Instant::now() + period, period);

        loop {
            tokio::select! {
                biased;
                event = rx.recv() => {
                    if let Some(event) = event {
                        match serde_json::to_string(&event) {
                            Ok(json) => {
                                buffer.push(json);
                                if buffer.len() >= batch_size {
                                    Self::flush(&pool, &mut buffer).await;
                                }
                            }
                            Err(e) => error!(%e, "failed to serialize event"),
                        }
                    } else {
                        // Channel closed, flush remaining and exit
                        if !buffer.is_empty() {
                            Self::flush(&pool, &mut buffer).await;
                        }
                        info!("event sink batch loop stopped");
                        break;
                    }
                }
                _ = interval.tick() => {
                    if !buffer.is_empty() {
                        Self::flush(&pool, &mut buffer).await;
                    }
                }
            }
        }
    }

    async fn flush(pool: &Pool, buffer: &mut Vec<String>) {
        let mut conn = match pool.get().await {
            Ok(c) => c,
            Err(e) => {
                error!(%e, count = buffer.len(), "failed to get Redis connection for event batch");
                return;
            }
        };

        let mut pipe = redis::pipe();
        for json in buffer.iter() {
            pipe.cmd("XADD")
                .arg(STREAM_KEY)
                .arg("*")
                .arg("event")
                .arg(json);
        }

        match pipe.exec_async(&mut *conn).await {
            Ok(()) => debug!(count = buffer.len(), "events batch flushed to Redis stream"),
            Err(e) => error!(%e, count = buffer.len(), "failed to flush events batch"),
        }

        buffer.clear();
    }
}

#[async_trait]
impl EventSink for RedisEventSink {
    async fn emit(&self, event: DomainEvent) {
        // All event kinds are serialized uniformly via serde; no per-variant
        // handling is needed here. The batch_loop serializes to JSON.
        if self.tx.send(event).await.is_err() {
            error!("event sink channel closed");
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::time::Duration;

    use orka_core::{DomainEvent, DomainEventKind, MessageId, SessionId, traits::EventSink};
    use orka_test_support::RedisService;
    use tokio::time::sleep;

    use super::*;

    async fn clear_event_stream(redis_url: &str) {
        let client = redis::Client::open(redis_url).unwrap();
        let mut conn = client
            .get_multiplexed_async_connection_with_config(
                &redis::AsyncConnectionConfig::new().set_response_timeout(None),
            )
            .await
            .unwrap();
        let _: () = redis::cmd("DEL")
            .arg(STREAM_KEY)
            .query_async(&mut conn)
            .await
            .unwrap();
    }

    #[serial_test::serial]
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "requires Redis"]
    async fn sink_flushes_single_event_to_stream() {
        let redis = RedisService::discover().await.unwrap();
        clear_event_stream(redis.url()).await;

        // batch_size=1 flushes after every event
        let sink = RedisEventSink::new(redis.url(), 1, 5_000).unwrap();

        let event = DomainEvent::new(DomainEventKind::MessageReceived {
            message_id: MessageId::new(),
            channel: "test-channel".to_string(),
            session_id: SessionId::new(),
        });
        sink.emit(event).await;

        // Allow background task to flush
        sleep(Duration::from_millis(200)).await;

        let client = redis::Client::open(redis.url()).unwrap();
        let mut conn = client.get_multiplexed_async_connection().await.unwrap();
        let len: i64 = redis::cmd("XLEN")
            .arg(STREAM_KEY)
            .query_async(&mut conn)
            .await
            .unwrap();
        assert!(len >= 1, "expected ≥1 entry in stream, got {len}");
    }

    #[serial_test::serial]
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "requires Redis"]
    async fn sink_flushes_on_interval_when_below_batch_size() {
        let redis = RedisService::discover().await.unwrap();
        clear_event_stream(redis.url()).await;

        // Large batch (100), short flush interval (100ms) — interval fires
        // before batch is full
        let sink = RedisEventSink::new(redis.url(), 100, 100).unwrap();

        // Emit 2 events — well below batch_size, so only the interval can flush
        // them
        for _ in 0..2u32 {
            sink.emit(DomainEvent::new(DomainEventKind::MessageReceived {
                message_id: MessageId::new(),
                channel: "interval-test".to_string(),
                session_id: SessionId::new(),
            }))
            .await;
        }

        // Wait long enough for the 100ms interval to tick
        sleep(Duration::from_millis(350)).await;

        let client = redis::Client::open(redis.url()).unwrap();
        let mut conn = client.get_multiplexed_async_connection().await.unwrap();
        let len: i64 = redis::cmd("XLEN")
            .arg(STREAM_KEY)
            .query_async(&mut conn)
            .await
            .unwrap();
        assert!(
            len >= 2,
            "expected >=2 entries from interval flush, got {len}"
        );
    }

    #[serial_test::serial]
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "requires Redis"]
    async fn sink_batches_events_and_flushes_at_batch_size() {
        let redis = RedisService::discover().await.unwrap();
        clear_event_stream(redis.url()).await;

        // batch_size=4, long flush interval: only flush when 4 events arrive
        let sink = RedisEventSink::new(redis.url(), 4, 60_000).unwrap();

        let mid = MessageId::new();
        for i in 0..4u32 {
            sink.emit(DomainEvent::new(DomainEventKind::SkillInvoked {
                skill_name: format!("skill_{i}"),
                message_id: mid,
                input_args: std::collections::HashMap::new(),
                caller_id: None,
            }))
            .await;
        }

        sleep(Duration::from_millis(300)).await;

        let client = redis::Client::open(redis.url()).unwrap();
        let mut conn = client.get_multiplexed_async_connection().await.unwrap();
        let len: i64 = redis::cmd("XLEN")
            .arg(STREAM_KEY)
            .query_async(&mut conn)
            .await
            .unwrap();
        assert!(len >= 4, "expected ≥4 entries in stream, got {len}");
    }
}
