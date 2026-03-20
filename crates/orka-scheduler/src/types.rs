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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_schedule() -> Schedule {
        Schedule {
            id: "sched-001".into(),
            name: "daily-report".into(),
            cron: Some("0 0 9 * * *".into()),
            run_at: None,
            timezone: Some("Europe/Rome".into()),
            skill: Some("send_report".into()),
            args: Some(HashMap::from([(
                "channel".into(),
                serde_json::json!("general"),
            )])),
            message: None,
            next_run: 1735689600,
            created_at: "2025-01-01T00:00:00Z".into(),
            completed: false,
        }
    }

    #[test]
    fn schedule_json_snapshot() {
        insta::assert_json_snapshot!("schedule", sample_schedule());
    }

    #[test]
    fn schedule_roundtrip() {
        let s = sample_schedule();
        let json = serde_json::to_string(&s).unwrap();
        let parsed: Schedule = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, s.id);
        assert_eq!(parsed.name, s.name);
        assert_eq!(parsed.cron, s.cron);
        assert_eq!(parsed.next_run, s.next_run);
    }
}
