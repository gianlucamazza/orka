use async_trait::async_trait;
use chrono::Utc;
use orka_core::traits::Skill;
use orka_core::{Result, SkillInput, SkillOutput, SkillSchema};
use std::str::FromStr;
use std::sync::Arc;
use uuid::Uuid;

use crate::store::RedisScheduleStore;
use crate::types::Schedule;

pub struct ScheduleCreateSkill {
    store: Arc<RedisScheduleStore>,
}

impl ScheduleCreateSkill {
    pub fn new(store: Arc<RedisScheduleStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Skill for ScheduleCreateSkill {
    fn name(&self) -> &str {
        "schedule_create"
    }

    fn description(&self) -> &str {
        "Create a scheduled task (cron or one-shot) to run a skill at a specific time"
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
            .ok_or_else(|| orka_core::Error::Skill("name is required".into()))?;

        let cron_expr = input.args.get("cron").and_then(|v| v.as_str());
        let run_at = input.args.get("run_at").and_then(|v| v.as_str());

        if cron_expr.is_none() && run_at.is_none() {
            return Err(orka_core::Error::Skill(
                "either 'cron' or 'run_at' is required".into(),
            ));
        }

        let next_run = if let Some(cron_str) = cron_expr {
            let schedule = cron::Schedule::from_str(cron_str)
                .map_err(|e| orka_core::Error::Skill(format!("invalid cron expression: {e}")))?;
            schedule
                .upcoming(Utc)
                .next()
                .map(|t| t.timestamp())
                .ok_or_else(|| orka_core::Error::Skill("no upcoming run for cron".into()))?
        } else if let Some(run_at_str) = run_at {
            chrono::DateTime::parse_from_rfc3339(run_at_str)
                .map(|dt| dt.timestamp())
                .map_err(|e| orka_core::Error::Skill(format!("invalid run_at datetime: {e}")))?
        } else {
            unreachable!()
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
            id: Uuid::new_v4().to_string(),
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
