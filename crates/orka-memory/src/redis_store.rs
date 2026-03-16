use async_trait::async_trait;
use deadpool_redis::{Config as DeadpoolConfig, Pool, Runtime};
use redis::AsyncCommands;
use tracing::debug;

use orka_core::traits::MemoryStore;
use orka_core::{Error, MemoryEntry, Result};

pub struct RedisMemoryStore {
    pool: Pool,
}

impl RedisMemoryStore {
    pub fn new(redis_url: &str) -> Result<Self> {
        let cfg = DeadpoolConfig::from_url(redis_url);
        let pool = cfg
            .create_pool(Some(Runtime::Tokio1))
            .map_err(|e| Error::memory(format!("failed to create Redis pool: {e}")))?;

        Ok(Self { pool })
    }

    fn key(k: &str) -> String {
        format!("orka:memory:{k}")
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
                if let Some(json) = value {
                    if let Ok(entry) = serde_json::from_str::<MemoryEntry>(&json) {
                        results.push(entry);
                    }
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
                    if let Some(json) = value {
                        if let Ok(entry) = serde_json::from_str::<MemoryEntry>(&json) {
                            if entry.tags.iter().any(|t| t.contains(query))
                                && !results.iter().any(|r: &MemoryEntry| r.key == entry.key)
                            {
                                results.push(entry);
                            }
                        }
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
                if let Some(json) = value {
                    if let Ok(entry) = serde_json::from_str::<MemoryEntry>(&json) {
                        entries.push((key, entry.updated_at));
                    }
                }
            }

            if next_cursor == 0 {
                break;
            }
            cursor = next_cursor;
        }

        // Keep the most recent 10_000 entries, delete the rest
        const MAX_ENTRIES: usize = 10_000;
        if entries.len() <= MAX_ENTRIES {
            return Ok(0);
        }

        // Sort by updated_at descending (newest first)
        entries.sort_by(|a, b| b.1.cmp(&a.1));

        let to_delete: Vec<String> = entries[MAX_ENTRIES..]
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
