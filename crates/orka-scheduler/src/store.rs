use orka_core::Result;
use redis::AsyncCommands;
use std::sync::Arc;

use crate::types::Schedule;

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

    /// Persist a schedule and add it to the sorted set with `next_run` as score.
    pub async fn add(&self, schedule: &Schedule) -> Result<()> {
        let mut conn =
            self.pool.get().await.map_err(|e| {
                orka_core::Error::Scheduler(format!("Redis connection failed: {e}"))
            })?;

        let data = serde_json::to_string(schedule)
            .map_err(|e| orka_core::Error::Scheduler(format!("serialization failed: {e}")))?;

        // Store schedule data
        let data_key = format!("{SCHEDULE_DATA_PREFIX}{}", schedule.id);
        let _: () = conn.set(&data_key, &data).await.map_err(|e| {
            orka_core::Error::Scheduler(format!("failed to store schedule data: {e}"))
        })?;

        // Add to sorted set with next_run as score
        let _: () = conn
            .zadd(SCHEDULE_KEY, &schedule.id, schedule.next_run as f64)
            .await
            .map_err(|e| {
                orka_core::Error::Scheduler(format!("failed to add to sorted set: {e}"))
            })?;

        Ok(())
    }

    /// Remove a schedule by ID. Returns `true` if it existed.
    pub async fn remove(&self, id: &str) -> Result<bool> {
        let mut conn =
            self.pool.get().await.map_err(|e| {
                orka_core::Error::Scheduler(format!("Redis connection failed: {e}"))
            })?;

        let removed: i64 = conn.zrem(SCHEDULE_KEY, id).await.map_err(|e| {
            orka_core::Error::Scheduler(format!("failed to remove from sorted set: {e}"))
        })?;

        let data_key = format!("{SCHEDULE_DATA_PREFIX}{id}");
        let _: () = conn.del(&data_key).await.map_err(|e| {
            orka_core::Error::Scheduler(format!("failed to remove schedule data: {e}"))
        })?;

        Ok(removed > 0)
    }

    /// Return all schedules whose `next_run` timestamp is ≤ `now`.
    pub async fn get_due(&self, now: i64) -> Result<Vec<Schedule>> {
        let mut conn =
            self.pool.get().await.map_err(|e| {
                orka_core::Error::Scheduler(format!("Redis connection failed: {e}"))
            })?;

        // Get all schedule IDs with score <= now
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

    /// List all schedules. Pass `include_completed = true` to include one-shot schedules that have already fired.
    pub async fn list(&self, include_completed: bool) -> Result<Vec<Schedule>> {
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

    /// Find a schedule by human-readable name (including completed ones).
    pub async fn find_by_name(&self, name: &str) -> Result<Option<Schedule>> {
        let all = self.list(true).await?;
        Ok(all.into_iter().find(|s| s.name == name))
    }

    /// Update the `next_run` timestamp for a recurring schedule.
    pub async fn update_next_run(&self, _id: &str, schedule: &Schedule) -> Result<()> {
        self.add(schedule).await
    }
}
