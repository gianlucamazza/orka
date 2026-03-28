//! LLM-based content moderation guardrail.
//!
//! Uses an LLM to analyze content for policy violations across multiple
//! categories:
//! - Hate speech, harassment, self-harm, violence
//! - Sexual content, dangerous activities, spam, profanity
//!
//! The guardrail returns a confidence score for each category and blocks
//! content that exceeds the configured threshold.

use std::sync::Arc;

use orka_core::{
    Result,
    traits::{Guardrail, GuardrailDecision},
    types::Session,
};
use orka_llm::client::{ChatMessage, LlmClient};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::config::{GuardrailRules, LlmModerationConfig};

/// Default system prompt for content moderation.
const DEFAULT_MODERATION_PROMPT: &str = r#"You are a content moderation assistant. Analyze the following text and identify any policy violations.

For each category, provide:
1. A confidence score (0.0 to 1.0) indicating how likely the content violates that category
2. A brief explanation if the score is above 0.5

Categories to check:
- hate: Hate speech, discrimination, dehumanization
- harassment: Bullying, threats, unwanted sexual attention
- self_harm: Suicide, self-injury, eating disorders
- violence: Physical harm, weapons, gore
- sexual: Explicit sexual content, nudity
- dangerous: Dangerous activities, illegal acts, weapons
- spam: Misleading, repetitive, commercial spam
- profanity: Obscene, vulgar language

Respond ONLY with valid JSON in this exact format:
{
  "violations": [
    {"category": "hate", "confidence": 0.85, "explanation": "..."},
    ...
  ],
  "overall_safe": true/false
}"#;

/// LLM-based content moderation guardrail.
pub struct LlmModerationGuardrail {
    llm: Arc<dyn LlmClient>,
    config: LlmModerationConfig,
    system_prompt: String,
}

/// Moderation response from LLM.
#[derive(Debug, Deserialize, Serialize)]
struct ModerationResponse {
    violations: Vec<ViolationDetail>,
    overall_safe: bool,
}

/// Details of a policy violation.
#[derive(Debug, Deserialize, Serialize)]
pub struct ViolationDetail {
    category: String,
    confidence: f32,
    explanation: Option<String>,
}

impl LlmModerationGuardrail {
    /// Create a new LLM moderation guardrail.
    pub fn new(llm: Arc<dyn LlmClient>, config: LlmModerationConfig) -> Self {
        let system_prompt = config
            .system_prompt
            .clone()
            .unwrap_or_else(|| DEFAULT_MODERATION_PROMPT.to_string());

        Self {
            llm,
            config,
            system_prompt,
        }
    }

    /// Create from config rules (if LLM moderation is enabled).
    pub fn from_config(llm: Arc<dyn LlmClient>, rules: &GuardrailRules) -> Option<Self> {
        if rules.llm_moderation.enabled {
            Some(Self::new(llm, rules.llm_moderation.clone()))
        } else {
            None
        }
    }

    /// Analyze content for policy violations.
    pub async fn analyze(&self, content: &str) -> Result<ModerationAnalysis> {
        let categories_str = self
            .config
            .categories
            .iter()
            .map(|c| format!("{c:?}"))
            .collect::<Vec<_>>()
            .join(", ");

        let user_prompt = format!(
            r"Analyze this text for violations of these categories: {categories_str}

Text to analyze:
{content}

Remember to respond with valid JSON only."
        );

        let messages = vec![
            ChatMessage::system(&self.system_prompt),
            ChatMessage::user(&user_prompt),
        ];

        // Use complete_with_tools to get CompletionResponse with blocks
        let response = self
            .llm
            .complete_with_tools(
                &messages,
                "",
                &[],
                orka_llm::client::CompletionOptions::default(),
            )
            .await?;

        // Extract text from content blocks
        let text_content = response
            .blocks
            .iter()
            .filter_map(|block| {
                if let orka_llm::client::ContentBlock::Text(text) = block {
                    Some(text.as_str())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("");

        let moderation: ModerationResponse = serde_json::from_str(&text_content).map_err(|e| {
            orka_core::Error::Guardrail(format!("Failed to parse moderation response: {e}"))
        })?;

        Ok(ModerationAnalysis {
            response: moderation,
            threshold: self.config.threshold,
        })
    }
}

/// Result of content moderation analysis.
pub struct ModerationAnalysis {
    response: ModerationResponse,
    threshold: f32,
}

impl ModerationAnalysis {
    /// Check if content is safe (no violations above threshold).
    pub fn is_safe(&self) -> bool {
        self.response.overall_safe
            && !self
                .response
                .violations
                .iter()
                .any(|v| v.confidence >= self.threshold)
    }

    /// Get violations that exceed the threshold.
    pub fn violations(&self) -> Vec<&ViolationDetail> {
        self.response
            .violations
            .iter()
            .filter(|v| v.confidence >= self.threshold)
            .collect()
    }

    /// Get all violations with their confidence scores.
    pub fn all_violations(&self) -> &[ViolationDetail] {
        &self.response.violations
    }
}

#[async_trait::async_trait]
impl Guardrail for LlmModerationGuardrail {
    async fn check_input(&self, input: &str, _session: &Session) -> Result<GuardrailDecision> {
        debug!("Checking input with LLM moderation");

        match self.analyze(input).await {
            Ok(analysis) => {
                if analysis.is_safe() {
                    Ok(GuardrailDecision::Allow)
                } else {
                    let violations: Vec<String> = analysis
                        .violations()
                        .iter()
                        .map(|v| {
                            format!(
                                "{} (confidence: {:.2}){}",
                                v.category,
                                v.confidence,
                                v.explanation
                                    .as_ref()
                                    .map(|e| format!(": {e}"))
                                    .unwrap_or_default()
                            )
                        })
                        .collect();

                    let reason = format!("Content moderation failed: {}", violations.join("; "));
                    warn!(reason, "LLM guardrail blocked input");
                    Ok(GuardrailDecision::Block(reason))
                }
            }
            Err(e) => {
                warn!(error = %e, "LLM moderation failed, allowing by default");
                // Fail open - allow content if moderation fails
                Ok(GuardrailDecision::Allow)
            }
        }
    }

    async fn check_output(&self, output: &str, _session: &Session) -> Result<GuardrailDecision> {
        debug!("Checking output with LLM moderation");

        match self.analyze(output).await {
            Ok(analysis) => {
                if analysis.is_safe() {
                    Ok(GuardrailDecision::Allow)
                } else {
                    let violations: Vec<String> = analysis
                        .violations()
                        .iter()
                        .map(|v| {
                            format!(
                                "{} (confidence: {:.2}){}",
                                v.category,
                                v.confidence,
                                v.explanation
                                    .as_ref()
                                    .map(|e| format!(": {e}"))
                                    .unwrap_or_default()
                            )
                        })
                        .collect();

                    let reason = format!("Content moderation failed: {}", violations.join("; "));
                    warn!(reason, "LLM guardrail blocked output");
                    Ok(GuardrailDecision::Block(reason))
                }
            }
            Err(e) => {
                warn!(error = %e, "LLM moderation failed on output, allowing by default");
                Ok(GuardrailDecision::Allow)
            }
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::field_reassign_with_default,
    clippy::default_trait_access,
    clippy::needless_pass_by_value,
    clippy::stable_sort_primitive
)]
mod tests {
    use super::*;
    use crate::config::ModerationCategory;

    // Mock LLM for testing
    struct MockLlm {
        response: String,
    }

    impl MockLlm {
        fn safe_response() -> Self {
            Self {
                response: r#"{"violations": [], "overall_safe": true}"#.to_string(),
            }
        }

        fn unsafe_response() -> Self {
            Self {
                response: r#"{"violations": [{"category": "hate", "confidence": 0.9, "explanation": "Contains hate speech"}], "overall_safe": false}"#.to_string(),
            }
        }
    }

    #[async_trait::async_trait]
    impl LlmClient for MockLlm {
        // complete() is the only required method; all others have default impls in the
        // trait.
        async fn complete(&self, _messages: Vec<ChatMessage>, _system: &str) -> Result<String> {
            Ok(self.response.clone())
        }
    }

    #[tokio::test]
    async fn test_safe_content_allowed() {
        let llm = Arc::new(MockLlm::safe_response());
        let config = LlmModerationConfig::default()
            .with_enabled(true)
            .with_model("mock")
            .with_threshold(0.7)
            .with_categories(vec![ModerationCategory::Hate]);

        let guardrail = LlmModerationGuardrail::new(llm, config);

        let session = Session::new("test", "user1");
        let decision = guardrail
            .check_input("Hello, how are you?", &session)
            .await
            .unwrap();

        assert!(matches!(decision, GuardrailDecision::Allow));
    }

    #[tokio::test]
    async fn test_unsafe_content_blocked() {
        let llm = Arc::new(MockLlm::unsafe_response());
        let config = LlmModerationConfig::default()
            .with_enabled(true)
            .with_model("mock")
            .with_threshold(0.7)
            .with_categories(vec![ModerationCategory::Hate]);

        let guardrail = LlmModerationGuardrail::new(llm, config);

        let session = Session::new("test", "user1");
        let decision = guardrail
            .check_input("Hate speech content", &session)
            .await
            .unwrap();

        assert!(matches!(decision, GuardrailDecision::Block(_)));
    }

    #[tokio::test]
    async fn test_moderation_analysis() {
        let llm = Arc::new(MockLlm::unsafe_response());
        let config = LlmModerationConfig::default()
            .with_enabled(true)
            .with_model("mock")
            .with_threshold(0.7)
            .with_categories(vec![ModerationCategory::Hate]);

        let guardrail = LlmModerationGuardrail::new(llm, config);

        let analysis = guardrail.analyze("test").await.unwrap();
        assert!(!analysis.is_safe());
        assert_eq!(analysis.violations().len(), 1);
        assert_eq!(analysis.violations()[0].category, "hate");
    }
}
