//! Push notification configuration backends for the A2A protocol.

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use redis::AsyncCommands;
use tokio::sync::Mutex;

use crate::{error::A2aError, types::PushNotificationConfig};

// ── Trait ─────────────────────────────────────────────────────────────────────

/// Persistence backend for A2A push notification configurations.
///
/// Each task has at most one active configuration. Calling
/// [`set`](PushNotificationStore::set) on a task that already has a config
/// replaces it.
#[async_trait]
pub trait PushNotificationStore: Send + Sync + 'static {
    /// Register or replace the push notification config for a task.
    async fn set(&self, config: PushNotificationConfig) -> Result<(), A2aError>;

    /// Retrieve the config for a task. Returns `None` if not registered.
    async fn get(&self, task_id: &str) -> Result<Option<PushNotificationConfig>, A2aError>;

    /// List all registered configs.
    async fn list(&self) -> Result<Vec<PushNotificationConfig>, A2aError>;

    /// Delete the config for a task. Returns `true` if it existed.
    async fn delete(&self, task_id: &str) -> Result<bool, A2aError>;
}

// ── InMemoryPushNotificationStore
// ─────────────────────────────────────────────

/// In-memory push notification config store backed by a `Mutex<HashMap>`.
#[derive(Debug, Default)]
pub struct InMemoryPushNotificationStore {
    configs: Mutex<HashMap<String, PushNotificationConfig>>,
}

#[async_trait]
impl PushNotificationStore for InMemoryPushNotificationStore {
    async fn set(&self, config: PushNotificationConfig) -> Result<(), A2aError> {
        self.configs
            .lock()
            .await
            .insert(config.task_id.clone(), config);
        Ok(())
    }

    async fn get(&self, task_id: &str) -> Result<Option<PushNotificationConfig>, A2aError> {
        Ok(self.configs.lock().await.get(task_id).cloned())
    }

    async fn list(&self) -> Result<Vec<PushNotificationConfig>, A2aError> {
        Ok(self.configs.lock().await.values().cloned().collect())
    }

    async fn delete(&self, task_id: &str) -> Result<bool, A2aError> {
        Ok(self.configs.lock().await.remove(task_id).is_some())
    }
}

// ── RedisPushNotificationStore
// ────────────────────────────────────────────────

/// Redis key prefix for individual push notification config data.
const PUSH_KEY_PREFIX: &str = "orka:a2a:push:";
/// Redis set key for the push notification config index.
const PUSH_INDEX_KEY: &str = "orka:a2a:push_configs";

/// Redis-backed push notification config store.
///
/// Config data is stored as serialised JSON at `orka:a2a:push:{task_id}`.
/// A set at `orka:a2a:push_configs` holds all task IDs for iteration.
pub struct RedisPushNotificationStore {
    pool: Arc<deadpool_redis::Pool>,
}

impl RedisPushNotificationStore {
    /// Create a new store backed by the given Redis URL.
    pub fn new(redis_url: &str) -> Result<Self, A2aError> {
        let cfg = deadpool_redis::Config::from_url(redis_url);
        let pool = cfg
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .map_err(|e| A2aError::Internal(format!("failed to create Redis pool: {e}")))?;
        Ok(Self {
            pool: Arc::new(pool),
        })
    }
}

#[async_trait]
impl PushNotificationStore for RedisPushNotificationStore {
    async fn set(&self, config: PushNotificationConfig) -> Result<(), A2aError> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| A2aError::Internal(format!("Redis connection failed: {e}")))?;

        let data = serde_json::to_string(&config)
            .map_err(|e| A2aError::Internal(format!("serialization failed: {e}")))?;

        let data_key = format!("{PUSH_KEY_PREFIX}{}", config.task_id);

        redis::pipe()
            .atomic()
            .set(&data_key, &data)
            .ignore()
            .sadd(PUSH_INDEX_KEY, &config.task_id)
            .ignore()
            .query_async::<()>(&mut *conn)
            .await
            .map_err(|e| A2aError::Internal(format!("failed to store push config: {e}")))?;

        Ok(())
    }

    async fn get(&self, task_id: &str) -> Result<Option<PushNotificationConfig>, A2aError> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| A2aError::Internal(format!("Redis connection failed: {e}")))?;

        let data_key = format!("{PUSH_KEY_PREFIX}{task_id}");
        let data: Option<String> = conn
            .get(&data_key)
            .await
            .map_err(|e| A2aError::Internal(format!("failed to get push config: {e}")))?;

        match data {
            Some(d) => {
                let cfg = serde_json::from_str(&d)
                    .map_err(|e| A2aError::Internal(format!("deserialization failed: {e}")))?;
                Ok(Some(cfg))
            }
            None => Ok(None),
        }
    }

    async fn list(&self) -> Result<Vec<PushNotificationConfig>, A2aError> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| A2aError::Internal(format!("Redis connection failed: {e}")))?;

        let ids: Vec<String> = conn
            .smembers(PUSH_INDEX_KEY)
            .await
            .map_err(|e| A2aError::Internal(format!("failed to list push configs: {e}")))?;

        let mut configs = Vec::new();
        for id in ids {
            let data_key = format!("{PUSH_KEY_PREFIX}{id}");
            let data: Option<String> = conn
                .get(&data_key)
                .await
                .map_err(|e| A2aError::Internal(format!("failed to get push config: {e}")))?;

            if let Some(d) = data
                && let Ok(cfg) = serde_json::from_str::<PushNotificationConfig>(&d)
            {
                configs.push(cfg);
            }
        }

        Ok(configs)
    }

    async fn delete(&self, task_id: &str) -> Result<bool, A2aError> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| A2aError::Internal(format!("Redis connection failed: {e}")))?;

        let data_key = format!("{PUSH_KEY_PREFIX}{task_id}");

        let (removed,): (i64,) = redis::pipe()
            .atomic()
            .srem(PUSH_INDEX_KEY, task_id)
            .del(&data_key)
            .ignore()
            .query_async(&mut *conn)
            .await
            .map_err(|e| A2aError::Internal(format!("failed to delete push config: {e}")))?;

        Ok(removed > 0)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config(task_id: &str) -> PushNotificationConfig {
        PushNotificationConfig {
            task_id: task_id.to_string(),
            url: "https://example.com/webhook".to_string(),
            token: Some("secret-token".to_string()),
            authentication: None,
        }
    }

    #[tokio::test]
    async fn in_memory_set_and_get() {
        let store = InMemoryPushNotificationStore::default();
        store.set(sample_config("t1")).await.unwrap();
        let got = store.get("t1").await.unwrap().unwrap();
        assert_eq!(got.task_id, "t1");
        assert_eq!(got.url, "https://example.com/webhook");
    }

    #[tokio::test]
    async fn in_memory_set_replaces_existing() {
        let store = InMemoryPushNotificationStore::default();
        store.set(sample_config("t1")).await.unwrap();
        let mut updated = sample_config("t1");
        updated.url = "https://other.com/hook".to_string();
        store.set(updated).await.unwrap();
        let got = store.get("t1").await.unwrap().unwrap();
        assert_eq!(got.url, "https://other.com/hook");
    }

    #[tokio::test]
    async fn in_memory_list() {
        let store = InMemoryPushNotificationStore::default();
        store.set(sample_config("a")).await.unwrap();
        store.set(sample_config("b")).await.unwrap();
        let list = store.list().await.unwrap();
        assert_eq!(list.len(), 2);
    }

    #[tokio::test]
    async fn in_memory_delete() {
        let store = InMemoryPushNotificationStore::default();
        store.set(sample_config("t2")).await.unwrap();
        assert!(store.delete("t2").await.unwrap());
        assert!(!store.delete("t2").await.unwrap()); // idempotent
        assert!(store.get("t2").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn in_memory_get_missing() {
        let store = InMemoryPushNotificationStore::default();
        assert!(store.get("nonexistent").await.unwrap().is_none());
    }
}
