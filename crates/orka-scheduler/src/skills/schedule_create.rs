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
    fn name(&self) -> &str {
        "schedule_create"
    }

    fn category(&self) -> &str {
        "schedule"
    }

    fn description(&self) -> &str {
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
