use chrono::Utc;
use orka_core::ErrorCategory;
use uuid::Uuid;

use crate::types::{OutcomeSignal, SkillTrace, Trajectory};

/// Collects domain event data during a single handler invocation and produces a
/// [`Trajectory`].
///
/// Usage: create at handler start, call record methods as events occur, then
/// call `finish()`.
#[derive(Debug)]
pub struct TrajectoryCollector {
    id: String,
    session_id: String,
    workspace: String,
    user_message: String,
    agent_response: Option<String>,
    skills: Vec<SkillTrace>,
    iterations: usize,
    total_tokens: u64,
    errors: Vec<String>,
    start: std::time::Instant,
}

impl TrajectoryCollector {
    /// Create a new collector for the given session and workspace.
    pub fn new(session_id: String, workspace: String, user_message: String) -> Self {
        Self {
            id: Uuid::now_v7().to_string(),
            session_id,
            workspace,
            user_message,
            agent_response: None,
            skills: Vec::new(),
            iterations: 0,
            total_tokens: 0,
            errors: Vec::new(),
            start: std::time::Instant::now(),
        }
    }

    /// Record a skill invocation.
    pub fn record_skill(
        &mut self,
        name: String,
        duration_ms: u64,
        success: bool,
        error_category: Option<ErrorCategory>,
        error_message: Option<String>,
    ) {
        self.skills.push(SkillTrace {
            name,
            duration_ms,
            success,
            error_category,
            error_message,
        });
    }

    /// Record one agent loop iteration and the tokens it consumed.
    pub fn record_iteration(&mut self, tokens_used: u64) {
        self.iterations += 1;
        self.total_tokens += tokens_used;
    }

    /// Record an error message encountered during the interaction.
    pub fn record_error(&mut self, message: String) {
        self.errors.push(message);
    }

    /// Set the agent's final response text.
    pub fn set_response(&mut self, response: String) {
        self.agent_response = Some(response);
    }

    /// Determine the outcome signal based on collected data.
    pub fn outcome(&self) -> OutcomeSignal {
        let has_skill_failure = self.skills.iter().any(|s| !s.success);
        if !self.errors.is_empty() || has_skill_failure {
            OutcomeSignal::Failure
        } else {
            OutcomeSignal::Success
        }
    }

    /// Consume the collector and produce a finalized trajectory.
    pub fn finish(self) -> Trajectory {
        let success = self.errors.is_empty() && self.skills.iter().all(|s| s.success);
        Trajectory {
            id: self.id,
            session_id: self.session_id,
            workspace: self.workspace,
            timestamp: Utc::now(),
            user_message: self.user_message,
            agent_response: self.agent_response.unwrap_or_default(),
            skills_used: self.skills,
            iterations: self.iterations,
            total_tokens: self.total_tokens,
            success,
            duration_ms: self.start.elapsed().as_millis() as u64,
            errors: self.errors,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collector_produces_trajectory() {
        let mut c = TrajectoryCollector::new("sess-1".into(), "default".into(), "hello".into());
        c.record_skill("web_search".into(), 100, true, None, None);
        c.record_iteration(500);
        c.set_response("Hi there!".into());

        let traj = c.finish();
        assert!(traj.success);
        assert_eq!(traj.iterations, 1);
        assert_eq!(traj.total_tokens, 500);
        assert_eq!(traj.skills_used.len(), 1);
        assert_eq!(traj.agent_response, "Hi there!");
    }

    #[test]
    fn collector_detects_failure() {
        let mut c =
            TrajectoryCollector::new("sess-2".into(), "default".into(), "do something".into());
        c.record_skill("broken_skill".into(), 50, false, None, None);
        c.record_iteration(200);
        assert!(matches!(c.outcome(), OutcomeSignal::Failure));

        let traj = c.finish();
        assert!(!traj.success);
        assert_eq!(traj.errors.len(), 0);
        assert!(!traj.skills_used[0].success);
    }
}
