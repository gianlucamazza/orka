use std::sync::Arc;

use chrono::Utc;
use orka_core::Result;
use orka_llm::client::{ChatMessage, CompletionOptions, LlmClient};
use tracing::{debug, warn};
use uuid::Uuid;

use crate::types::{Principle, PrincipleKind, Trajectory};
use crate::utils::extract_json_array;

const REFLECTION_SYSTEM_PROMPT: &str = "\
You are a reflection engine for an AI agent orchestration system. \
Your job is to analyze a completed interaction trajectory and extract reusable principles.

For each principle, output a JSON object with:
- \"text\": a concise, actionable statement (1-2 sentences)
- \"kind\": \"do\" (something that worked well or should be done) or \"avoid\" (something that failed or should be avoided)

Output a JSON array of principle objects. Only include genuinely useful, non-obvious insights. \
If the interaction was routine with nothing notable, output an empty array [].

Examples:
[{\"text\": \"When the user asks about system status, use the os_info skill before shell_exec for safer information gathering.\", \"kind\": \"do\"},
 {\"text\": \"Avoid calling web_search with queries longer than 100 characters — the API truncates them silently.\", \"kind\": \"avoid\"}]";

/// Extracts principles from trajectories using an LLM.
pub struct PrincipleReflector {
    llm: Arc<dyn LlmClient>,
    model: Option<String>,
    max_tokens: u32,
}

impl PrincipleReflector {
    /// Create a new reflector with the given LLM client and generation settings.
    pub fn new(llm: Arc<dyn LlmClient>, model: Option<String>, max_tokens: u32) -> Self {
        Self {
            llm,
            model,
            max_tokens,
        }
    }

    /// Reflect on a trajectory and extract principles.
    pub async fn reflect(
        &self,
        trajectory: &Trajectory,
        workspace: &str,
    ) -> Result<Vec<Principle>> {
        let user_prompt = self.build_reflection_prompt(trajectory);

        let messages = vec![ChatMessage::user(user_prompt)];

        let mut options = CompletionOptions::default();
        options.model = self.model.clone();
        options.max_tokens = Some(self.max_tokens);

        let response = self
            .llm
            .complete_with_options(messages, REFLECTION_SYSTEM_PROMPT, options)
            .await?;

        let principles = self.parse_principles(&response, workspace);

        debug!(
            trajectory_id = %trajectory.id,
            count = principles.len(),
            "reflection produced principles"
        );

        Ok(principles)
    }

    fn build_reflection_prompt(&self, trajectory: &Trajectory) -> String {
        let mut prompt = String::new();
        prompt.push_str("## Interaction Trajectory\n\n");
        prompt.push_str(&format!(
            "- **Outcome**: {}\n",
            if trajectory.success {
                "SUCCESS"
            } else {
                "FAILURE"
            }
        ));
        prompt.push_str(&format!("- **Iterations**: {}\n", trajectory.iterations));
        prompt.push_str(&format!("- **Tokens**: {}\n", trajectory.total_tokens));
        prompt.push_str(&format!("- **Duration**: {}ms\n", trajectory.duration_ms));
        prompt.push_str(&format!("- **Workspace**: {}\n\n", trajectory.workspace));

        prompt.push_str("### User Message\n");
        // Truncate very long messages
        let msg = if trajectory.user_message.len() > 500 {
            format!("{}...", &trajectory.user_message[..500])
        } else {
            trajectory.user_message.clone()
        };
        prompt.push_str(&msg);
        prompt.push_str("\n\n");

        if !trajectory.skills_used.is_empty() {
            prompt.push_str("### Skills Used\n");
            for skill in &trajectory.skills_used {
                let status = if skill.success { "OK" } else { "FAILED" };
                prompt.push_str(&format!(
                    "- {} ({}ms, {})\n",
                    skill.name, skill.duration_ms, status
                ));
            }
            prompt.push('\n');
        }

        if !trajectory.errors.is_empty() {
            prompt.push_str("### Errors\n");
            for err in &trajectory.errors {
                prompt.push_str(&format!("- {err}\n"));
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

    fn parse_principles(&self, response: &str, workspace: &str) -> Vec<Principle> {
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
mod tests {
    use super::*;

    #[test]
    fn parse_principles_valid() {
        let reflector = PrincipleReflector::new(Arc::new(MockLlm), None, 1024);
        let response = r#"[{"text": "Use web_search for current info", "kind": "do"}, {"text": "Avoid long queries", "kind": "avoid"}]"#;
        let principles = reflector.parse_principles(response, "default");
        assert_eq!(principles.len(), 2);
        assert_eq!(principles[0].kind, PrincipleKind::Do);
        assert_eq!(principles[1].kind, PrincipleKind::Avoid);
    }

    #[test]
    fn parse_principles_empty_array() {
        let reflector = PrincipleReflector::new(Arc::new(MockLlm), None, 1024);
        let principles = reflector.parse_principles("[]", "default");
        assert!(principles.is_empty());
    }

    #[test]
    fn parse_principles_invalid_json() {
        let reflector = PrincipleReflector::new(Arc::new(MockLlm), None, 1024);
        let principles = reflector.parse_principles("not json at all", "default");
        assert!(principles.is_empty());
    }

    /// Minimal mock LLM for unit tests (not used in parse tests but needed for constructor).
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
            _messages: &[orka_llm::ChatMessageExt],
            _system: &str,
            _tools: &[orka_llm::ToolDefinition],
            _options: CompletionOptions,
        ) -> orka_core::Result<orka_llm::CompletionResponse> {
            Err(orka_core::Error::Other("not implemented".into()))
        }

        async fn complete_stream_with_tools(
            &self,
            _messages: &[orka_llm::ChatMessageExt],
            _system: &str,
            _tools: &[orka_llm::ToolDefinition],
            _options: CompletionOptions,
        ) -> orka_core::Result<orka_llm::LlmToolStream> {
            Err(orka_core::Error::Other("not implemented".into()))
        }
    }
}
