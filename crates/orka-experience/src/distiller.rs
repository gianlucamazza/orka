use std::{fmt::Write as _, sync::Arc};

use chrono::Utc;
use orka_core::Result;
use orka_llm::client::{ChatMessage, CompletionOptions, LlmClient};
use orka_prompts::template::TemplateRegistry;
use tracing::{debug, warn};
use uuid::Uuid;

use crate::{
    types::{Principle, PrincipleKind, Trajectory},
    utils::extract_json_array,
};

/// Default distillation system prompt (used when template registry is not
/// available).
const DEFAULT_DISTILLATION_PROMPT: &str =
    include_str!("../../orka-prompts/templates/system/distillation.hbs");

/// Synthesizes principles from a batch of trajectories using an LLM.
///
/// Unlike [`crate::reflector::PrincipleReflector`] which reflects on a single
/// trajectory, `Distiller` identifies patterns across multiple interactions.
pub struct Distiller {
    llm: Arc<dyn LlmClient>,
    model: Option<String>,
    max_tokens: u32,
    templates: Option<Arc<TemplateRegistry>>,
}

impl Distiller {
    /// Create a new distiller with the given LLM client and generation
    /// settings.
    pub fn new(llm: Arc<dyn LlmClient>, model: Option<String>, max_tokens: u32) -> Self {
        Self {
            llm,
            model,
            max_tokens,
            templates: None,
        }
    }

    /// Set the template registry for prompt rendering.
    #[must_use]
    pub fn with_templates(mut self, templates: Arc<TemplateRegistry>) -> Self {
        self.templates = Some(templates);
        self
    }

    /// Get the system prompt for distillation.
    async fn get_system_prompt(&self) -> String {
        if let Some(templates) = &self.templates
            && templates.has_template("system/distillation").await
        {
            return templates
                .render("system/distillation", &{})
                .await
                .unwrap_or_else(|e| {
                    warn!(error = %e, "failed to render distillation template, using default");
                    DEFAULT_DISTILLATION_PROMPT.to_string()
                });
        }
        DEFAULT_DISTILLATION_PROMPT.to_string()
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

        let system_prompt = self.get_system_prompt().await;

        let response = self
            .llm
            .complete_with_options(messages, &system_prompt, &options)
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
    write!(
        prompt,
        "Total: {} interactions ({} successful, {} failed)\n\n",
        trajectories.len(),
        success_count,
        failure_count
    )
    .unwrap_or(());

    for (i, t) in trajectories.iter().enumerate() {
        writeln!(
            prompt,
            "### Trajectory {} — {}",
            i + 1,
            if t.success { "SUCCESS" } else { "FAILURE" }
        )
        .unwrap_or(());
        writeln!(prompt, "- Workspace: {}", t.workspace).unwrap_or(());
        writeln!(
            prompt,
            "- Iterations: {}, Tokens: {}, Duration: {}ms",
            t.iterations, t.total_tokens, t.duration_ms
        )
        .unwrap_or(());

        // Truncate long messages
        let msg = if t.user_message.len() > 200 {
            let boundary = t.user_message.floor_char_boundary(200);
            format!("{}...", &t.user_message[..boundary])
        } else {
            t.user_message.clone()
        };
        writeln!(prompt, "- User: {msg}").unwrap_or(());

        if !t.skills_used.is_empty() {
            let skills: Vec<String> = t
                .skills_used
                .iter()
                .map(|s| format!("{}({})", s.name, if s.success { "ok" } else { "fail" }))
                .collect();
            writeln!(prompt, "- Skills: {}", skills.join(", ")).unwrap_or(());
        }

        if !t.errors.is_empty() {
            writeln!(prompt, "- Errors: {}", t.errors.join("; ")).unwrap_or(());
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
    use chrono::Utc;

    use super::*;
    use crate::types::SkillTrace;

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
