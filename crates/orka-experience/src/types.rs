use chrono::{DateTime, Utc};
use orka_core::ErrorCategory;
use serde::{Deserialize, Serialize};

/// A structured trajectory record aggregated from domain events during a single
/// handler invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trajectory {
    /// Unique trajectory identifier (UUID v7).
    pub id: String,
    /// Session that produced this trajectory.
    pub session_id: String,
    /// Workspace in which the interaction took place.
    pub workspace: String,
    /// When the interaction completed.
    pub timestamp: DateTime<Utc>,
    /// The user's original message.
    pub user_message: String,
    /// The agent's final response.
    pub agent_response: String,
    /// Skills invoked during this interaction.
    pub skills_used: Vec<SkillTrace>,
    /// Total LLM iterations in the agent loop.
    pub iterations: usize,
    /// Total tokens consumed.
    pub total_tokens: u64,
    /// Whether the overall interaction succeeded (no unrecovered errors).
    pub success: bool,
    /// Total duration in milliseconds.
    pub duration_ms: u64,
    /// Error messages encountered, if any.
    pub errors: Vec<String>,
}

/// A record of a single skill invocation within a trajectory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillTrace {
    /// Skill name.
    pub name: String,
    /// How long the skill took to execute, in milliseconds.
    pub duration_ms: u64,
    /// Whether the skill invocation succeeded.
    pub success: bool,
    /// Error category, if the skill failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_category: Option<ErrorCategory>,
    /// Error message, if the skill failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

/// Structural action recommended by the reflection system.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action")]
pub enum StructuralAction {
    /// Disable a skill via the circuit breaker.
    #[serde(rename = "disable_skill")]
    DisableSkill {
        /// Name of the skill to disable.
        skill_name: String,
        /// Human-readable reason for disabling.
        reason: String,
    },
}

/// A principle extracted from trajectory reflection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Principle {
    /// Unique principle identifier (UUID v7).
    pub id: String,
    /// Concise, actionable principle text.
    pub text: String,
    /// The category of principle: "do" (positive) or "avoid" (negative).
    pub kind: PrincipleKind,
    /// The workspace this principle applies to, or "global".
    pub scope: String,
    /// When this principle was first created.
    pub created_at: DateTime<Utc>,
    /// How many times this principle has been reinforced by subsequent
    /// reflections.
    pub reinforcement_count: u32,
    /// Relevance score from the last retrieval (transient, not stored).
    #[serde(skip)]
    pub relevance_score: f32,
}

pub use orka_core::PrincipleKind;

/// Outcome signal for a completed interaction, used to decide whether to
/// reflect.
#[derive(Debug, Clone)]
pub enum OutcomeSignal {
    /// At least one skill or the overall handler failed.
    Failure,
    /// All skills succeeded and a response was produced.
    Success,
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::field_reassign_with_default,
    clippy::default_trait_access,
    clippy::needless_pass_by_value,
    clippy::stable_sort_primitive,
    clippy::too_many_lines
)]
mod tests {
    use super::*;

    fn sample_trajectory() -> Trajectory {
        Trajectory {
            id: "t1".into(),
            session_id: "s1".into(),
            workspace: "default".into(),
            timestamp: Utc::now(),
            user_message: "hello".into(),
            agent_response: "world".into(),
            skills_used: vec![SkillTrace {
                name: "web_search".into(),
                duration_ms: 100,
                success: true,
                error_category: None,
                error_message: None,
            }],
            iterations: 2,
            total_tokens: 500,
            success: true,
            duration_ms: 1500,
            errors: vec![],
        }
    }

    #[test]
    fn trajectory_serde_roundtrip() {
        let t = sample_trajectory();
        let json = serde_json::to_string(&t).unwrap();
        let back: Trajectory = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "t1");
        assert_eq!(back.skills_used.len(), 1);
        assert!(back.success);
    }

    #[test]
    fn skill_trace_serde_optional_fields() {
        let trace = SkillTrace {
            name: "echo".into(),
            duration_ms: 50,
            success: true,
            error_category: None,
            error_message: None,
        };
        let json = serde_json::to_string(&trace).unwrap();
        // Optional fields should be omitted
        assert!(!json.contains("error_category"));
        assert!(!json.contains("error_message"));
    }

    #[test]
    fn principle_kind_serde() {
        let do_json = serde_json::to_string(&PrincipleKind::Do).unwrap();
        assert_eq!(do_json, "\"do\"");
        let avoid_json = serde_json::to_string(&PrincipleKind::Avoid).unwrap();
        assert_eq!(avoid_json, "\"avoid\"");
        let back: PrincipleKind = serde_json::from_str("\"do\"").unwrap();
        assert_eq!(back, PrincipleKind::Do);
    }

    #[test]
    fn structural_action_serde() {
        let action = StructuralAction::DisableSkill {
            skill_name: "broken".into(),
            reason: "keeps failing".into(),
        };
        let json = serde_json::to_string(&action).unwrap();
        assert!(json.contains("disable_skill"));
        let back: StructuralAction = serde_json::from_str(&json).unwrap();
        let StructuralAction::DisableSkill { skill_name, .. } = back;
        assert_eq!(skill_name, "broken");
    }
}
