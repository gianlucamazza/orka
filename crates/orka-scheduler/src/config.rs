//! Scheduler configuration owned by `orka-scheduler`.

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
