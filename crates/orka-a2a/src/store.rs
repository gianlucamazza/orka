//! Task persistence backends for the A2A protocol.

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use redis::AsyncCommands;
use tokio::sync::Mutex;

use crate::{
    error::A2aError,
    types::{ListTasksParams, ListTasksResult, Task},
};

// ── Trait ─────────────────────────────────────────────────────────────────────

/// Persistence backend for A2A tasks.
#[async_trait]
pub trait TaskStore: Send + Sync + 'static {
    /// Persist or update a task.
    async fn put(&self, task: Task) -> Result<(), A2aError>;

    /// Retrieve a task by ID. Returns `None` if not found.
    async fn get(&self, id: &str) -> Result<Option<Task>, A2aError>;

    /// Delete a task by ID. Returns `true` if it existed.
    async fn delete(&self, id: &str) -> Result<bool, A2aError>;

    /// List tasks with optional state filtering and pagination.
    async fn list(&self, params: &ListTasksParams) -> Result<ListTasksResult, A2aError>;
}

// ── InMemoryTaskStore
// ─────────────────────────────────────────────────────────

/// Maximum number of tasks kept in the in-memory store before the oldest is
/// evicted. Prevents unbounded memory growth on long-running development
/// servers.
const IN_MEMORY_MAX_TASKS: usize = 10_000;

/// In-memory task store backed by a `Mutex<HashMap>`.
///
/// Suitable for development and single-node deployments. All state is lost on
/// restart. Caps at [`IN_MEMORY_MAX_TASKS`] entries, evicting the
/// oldest task (by `created_at`) when the limit is reached.
#[derive(Debug, Default)]
pub struct InMemoryTaskStore {
    tasks: Mutex<HashMap<String, Task>>,
}

#[async_trait]
impl TaskStore for InMemoryTaskStore {
    async fn put(&self, task: Task) -> Result<(), A2aError> {
        let mut tasks = self.tasks.lock().await;
        // Evict the oldest entry if we're at capacity and this is a new task.
        if tasks.len() >= IN_MEMORY_MAX_TASKS
            && !tasks.contains_key(&task.id)
            && let Some(oldest_id) =
                tasks.values().min_by_key(|t| t.created_at).map(|t| t.id.clone())
        {
            tasks.remove(&oldest_id);
        }
        tasks.insert(task.id.clone(), task);
        Ok(())
    }

    async fn get(&self, id: &str) -> Result<Option<Task>, A2aError> {
        Ok(self.tasks.lock().await.get(id).cloned())
    }

    async fn delete(&self, id: &str) -> Result<bool, A2aError> {
        Ok(self.tasks.lock().await.remove(id).is_some())
    }

    async fn list(&self, params: &ListTasksParams) -> Result<ListTasksResult, A2aError> {
        let tasks = self.tasks.lock().await;
        let mut all: Vec<&Task> = tasks.values().collect();
        all.sort_by(|a, b| a.created_at.cmp(&b.created_at));

        let filtered: Vec<Task> = all
            .into_iter()
            .filter(|t| params.states.is_empty() || params.states.contains(&t.status.state))
            .cloned()
            .collect();

        let offset: usize = params
            .page_token
            .as_deref()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let page_size = params.page_size.clamp(1, 100);
        let slice = &filtered[offset.min(filtered.len())..];
        let page: Vec<Task> = slice.iter().take(page_size).cloned().collect();
        let next_page_token = if offset + page.len() < filtered.len() {
            Some((offset + page.len()).to_string())
        } else {
            None
        };

        Ok(ListTasksResult {
            tasks: page,
            next_page_token,
        })
    }
}

// ── RedisTaskStore
// ────────────────────────────────────────────────────────────

/// Redis key prefix for individual task data.
const TASK_KEY_PREFIX: &str = "orka:a2a:task:";
/// Redis sorted-set key for the task index (score = `created_at` millis).
const TASK_INDEX_KEY: &str = "orka:a2a:tasks";

/// Redis-backed task store using a sorted set as an index.
///
/// Task data is stored as serialised JSON at `orka:a2a:task:{id}`.
/// An index sorted set at `orka:a2a:tasks` maps IDs → `created_at` millis
/// for ordered listing and pagination.
pub struct RedisTaskStore {
    pool: Arc<deadpool_redis::Pool>,
}

impl RedisTaskStore {
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
impl TaskStore for RedisTaskStore {
    async fn put(&self, task: Task) -> Result<(), A2aError> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| A2aError::Internal(format!("Redis connection failed: {e}")))?;

        let data = serde_json::to_string(&task)
            .map_err(|e| A2aError::Internal(format!("serialization failed: {e}")))?;

        let data_key = format!("{TASK_KEY_PREFIX}{}", task.id);
        let score = task.created_at.timestamp_millis() as f64;

        redis::pipe()
            .atomic()
            .set(&data_key, &data)
            .ignore()
            .zadd(TASK_INDEX_KEY, &task.id, score)
            .ignore()
            .query_async::<()>(&mut *conn)
            .await
            .map_err(|e| A2aError::Internal(format!("failed to store task: {e}")))?;

        Ok(())
    }

    async fn get(&self, id: &str) -> Result<Option<Task>, A2aError> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| A2aError::Internal(format!("Redis connection failed: {e}")))?;

        let data_key = format!("{TASK_KEY_PREFIX}{id}");
        let data: Option<String> = conn
            .get(&data_key)
            .await
            .map_err(|e| A2aError::Internal(format!("failed to get task: {e}")))?;

        match data {
            Some(d) => {
                let task = serde_json::from_str(&d)
                    .map_err(|e| A2aError::Internal(format!("deserialization failed: {e}")))?;
                Ok(Some(task))
            }
            None => Ok(None),
        }
    }

    async fn delete(&self, id: &str) -> Result<bool, A2aError> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| A2aError::Internal(format!("Redis connection failed: {e}")))?;

        let data_key = format!("{TASK_KEY_PREFIX}{id}");

        let (removed,): (i64,) = redis::pipe()
            .atomic()
            .zrem(TASK_INDEX_KEY, id)
            .del(&data_key)
            .ignore()
            .query_async(&mut *conn)
            .await
            .map_err(|e| A2aError::Internal(format!("failed to delete task: {e}")))?;

        Ok(removed > 0)
    }

    async fn list(&self, params: &ListTasksParams) -> Result<ListTasksResult, A2aError> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| A2aError::Internal(format!("Redis connection failed: {e}")))?;

        let ids: Vec<String> = conn
            .zrangebyscore(TASK_INDEX_KEY, "-inf", "+inf")
            .await
            .map_err(|e| A2aError::Internal(format!("failed to list task index: {e}")))?;

        if ids.is_empty() {
            return Ok(ListTasksResult {
                tasks: Vec::new(),
                next_page_token: None,
            });
        }

        // Batch-fetch all task payloads in a single MGET instead of N GETs.
        let data_keys: Vec<String> = ids
            .iter()
            .map(|id| format!("{TASK_KEY_PREFIX}{id}"))
            .collect();
        let raw_values: Vec<Option<String>> = conn
            .mget(&data_keys)
            .await
            .map_err(|e| A2aError::Internal(format!("failed to mget tasks: {e}")))?;

        let mut tasks = Vec::new();
        for raw in raw_values {
            if let Some(d) = raw
                && let Ok(task) = serde_json::from_str::<Task>(&d)
                && (params.states.is_empty() || params.states.contains(&task.status.state))
            {
                tasks.push(task);
            }
        }

        // IDs are already ordered by score (created_at millis).
        let offset: usize = params
            .page_token
            .as_deref()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let page_size = params.page_size.clamp(1, 100);
        let slice = &tasks[offset.min(tasks.len())..];
        let page: Vec<Task> = slice.iter().take(page_size).cloned().collect();
        let next_page_token = if offset + page.len() < tasks.len() {
            Some((offset + page.len()).to_string())
        } else {
            None
        };

        Ok(ListTasksResult {
            tasks: page,
            next_page_token,
        })
    }
}
