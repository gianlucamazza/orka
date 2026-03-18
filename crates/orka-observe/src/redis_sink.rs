use async_trait::async_trait;
use deadpool_redis::{Config as DeadpoolConfig, Pool, Runtime};
use orka_core::DomainEvent;
use orka_core::traits::EventSink;
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
            .map_err(|e| orka_core::Error::Observe(format!("failed to create Redis pool: {e}")))?;

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
                    match event {
                        Some(event) => {
                            match serde_json::to_string(&event) {
                                Ok(json) => {
                                    buffer.push(json);
                                    if buffer.len() >= batch_size {
                                        Self::flush(&pool, &mut buffer).await;
                                    }
                                }
                                Err(e) => error!(%e, "failed to serialize event"),
                            }
                        }
                        None => {
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
