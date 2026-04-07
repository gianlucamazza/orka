#![allow(clippy::unwrap_used)]

use std::collections::HashSet;

use async_trait::async_trait;
use orka_core::Result;
use tokio::sync::Mutex;

use crate::{store::ScheduleStore, types::Schedule};

/// In-memory [`ScheduleStore`] for use in tests (no Redis required).
pub struct InMemoryScheduleStore {
    schedules: Mutex<Vec<Schedule>>,
    /// Tracks acquired execution locks: `"{id}:{run_at}"`.
    locks: Mutex<HashSet<String>>,
}

impl InMemoryScheduleStore {
    /// Create an empty schedule store.
    pub fn new() -> Self {
        Self {
            schedules: Mutex::new(Vec::new()),
            locks: Mutex::new(HashSet::new()),
        }
    }
}

impl Default for InMemoryScheduleStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ScheduleStore for InMemoryScheduleStore {
    async fn add(&self, schedule: &Schedule) -> Result<()> {
        let mut schedules = self.schedules.lock().await;
        schedules.retain(|s| s.id != schedule.id);
        schedules.push(schedule.clone());
        Ok(())
    }

    async fn remove(&self, id: &str) -> Result<bool> {
        let mut schedules = self.schedules.lock().await;
        let before = schedules.len();
        schedules.retain(|s| s.id != id);
        Ok(schedules.len() < before)
    }

    async fn get_due(&self, now: i64) -> Result<Vec<Schedule>> {
        let schedules = self.schedules.lock().await;
        Ok(schedules
            .iter()
            .filter(|s| s.next_run <= now)
            .cloned()
            .collect())
    }

    async fn list(&self, include_completed: bool) -> Result<Vec<Schedule>> {
        let schedules = self.schedules.lock().await;
        Ok(schedules
            .iter()
            .filter(|s| include_completed || !s.completed)
            .cloned()
            .collect())
    }

    async fn find_by_name(&self, name: &str) -> Result<Option<Schedule>> {
        let schedules = self.schedules.lock().await;
        Ok(schedules.iter().find(|s| s.name == name).cloned())
    }

    async fn update_next_run(&self, _id: &str, schedule: &Schedule) -> Result<()> {
        self.add(schedule).await
    }

    async fn try_lock_execution(&self, id: &str, run_at: i64, _ttl_secs: u64) -> Result<bool> {
        let mut locks = self.locks.lock().await;
        let key = format!("{id}:{run_at}");
        Ok(locks.insert(key))
    }

    async fn release_execution_lock(&self, id: &str, run_at: i64) -> Result<()> {
        let mut locks = self.locks.lock().await;
        locks.remove(&format!("{id}:{run_at}"));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_schedule(id: &str, name: &str, next_run: i64) -> Schedule {
        Schedule {
            id: id.to_string(),
            name: name.to_string(),
            cron: None,
            run_at: None,
            timezone: None,
            skill: None,
            args: None,
            message: None,
            next_run,
            created_at: "2025-01-01T00:00:00Z".to_string(),
            completed: false,
        }
    }

    #[tokio::test]
    async fn add_and_list() {
        let store = InMemoryScheduleStore::new();
        let s = make_schedule("s1", "test", 100);
        store.add(&s).await.unwrap();

        let all = store.list(false).await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name, "test");
    }

    #[tokio::test]
    async fn add_replaces_existing() {
        let store = InMemoryScheduleStore::new();
        store.add(&make_schedule("s1", "v1", 100)).await.unwrap();
        store.add(&make_schedule("s1", "v2", 200)).await.unwrap();

        let all = store.list(false).await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name, "v2");
    }

    #[tokio::test]
    async fn remove_returns_true_if_existed() {
        let store = InMemoryScheduleStore::new();
        store.add(&make_schedule("s1", "test", 100)).await.unwrap();
        assert!(store.remove("s1").await.unwrap());
        assert!(!store.remove("s1").await.unwrap());
    }

    #[tokio::test]
    async fn get_due_filters_by_timestamp() {
        let store = InMemoryScheduleStore::new();
        store.add(&make_schedule("s1", "past", 50)).await.unwrap();
        store
            .add(&make_schedule("s2", "future", 200))
            .await
            .unwrap();

        let due = store.get_due(100).await.unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].name, "past");
    }

    #[tokio::test]
    async fn list_excludes_completed_by_default() {
        let store = InMemoryScheduleStore::new();
        let mut s = make_schedule("s1", "done", 100);
        s.completed = true;
        store.add(&s).await.unwrap();
        store
            .add(&make_schedule("s2", "active", 200))
            .await
            .unwrap();

        assert_eq!(store.list(false).await.unwrap().len(), 1);
        assert_eq!(store.list(true).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn find_by_name() {
        let store = InMemoryScheduleStore::new();
        store
            .add(&make_schedule("s1", "daily-report", 100))
            .await
            .unwrap();

        assert!(store.find_by_name("daily-report").await.unwrap().is_some());
        assert!(store.find_by_name("nonexistent").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn update_next_run() {
        let store = InMemoryScheduleStore::new();
        let mut s = make_schedule("s1", "test", 100);
        store.add(&s).await.unwrap();

        s.next_run = 500;
        store.update_next_run("s1", &s).await.unwrap();

        let all = store.list(false).await.unwrap();
        assert_eq!(all[0].next_run, 500);
    }
}
