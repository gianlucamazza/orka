use std::sync::Arc;

use async_trait::async_trait;
use orka_core::Result;
use redis::AsyncCommands;

use crate::types::Schedule;

/// Trait for schedule persistence backends.
#[async_trait]
pub trait ScheduleStore: Send + Sync + 'static {
    /// Persist a schedule and register it for future execution.
    async fn add(&self, schedule: &Schedule) -> Result<()>;

    /// Remove a schedule by ID. Returns `true` if it existed.
    async fn remove(&self, id: &str) -> Result<bool>;

    /// Return all schedules whose `next_run` timestamp is ≤ `now`.
    async fn get_due(&self, now: i64) -> Result<Vec<Schedule>>;

    /// List all schedules. Pass `include_completed = true` to include
    /// one-shot schedules that have already fired.
    async fn list(&self, include_completed: bool) -> Result<Vec<Schedule>>;

    /// Find a schedule by human-readable name (including completed ones).
    async fn find_by_name(&self, name: &str) -> Result<Option<Schedule>>;

    /// Update the `next_run` timestamp for a recurring schedule.
    async fn update_next_run(&self, id: &str, schedule: &Schedule) -> Result<()>;

    /// Attempt to acquire an execution lock for a specific schedule run.
    ///
    /// Returns `true` if the lock was acquired (this instance should execute),
    /// `false` if another instance already holds it.  The lock expires
    /// automatically after `ttl_secs` seconds to prevent stale locks from
    /// blocking future runs.
    ///
    /// Default implementation always returns `true` (no distributed locking).
    async fn try_lock_execution(&self, id: &str, run_at: i64, ttl_secs: u64) -> Result<bool> {
        let _ = (id, run_at, ttl_secs);
        Ok(true)
    }

    /// Release the execution lock acquired by [`Self::try_lock_execution`].
    ///
    /// Should be called after execution completes so the lock is freed before
    /// its TTL expires.  Default implementation is a no-op.
    async fn release_execution_lock(&self, id: &str, run_at: i64) -> Result<()> {
        let _ = (id, run_at);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Redis implementation
// ---------------------------------------------------------------------------

const SCHEDULE_KEY: &str = "orka:schedules";
const SCHEDULE_DATA_PREFIX: &str = "orka:schedule:";

/// Redis-backed schedule store using sorted sets.
pub struct RedisScheduleStore {
    pool: Arc<deadpool_redis::Pool>,
}

impl RedisScheduleStore {
    /// Create a new store backed by the given Redis URL.
    pub fn new(redis_url: &str) -> Result<Self> {
        let cfg = deadpool_redis::Config::from_url(redis_url);
        let pool = cfg
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .map_err(|e| {
                orka_core::Error::Scheduler(format!("failed to create Redis pool: {e}"))
            })?;

        Ok(Self {
            pool: Arc::new(pool),
        })
    }
}

#[async_trait]
impl ScheduleStore for RedisScheduleStore {
    async fn add(&self, schedule: &Schedule) -> Result<()> {
        let mut conn =
            self.pool.get().await.map_err(|e| {
                orka_core::Error::Scheduler(format!("Redis connection failed: {e}"))
            })?;

        let data = serde_json::to_string(schedule)
            .map_err(|e| orka_core::Error::Scheduler(format!("serialization failed: {e}")))?;

        let data_key = format!("{SCHEDULE_DATA_PREFIX}{}", schedule.id);

        redis::pipe()
            .atomic()
            .set(&data_key, &data)
            .ignore()
            .zadd(SCHEDULE_KEY, &schedule.id, schedule.next_run as f64)
            .ignore()
            .query_async::<()>(&mut *conn)
            .await
            .map_err(|e| orka_core::Error::Scheduler(format!("failed to store schedule: {e}")))?;

        Ok(())
    }

    async fn remove(&self, id: &str) -> Result<bool> {
        let mut conn =
            self.pool.get().await.map_err(|e| {
                orka_core::Error::Scheduler(format!("Redis connection failed: {e}"))
            })?;

        let data_key = format!("{SCHEDULE_DATA_PREFIX}{id}");

        let (removed,): (i64,) = redis::pipe()
            .atomic()
            .zrem(SCHEDULE_KEY, id)
            .del(&data_key)
            .ignore()
            .query_async(&mut *conn)
            .await
            .map_err(|e| orka_core::Error::Scheduler(format!("failed to remove schedule: {e}")))?;

        Ok(removed > 0)
    }

    async fn get_due(&self, now: i64) -> Result<Vec<Schedule>> {
        let mut conn =
            self.pool.get().await.map_err(|e| {
                orka_core::Error::Scheduler(format!("Redis connection failed: {e}"))
            })?;

        let ids: Vec<String> = conn
            .zrangebyscore(SCHEDULE_KEY, "-inf", now as f64)
            .await
            .map_err(|e| orka_core::Error::Scheduler(format!("failed to query sorted set: {e}")))?;

        let mut schedules = Vec::new();
        for id in ids {
            let data_key = format!("{SCHEDULE_DATA_PREFIX}{id}");
            let data: Option<String> = conn.get(&data_key).await.map_err(|e| {
                orka_core::Error::Scheduler(format!("failed to get schedule data: {e}"))
            })?;

            if let Some(data) = data
                && let Ok(schedule) = serde_json::from_str::<Schedule>(&data)
            {
                schedules.push(schedule);
            }
        }

        Ok(schedules)
    }

    async fn list(&self, include_completed: bool) -> Result<Vec<Schedule>> {
        let mut conn =
            self.pool.get().await.map_err(|e| {
                orka_core::Error::Scheduler(format!("Redis connection failed: {e}"))
            })?;

        let ids: Vec<String> = conn
            .zrangebyscore(SCHEDULE_KEY, "-inf", "+inf")
            .await
            .map_err(|e| orka_core::Error::Scheduler(format!("failed to list schedules: {e}")))?;

        let mut schedules = Vec::new();
        for id in ids {
            let data_key = format!("{SCHEDULE_DATA_PREFIX}{id}");
            let data: Option<String> = conn.get(&data_key).await.map_err(|e| {
                orka_core::Error::Scheduler(format!("failed to get schedule data: {e}"))
            })?;

            if let Some(data) = data
                && let Ok(schedule) = serde_json::from_str::<Schedule>(&data)
                && (include_completed || !schedule.completed)
            {
                schedules.push(schedule);
            }
        }

        Ok(schedules)
    }

    async fn find_by_name(&self, name: &str) -> Result<Option<Schedule>> {
        let all = self.list(true).await?;
        Ok(all.into_iter().find(|s| s.name == name))
    }

    async fn update_next_run(&self, _id: &str, schedule: &Schedule) -> Result<()> {
        self.add(schedule).await
    }

    async fn try_lock_execution(&self, id: &str, run_at: i64, ttl_secs: u64) -> Result<bool> {
        let mut conn =
            self.pool.get().await.map_err(|e| {
                orka_core::Error::Scheduler(format!("Redis connection failed: {e}"))
            })?;
        let lock_key = format!("orka:schedule:exec:{id}:{run_at}");
        let result: Option<String> = redis::cmd("SET")
            .arg(&lock_key)
            .arg("1")
            .arg("NX")
            .arg("PX")
            .arg(ttl_secs * 1000)
            .query_async(&mut *conn)
            .await
            .map_err(|e| {
                orka_core::Error::Scheduler(format!("failed to acquire execution lock: {e}"))
            })?;
        // SET NX returns "OK" if the key was set (lock acquired), nil if it already existed
        Ok(result.is_some())
    }

    async fn release_execution_lock(&self, id: &str, run_at: i64) -> Result<()> {
        let mut conn =
            self.pool.get().await.map_err(|e| {
                orka_core::Error::Scheduler(format!("Redis connection failed: {e}"))
            })?;
        let lock_key = format!("orka:schedule:exec:{id}:{run_at}");
        let _: () = conn.del(&lock_key).await.map_err(|e| {
            orka_core::Error::Scheduler(format!("failed to release execution lock: {e}"))
        })?;
        Ok(())
    }
}
