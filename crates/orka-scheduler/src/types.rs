use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A scheduled task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schedule {
    pub id: String,
    pub name: String,
    pub cron: Option<String>,
    pub run_at: Option<String>,
    pub timezone: Option<String>,
    pub skill: Option<String>,
    pub args: Option<HashMap<String, serde_json::Value>>,
    pub message: Option<String>,
    pub next_run: i64,
    pub created_at: String,
    pub completed: bool,
}
