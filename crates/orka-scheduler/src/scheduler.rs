use crate::store::RedisScheduleStore;
use chrono::Utc;
use std::str::FromStr;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

/// Async poll loop that checks for due tasks and executes them.
pub struct Scheduler {
    store: Arc<RedisScheduleStore>,
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
        store: Arc<RedisScheduleStore>,
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
                _ = cancel.cancelled() => {
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
            let permit = semaphore.clone().acquire_owned().await;
            if permit.is_err() {
                break;
            }
            let permit = permit.unwrap();

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

                // Handle next run: if cron, compute next; if one-shot, remove
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
            });
        }

        Ok(())
    }
}

#[cfg(test)]
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
}
