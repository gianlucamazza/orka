use std::{str::FromStr, sync::Arc};

use async_trait::async_trait;
use chrono::Utc;
use orka_core::{ErrorCategory, Result, SkillInput, SkillOutput, SkillSchema, traits::Skill};
use uuid::Uuid;

use crate::{store::ScheduleStore, types::Schedule};

/// Skill that creates a new schedule entry in the store.
pub struct ScheduleCreateSkill {
    store: Arc<dyn ScheduleStore>,
}

impl ScheduleCreateSkill {
    /// Create a new skill backed by the given schedule store.
    pub fn new(store: Arc<dyn ScheduleStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Skill for ScheduleCreateSkill {
    fn name(&self) -> &'static str {
        "schedule_create"
    }

    fn category(&self) -> &'static str {
        "schedule"
    }

    fn description(&self) -> &'static str {
        "Create a scheduled task (cron or one-shot) to run a skill at a specific time."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Name for this schedule"
                },
                "cron": {
                    "type": "string",
                    "description": "Cron expression (e.g. '0 0 9 * * *' for daily at 9am)"
                },
                "run_at": {
                    "type": "string",
                    "description": "ISO 8601 datetime for one-shot execution"
                },
                "skill": {
                    "type": "string",
                    "description": "Skill name to invoke"
                },
                "args": {
                    "type": "object",
                    "description": "Arguments to pass to the skill"
                },
                "message": {
                    "type": "string",
                    "description": "Message to send (alternative to skill)"
                },
                "timezone": {
                    "type": "string",
                    "description": "Timezone (e.g. 'Europe/Rome')"
                }
            },
            "required": ["name"]
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let name = input
            .args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| orka_core::Error::SkillCategorized {
                message: "name is required".into(),
                category: ErrorCategory::Input,
            })?;

        let cron_expr = input.args.get("cron").and_then(|v| v.as_str());
        let run_at = input.args.get("run_at").and_then(|v| v.as_str());

        if cron_expr.is_none() && run_at.is_none() {
            return Err(orka_core::Error::SkillCategorized {
                message: "either 'cron' or 'run_at' is required".into(),
                category: ErrorCategory::Input,
            });
        }

        let next_run = if let Some(cron_str) = cron_expr {
            let schedule = cron::Schedule::from_str(cron_str).map_err(|e| {
                orka_core::Error::SkillCategorized {
                    message: format!("invalid cron expression: {e}"),
                    category: ErrorCategory::Input,
                }
            })?;
            schedule
                .upcoming(Utc)
                .next()
                .map(|t| t.timestamp())
                .ok_or_else(|| orka_core::Error::SkillCategorized {
                    message: "no upcoming run for cron".into(),
                    category: ErrorCategory::Input,
                })?
        } else if let Some(run_at_str) = run_at {
            chrono::DateTime::parse_from_rfc3339(run_at_str)
                .map(|dt| dt.timestamp())
                .map_err(|e| orka_core::Error::SkillCategorized {
                    message: format!("invalid run_at datetime: {e}"),
                    category: ErrorCategory::Input,
                })?
        } else {
            return Err(orka_core::Error::SkillCategorized {
                message: "either 'cron' or 'run_at' must be provided".into(),
                category: ErrorCategory::Input,
            });
        };

        let skill = input
            .args
            .get("skill")
            .and_then(|v| v.as_str())
            .map(String::from);

        let args = input
            .args
            .get("args")
            .and_then(|v| v.as_object())
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect());

        let message = input
            .args
            .get("message")
            .and_then(|v| v.as_str())
            .map(String::from);

        let timezone = input
            .args
            .get("timezone")
            .and_then(|v| v.as_str())
            .map(String::from);

        let schedule = Schedule {
            id: Uuid::now_v7().to_string(),
            name: name.to_string(),
            cron: cron_expr.map(String::from),
            run_at: run_at.map(String::from),
            timezone,
            skill,
            args,
            message,
            next_run,
            created_at: Utc::now().to_rfc3339(),
            completed: false,
        };

        self.store.add(&schedule).await?;

        Ok(SkillOutput::new(serde_json::json!({
            "created": true,
            "id": schedule.id,
            "name": schedule.name,
            "next_run": chrono::DateTime::from_timestamp(next_run, 0)
                .map(|dt| dt.to_rfc3339()),
        })))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::needless_pass_by_value)]
mod tests {
    use std::collections::HashMap;

    use orka_core::{SkillInput, traits::Skill};

    use super::*;
    use crate::InMemoryScheduleStore;

    fn args(json: serde_json::Value) -> SkillInput {
        let map = json
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<HashMap<_, _>>();
        SkillInput::new(map)
    }

    #[tokio::test]
    async fn create_cron_schedule_succeeds() {
        let store = Arc::new(InMemoryScheduleStore::new());
        let skill = ScheduleCreateSkill::new(store.clone());

        let input = args(serde_json::json!({
            "name": "daily-report",
            "cron": "0 0 9 * * *",
        }));
        let output = skill.execute(input).await.unwrap();

        assert_eq!(output.data["created"], true);
        assert_eq!(output.data["name"], "daily-report");
        assert!(output.data["id"].as_str().is_some());

        let schedules = store.list(false).await.unwrap();
        assert_eq!(schedules.len(), 1);
        assert_eq!(schedules[0].name, "daily-report");
        assert!(schedules[0].cron.is_some());
    }

    #[tokio::test]
    async fn create_one_shot_schedule_succeeds() {
        let store = Arc::new(InMemoryScheduleStore::new());
        let skill = ScheduleCreateSkill::new(store.clone());

        let input = args(serde_json::json!({
            "name": "one-time-task",
            "run_at": "2099-06-15T10:00:00Z",
            "skill": "send_email",
        }));
        let output = skill.execute(input).await.unwrap();

        assert_eq!(output.data["created"], true);

        let schedules = store.list(false).await.unwrap();
        assert_eq!(schedules.len(), 1);
        assert!(schedules[0].next_run > 0);
        assert_eq!(schedules[0].skill.as_deref(), Some("send_email"));
    }

    #[tokio::test]
    async fn create_missing_name_fails() {
        let store = Arc::new(InMemoryScheduleStore::new());
        let skill = ScheduleCreateSkill::new(store);
        let input = args(serde_json::json!({"cron": "0 * * * * *"}));
        assert!(skill.execute(input).await.is_err());
    }

    #[tokio::test]
    async fn create_missing_cron_and_run_at_fails() {
        let store = Arc::new(InMemoryScheduleStore::new());
        let skill = ScheduleCreateSkill::new(store);
        let input = args(serde_json::json!({"name": "no-trigger"}));
        assert!(skill.execute(input).await.is_err());
    }

    #[tokio::test]
    async fn create_invalid_cron_fails() {
        let store = Arc::new(InMemoryScheduleStore::new());
        let skill = ScheduleCreateSkill::new(store);
        let input = args(serde_json::json!({
            "name": "bad-cron",
            "cron": "not-a-cron",
        }));
        assert!(skill.execute(input).await.is_err());
    }
}
