use std::sync::Arc;

use async_trait::async_trait;
use orka_core::{Result, SkillInput, SkillOutput, SkillSchema, traits::Skill};

use crate::store::ScheduleStore;

/// Skill that lists all active schedules from the store.
pub struct ScheduleListSkill {
    store: Arc<dyn ScheduleStore>,
}

impl ScheduleListSkill {
    /// Create a new skill backed by the given schedule store.
    pub fn new(store: Arc<dyn ScheduleStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Skill for ScheduleListSkill {
    fn name(&self) -> &str {
        "schedule_list"
    }

    fn category(&self) -> &str {
        "schedule"
    }

    fn description(&self) -> &str {
        "List active scheduled tasks."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "include_completed": {
                    "type": "boolean",
                    "default": false,
                    "description": "Include completed one-shot schedules"
                }
            }
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let include_completed = input
            .args
            .get("include_completed")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let schedules = self.store.list(include_completed).await?;

        Ok(SkillOutput::new(serde_json::json!({
            "schedules": schedules,
            "count": schedules.len(),
        })))
    }
}
