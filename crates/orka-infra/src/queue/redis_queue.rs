use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use deadpool_redis::{Config, Pool, Runtime};
use orka_core::{
    MessageId, Result,
    error::Error,
    traits::{DeadLetterQueue, PriorityQueue},
    types::{Envelope, Priority},
};
use redis::AsyncCommands;
use tracing::{debug, warn};

const PENDING_KEY: &str = "orka:queue:pending";
const DATA_KEY_PREFIX: &str = "orka:queue:data:";
const DLQ_KEY: &str = "orka:queue:dlq";

/// Redis-backed priority queue using sorted sets.
pub struct RedisPriorityQueue {
    pool: Pool,
}

impl RedisPriorityQueue {
    /// Create a new queue connected to the given Redis URL.
    pub fn new(redis_url: &str) -> Result<Self> {
        let cfg = Config::from_url(redis_url);
        let pool = cfg
            .create_pool(Some(Runtime::Tokio1))
            .map_err(|e| Error::queue(format!("failed to create Redis pool: {e}")))?;
        Ok(Self { pool })
    }
}

/// Compute the sorted-set score for a given priority and timestamp.
///
/// Lower score = higher priority. Urgent maps to bucket 0, Normal to 1,
/// Background to 2. Within the same bucket, earlier timestamps sort first
/// (FIFO).
pub fn priority_score(priority: &Priority, timestamp: DateTime<Utc>) -> f64 {
    #[allow(clippy::match_same_arms)] // canonical mapping vs. fallback for unknown variants
    let bucket: u64 = match priority {
        Priority::Urgent => 0,
        Priority::Normal => 1,
        Priority::Background => 2,
        _ => 1, // default to Normal bucket for unknown priorities
    };
    let ts_micros = timestamp.timestamp_micros() as u64;
    (bucket * 1_000_000_000_000_000 + ts_micros) as f64
}

fn data_key(message_id: &str) -> String {
    format!("{DATA_KEY_PREFIX}{message_id}")
}

#[async_trait]
impl PriorityQueue for RedisPriorityQueue {
    async fn push(&self, envelope: &Envelope) -> Result<()> {
        let mut conn = crate::retry::get_conn_with_retry(&self.pool)
            .await
            .map_err(|e| Error::queue(format!("pool error: {e}")))?;

        let id_str = envelope.id.to_string();
        let data = serde_json::to_string(envelope)?;
        let score = priority_score(&envelope.priority, envelope.timestamp);

        debug!(id = %id_str, score, "pushing envelope to queue");

        redis::pipe()
            .atomic()
            .cmd("SET")
            .arg(data_key(&id_str))
            .arg(&data)
            .cmd("ZADD")
            .arg(PENDING_KEY)
            .arg(score)
            .arg(&id_str)
            .exec_async(&mut *conn)
            .await
            .map_err(|e| Error::queue(format!("push failed: {e}")))?;

        Ok(())
    }

    async fn pop(&self, timeout: Duration) -> Result<Option<Envelope>> {
        let mut conn = crate::retry::get_conn_with_retry(&self.pool)
            .await
            .map_err(|e| Error::queue(format!("pool error: {e}")))?;

        let timeout_secs = timeout.as_secs().max(1);

        // Use BZPOPMIN to wait, then atomic Lua for pop+get+del
        let result: Option<(String, String, f64)> = redis::cmd("BZPOPMIN")
            .arg(PENDING_KEY)
            .arg(timeout_secs)
            .query_async(&mut *conn)
            .await
            .map_err(|e| Error::queue(format!("pop failed: {e}")))?;

        let Some((_key, member, score)) = result else {
            return Ok(None);
        };

        // Fetch and delete data key atomically via pipeline
        // (BZPOPMIN already removed from ZSET, so we just need GET+DEL atomically)
        let dkey = data_key(&member);
        let data: Option<String> = redis::Script::new(
            r"
            local data = redis.call('GET', KEYS[1])
            if data then
                redis.call('DEL', KEYS[1])
                return data
            end
            return nil
            ",
        )
        .key(&dkey)
        .invoke_async(&mut *conn)
        .await
        .map_err(|e| Error::queue(format!("atomic get+del failed: {e}")))?;

        if let Some(json) = data {
            let envelope: Envelope = serde_json::from_str(&json)?;

            // Check not_before: if message is not yet mature, re-enqueue it
            if let Some(not_before_val) = envelope.metadata.get("not_before")
                && let Some(not_before_str) = not_before_val.as_str()
                && let Ok(not_before) = chrono::DateTime::parse_from_rfc3339(not_before_str)
                && Utc::now() < not_before
            {
                debug!(id = %member, "message not yet mature, re-enqueuing");
                // Re-add to sorted set with original score and restore data
                redis::pipe()
                    .atomic()
                    .cmd("SET")
                    .arg(&dkey)
                    .arg(&json)
                    .cmd("ZADD")
                    .arg(PENDING_KEY)
                    .arg(score)
                    .arg(&member)
                    .exec_async(&mut *conn)
                    .await
                    .map_err(|e| Error::queue(format!("re-enqueue not_before failed: {e}")))?;
                return Ok(None);
            }

            debug!(id = %member, "popped envelope from queue");
            Ok(Some(envelope))
        } else {
            warn!(id = %member, "envelope data missing for popped member");
            Ok(None)
        }
    }

    async fn len(&self) -> Result<usize> {
        let mut conn = crate::retry::get_conn_with_retry(&self.pool)
            .await
            .map_err(|e| Error::queue(format!("pool error: {e}")))?;

        let count: usize = conn
            .zcard(PENDING_KEY)
            .await
            .map_err(|e| Error::queue(format!("zcard failed: {e}")))?;

        Ok(count)
    }
}

#[async_trait]
impl DeadLetterQueue for RedisPriorityQueue {
    async fn push(&self, envelope: &Envelope) -> Result<()> {
        let mut conn = crate::retry::get_conn_with_retry(&self.pool)
            .await
            .map_err(|e| Error::queue(format!("pool error: {e}")))?;

        let data = serde_json::to_string(envelope)?;
        let score = chrono::Utc::now().timestamp_micros() as f64;

        redis::cmd("ZADD")
            .arg(DLQ_KEY)
            .arg(score)
            .arg(&data)
            .query_async::<i64>(&mut *conn)
            .await
            .map_err(|e| Error::queue(format!("DLQ push failed: {e}")))?;

        Ok(())
    }

    async fn list(&self) -> Result<Vec<Envelope>> {
        let mut conn = crate::retry::get_conn_with_retry(&self.pool)
            .await
            .map_err(|e| Error::queue(format!("pool error: {e}")))?;

        let items: Vec<String> = redis::cmd("ZRANGE")
            .arg(DLQ_KEY)
            .arg(0i64)
            .arg(-1i64)
            .query_async(&mut *conn)
            .await
            .map_err(|e| Error::queue(format!("DLQ list failed: {e}")))?;

        let mut envelopes = Vec::with_capacity(items.len());
        for json in items {
            let envelope: Envelope = serde_json::from_str(&json)?;
            envelopes.push(envelope);
        }
        Ok(envelopes)
    }

    async fn purge(&self) -> Result<usize> {
        let mut conn = crate::retry::get_conn_with_retry(&self.pool)
            .await
            .map_err(|e| Error::queue(format!("pool error: {e}")))?;

        let count: usize = conn
            .zcard(DLQ_KEY)
            .await
            .map_err(|e| Error::queue(format!("DLQ zcard failed: {e}")))?;

        let _: () = conn
            .del(DLQ_KEY)
            .await
            .map_err(|e| Error::queue(format!("DLQ purge failed: {e}")))?;

        Ok(count)
    }

    async fn replay(&self, id: &MessageId) -> Result<bool> {
        let mut conn = crate::retry::get_conn_with_retry(&self.pool)
            .await
            .map_err(|e| Error::queue(format!("pool error: {e}")))?;

        let items: Vec<String> = redis::cmd("ZRANGE")
            .arg(DLQ_KEY)
            .arg(0i64)
            .arg(-1i64)
            .query_async(&mut *conn)
            .await
            .map_err(|e| Error::queue(format!("DLQ list failed: {e}")))?;

        for json in &items {
            let mut envelope: Envelope = serde_json::from_str(json)?;
            if &envelope.id == id {
                // Remove from DLQ
                let _: i64 = redis::cmd("ZREM")
                    .arg(DLQ_KEY)
                    .arg(json.as_str())
                    .query_async(&mut *conn)
                    .await
                    .map_err(|e| Error::queue(format!("DLQ zrem failed: {e}")))?;

                // Reset retry state and re-enqueue
                envelope.metadata.remove("retry_count");
                envelope.priority = Priority::Normal;
                PriorityQueue::push(self, &envelope).await?;
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    #[test]
    fn priority_score_ordering() {
        let t = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();

        let urgent = priority_score(&Priority::Urgent, t);
        let normal = priority_score(&Priority::Normal, t);
        let background = priority_score(&Priority::Background, t);

        assert!(
            urgent < normal,
            "Urgent should have lower score than Normal"
        );
        assert!(
            normal < background,
            "Normal should have lower score than Background"
        );
    }

    #[test]
    fn priority_score_fifo_within_same_priority() {
        let t1 = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 1).unwrap();

        let s1 = priority_score(&Priority::Normal, t1);
        let s2 = priority_score(&Priority::Normal, t2);

        assert!(s1 < s2, "Earlier timestamp should have lower score (FIFO)");
    }

    #[test]
    fn urgent_strictly_below_background_at_same_time() {
        let t = Utc.with_ymd_and_hms(2025, 6, 1, 12, 0, 0).unwrap();
        let urgent = priority_score(&Priority::Urgent, t);
        let background = priority_score(&Priority::Background, t);
        // Two full buckets apart — a large gap, not just "slightly less"
        assert!(
            urgent < background,
            "urgent={urgent} should be much less than background={background}"
        );
    }

    #[test]
    fn same_priority_same_time_gives_same_score() {
        let t = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
        let s1 = priority_score(&Priority::Urgent, t);
        let s2 = priority_score(&Priority::Urgent, t);
        assert!(
            (s1 - s2).abs() < f64::EPSILON,
            "same priority+time must yield identical score"
        );
    }

    #[test]
    fn urgent_later_still_beats_normal_earlier() {
        let t_early = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
        let t_late = Utc.with_ymd_and_hms(2025, 12, 31, 23, 59, 59).unwrap();
        let normal_early = priority_score(&Priority::Normal, t_early);
        let urgent_late = priority_score(&Priority::Urgent, t_late);
        assert!(
            urgent_late < normal_early,
            "Urgent message late in year must still outrank Normal message early in year"
        );
    }

    #[test]
    fn fifo_ordering_holds_for_urgent() {
        let t1 = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 1).unwrap();
        let s1 = priority_score(&Priority::Urgent, t1);
        let s2 = priority_score(&Priority::Urgent, t2);
        assert!(s1 < s2, "Earlier urgent message must rank higher");
    }
}
