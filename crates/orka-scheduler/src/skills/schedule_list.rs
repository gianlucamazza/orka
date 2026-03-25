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

#[cfg(test)]
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

    async fn store_with_schedules(names: &[&str]) -> Arc<InMemoryScheduleStore> {
        let store = Arc::new(InMemoryScheduleStore::new());
        for (i, name) in names.iter().enumerate() {
            store
                .add(&Schedule {
                    id: format!("id-{i}"),
                    name: name.to_string(),
                    cron: Some("0 * * * * *".into()),
                    run_at: None,
                    timezone: None,
                    skill: None,
                    args: None,
                    message: None,
                    next_run: i64::MAX,
                    created_at: Utc::now().to_rfc3339(),
                    completed: false,
                })
                .await
                .unwrap();
        }
        store
    }

    #[tokio::test]
    async fn list_returns_all_schedules() {
        let store = store_with_schedules(&["task-a", "task-b"]).await;
        let skill = ScheduleListSkill::new(store);

        let output = skill.execute(args(serde_json::json!({}))).await.unwrap();
        assert_eq!(output.data["count"], 2);
        assert_eq!(output.data["schedules"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn list_empty_store() {
        let store = Arc::new(InMemoryScheduleStore::new());
        let skill = ScheduleListSkill::new(store);

        let output = skill.execute(args(serde_json::json!({}))).await.unwrap();
        assert_eq!(output.data["count"], 0);
        assert!(output.data["schedules"].as_array().unwrap().is_empty());
    }
}
