use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A scheduled task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schedule {
    /// Unique schedule identifier.
    pub id: String,
    /// Human-readable name used for lookup and display.
    pub name: String,
    /// Cron expression (if this is a recurring schedule).
    pub cron: Option<String>,
    /// ISO-8601 timestamp for a one-shot schedule.
    pub run_at: Option<String>,
    /// IANA timezone name (e.g. `"UTC"`).
    pub timezone: Option<String>,
    /// Name of the skill to invoke when the schedule fires.
    pub skill: Option<String>,
    /// Arguments passed to the skill.
    pub args: Option<HashMap<String, serde_json::Value>>,
    /// Optional plain-text message payload.
    pub message: Option<String>,
    /// Unix timestamp of the next scheduled run.
    pub next_run: i64,
    /// ISO-8601 creation timestamp.
    pub created_at: String,
    /// Whether this one-shot schedule has already fired.
    pub completed: bool,
}
