use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Direction of improvement for a numeric evaluation metric.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonDirection {
    /// Larger metric values are better.
    HigherIsBetter,
    /// Smaller metric values are better.
    LowerIsBetter,
}

/// Numeric metric extraction and comparison configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluationMetricConfig {
    /// Human-readable metric name.
    pub name: String,
    /// Optional regular expression used to extract the first capture group as a
    /// floating-point value from combined stdout/stderr.
    pub regex: Option<String>,
    /// Improvement direction.
    pub direction: ComparisonDirection,
    /// Optional baseline value to compare against. When absent, the run is
    /// accepted on verification success alone.
    pub baseline_value: Option<f64>,
    /// Optional minimum improvement threshold.
    pub min_improvement: Option<f64>,
}

/// Mutable input used to create a research campaign.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateResearchCampaign {
    /// Human-readable campaign name.
    pub name: String,
    /// Workspace name associated with the campaign.
    pub workspace: String,
    /// Repository working directory where the campaign should operate.
    pub repo_path: String,
    /// Git ref used as the worktree base.
    pub baseline_ref: String,
    /// Imperative coding task passed to `coding_delegate`.
    pub task: String,
    /// Optional additional context appended to the coding prompt.
    pub context: Option<String>,
    /// Verification command executed in the candidate worktree.
    pub verification_command: String,
    /// Paths that the campaign is allowed to modify.
    pub editable_paths: Vec<String>,
    /// Optional metric extraction/comparison config.
    pub metric: Option<EvaluationMetricConfig>,
    /// Optional cron expression for recurring runs.
    pub cron: Option<String>,
    /// Target branch used for promotion.
    pub target_branch: String,
}

/// Stored campaign definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchCampaign {
    /// Stable campaign identifier.
    pub id: String,
    /// Human-readable campaign name.
    pub name: String,
    /// Workspace name.
    pub workspace: String,
    /// Repository working directory.
    pub repo_path: String,
    /// Baseline git ref.
    pub baseline_ref: String,
    /// Coding task delegated to the coding backend.
    pub task: String,
    /// Optional extra prompt context.
    pub context: Option<String>,
    /// Verification command executed after code changes.
    pub verification_command: String,
    /// Explicit editable path allowlist.
    pub editable_paths: Vec<String>,
    /// Optional metric config.
    pub metric: Option<EvaluationMetricConfig>,
    /// Optional cron schedule used by the scheduler.
    pub cron: Option<String>,
    /// Scheduler schedule ID, when provisioned.
    pub schedule_id: Option<String>,
    /// Target branch for promotion.
    pub target_branch: String,
    /// Whether the campaign is currently active.
    pub active: bool,
    /// Most recent run ID.
    pub last_run_id: Option<String>,
    /// Current best candidate ID.
    pub best_candidate_id: Option<String>,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Update timestamp.
    pub updated_at: DateTime<Utc>,
}

/// Lifecycle state of a campaign run.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResearchRunStatus {
    /// Run has been created but execution has not started.
    Queued,
    /// Run is currently executing.
    Running,
    /// Run finished successfully.
    Completed,
    /// Run failed.
    Failed,
}

/// Result of an evaluation command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluationResult {
    /// Command that was executed.
    pub command: String,
    /// Working directory used during execution.
    pub working_dir: String,
    /// Exit code returned by the command.
    pub exit_code: Option<i32>,
    /// Captured stdout.
    pub stdout: String,
    /// Captured stderr.
    pub stderr: String,
    /// Execution duration in milliseconds.
    pub duration_ms: u64,
    /// Whether the evaluation was successful.
    pub success: bool,
    /// Optional metric name.
    pub metric_name: Option<String>,
    /// Optional metric value extracted from output.
    pub metric_value: Option<f64>,
}

/// Candidate disposition.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CandidateStatus {
    /// Candidate passed verification and remains eligible.
    Kept,
    /// Candidate failed verification or comparison.
    Discarded,
    /// Candidate has been promoted into the target branch.
    Promoted,
}

/// Stored candidate artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchCandidate {
    /// Stable candidate identifier.
    pub id: String,
    /// Parent campaign.
    pub campaign_id: String,
    /// Originating run.
    pub run_id: String,
    /// Candidate branch.
    pub branch: String,
    /// Candidate worktree path.
    pub worktree_path: String,
    /// Backend selected by `coding_delegate`.
    pub backend: Option<String>,
    /// Normalized coding backend summary.
    pub coding_summary: String,
    /// Git diff summary.
    pub diff_summary: String,
    /// Evaluation outcome.
    pub evaluation: EvaluationResult,
    /// Positive improvement amount when available.
    pub improvement: Option<f64>,
    /// Current disposition.
    pub status: CandidateStatus,
    /// Optional promotion timestamp.
    pub promoted_at: Option<DateTime<Utc>>,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
}

/// Promotion request lifecycle state.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PromotionRequestStatus {
    /// Waiting for explicit human approval.
    Pending,
    /// Approved and applied successfully.
    Approved,
    /// Rejected by a human operator.
    Rejected,
}

/// Persistent HITL request for promoting a candidate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchPromotionRequest {
    /// Stable request identifier.
    pub id: String,
    /// Candidate under review.
    pub candidate_id: String,
    /// Parent campaign.
    pub campaign_id: String,
    /// Branch targeted for promotion.
    pub target_branch: String,
    /// Current approval state.
    pub status: PromotionRequestStatus,
    /// Optional rejection reason.
    pub reason: Option<String>,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Resolution timestamp.
    pub resolved_at: Option<DateTime<Utc>>,
}

/// Result of submitting a promotion attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum PromotionSubmission {
    /// The candidate was promoted immediately.
    Promoted {
        /// The promoted candidate artifact.
        candidate: Box<ResearchCandidate>,
    },
    /// The candidate now waits for human approval.
    ApprovalRequired {
        /// The pending promotion request.
        request: ResearchPromotionRequest,
    },
}

/// Stored run record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchRun {
    /// Stable run identifier.
    pub id: String,
    /// Parent campaign.
    pub campaign_id: String,
    /// Current run status.
    pub status: ResearchRunStatus,
    /// Branch created for this run.
    pub branch: Option<String>,
    /// Worktree created for this run.
    pub worktree_path: Option<String>,
    /// Candidate generated by this run.
    pub candidate_id: Option<String>,
    /// Evaluation result.
    pub evaluation: Option<EvaluationResult>,
    /// Optional failure message.
    pub error: Option<String>,
    /// Free-form metadata for future extensions.
    pub metadata: HashMap<String, serde_json::Value>,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Start timestamp.
    pub started_at: Option<DateTime<Utc>>,
    /// Completion timestamp.
    pub finished_at: Option<DateTime<Utc>>,
}
