use async_trait::async_trait;
use deadpool_redis::{Config as DeadpoolConfig, Pool, Runtime};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};
use tracing::{debug, error, warn};

use orka_core::traits::MessageBus;
use orka_core::{Envelope, Error, MessageId, MessageStream, Result};

/// Redis Streams implementation of [`orka_core::traits::MessageBus`].
pub struct RedisBus {
    pool: Pool,
    group: String,
    consumer: String,
    pending: Arc<Mutex<HashMap<String, (String, String)>>>, // message_id_str -> (stream_key, redis_entry_id)
    block_ms: u64,
    batch_size: usize,
    backoff_initial_secs: u64,
    backoff_max_secs: u64,
}

impl RedisBus {
    /// Connect to Redis and create a new bus with the given configuration.
    pub fn new(redis_url: &str, config: &orka_core::config::BusConfig) -> Result<Self> {
        let cfg = DeadpoolConfig::from_url(redis_url);
        let pool = cfg
            .create_pool(Some(Runtime::Tokio1))
            .map_err(|e| Error::bus(format!("failed to create Redis pool: {e}")))?;

        let consumer = format!("orka-{}", uuid::Uuid::now_v7());
        Ok(Self {
            pool,
            group: "orka".to_string(),
            consumer,
            pending: Arc::new(Mutex::new(HashMap::new())),
            block_ms: config.block_ms,
            batch_size: config.batch_size,
            backoff_initial_secs: config.backoff_initial_secs,
            backoff_max_secs: config.backoff_max_secs,
        })
    }

    /// Override the consumer group name (default: `"orka"`).
    pub fn with_group(mut self, group: impl Into<String>) -> Self {
        self.group = group.into();
        self
    }

    /// Return the Redis key used for a given topic's stream.
    pub fn stream_key(topic: &str) -> String {
        format!("orka:bus:{topic}")
    }
}

#[async_trait]
impl MessageBus for RedisBus {
    async fn publish(&self, topic: &str, msg: &Envelope) -> Result<()> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| Error::bus(format!("pool error: {e}")))?;
        let key = Self::stream_key(topic);
        let payload = serde_json::to_string(msg)?;

        redis::cmd("XADD")
            .arg(&key)
            .arg("*")
            .arg("envelope")
            .arg(&payload)
            .query_async::<String>(&mut conn)
            .await
            .map_err(|e| Error::bus(format!("XADD failed: {e}")))?;

        debug!(topic, id = %msg.id, "published envelope");
        Ok(())
    }

    async fn subscribe(&self, topic: &str) -> Result<MessageStream> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| Error::bus(format!("pool error: {e}")))?;
        let key = Self::stream_key(topic);
        let group = self.group.clone();
        let consumer = self.consumer.clone();
        let pending = self.pending.clone();

        // Create consumer group, ignore BUSYGROUP error
        let result: redis::RedisResult<()> = redis::cmd("XGROUP")
            .arg("CREATE")
            .arg(&key)
            .arg(&group)
            .arg("0")
            .arg("MKSTREAM")
            .query_async(&mut conn)
            .await;

        match result {
            Ok(()) => {}
            Err(e) if e.to_string().contains("BUSYGROUP") => {}
            Err(e) => return Err(Error::bus(format!("XGROUP CREATE failed: {e}"))),
        }

        let (tx, rx) = mpsc::channel::<Envelope>(256);
        let pool = self.pool.clone();
        let block_ms = self.block_ms;
        let batch_size = self.batch_size;
        let initial_backoff = std::time::Duration::from_secs(self.backoff_initial_secs);
        let max_backoff = std::time::Duration::from_secs(self.backoff_max_secs);

        tokio::spawn(async move {
            let mut backoff = initial_backoff;
            loop {
                let mut conn = match pool.get().await {
                    Ok(c) => c,
                    Err(e) => {
                        error!(error = %e, "failed to get Redis connection");
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff * 2).min(max_backoff);
                        continue;
                    }
                };

                let result: redis::RedisResult<redis::Value> = redis::cmd("XREADGROUP")
                    .arg("GROUP")
                    .arg(&group)
                    .arg(&consumer)
                    .arg("BLOCK")
                    .arg(block_ms)
                    .arg("COUNT")
                    .arg(batch_size)
                    .arg("STREAMS")
                    .arg(&key)
                    .arg(">")
                    .query_async(&mut conn)
                    .await;

                match result {
                    Ok(redis::Value::Nil) => continue,
                    Ok(value) => {
                        if let Some(entries) = parse_xreadgroup_response(&value) {
                            backoff = initial_backoff;
                            for (entry_id, envelope_json) in entries {
                                match serde_json::from_str::<Envelope>(&envelope_json) {
                                    Ok(envelope) => {
                                        let msg_id = envelope.id.to_string();
                                        pending
                                            .lock()
                                            .await
                                            .insert(msg_id, (key.clone(), entry_id));
                                        if tx.send(envelope).await.is_err() {
                                            debug!("subscriber dropped, stopping reader");
                                            return;
                                        }
                                    }
                                    Err(e) => {
                                        warn!(error = %e, entry_id = %entry_id, "failed to deserialize envelope, routing to DLQ");
                                        // XACK to remove from the PEL so the entry is not
                                        // redelivered indefinitely, then XADD raw payload to
                                        // the DLQ stream for offline inspection.
                                        let pool_dlq = pool.clone();
                                        let key_dlq = key.clone();
                                        let group_dlq = group.clone();
                                        let entry_id_dlq = entry_id.clone();
                                        let raw_dlq = envelope_json.clone();
                                        let err_dlq = e.to_string();
                                        tokio::spawn(async move {
                                            let mut conn = match pool_dlq.get().await {
                                                Ok(c) => c,
                                                Err(e) => {
                                                    error!(error = %e, "failed to get connection for DLQ routing");
                                                    return;
                                                }
                                            };
                                            let _ = redis::cmd("XACK")
                                                .arg(&key_dlq)
                                                .arg(&group_dlq)
                                                .arg(&entry_id_dlq)
                                                .query_async::<i64>(&mut conn)
                                                .await;
                                            let _ = redis::cmd("XADD")
                                                .arg("orka:bus:dlq")
                                                .arg("*")
                                                .arg("raw")
                                                .arg(&raw_dlq)
                                                .arg("error")
                                                .arg(&err_dlq)
                                                .arg("source_stream")
                                                .arg(&key_dlq)
                                                .query_async::<String>(&mut conn)
                                                .await;
                                        });
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!(error = %e, "XREADGROUP failed");
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff * 2).min(max_backoff);
                    }
                }
            }
        });

        Ok(rx)
    }

    async fn ack(&self, id: &MessageId) -> Result<()> {
        let id_str = id.to_string();
        let (stream_key, entry_id) = {
            let mut pending = self.pending.lock().await;
            pending
                .remove(&id_str)
                .ok_or_else(|| Error::bus(format!("unknown message ID: {id_str}")))?
        };

        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| Error::bus(format!("pool error: {e}")))?;

        redis::cmd("XACK")
            .arg(&stream_key)
            .arg(&self.group)
            .arg(&entry_id)
            .query_async::<i64>(&mut conn)
            .await
            .map_err(|e| Error::bus(format!("XACK failed: {e}")))?;

        debug!(message_id = %id_str, "acknowledged message");
        Ok(())
    }
}

/// Extract `(entry_id, envelope_json)` pairs from an XREADGROUP response.
///
/// The Redis response format is:
/// ```text
/// Array([
///     Array([
///         BulkString(stream_key),
///         Array([
///             Array([
///                 BulkString(entry_id),
///                 Array([
///                     BulkString("envelope"),
///                     BulkString(json_data),
///                 ])
///             ])
///         ])
///     ])
/// ])
/// ```
fn parse_xreadgroup_response(value: &redis::Value) -> Option<Vec<(String, String)>> {
    let mut results = Vec::new();

    // Top-level array: list of streams
    let streams = match value {
        redis::Value::Array(arr) => arr,
        _ => return None,
    };

    for stream in streams {
        // Each stream: [stream_key, entries_array]
        let stream_parts = match stream {
            redis::Value::Array(arr) if arr.len() >= 2 => arr,
            _ => continue,
        };

        // stream_parts[1] is the entries array
        let entries = match &stream_parts[1] {
            redis::Value::Array(arr) => arr,
            _ => continue,
        };

        for entry in entries {
            // Each entry: [entry_id, fields_array]
            let entry_parts = match entry {
                redis::Value::Array(arr) if arr.len() >= 2 => arr,
                _ => continue,
            };

            let entry_id = value_to_string(&entry_parts[0])?;

            // fields_array: [field_name, field_value, ...]
            let fields = match &entry_parts[1] {
                redis::Value::Array(arr) => arr,
                _ => continue,
            };

            // Look for "envelope" key followed by its value
            let mut i = 0;
            while i + 1 < fields.len() {
                if let Some(field_name) = value_to_string(&fields[i])
                    && field_name == "envelope"
                {
                    if let Some(field_value) = value_to_string(&fields[i + 1]) {
                        results.push((entry_id.clone(), field_value));
                    }
                    break;
                }
                i += 2;
            }
        }
    }

    if results.is_empty() {
        None
    } else {
        Some(results)
    }
}

/// Convert a Redis Value to a String, handling both BulkString and SimpleString variants.
fn value_to_string(value: &redis::Value) -> Option<String> {
    match value {
        redis::Value::BulkString(bytes) => String::from_utf8(bytes.clone()).ok(),
        redis::Value::SimpleString(s) => Some(s.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_key_format() {
        assert_eq!(RedisBus::stream_key("events"), "orka:bus:events");
    }

    #[test]
    fn stream_key_empty_topic() {
        assert_eq!(RedisBus::stream_key(""), "orka:bus:");
    }

    #[test]
    fn value_to_string_bulk_string() {
        let v = redis::Value::BulkString(b"hello".to_vec());
        assert_eq!(value_to_string(&v), Some("hello".into()));
    }

    #[test]
    fn value_to_string_simple_string() {
        let v = redis::Value::SimpleString("ok".into());
        assert_eq!(value_to_string(&v), Some("ok".into()));
    }

    #[test]
    fn value_to_string_non_string_returns_none() {
        let v = redis::Value::Int(42);
        assert_eq!(value_to_string(&v), None);
    }

    /// Build a minimal XREADGROUP response with one stream, one entry.
    fn xread_response(entry_id: &str, field: &str, value: &str) -> redis::Value {
        redis::Value::Array(vec![redis::Value::Array(vec![
            redis::Value::BulkString(b"orka:bus:test".to_vec()),
            redis::Value::Array(vec![redis::Value::Array(vec![
                redis::Value::BulkString(entry_id.as_bytes().to_vec()),
                redis::Value::Array(vec![
                    redis::Value::BulkString(field.as_bytes().to_vec()),
                    redis::Value::BulkString(value.as_bytes().to_vec()),
                ]),
            ])]),
        ])])
    }

    #[test]
    fn parse_xreadgroup_valid_single_entry() {
        let resp = xread_response("1-0", "envelope", r#"{"id":"msg1"}"#);
        let result = parse_xreadgroup_response(&resp).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "1-0");
        assert_eq!(result[0].1, r#"{"id":"msg1"}"#);
    }

    #[test]
    fn parse_xreadgroup_empty_array_returns_none() {
        let resp = redis::Value::Array(vec![]);
        assert!(parse_xreadgroup_response(&resp).is_none());
    }

    #[test]
    fn parse_xreadgroup_missing_envelope_field() {
        let resp = xread_response("1-0", "other_field", "data");
        assert!(parse_xreadgroup_response(&resp).is_none());
    }
}
