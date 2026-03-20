use async_trait::async_trait;
use orka_core::traits::Skill;
use orka_core::{Result, SkillInput, SkillOutput, SkillSchema};
use std::sync::Arc;

use crate::store::ScheduleStore;

/// Skill that deletes a schedule by name or ID from the store.
pub struct ScheduleDeleteSkill {
    store: Arc<dyn ScheduleStore>,
}

impl ScheduleDeleteSkill {
    /// Create a new skill backed by the given schedule store.
    pub fn new(store: Arc<dyn ScheduleStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Skill for ScheduleDeleteSkill {
    fn name(&self) -> &str {
        "schedule_delete"
    }

    fn description(&self) -> &str {
        "Delete a scheduled task by ID or name"
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Schedule ID to delete"
                },
                "name": {
                    "type": "string",
                    "description": "Schedule name to delete"
                }
            }
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let id = if let Some(id) = input.args.get("id").and_then(|v| v.as_str()) {
            id.to_string()
        } else if let Some(name) = input.args.get("name").and_then(|v| v.as_str()) {
            // Look up by name
            match self.store.find_by_name(name).await? {
                Some(schedule) => schedule.id,
                None => {
                    return Ok(SkillOutput::new(serde_json::json!({
                        "deleted": false,
                        "reason": format!("schedule with name '{}' not found", name),
                    })));
                }
            }
        } else {
            return Err(orka_core::Error::Skill(
                "either 'id' or 'name' is required".into(),
            ));
        };

        let deleted = self.store.remove(&id).await?;

        Ok(SkillOutput::new(serde_json::json!({
            "deleted": deleted,
            "id": id,
        })))
    }
}
