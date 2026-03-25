//! Task planning types for the `PlanningMode::Adaptive` feature.
//!
//! When `planning_mode` is set to [`PlanningMode::Adaptive`] on an [`Agent`],
//! two synthetic tools are injected into the LLM tool list:
//!
//! - `create_plan` — creates a structured plan and saves it to
//!   [`ExecutionContext`] state.
//! - `update_plan_step` — marks a step as in-progress, completed, failed, or
//!   skipped.
//!
//! The plan is persisted under [`PLAN_SLOT`] so the executor can checkpoint it
//! alongside the rest of the execution state.
//!
//! [`Agent`]: crate::agent::Agent
//! [`ExecutionContext`]: crate::context::ExecutionContext

use serde::{Deserialize, Serialize};

/// Shared slot key used to store the active [`Plan`] in `ExecutionContext`.
pub const PLAN_SLOT: &str = "__orka_plan";

/// Controls when (and whether) an agent generates a task plan.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PlanningMode {
    /// No planning — current behaviour (default).
    #[default]
    None,
    /// Inject `create_plan` and `update_plan_step` tools; the model decides
    /// when to use them.
    Adaptive,
    /// Generate a plan automatically before the first LLM iteration via a
    /// dedicated LLM call, then inject it into the system prompt.
    Always,
}

/// A structured execution plan created by the model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    /// High-level description of the goal.
    pub goal: String,
    /// Ordered list of steps to accomplish the goal.
    pub steps: Vec<PlanStep>,
}

impl Plan {
    /// Returns a human-readable summary suitable for injection into a tool
    /// result or log message.
    pub fn display_summary(&self) -> String {
        let mut out = format!("Plan: {}\n", self.goal);
        for step in &self.steps {
            let status = match &step.status {
                StepStatus::Pending => "[ ]".to_string(),
                StepStatus::InProgress => "[→]".to_string(),
                StepStatus::Completed { summary } if summary.is_empty() => "[✓]".to_string(),
                StepStatus::Completed { summary } => format!("[✓] ({summary})"),
                StepStatus::Failed { summary } => format!("[✗] ({summary})"),
                StepStatus::Skipped { summary } => format!("[~] ({summary})"),
            };
            out.push_str(&format!("  {status} {}: {}\n", step.id, step.description));
        }
        out
    }
}

/// A single step within a [`Plan`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    /// Short unique identifier (e.g. `"s1"`, `"fetch-data"`).
    pub id: String,
    /// Human-readable description of what this step accomplishes.
    pub description: String,
    /// Current execution status.
    pub status: StepStatus,
}

/// Execution status of a [`PlanStep`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum StepStatus {
    /// Not yet started.
    Pending,
    /// Currently executing.
    InProgress,
    /// Successfully completed.
    Completed {
        /// Optional summary of what was accomplished.
        #[serde(default)]
        summary: String,
    },
    /// Execution failed.
    Failed {
        /// Reason for failure.
        #[serde(default)]
        summary: String,
    },
    /// Intentionally skipped.
    Skipped {
        /// Reason for skipping.
        #[serde(default)]
        summary: String,
    },
}

/// Tool definitions injected when `PlanningMode::Adaptive` is active.
pub(crate) fn planning_tools() -> Vec<orka_llm::client::ToolDefinition> {
    vec![
        orka_llm::client::ToolDefinition::new(
            "create_plan",
            "Create a structured step-by-step plan for the current task. \
             Call this at the start of a complex multi-step task to make your \
             approach explicit and trackable.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "goal": {
                        "type": "string",
                        "description": "High-level description of what you are trying to achieve"
                    },
                    "steps": {
                        "type": "array",
                        "description": "Ordered list of steps to accomplish the goal",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": {
                                    "type": "string",
                                    "description": "Short unique step identifier (e.g. \"s1\", \"fetch-data\")"
                                },
                                "description": {
                                    "type": "string",
                                    "description": "What this step does"
                                }
                            },
                            "required": ["id", "description"]
                        }
                    }
                },
                "required": ["goal", "steps"]
            }),
        ),
        orka_llm::client::ToolDefinition::new(
            "update_plan_step",
            "Update the status of a plan step as you make progress. \
             Use this after completing, failing, or skipping a step.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "step_id": {
                        "type": "string",
                        "description": "The id of the step to update"
                    },
                    "status": {
                        "type": "string",
                        "enum": ["in_progress", "completed", "failed", "skipped"],
                        "description": "New status for the step"
                    },
                    "summary": {
                        "type": "string",
                        "description": "Optional summary of what was accomplished or why the step failed/was skipped"
                    }
                },
                "required": ["step_id", "status"]
            }),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_plan() -> Plan {
        Plan {
            goal: "Deploy the service".into(),
            steps: vec![
                PlanStep {
                    id: "s1".into(),
                    description: "Build the binary".into(),
                    status: StepStatus::Completed {
                        summary: "success".into(),
                    },
                },
                PlanStep {
                    id: "s2".into(),
                    description: "Run tests".into(),
                    status: StepStatus::InProgress,
                },
                PlanStep {
                    id: "s3".into(),
                    description: "Push image".into(),
                    status: StepStatus::Pending,
                },
            ],
        }
    }

    #[test]
    fn display_summary_includes_goal_and_steps() {
        let plan = make_plan();
        let s = plan.display_summary();
        assert!(s.contains("Deploy the service"));
        assert!(s.contains("s1"));
        assert!(s.contains("s2"));
        assert!(s.contains("s3"));
    }

    #[test]
    fn plan_serde_roundtrip() {
        let plan = make_plan();
        let json = serde_json::to_string(&plan).unwrap();
        let back: Plan = serde_json::from_str(&json).unwrap();
        assert_eq!(back.goal, plan.goal);
        assert_eq!(back.steps.len(), plan.steps.len());
    }

    #[test]
    fn planning_tools_returns_two_definitions() {
        let tools = planning_tools();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name, "create_plan");
        assert_eq!(tools[1].name, "update_plan_step");
    }

    #[test]
    fn planning_mode_default_is_none() {
        assert_eq!(PlanningMode::default(), PlanningMode::None);
    }
}
