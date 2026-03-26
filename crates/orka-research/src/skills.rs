#![allow(clippy::unnecessary_literal_bound)]

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use orka_core::{
    Error, ErrorCategory, Result, SkillInput, SkillOutput, SkillSchema, traits::Skill,
};

use crate::{
    service::ResearchService,
    types::{CandidateStatus, ComparisonDirection, EvaluationResult},
    util::extract_metric,
};

/// Skill that executes a verification command and optionally extracts a metric.
pub struct ExperimentRunSkill {
    service: Arc<ResearchService>,
}

impl ExperimentRunSkill {
    /// Create a new experiment runner.
    pub fn new(service: Arc<ResearchService>) -> Self {
        Self { service }
    }
}

#[async_trait]
impl Skill for ExperimentRunSkill {
    fn name(&self) -> &'static str {
        "experiment_run"
    }

    fn category(&self) -> &'static str {
        "research"
    }

    fn description(&self) -> &'static str {
        "Run a verification command in a candidate worktree and extract an optional numeric metric."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "required": ["command"],
            "properties": {
                "command": { "type": "string" },
                "working_dir": { "type": "string" },
                "metric_name": { "type": ["string", "null"] },
                "metric_regex": { "type": ["string", "null"] }
            }
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let command = input
            .args
            .get("command")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| Error::SkillCategorized {
                message: "missing 'command' argument".into(),
                category: ErrorCategory::Input,
            })?;
        let working_dir = input
            .args
            .get("working_dir")
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                input
                    .context
                    .as_ref()
                    .and_then(|ctx| ctx.worktree_cwd.as_deref())
            })
            .or_else(|| {
                input
                    .context
                    .as_ref()
                    .and_then(|ctx| ctx.user_cwd.as_deref())
            })
            .ok_or_else(|| Error::SkillCategorized {
                message: "missing working directory for experiment_run".into(),
                category: ErrorCategory::Input,
            })?
            .to_string();
        let metric_name = input
            .args
            .get("metric_name")
            .and_then(serde_json::Value::as_str)
            .map(String::from);
        let metric_regex = input
            .args
            .get("metric_regex")
            .and_then(serde_json::Value::as_str);

        let shell = self
            .service
            .registry()?
            .invoke(
                "shell_exec",
                SkillInput::new(HashMap::from([
                    ("command".into(), serde_json::json!(command)),
                    ("cwd".into(), serde_json::json!(working_dir)),
                ]))
                .with_context(orka_core::SkillContext::new(
                    input
                        .context
                        .as_ref()
                        .map(|ctx| ctx.secrets.clone())
                        .ok_or_else(|| Error::SkillCategorized {
                            message: "missing skill context for experiment_run".into(),
                            category: ErrorCategory::Environmental,
                        })?,
                    input
                        .context
                        .as_ref()
                        .and_then(|ctx| ctx.event_sink.clone()),
                )),
            )
            .await?;

        let stdout = shell.data["stdout"].as_str().unwrap_or("").to_string();
        let stderr = shell.data["stderr"].as_str().unwrap_or("").to_string();
        let combined = format!("{stdout}\n{stderr}");
        let metric_value = extract_metric(&combined, metric_regex)?;

        Ok(SkillOutput::new(serde_json::to_value(EvaluationResult {
            command: command.to_string(),
            working_dir,
            exit_code: shell.data["exit_code"].as_i64().map(|value| value as i32),
            stdout,
            stderr,
            duration_ms: shell.data["duration_ms"].as_u64().unwrap_or(0),
            success: shell.data["exit_code"].as_i64().unwrap_or(1) == 0,
            metric_name,
            metric_value,
        })?))
    }
}

/// Pure comparison utility for numeric candidate metrics.
pub struct CandidateCompareSkill;

#[async_trait]
impl Skill for CandidateCompareSkill {
    fn name(&self) -> &'static str {
        "candidate_compare"
    }

    fn category(&self) -> &'static str {
        "research"
    }

    fn description(&self) -> &'static str {
        "Compare baseline and candidate metrics deterministically and report whether the candidate should be kept."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "required": ["baseline", "candidate", "direction"],
            "properties": {
                "baseline": { "type": "number" },
                "candidate": { "type": "number" },
                "direction": { "type": "string", "enum": ["higher_is_better", "lower_is_better"] },
                "min_improvement": { "type": "number" }
            }
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let baseline = input
            .args
            .get("baseline")
            .and_then(serde_json::Value::as_f64)
            .ok_or_else(|| Error::SkillCategorized {
                message: "missing 'baseline'".into(),
                category: ErrorCategory::Input,
            })?;
        let candidate = input
            .args
            .get("candidate")
            .and_then(serde_json::Value::as_f64)
            .ok_or_else(|| Error::SkillCategorized {
                message: "missing 'candidate'".into(),
                category: ErrorCategory::Input,
            })?;
        let direction = match input
            .args
            .get("direction")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("higher_is_better")
        {
            "lower_is_better" => ComparisonDirection::LowerIsBetter,
            _ => ComparisonDirection::HigherIsBetter,
        };
        let min_improvement = input
            .args
            .get("min_improvement")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0);

        let improvement = match direction {
            ComparisonDirection::HigherIsBetter => candidate - baseline,
            ComparisonDirection::LowerIsBetter => baseline - candidate,
        };
        let keep = improvement >= min_improvement;

        Ok(SkillOutput::new(serde_json::json!({
            "improvement": improvement,
            "keep": keep,
            "status": if keep { CandidateStatus::Kept } else { CandidateStatus::Discarded },
        })))
    }
}

/// Skill wrapper that promotes a candidate through the research service.
pub struct ResearchPromoteSkill {
    service: Arc<ResearchService>,
}

impl ResearchPromoteSkill {
    /// Create a new promotion skill.
    pub fn new(service: Arc<ResearchService>) -> Self {
        Self { service }
    }
}

#[async_trait]
impl Skill for ResearchPromoteSkill {
    fn name(&self) -> &'static str {
        "research_promote"
    }

    fn category(&self) -> &'static str {
        "research"
    }

    fn description(&self) -> &'static str {
        "Promote a previously kept research candidate into the campaign target branch."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "required": ["candidate_id"],
            "properties": {
                "candidate_id": { "type": "string" },
                "approved": { "type": "boolean", "default": false }
            }
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let candidate_id = input
            .args
            .get("candidate_id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| Error::SkillCategorized {
                message: "missing 'candidate_id'".into(),
                category: ErrorCategory::Input,
            })?;
        let approved = input
            .args
            .get("approved")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        Ok(SkillOutput::new(serde_json::to_value(
            self.service
                .submit_promotion(candidate_id, approved)
                .await?,
        )?))
    }
}

/// Scheduler-facing skill that executes a stored campaign by ID.
pub struct ResearchCampaignRunSkill {
    service: Arc<ResearchService>,
}

impl ResearchCampaignRunSkill {
    /// Create a new campaign runner skill.
    pub fn new(service: Arc<ResearchService>) -> Self {
        Self { service }
    }
}

#[async_trait]
impl Skill for ResearchCampaignRunSkill {
    fn name(&self) -> &'static str {
        "research_campaign_run"
    }

    fn category(&self) -> &'static str {
        "research"
    }

    fn description(&self) -> &'static str {
        "Execute a stored research campaign once. Used by the scheduler and management API."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "required": ["campaign_id"],
            "properties": {
                "campaign_id": { "type": "string" }
            }
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let campaign_id = input
            .args
            .get("campaign_id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| Error::SkillCategorized {
                message: "missing 'campaign_id'".into(),
                category: ErrorCategory::Input,
            })?;
        Ok(SkillOutput::new(serde_json::to_value(
            self.service.run_campaign(campaign_id).await?,
        )?))
    }
}
