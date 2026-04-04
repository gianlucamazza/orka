use async_trait::async_trait;
use deadpool_redis::{Config as DeadpoolConfig, Pool, Runtime};
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
        let cfg = DeadpoolConfig::from_url(redis_url);
        let pool = cfg
            .create_pool(Some(Runtime::Tokio1))
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
        let mut interval =
            tokio::time::interval(std::time::Duration::from_millis(flush_interval_ms));

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if !buffer.is_empty() {
                        Self::flush(&pool, &mut buffer).await;
                    }
                }
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

        let result: redis::RedisResult<Vec<String>> = pipe.query_async(&mut *conn).await;
        match result {
            Ok(_) => debug!(count = buffer.len(), "events batch flushed to Redis stream"),
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

    #[serial_test::serial]
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "requires Redis"]
    async fn sink_flushes_single_event_to_stream() {
        let redis = RedisService::discover().await.unwrap();

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
        let mut conn = client
            .get_multiplexed_async_connection()
            .await
            .unwrap();
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

        // Large batch (100), short flush interval (100ms) — interval fires before batch is full
        let sink = RedisEventSink::new(redis.url(), 100, 100).unwrap();

        // Emit 2 events — well below batch_size, so only the interval can flush them
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
        let mut conn = client
            .get_multiplexed_async_connection()
            .await
            .unwrap();
        let len: i64 = redis::cmd("XLEN")
            .arg(STREAM_KEY)
            .query_async(&mut conn)
            .await
            .unwrap();
        assert!(len >= 2, "expected >=2 entries from interval flush, got {len}");
    }

    #[serial_test::serial]
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "requires Redis"]
    async fn sink_batches_events_and_flushes_at_batch_size() {
        let redis = RedisService::discover().await.unwrap();

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
        let mut conn = client
            .get_multiplexed_async_connection()
            .await
            .unwrap();
        let len: i64 = redis::cmd("XLEN")
            .arg(STREAM_KEY)
            .query_async(&mut conn)
            .await
            .unwrap();
        assert!(len >= 4, "expected ≥4 entries in stream, got {len}");
    }
}
