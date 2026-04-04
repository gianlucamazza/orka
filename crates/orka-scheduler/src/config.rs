//! Scheduler configuration owned by `orka-scheduler`.

use std::str::FromStr as _;

use serde::Deserialize;

/// Cron scheduler configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct SchedulerConfig {
    /// Enable scheduler.
    #[serde(default = "default_scheduler_enabled")]
    pub enabled: bool,
    /// How often (in seconds) to poll for due tasks.
    #[serde(default = "default_scheduler_poll_interval_secs")]
    pub poll_interval_secs: u64,
    /// Maximum number of tasks to execute concurrently.
    #[serde(default = "default_scheduler_max_concurrent")]
    pub max_concurrent: usize,
    /// Scheduled jobs.
    #[serde(default)]
    pub jobs: Vec<ScheduledJob>,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            enabled: default_scheduler_enabled(),
            poll_interval_secs: default_scheduler_poll_interval_secs(),
            max_concurrent: default_scheduler_max_concurrent(),
            jobs: Vec::new(),
        }
    }
}

/// Scheduled job definition.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct ScheduledJob {
    /// Job name.
    pub name: String,
    /// Cron expression.
    pub schedule: String,
    /// Command to execute.
    pub command: String,
    /// Workspace to run in.
    pub workspace: Option<String>,
    /// Enable this job.
    #[serde(default = "default_job_enabled")]
    pub enabled: bool,
}

impl SchedulerConfig {
    /// Validate the scheduler configuration.
    pub fn validate(&self) -> orka_core::Result<()> {
        if self.poll_interval_secs == 0 {
            return Err(orka_core::Error::Config(
                "scheduler.poll_interval_secs must be greater than 0".into(),
            ));
        }
        if self.max_concurrent == 0 {
            return Err(orka_core::Error::Config(
                "scheduler.max_concurrent must be greater than 0".into(),
            ));
        }
        for job in &self.jobs {
            if job.name.is_empty() {
                return Err(orka_core::Error::Config(
                    "scheduler job name must not be empty".into(),
                ));
            }
            if job.command.is_empty() {
                return Err(orka_core::Error::Config(format!(
                    "scheduler job '{}': command must not be empty",
                    job.name
                )));
            }
            if !job.schedule.is_empty() {
                cron::Schedule::from_str(&job.schedule).map_err(|e| {
                    orka_core::Error::Config(format!(
                        "scheduler job '{}': invalid cron expression '{}': {e}",
                        job.name, job.schedule
                    ))
                })?;
            }
        }
        Ok(())
    }
}

// --- Private defaults ---

const fn default_scheduler_enabled() -> bool {
    false
}

const fn default_scheduler_poll_interval_secs() -> u64 {
    30
}

const fn default_scheduler_max_concurrent() -> usize {
    4
}

const fn default_job_enabled() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_job(name: &str, command: &str, schedule: &str) -> ScheduledJob {
        ScheduledJob {
            name: name.to_string(),
            schedule: schedule.to_string(),
            command: command.to_string(),
            workspace: None,
            enabled: true,
        }
    }

    #[test]
    fn default_config_is_valid() {
        assert!(SchedulerConfig::default().validate().is_ok());
    }

    #[test]
    fn poll_interval_zero_is_invalid() {
        let cfg = SchedulerConfig {
            poll_interval_secs: 0,
            ..Default::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn max_concurrent_zero_is_invalid() {
        let cfg = SchedulerConfig {
            max_concurrent: 0,
            ..Default::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn job_with_empty_name_is_invalid() {
        let mut cfg = SchedulerConfig::default();
        cfg.jobs.push(make_job("", "do_something", "0 * * * * *"));
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn job_with_empty_command_is_invalid() {
        let mut cfg = SchedulerConfig::default();
        cfg.jobs.push(make_job("my-job", "", "0 * * * * *"));
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn job_with_invalid_cron_is_invalid() {
        let mut cfg = SchedulerConfig::default();
        cfg.jobs.push(make_job("my-job", "run_it", "not-a-cron"));
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn job_with_empty_schedule_is_valid_oneshot() {
        let mut cfg = SchedulerConfig::default();
        cfg.jobs.push(make_job("oneshot", "do_it", ""));
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn job_with_valid_cron_is_valid() {
        let mut cfg = SchedulerConfig::default();
        cfg.jobs.push(make_job("daily", "report", "0 0 9 * * *"));
        assert!(cfg.validate().is_ok());
    }
}
