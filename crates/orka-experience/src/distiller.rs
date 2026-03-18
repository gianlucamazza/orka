use std::sync::Arc;

use chrono::Utc;
use orka_core::Result;
use orka_llm::client::{ChatMessage, CompletionOptions, LlmClient};
use tracing::{debug, warn};
use uuid::Uuid;

use crate::types::{Principle, PrincipleKind, Trajectory};
use crate::utils::extract_json_array;

const DISTILLATION_SYSTEM_PROMPT: &str = "\
You are a meta-reflection engine for an AI agent orchestration system. \
You receive a batch of interaction trajectories and must synthesize cross-cutting patterns.

Unlike single-trajectory reflection, your goal is to identify:
- Patterns that appear across multiple interactions (successes and failures)
- Systematic strengths or weaknesses in the agent's approach
- Skills or strategies that consistently help or hinder task completion

For each principle, output a JSON object with:
- \"text\": a concise, actionable statement (1-2 sentences)
- \"kind\": \"do\" (something that works well) or \"avoid\" (something that fails or hinders)

Output a JSON array of principle objects. Focus on patterns observed in at least 2 trajectories. \
If no clear patterns emerge, output an empty array [].

Examples:
[{\"text\": \"For queries involving real-time data, always chain web_search before summarize to ensure current information.\", \"kind\": \"do\"},
 {\"text\": \"Avoid calling shell_exec without first checking if a safer skill exists — it consistently triggers permission errors.\", \"kind\": \"avoid\"}]";

/// Synthesizes principles from a batch of trajectories using an LLM.
///
/// Unlike [`crate::reflector::PrincipleReflector`] which reflects on a single trajectory,
/// `Distiller` identifies patterns across multiple interactions.
pub struct Distiller {
    llm: Arc<dyn LlmClient>,
    model: Option<String>,
    max_tokens: u32,
}

impl Distiller {
    /// Create a new distiller with the given LLM client and generation settings.
    pub fn new(llm: Arc<dyn LlmClient>, model: Option<String>, max_tokens: u32) -> Self {
        Self {
            llm,
            model,
            max_tokens,
        }
    }

    /// Distill principles from a batch of trajectories.
    pub async fn distill(
        &self,
        trajectories: &[Trajectory],
        workspace: &str,
    ) -> Result<Vec<Principle>> {
        if trajectories.is_empty() {
            return Ok(Vec::new());
        }

        let prompt = build_distillation_prompt(trajectories);
        let messages = vec![ChatMessage::user(prompt)];

        let mut options = CompletionOptions::default();
        options.model = self.model.clone();
        options.max_tokens = Some(self.max_tokens);

        let response = self
            .llm
            .complete_with_options(messages, DISTILLATION_SYSTEM_PROMPT, options)
            .await?;

        let principles = parse_principles(&response, workspace);

        debug!(
            trajectory_count = trajectories.len(),
            principles = principles.len(),
            workspace,
            "distillation completed"
        );

        Ok(principles)
    }
}

fn build_distillation_prompt(trajectories: &[Trajectory]) -> String {
    let success_count = trajectories.iter().filter(|t| t.success).count();
    let failure_count = trajectories.len() - success_count;

    let mut prompt = String::new();
    prompt.push_str("## Trajectory Batch\n\n");
    prompt.push_str(&format!(
        "Total: {} interactions ({} successful, {} failed)\n\n",
        trajectories.len(),
        success_count,
        failure_count
    ));

    for (i, t) in trajectories.iter().enumerate() {
        prompt.push_str(&format!(
            "### Trajectory {} — {}\n",
            i + 1,
            if t.success { "SUCCESS" } else { "FAILURE" }
        ));
        prompt.push_str(&format!("- Workspace: {}\n", t.workspace));
        prompt.push_str(&format!(
            "- Iterations: {}, Tokens: {}, Duration: {}ms\n",
            t.iterations, t.total_tokens, t.duration_ms
        ));

        // Truncate long messages
        let msg = if t.user_message.len() > 200 {
            format!("{}...", &t.user_message[..200])
        } else {
            t.user_message.clone()
        };
        prompt.push_str(&format!("- User: {}\n", msg));

        if !t.skills_used.is_empty() {
            let skills: Vec<String> = t
                .skills_used
                .iter()
                .map(|s| format!("{}({})", s.name, if s.success { "ok" } else { "fail" }))
                .collect();
            prompt.push_str(&format!("- Skills: {}\n", skills.join(", ")));
        }

        if !t.errors.is_empty() {
            prompt.push_str(&format!("- Errors: {}\n", t.errors.join("; ")));
        }

        prompt.push('\n');
    }

    prompt.push_str("Identify patterns across these trajectories and extract reusable principles as a JSON array.");
    prompt
}

fn parse_principles(response: &str, workspace: &str) -> Vec<Principle> {
    let json_str = extract_json_array(response);

    let parsed: Vec<RawPrinciple> = match serde_json::from_str(&json_str) {
        Ok(v) => v,
        Err(e) => {
            warn!(%e, "failed to parse distillation response as JSON");
            return Vec::new();
        }
    };

    parsed
        .into_iter()
        .filter(|p| !p.text.is_empty())
        .map(|p| Principle {
            id: Uuid::now_v7().to_string(),
            text: p.text,
            kind: match p.kind.as_deref() {
                Some("avoid") => PrincipleKind::Avoid,
                _ => PrincipleKind::Do,
            },
            scope: workspace.to_string(),
            created_at: Utc::now(),
            reinforcement_count: 0,
            relevance_score: 0.0,
        })
        .collect()
}

#[derive(serde::Deserialize)]
struct RawPrinciple {
    text: String,
    kind: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SkillTrace;
    use chrono::Utc;

    fn make_trajectory(success: bool, skills: &[(&str, bool)]) -> Trajectory {
        Trajectory {
            id: Uuid::now_v7().to_string(),
            session_id: "sess".into(),
            workspace: "default".into(),
            timestamp: Utc::now(),
            user_message: "search for something".into(),
            agent_response: "done".into(),
            skills_used: skills
                .iter()
                .map(|(name, ok)| SkillTrace {
                    name: name.to_string(),
                    duration_ms: 100,
                    success: *ok,
                    error_category: None,
                    error_message: None,
                })
                .collect(),
            iterations: 1,
            total_tokens: 500,
            success,
            duration_ms: 200,
            errors: if success { vec![] } else { vec!["err".into()] },
        }
    }

    #[test]
    fn prompt_includes_summary() {
        let trajectories = vec![
            make_trajectory(true, &[("web_search", true)]),
            make_trajectory(false, &[("shell_exec", false)]),
        ];
        let prompt = build_distillation_prompt(&trajectories);
        assert!(prompt.contains("2 interactions"));
        assert!(prompt.contains("1 successful"));
        assert!(prompt.contains("1 failed"));
        assert!(prompt.contains("web_search"));
        assert!(prompt.contains("shell_exec"));
    }

    #[test]
    fn parse_principles_valid() {
        let response = r#"[{"text": "Always chain search before summarize", "kind": "do"}, {"text": "Avoid shell without permission check", "kind": "avoid"}]"#;
        let principles = parse_principles(response, "default");
        assert_eq!(principles.len(), 2);
        assert_eq!(principles[0].kind, PrincipleKind::Do);
        assert_eq!(principles[1].kind, PrincipleKind::Avoid);
        assert_eq!(principles[0].scope, "default");
    }

    #[test]
    fn parse_principles_empty() {
        let principles = parse_principles("[]", "default");
        assert!(principles.is_empty());
    }

    #[test]
    fn distill_empty_trajectories_returns_early() {
        // Just tests the sync path — async tested in integration tests
        let prompt = build_distillation_prompt(&[]);
        // Should still build a prompt (0 trajectories)
        assert!(prompt.contains("0 interactions"));
    }
}
