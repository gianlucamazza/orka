use std::sync::Arc;

use async_trait::async_trait;
use orka_core::{ErrorCategory, Result, SkillInput, SkillOutput, SkillSchema, traits::Skill};

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
    fn name(&self) -> &'static str {
        "schedule_delete"
    }

    fn category(&self) -> &'static str {
        "schedule"
    }

    fn description(&self) -> &'static str {
        "Delete a scheduled task by ID or name."
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
            return Err(orka_core::Error::SkillCategorized {
                message: "either 'id' or 'name' is required".into(),
                category: ErrorCategory::Input,
            });
        };

        let deleted = self.store.remove(&id).await?;

        Ok(SkillOutput::new(serde_json::json!({
            "deleted": deleted,
            "id": id,
        })))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::needless_pass_by_value)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use chrono::Utc;
    use orka_core::{SkillInput, traits::Skill};

    use super::*;
    use crate::{InMemoryScheduleStore, types::Schedule};

    fn args(json: serde_json::Value) -> SkillInput {
        let map = json
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<HashMap<_, _>>();
        SkillInput::new(map)
    }

    async fn store_with_schedule(id: &str, name: &str) -> Arc<InMemoryScheduleStore> {
        let store = Arc::new(InMemoryScheduleStore::new());
        store
            .add(&Schedule {
                id: id.to_string(),
                name: name.to_string(),
                cron: None,
                run_at: Some("2099-01-01T00:00:00Z".into()),
                timezone: None,
                skill: None,
                args: None,
                message: None,
                next_run: 4_102_444_800,
                created_at: Utc::now().to_rfc3339(),
                completed: false,
            })
            .await
            .unwrap();
        store
    }

    #[tokio::test]
    async fn delete_by_id_succeeds() {
        let store = store_with_schedule("sched-1", "my-task").await;
        let skill = ScheduleDeleteSkill::new(store.clone());

        let output = skill
            .execute(args(serde_json::json!({"id": "sched-1"})))
            .await
            .unwrap();
        assert_eq!(output.data["deleted"], true);
        assert!(store.list(true).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn delete_by_name_succeeds() {
        let store = store_with_schedule("sched-2", "named-task").await;
        let skill = ScheduleDeleteSkill::new(store.clone());

        let output = skill
            .execute(args(serde_json::json!({"name": "named-task"})))
            .await
            .unwrap();
        assert_eq!(output.data["deleted"], true);
        assert!(store.list(true).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn delete_nonexistent_name_returns_not_found() {
        let store = Arc::new(InMemoryScheduleStore::new());
        let skill = ScheduleDeleteSkill::new(store);

        let output = skill
            .execute(args(serde_json::json!({"name": "ghost"})))
            .await
            .unwrap();
        assert_eq!(output.data["deleted"], false);
        assert!(output.data["reason"].as_str().unwrap().contains("ghost"));
    }

    #[tokio::test]
    async fn delete_missing_id_and_name_fails() {
        let store = Arc::new(InMemoryScheduleStore::new());
        let skill = ScheduleDeleteSkill::new(store);
        assert!(skill.execute(args(serde_json::json!({}))).await.is_err());
    }
}
