use std::{str::FromStr, sync::Arc};

use chrono::Utc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::store::ScheduleStore;

/// Async poll loop that checks for due tasks and executes them.
pub struct Scheduler {
    store: Arc<dyn ScheduleStore>,
    skills: Arc<dyn SkillRegistry>,
    poll_interval_secs: u64,
    max_concurrent: usize,
}

/// Minimal interface for the scheduler to invoke skills.
#[async_trait::async_trait]
pub trait SkillRegistry: Send + Sync + 'static {
    /// Invoke the named skill with the given input.
    async fn invoke(
        &self,
        name: &str,
        input: orka_core::SkillInput,
    ) -> orka_core::Result<orka_core::SkillOutput>;
}

impl Scheduler {
    /// Create a new [`Scheduler`].
    ///
    /// `poll_interval_secs` controls how often due tasks are checked.
    /// `max_concurrent` limits how many tasks run simultaneously.
    pub fn new(
        store: Arc<dyn ScheduleStore>,
        skills: Arc<dyn SkillRegistry>,
        poll_interval_secs: u64,
        max_concurrent: usize,
    ) -> Self {
        Self {
            store,
            skills,
            poll_interval_secs,
            max_concurrent,
        }
    }

    /// Run the scheduler tick loop until the cancellation token is triggered.
    pub async fn run(&self, cancel: CancellationToken) {
        info!(poll_interval = self.poll_interval_secs, "scheduler started");

        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(self.poll_interval_secs));

        loop {
            tokio::select! {
                () = cancel.cancelled() => {
                    info!("scheduler stopping");
                    break;
                }
                _ = interval.tick() => {
                    if let Err(e) = self.poll_and_execute().await {
                        error!(%e, "scheduler poll error");
                    }
                }
            }
        }
    }

    async fn poll_and_execute(&self) -> orka_core::Result<()> {
        let now = Utc::now().timestamp();
        let due = self.store.get_due(now).await?;

        if due.is_empty() {
            return Ok(());
        }

        debug!(count = due.len(), "found due tasks");

        let semaphore = Arc::new(tokio::sync::Semaphore::new(self.max_concurrent));

        for schedule in due {
            // Acquire distributed execution lock before spawning.  If another
            // instance already holds the lock for this exact run, skip it to
            // prevent duplicate execution in multi-instance deployments.
            // TTL of 300 s covers most tasks; the lock is explicitly released
            // after execution so it is freed as soon as possible.
            match self
                .store
                .try_lock_execution(&schedule.id, schedule.next_run, 300)
                .await
            {
                Ok(true) => {} // lock acquired — proceed
                Ok(false) => {
                    debug!(
                        id = %schedule.id,
                        name = %schedule.name,
                        "skipping due task — execution lock held by another instance"
                    );
                    continue;
                }
                Err(e) => {
                    // Locking failed (e.g. Redis unavailable) — log and proceed
                    // rather than dropping the task entirely.
                    warn!(%e, id = %schedule.id, "failed to acquire execution lock, proceeding");
                }
            }

            let Ok(permit) = semaphore.clone().acquire_owned().await else {
                break;
            };

            let store = self.store.clone();
            let skills = self.skills.clone();

            tokio::spawn(async move {
                let _permit = permit;

                // Execute the scheduled skill
                if let Some(ref skill_name) = schedule.skill {
                    let input = orka_core::SkillInput::new(
                        schedule
                            .args
                            .clone()
                            .unwrap_or_default()
                            .into_iter()
                            .collect(),
                    );

                    match skills.invoke(skill_name, input).await {
                        Ok(_) => {
                            info!(
                                schedule_name = %schedule.name,
                                skill = %skill_name,
                                "scheduled task completed"
                            );
                        }
                        Err(e) => {
                            error!(
                                schedule_name = %schedule.name,
                                skill = %skill_name,
                                %e,
                                "scheduled task failed"
                            );
                        }
                    }
                }

                // Handle next run: if cron, compute next; if one-shot, remove.
                // Release the execution lock after updating/removing so the
                // full critical section (fetch → execute → reschedule) is
                // covered, preventing a second instance from re-picking the
                // same run.
                if let Some(ref cron_expr) = schedule.cron {
                    match cron::Schedule::from_str(cron_expr) {
                        Ok(cron_schedule) => {
                            if let Some(next) = cron_schedule.upcoming(Utc).next() {
                                let mut updated = schedule.clone();
                                updated.next_run = next.timestamp();
                                if let Err(e) = store.update_next_run(&schedule.id, &updated).await
                                {
                                    error!(%e, "failed to update next run");
                                }
                            }
                        }
                        Err(e) => {
                            warn!(cron = cron_expr, %e, "invalid cron expression");
                        }
                    }
                } else {
                    // One-shot: remove from sorted set
                    if let Err(e) = store.remove(&schedule.id).await {
                        error!(%e, "failed to remove completed one-shot schedule");
                    }
                }

                // Explicitly release the lock so it is freed before the TTL
                // expires.  Errors here are non-fatal — the TTL acts as a
                // safety net.
                if let Err(e) = store
                    .release_execution_lock(&schedule.id, schedule.next_run)
                    .await
                {
                    warn!(%e, id = %schedule.id, "failed to release execution lock");
                }
            });
        }

        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::similar_names)]
mod tests {
    use super::*;

    #[test]
    fn cron_schedule_computes_next_run() {
        // Verify that a valid cron expression parses and yields future timestamps
        use cron::Schedule as CronSchedule;

        let expr = "0 * * * * *"; // every minute
        let cron = CronSchedule::from_str(expr).unwrap();
        let next = cron.upcoming(Utc).next();
        assert!(next.is_some());
        assert!(next.unwrap().timestamp() > Utc::now().timestamp());
    }

    #[test]
    fn invalid_cron_expression_does_not_panic() {
        use cron::Schedule as CronSchedule;
        let result = CronSchedule::from_str("not a cron");
        assert!(result.is_err());
    }

    use crate::{memory_store::InMemoryScheduleStore, types::Schedule};

    fn make_schedule(id: &str, name: &str, skill: &str, next_run: i64) -> Schedule {
        Schedule {
            id: id.to_string(),
            name: name.to_string(),
            cron: None,
            run_at: None,
            timezone: None,
            skill: Some(skill.to_string()),
            args: None,
            message: None,
            next_run,
            created_at: "2025-01-01T00:00:00Z".to_string(),
            completed: false,
        }
    }

    /// Mock skill registry that records invocations.
    struct MockSkillRegistry {
        invocations: Arc<tokio::sync::Mutex<Vec<(String, orka_core::SkillInput)>>>,
    }

    impl MockSkillRegistry {
        fn new() -> Self {
            Self {
                invocations: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            }
        }

        async fn invocation_count(&self) -> usize {
            self.invocations.lock().await.len()
        }
    }

    #[async_trait::async_trait]
    impl SkillRegistry for MockSkillRegistry {
        async fn invoke(
            &self,
            name: &str,
            input: orka_core::SkillInput,
        ) -> orka_core::Result<orka_core::SkillOutput> {
            self.invocations
                .lock()
                .await
                .push((name.to_string(), input));
            Ok(orka_core::SkillOutput::new(serde_json::json!({"ok": true})))
        }
    }

    #[tokio::test]
    async fn poll_and_execute_fires_due_task() {
        let store = Arc::new(InMemoryScheduleStore::new());
        let skills = Arc::new(MockSkillRegistry::new());

        // Add a task that is already due
        let s = make_schedule("s1", "test-task", "echo", 0);
        store.add(&s).await.unwrap();

        let scheduler = Scheduler::new(store.clone(), skills.clone(), 1, 4);
        scheduler.poll_and_execute().await.unwrap();

        // Wait for spawned task
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(skills.invocation_count().await, 1);
    }

    #[tokio::test]
    async fn no_tasks_due_is_noop() {
        let store = Arc::new(InMemoryScheduleStore::new());
        let skills = Arc::new(MockSkillRegistry::new());

        // Add a task in the far future
        let s = make_schedule("s1", "future", "echo", i64::MAX);
        store.add(&s).await.unwrap();

        let scheduler = Scheduler::new(store, skills.clone(), 1, 4);
        scheduler.poll_and_execute().await.unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(skills.invocation_count().await, 0);
    }

    #[tokio::test]
    async fn one_shot_removed_after_execution() {
        let store = Arc::new(InMemoryScheduleStore::new());
        let skills = Arc::new(MockSkillRegistry::new());

        let s = make_schedule("s1", "one-shot", "echo", 0);
        store.add(&s).await.unwrap();

        let scheduler = Scheduler::new(store.clone(), skills, 1, 4);
        scheduler.poll_and_execute().await.unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(store.list(true).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn recurring_schedule_updates_next_run() {
        let store = Arc::new(InMemoryScheduleStore::new());
        let skills = Arc::new(MockSkillRegistry::new());

        let mut s = make_schedule("s1", "recurring", "echo", 0);
        s.cron = Some("0 * * * * *".to_string()); // every minute
        store.add(&s).await.unwrap();

        let scheduler = Scheduler::new(store.clone(), skills, 1, 4);
        scheduler.poll_and_execute().await.unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let schedules = store.list(true).await.unwrap();
        assert_eq!(schedules.len(), 1);
        assert!(schedules[0].next_run > 0); // Updated to future
    }
}
