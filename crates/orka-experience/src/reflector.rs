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

/// Default reflection system prompt (used when template registry is not
/// available).
const DEFAULT_REFLECTION_PROMPT: &str =
    include_str!("../../orka-prompts/templates/system/reflection.hbs");

/// Extracts principles from trajectories using an LLM.
pub struct PrincipleReflector {
    llm: Arc<dyn LlmClient>,
    model: Option<String>,
    max_tokens: u32,
    templates: Option<Arc<TemplateRegistry>>,
}

impl PrincipleReflector {
    /// Create a new reflector with the given LLM client and generation
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

    /// Get the system prompt for reflection.
    async fn get_system_prompt(&self) -> String {
        if let Some(templates) = &self.templates
            && templates.has_template("system/reflection").await
        {
            // Render template with empty context (template has no variables)
            return templates
                .render("system/reflection", &{})
                .await
                .unwrap_or_else(|e| {
                    warn!(error = %e, "failed to render reflection template, using default");
                    DEFAULT_REFLECTION_PROMPT.to_string()
                });
        }
        DEFAULT_REFLECTION_PROMPT.to_string()
    }

    /// Reflect on a trajectory and extract principles.
    pub async fn reflect(
        &self,
        trajectory: &Trajectory,
        workspace: &str,
    ) -> Result<Vec<Principle>> {
        let user_prompt = Self::build_reflection_prompt(trajectory);

        let messages = vec![ChatMessage::user(user_prompt)];

        let mut options = CompletionOptions::default();
        options.model = self.model.clone();
        options.max_tokens = Some(self.max_tokens);

        let system_prompt = self.get_system_prompt().await;

        let response = self
            .llm
            .complete_with_options(messages, &system_prompt, options)
            .await?;

        let principles = Self::parse_principles(&response, workspace);

        debug!(
            trajectory_id = %trajectory.id,
            count = principles.len(),
            "reflection produced principles"
        );

        Ok(principles)
    }

    fn build_reflection_prompt(trajectory: &Trajectory) -> String {
        let mut prompt = String::new();
        prompt.push_str("## Interaction Trajectory\n\n");
        writeln!(
            prompt,
            "- **Outcome**: {}",
            if trajectory.success {
                "SUCCESS"
            } else {
                "FAILURE"
            }
        )
        .unwrap_or(());
        writeln!(prompt, "- **Iterations**: {}", trajectory.iterations).unwrap_or(());
        writeln!(prompt, "- **Tokens**: {}", trajectory.total_tokens).unwrap_or(());
        writeln!(prompt, "- **Duration**: {}ms", trajectory.duration_ms).unwrap_or(());
        writeln!(prompt, "- **Workspace**: {}\n", trajectory.workspace).unwrap_or(());

        prompt.push_str("### User Message\n");
        // Truncate very long messages
        let msg = if trajectory.user_message.len() > 500 {
            let boundary = trajectory.user_message.floor_char_boundary(500);
            format!("{}...", &trajectory.user_message[..boundary])
        } else {
            trajectory.user_message.clone()
        };
        prompt.push_str(&msg);
        prompt.push_str("\n\n");

        if !trajectory.skills_used.is_empty() {
            prompt.push_str("### Skills Used\n");
            for skill in &trajectory.skills_used {
                let status = if skill.success { "OK" } else { "FAILED" };
                let mut line = format!("- {} ({}ms, {})", skill.name, skill.duration_ms, status);
                if let Some(cat) = skill.error_category {
                    write!(line, ", category={cat:?}").unwrap_or(());
                }
                if let Some(ref msg) = skill.error_message {
                    write!(line, ", error=\"{msg}\"").unwrap_or(());
                }
                line.push('\n');
                prompt.push_str(&line);
            }
            prompt.push('\n');
        }

        if !trajectory.errors.is_empty() {
            prompt.push_str("### Errors\n");
            for err in &trajectory.errors {
                writeln!(prompt, "- {err}").unwrap_or(());
            }
            prompt.push('\n');
        }

        prompt.push_str("### Agent Response\n");
        let resp = if trajectory.agent_response.len() > 500 {
            format!("{}...", &trajectory.agent_response[..500])
        } else {
            trajectory.agent_response.clone()
        };
        prompt.push_str(&resp);
        prompt.push_str("\n\n");

        prompt.push_str("Analyze this trajectory and extract reusable principles as a JSON array.");

        prompt
    }

    fn parse_principles(response: &str, workspace: &str) -> Vec<Principle> {
        // Extract JSON array from the response (may be wrapped in markdown code blocks)
        let json_str = extract_json_array(response);

        let parsed: Vec<RawPrinciple> = match serde_json::from_str(&json_str) {
            Ok(v) => v,
            Err(e) => {
                warn!(%e, "failed to parse reflection response as JSON");
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
}

#[derive(serde::Deserialize)]
struct RawPrinciple {
    text: String,
    kind: Option<String>,
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::field_reassign_with_default,
    clippy::default_trait_access,
    clippy::needless_pass_by_value,
    clippy::stable_sort_primitive,
    clippy::too_many_lines,
    dead_code
)]
mod tests {
    use super::*;

    #[test]
    fn parse_principles_valid() {
        let response = r#"[{"text": "Use web_search for current info", "kind": "do"}, {"text": "Avoid long queries", "kind": "avoid"}]"#;
        let principles = PrincipleReflector::parse_principles(response, "default");
        assert_eq!(principles.len(), 2);
        assert_eq!(principles[0].kind, PrincipleKind::Do);
        assert_eq!(principles[1].kind, PrincipleKind::Avoid);
    }

    #[test]
    fn parse_principles_empty_array() {
        let principles = PrincipleReflector::parse_principles("[]", "default");
        assert!(principles.is_empty());
    }

    #[test]
    fn build_reflection_prompt_success_trajectory() {
        let trajectory = crate::types::Trajectory {
            id: "t1".into(),
            session_id: "s1".into(),
            workspace: "default".into(),
            timestamp: Utc::now(),
            user_message: "hello".into(),
            agent_response: "world".into(),
            skills_used: vec![],
            iterations: 3,
            total_tokens: 800,
            success: true,
            duration_ms: 2000,
            errors: vec![],
        };
        let prompt = PrincipleReflector::build_reflection_prompt(&trajectory);
        assert!(prompt.contains("SUCCESS"));
        assert!(prompt.contains('3')); // iterations
        assert!(prompt.contains("800")); // tokens
    }

    #[test]
    fn build_reflection_prompt_truncates_long_message() {
        let long_msg = "x".repeat(600);
        let trajectory = crate::types::Trajectory {
            id: "t1".into(),
            session_id: "s1".into(),
            workspace: "default".into(),
            timestamp: Utc::now(),
            user_message: long_msg,
            agent_response: "ok".into(),
            skills_used: vec![],
            iterations: 1,
            total_tokens: 100,
            success: true,
            duration_ms: 500,
            errors: vec![],
        };
        let prompt = PrincipleReflector::build_reflection_prompt(&trajectory);
        // Should be truncated at 500 chars + "..."
        assert!(prompt.contains("..."));
    }

    #[test]
    fn parse_principles_filters_empty_text() {
        let response =
            r#"[{"text": "", "kind": "do"}, {"text": "valid principle", "kind": "avoid"}]"#;
        let principles = PrincipleReflector::parse_principles(response, "default");
        assert_eq!(principles.len(), 1);
        assert_eq!(principles[0].text, "valid principle");
    }

    #[test]
    fn parse_principles_invalid_json() {
        let principles = PrincipleReflector::parse_principles("not json at all", "default");
        assert!(principles.is_empty());
    }

    /// Minimal mock LLM for unit tests (not used in parse tests but needed for
    /// constructor).
    struct MockLlm;

    #[async_trait::async_trait]
    impl LlmClient for MockLlm {
        async fn complete(
            &self,
            _messages: Vec<ChatMessage>,
            _system: &str,
        ) -> orka_core::Result<String> {
            Ok("[]".to_string())
        }

        async fn complete_with_options(
            &self,
            _messages: Vec<ChatMessage>,
            _system: &str,
            _options: CompletionOptions,
        ) -> orka_core::Result<String> {
            Ok("[]".to_string())
        }

        async fn complete_stream(
            &self,
            _messages: Vec<ChatMessage>,
            _system: &str,
        ) -> orka_core::Result<orka_llm::LlmStream> {
            Err(orka_core::Error::Other("not implemented".into()))
        }

        async fn complete_with_tools(
            &self,
            _messages: &[orka_llm::ChatMessage],
            _system: &str,
            _tools: &[orka_llm::ToolDefinition],
            _options: CompletionOptions,
        ) -> orka_core::Result<orka_llm::CompletionResponse> {
            Err(orka_core::Error::Other("not implemented".into()))
        }

        async fn complete_stream_with_tools(
            &self,
            _messages: &[orka_llm::ChatMessage],
            _system: &str,
            _tools: &[orka_llm::ToolDefinition],
            _options: CompletionOptions,
        ) -> orka_core::Result<orka_llm::LlmToolStream> {
            Err(orka_core::Error::Other("not implemented".into()))
        }
    }
}
