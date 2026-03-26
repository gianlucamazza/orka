use async_trait::async_trait;
use deadpool_redis::{Config as DeadpoolConfig, Pool, Runtime};
use orka_core::{
    Error, MemoryEntry, Result,
    traits::{MemoryStore, SessionLock},
};
use redis::AsyncCommands;
use tracing::{debug, warn};

/// Redis implementation of [`orka_core::traits::MemoryStore`].
pub struct RedisMemoryStore {
    pool: Pool,
    max_entries: usize,
}

impl RedisMemoryStore {
    /// Connect to Redis and create a new memory store with the given capacity.
    pub fn new(redis_url: &str, max_entries: usize) -> Result<Self> {
        let cfg = DeadpoolConfig::from_url(redis_url);
        let pool = cfg
            .create_pool(Some(Runtime::Tokio1))
            .map_err(|e| Error::memory(format!("failed to create Redis pool: {e}")))?;

        Ok(Self { pool, max_entries })
    }

    fn key(k: &str) -> String {
        format!("orka:memory:{k}")
    }

    fn lock_key(session_id: &str) -> String {
        format!("orka:lock:session:{session_id}")
    }
}

#[async_trait]
impl MemoryStore for RedisMemoryStore {
    async fn store(
        &self,
        key: &str,
        value: MemoryEntry,
        ttl: Option<std::time::Duration>,
    ) -> Result<()> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| Error::memory(format!("redis pool error: {e}")))?;

        let json = serde_json::to_string(&value)?;
        let redis_key = Self::key(key);

        if let Some(ttl) = ttl {
            let ms = ttl.as_millis() as i64;
            redis::pipe()
                .set(&redis_key, &json)
                .pexpire(&redis_key, ms)
                .query_async::<()>(&mut *conn)
                .await
                .map_err(|e| Error::memory(format!("redis SET+PEXPIRE error: {e}")))?;
        } else {
            let _: () = conn
                .set(redis_key, json)
                .await
                .map_err(|e| Error::memory(format!("redis SET error: {e}")))?;
        }

        debug!(key, "memory entry stored");
        Ok(())
    }

    async fn recall(&self, key: &str) -> Result<Option<MemoryEntry>> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| Error::memory(format!("redis pool error: {e}")))?;

        let value: Option<String> = conn
            .get(Self::key(key))
            .await
            .map_err(|e| Error::memory(format!("redis GET error: {e}")))?;

        match value {
            Some(json) => {
                let entry: MemoryEntry = serde_json::from_str(&json)?;
                debug!(key, "memory entry recalled");
                Ok(Some(entry))
            }
            None => Ok(None),
        }
    }

    async fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryEntry>> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| Error::memory(format!("redis pool error: {e}")))?;

        let pattern = format!("orka:memory:*{query}*");
        let mut results = Vec::new();
        let mut cursor: u64 = 0;

        loop {
            let (next_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(&pattern)
                .arg("COUNT")
                .arg(100)
                .query_async(&mut *conn)
                .await
                .map_err(|e| Error::memory(format!("redis SCAN error: {e}")))?;

            for key in keys {
                if results.len() >= limit {
                    break;
                }
                let value: Option<String> = conn
                    .get(&key)
                    .await
                    .map_err(|e| Error::memory(format!("redis GET error: {e}")))?;
                if let Some(json) = value
                    && let Ok(entry) = serde_json::from_str::<MemoryEntry>(&json)
                {
                    results.push(entry);
                }
            }

            if results.len() >= limit || next_cursor == 0 {
                break;
            }
            cursor = next_cursor;
        }

        // Also do a full scan for tag matches not caught by key pattern
        if results.len() < limit {
            cursor = 0;
            loop {
                let (next_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                    .arg(cursor)
                    .arg("MATCH")
                    .arg("orka:memory:*")
                    .arg("COUNT")
                    .arg(100)
                    .query_async(&mut *conn)
                    .await
                    .map_err(|e| Error::memory(format!("redis SCAN error: {e}")))?;

                for key in keys {
                    if results.len() >= limit {
                        break;
                    }
                    let value: Option<String> = conn
                        .get(&key)
                        .await
                        .map_err(|e| Error::memory(format!("redis GET error: {e}")))?;
                    if let Some(json) = value
                        && let Ok(entry) = serde_json::from_str::<MemoryEntry>(&json)
                        && entry.tags.iter().any(|t| t.contains(query))
                        && !results.iter().any(|r: &MemoryEntry| r.key == entry.key)
                    {
                        results.push(entry);
                    }
                }

                if results.len() >= limit || next_cursor == 0 {
                    break;
                }
                cursor = next_cursor;
            }
        }

        debug!(query, count = results.len(), "memory search completed");
        Ok(results)
    }

    async fn compact(&self) -> Result<usize> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| Error::memory(format!("redis pool error: {e}")))?;

        // Collect all memory keys with their updated_at timestamps
        let mut entries: Vec<(String, chrono::DateTime<chrono::Utc>)> = Vec::new();
        let mut cursor: u64 = 0;
        loop {
            let (next_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg("orka:memory:*")
                .arg("COUNT")
                .arg(200)
                .query_async(&mut *conn)
                .await
                .map_err(|e| Error::memory(format!("redis SCAN error: {e}")))?;

            for key in keys {
                let value: Option<String> = conn
                    .get(&key)
                    .await
                    .map_err(|e| Error::memory(format!("redis GET error: {e}")))?;
                if let Some(json) = value
                    && let Ok(entry) = serde_json::from_str::<MemoryEntry>(&json)
                {
                    entries.push((key, entry.updated_at));
                }
            }

            if next_cursor == 0 {
                break;
            }
            cursor = next_cursor;
        }

        // Keep the most recent N entries, delete the rest
        if entries.len() <= self.max_entries {
            return Ok(0);
        }

        // Sort by updated_at descending (newest first)
        entries.sort_by(|a, b| b.1.cmp(&a.1));

        let to_delete: Vec<String> = entries[self.max_entries..]
            .iter()
            .map(|(k, _)| k.clone())
            .collect();
        let count = to_delete.len();

        for key in &to_delete {
            let _: () = conn
                .del(key)
                .await
                .map_err(|e| Error::memory(format!("redis DEL error: {e}")))?;
        }

        debug!(removed = count, "memory compact completed");
        Ok(count)
    }
}

#[async_trait]
impl SessionLock for RedisMemoryStore {
    async fn try_acquire(&self, session_id: &str, ttl_ms: u64) -> bool {
        let key = Self::lock_key(session_id);
        match self.pool.get().await {
            Ok(mut conn) => {
                // Atomic SET key 1 NX PX ttl_ms — returns "OK" on success, nil if already held
                let result: Option<String> = redis::cmd("SET")
                    .arg(&key)
                    .arg("1")
                    .arg("NX")
                    .arg("PX")
                    .arg(ttl_ms)
                    .query_async(&mut *conn)
                    .await
                    .unwrap_or(None);
                result.is_some()
            }
            Err(e) => {
                // Fail-open: if Redis is unavailable, allow processing rather than stalling all
                // workers
                warn!(%e, session_id, "redis conn failed for session lock; proceeding without lock");
                true
            }
        }
    }

    async fn release(&self, session_id: &str) {
        let key = Self::lock_key(session_id);
        match self.pool.get().await {
            Ok(mut conn) => {
                let _: core::result::Result<i64, _> = conn.del(&key).await;
            }
            Err(e) => {
                warn!(%e, session_id, "redis conn failed for session lock release");
            }
        }
    }

    async fn extend(&self, session_id: &str, ttl_ms: u64) -> bool {
        let key = Self::lock_key(session_id);
        match self.pool.get().await {
            Ok(mut conn) => {
                // PEXPIRE returns 1 if the key exists and was updated, 0 if not
                let result: core::result::Result<i64, _> =
                    redis::cmd("PEXPIRE").arg(&key).arg(ttl_ms).query_async(&mut *conn).await;
                result.unwrap_or(0) == 1
            }
            Err(e) => {
                warn!(%e, session_id, "redis conn failed for session lock extend");
                false
            }
        }
    }
}
