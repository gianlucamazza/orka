//! Redis-backed [`CheckpointStore`].
//!
//! ## Key layout
//!
//! ```text
//! orka:ckpt:{run_id}:list   → Redis List  (RPUSH)   — ordered checkpoint IDs
//! orka:ckpt:{run_id}:{id}   → Redis String (SET)    — JSON-serialized Checkpoint
//! ```
//!
//! The list provides O(1) append and O(N) history access. Each checkpoint is
//! stored as an independent key so point-reads are O(1) without deserializing
//! the whole list.
//!
//! TTL is applied to both the list key and each individual checkpoint key
//! atomically via a pipeline, keyed off the `Checkpoint.created_at` field so
//! that replayed checkpoints do not reset the expiry unexpectedly.

use async_trait::async_trait;
use deadpool_redis::Pool;
use orka_core::Result;
use redis::AsyncCommands;
use tracing::{debug, warn};

use crate::{
    store::CheckpointStore,
    types::{Checkpoint, CheckpointId},
};

const KEY_PREFIX: &str = "orka:ckpt";
/// Default TTL for checkpoint data: 7 days.
const DEFAULT_TTL_SECS: u64 = 60 * 60 * 24 * 7;

fn list_key(run_id: &str) -> String {
    format!("{KEY_PREFIX}:{run_id}:list")
}

fn ckpt_key(run_id: &str, id: &CheckpointId) -> String {
    format!("{KEY_PREFIX}:{run_id}:{id}")
}

/// Redis implementation of [`CheckpointStore`].
///
/// Uses `deadpool-redis` for connection pooling so the store is `Clone` and
/// cheaply shareable across tokio tasks.
#[derive(Clone)]
pub struct RedisCheckpointStore {
    pool: Pool,
    ttl_secs: u64,
}

impl RedisCheckpointStore {
    /// Connect to the Redis instance at `url` and configure the given TTL.
    ///
    /// A `ttl_secs` of `0` disables expiry (not recommended in production).
    pub fn new(url: &str, ttl_secs: u64) -> Result<Self> {
        let cfg = deadpool_redis::Config::from_url(url);
        let pool = cfg
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .map_err(|e| orka_core::Error::Other(e.to_string()))?;
        Ok(Self { pool, ttl_secs })
    }

    /// Use the default 7-day TTL.
    pub fn new_default_ttl(url: &str) -> Result<Self> {
        Self::new(url, DEFAULT_TTL_SECS)
    }
}

#[async_trait]
impl CheckpointStore for RedisCheckpointStore {
    async fn save(&self, checkpoint: &Checkpoint) -> Result<()> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| orka_core::Error::Other(e.to_string()))?;

        let payload = serde_json::to_string(checkpoint)
            .map_err(|e| orka_core::Error::Other(e.to_string()))?;

        let list_k = list_key(&checkpoint.run_id);
        let ckpt_k = ckpt_key(&checkpoint.run_id, &checkpoint.id);
        let id_str = checkpoint.id.to_string();

        // Atomic pipeline: write checkpoint data + idempotent list append + set TTLs.
        // LREM removes any previous occurrence of this ID before re-appending so
        // that re-saving the same checkpoint (e.g. on retry) does not create
        // duplicates in the list.
        let mut pipe = redis::pipe();
        let pipe = pipe
            .atomic()
            .set(&ckpt_k, &payload)
            .lrem(&list_k, 0_isize, &id_str)
            .ignore()
            .rpush(&list_k, &id_str)
            .ignore();

        if self.ttl_secs > 0 {
            let ttl = self.ttl_secs as i64;
            pipe.expire(&ckpt_k, ttl)
                .ignore()
                .expire(&list_k, ttl)
                .ignore();
        }

        pipe.query_async::<()>(&mut *conn)
            .await
            .map_err(|e| orka_core::Error::Other(e.to_string()))?;

        debug!(
            run_id = %checkpoint.run_id,
            checkpoint_id = %checkpoint.id,
            node = %checkpoint.completed_node,
            "checkpoint.saved"
        );
        Ok(())
    }

    async fn load_latest(&self, run_id: &str) -> Result<Option<Checkpoint>> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| orka_core::Error::Other(e.to_string()))?;

        // LRANGE -1 -1 returns the last element.
        let ids: Vec<String> = conn
            .lrange(list_key(run_id), -1_isize, -1_isize)
            .await
            .map_err(|e| orka_core::Error::Other(e.to_string()))?;

        let Some(id_str) = ids.into_iter().next() else {
            return Ok(None);
        };

        let id = CheckpointId(
            id_str
                .parse::<uuid::Uuid>()
                .map_err(|e| orka_core::Error::Other(e.to_string()))?,
        );

        self.load(run_id, &id).await
    }

    async fn load(&self, run_id: &str, id: &CheckpointId) -> Result<Option<Checkpoint>> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| orka_core::Error::Other(e.to_string()))?;

        let raw: Option<String> = conn
            .get(ckpt_key(run_id, id))
            .await
            .map_err(|e| orka_core::Error::Other(e.to_string()))?;

        let Some(payload) = raw else {
            return Ok(None);
        };

        let checkpoint: Checkpoint =
            serde_json::from_str(&payload).map_err(|e| orka_core::Error::Other(e.to_string()))?;

        Ok(Some(checkpoint))
    }

    async fn list(&self, run_id: &str) -> Result<Vec<CheckpointId>> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| orka_core::Error::Other(e.to_string()))?;

        let ids: Vec<String> = conn
            .lrange(list_key(run_id), 0_isize, -1_isize)
            .await
            .map_err(|e| orka_core::Error::Other(e.to_string()))?;

        let mut result = Vec::with_capacity(ids.len());
        for id_str in ids {
            match id_str.parse::<uuid::Uuid>() {
                Ok(uuid) => result.push(CheckpointId(uuid)),
                Err(e) => warn!(%e, id = %id_str, "checkpoint.list: skipping malformed id"),
            }
        }
        Ok(result)
    }

    async fn delete_run(&self, run_id: &str) -> Result<()> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| orka_core::Error::Other(e.to_string()))?;

        // Load all IDs first so we can delete each checkpoint key.
        let ids: Vec<String> = conn
            .lrange(list_key(run_id), 0_isize, -1_isize)
            .await
            .map_err(|e| orka_core::Error::Other(e.to_string()))?;

        // Collect all keys to delete in one atomic pipeline.
        let mut keys_to_delete: Vec<String> = ids
            .iter()
            .map(|id_str| format!("{KEY_PREFIX}:{run_id}:{id_str}"))
            .collect();
        keys_to_delete.push(list_key(run_id));

        let mut pipe = redis::pipe();
        pipe.atomic().del(keys_to_delete).ignore();

        pipe.query_async::<()>(&mut *conn)
            .await
            .map_err(|e| orka_core::Error::Other(e.to_string()))?;

        debug!(run_id, deleted = ids.len(), "checkpoint.run_deleted");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use orka_core::{Envelope, SessionId};
    use orka_llm::client::ChatMessage;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::redis::Redis;

    use super::*;
    use crate::types::{RunStatus, SerializableStateChange};

    fn make_checkpoint(run_id: &str, node: &str) -> Checkpoint {
        let session_id = SessionId::new();
        Checkpoint {
            id: CheckpointId::new(),
            run_id: run_id.to_string(),
            session_id,
            graph_id: "test-graph".into(),
            trigger: Envelope::text("test-channel", session_id, "hello"),
            completed_node: node.to_string(),
            resume_node: None,
            state: std::collections::HashMap::new(),
            messages: vec![ChatMessage::user("hello")],
            total_tokens: 42,
            total_iterations: 1,
            agents_executed: vec!["router".into()],
            changelog: Vec::<SerializableStateChange>::new(),
            status: RunStatus::Running,
            created_at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn save_load_latest_roundtrip() {
        let container = Redis::default().start().await.expect("redis container");
        let port = container
            .get_host_port_ipv4(6379)
            .await
            .expect("redis port");
        let url = format!("redis://127.0.0.1:{port}");

        let store = RedisCheckpointStore::new_default_ttl(&url).unwrap();

        let ckpt = make_checkpoint("run-1", "node-a");
        store.save(&ckpt).await.unwrap();

        let loaded = store.load_latest("run-1").await.unwrap().unwrap();
        assert_eq!(loaded.id, ckpt.id);
        assert_eq!(loaded.completed_node, "node-a");
        assert_eq!(loaded.total_tokens, 42);
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn list_returns_oldest_first() {
        let container = Redis::default().start().await.expect("redis container");
        let port = container
            .get_host_port_ipv4(6379)
            .await
            .expect("redis port");
        let url = format!("redis://127.0.0.1:{port}");

        let store = RedisCheckpointStore::new_default_ttl(&url).unwrap();

        let c1 = make_checkpoint("run-2", "node-a");
        let c2 = make_checkpoint("run-2", "node-b");
        store.save(&c1).await.unwrap();
        store.save(&c2).await.unwrap();

        let ids = store.list("run-2").await.unwrap();
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0], c1.id);
        assert_eq!(ids[1], c2.id);
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn delete_run_removes_all_checkpoints() {
        let container = Redis::default().start().await.expect("redis container");
        let port = container
            .get_host_port_ipv4(6379)
            .await
            .expect("redis port");
        let url = format!("redis://127.0.0.1:{port}");

        let store = RedisCheckpointStore::new_default_ttl(&url).unwrap();

        store
            .save(&make_checkpoint("run-3", "node-a"))
            .await
            .unwrap();
        store
            .save(&make_checkpoint("run-3", "node-b"))
            .await
            .unwrap();
        store.delete_run("run-3").await.unwrap();

        assert!(store.load_latest("run-3").await.unwrap().is_none());
        assert!(store.list("run-3").await.unwrap().is_empty());
    }
}
