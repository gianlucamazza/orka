//! LLM-as-Judge evaluation for semantic quality assessment.
//!
//! Provides [`LlmJudge`] which uses an LLM to evaluate skill outputs against
//! human-defined [`JudgeCriterion`] entries.  Each criterion is evaluated
//! independently; the judge returns one [`AssertionResult`] per criterion.
//!
//! ## Usage in an eval file
//!
//! ```toml
//! [[scenarios]]
//! name = "helpful_response"
//! [scenarios.input]
//! query = "What is Rust?"
//! [scenarios.expected]
//! [[scenarios.expected.judge]]
//! name    = "helpfulness"
//! description = "The response clearly explains what Rust is"
//! rubric  = "Pass if the answer covers memory safety and systems programming"
//! ```

use std::{fmt::Write as _, sync::Arc};

use orka_llm::{ChatMessage, CompletionOptions, LlmClient, client::ResponseFormat};
use serde::Deserialize;
use tracing::warn;

use crate::{assertion::AssertionResult, scenario::JudgeCriterion};

/// Evaluates skill output against semantic criteria using an LLM.
pub struct LlmJudge {
    client: Arc<dyn LlmClient>,
    /// Optional model override (defaults to the client's default model).
    pub model: Option<String>,
}

impl LlmJudge {
    /// Create a judge backed by the given LLM client.
    pub fn new(client: Arc<dyn LlmClient>) -> Self {
        Self {
            client,
            model: None,
        }
    }

    /// Override the model used for judging.
    #[must_use]
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Evaluate `output` against all `criteria` and return one
    /// [`AssertionResult`] per criterion.
    pub async fn evaluate(
        &self,
        criteria: &[JudgeCriterion],
        output: &str,
        input: &serde_json::Value,
    ) -> Vec<AssertionResult> {
        let mut results = Vec::with_capacity(criteria.len());
        for criterion in criteria {
            let result = self.evaluate_one(criterion, output, input).await;
            results.push(result);
        }
        results
    }

    async fn evaluate_one(
        &self,
        criterion: &JudgeCriterion,
        output: &str,
        input: &serde_json::Value,
    ) -> AssertionResult {
        let check_name = format!("judge: {}", criterion.name);

        let mut prompt = format!(
            "Criterion: {}\nDescription: {}",
            criterion.name, criterion.description
        );
        if let Some(ref rubric) = criterion.rubric {
            write!(prompt, "\nRubric: {rubric}").unwrap_or(());
        }
        write!(
            prompt,
            "\n\nSkill input:\n{}\n\nSkill output:\n{}\n\nRespond with a JSON object: {{\"passed\": true/false, \"reasoning\": \"brief explanation\"}}",
            serde_json::to_string_pretty(input).unwrap_or_else(|_| input.to_string()),
            truncate(output, 2000),
        ).unwrap_or(());

        let system = "You are an impartial evaluator. Assess whether the skill output meets the criterion. Be concise.";
        let messages = vec![ChatMessage::user(prompt)];
        let mut options = CompletionOptions::default();
        options.model = self.model.clone();
        options.max_tokens = Some(256);
        options.response_format = Some(ResponseFormat::Json);

        let raw = match self
            .client
            .complete_with_options(messages, system, &options)
            .await
        {
            Ok(text) => text,
            Err(e) => {
                warn!(criterion = %criterion.name, %e, "LLM judge call failed");
                return AssertionResult::fail_test(check_name, format!("LLM call failed: {e}"));
            }
        };

        match serde_json::from_str::<JudgeResponse>(&raw) {
            Ok(resp) => {
                if resp.passed {
                    AssertionResult::pass_test(check_name)
                } else {
                    AssertionResult::fail_test(check_name, resp.reasoning)
                }
            }
            Err(e) => {
                warn!(criterion = %criterion.name, %e, raw = %truncate(&raw, 200), "failed to parse judge response");
                AssertionResult::fail_test(
                    check_name,
                    format!(
                        "could not parse judge JSON: {e} — raw: {}",
                        truncate(&raw, 100)
                    ),
                )
            }
        }
    }
}

#[derive(Deserialize)]
struct JudgeResponse {
    passed: bool,
    reasoning: String,
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max { s } else { &s[..max] }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use orka_llm::testing::MockLlmClient;
    use serde_json::json;

    use super::*;
    use crate::scenario::JudgeCriterion;

    fn criterion(name: &str, description: &str) -> JudgeCriterion {
        JudgeCriterion {
            name: name.to_string(),
            description: description.to_string(),
            rubric: None,
        }
    }

    #[tokio::test]
    async fn evaluate_returns_pass_when_llm_says_passed_true() {
        let mock = Arc::new(
            MockLlmClient::new()
                .with_text_response(r#"{"passed": true, "reasoning": "looks good"}"#),
        );
        let judge = LlmJudge::new(mock);
        let criteria = vec![criterion("helpfulness", "Response is helpful")];
        let results = judge
            .evaluate(&criteria, "Great answer!", &json!({"query": "test"}))
            .await;

        assert_eq!(results.len(), 1);
        assert!(results[0].passed, "expected pass from judge");
        assert!(results[0].detail.is_none());
    }

    #[tokio::test]
    async fn evaluate_returns_fail_when_llm_says_passed_false() {
        let mock = Arc::new(
            MockLlmClient::new()
                .with_text_response(r#"{"passed": false, "reasoning": "too vague"}"#),
        );
        let judge = LlmJudge::new(mock);
        let criteria = vec![criterion("helpfulness", "Response is helpful")];
        let results = judge.evaluate(&criteria, "ok.", &json!({})).await;

        assert_eq!(results.len(), 1);
        assert!(!results[0].passed, "expected fail from judge");
        assert_eq!(results[0].detail.as_deref(), Some("too vague"));
    }

    #[tokio::test]
    async fn evaluate_handles_parse_failure_gracefully() {
        let mock = Arc::new(MockLlmClient::new().with_text_response("not valid json at all"));
        let judge = LlmJudge::new(mock);
        let criteria = vec![criterion("x", "y")];
        let results = judge.evaluate(&criteria, "out", &json!({})).await;

        assert_eq!(results.len(), 1);
        assert!(
            !results[0].passed,
            "parse failure should produce a failing result"
        );
        let detail = results[0].detail.as_deref().unwrap_or("");
        assert!(
            detail.contains("could not parse"),
            "detail should mention parse error: {detail}"
        );
    }

    #[tokio::test]
    async fn evaluate_multiple_criteria_returns_one_result_each() {
        let mock = Arc::new(
            MockLlmClient::new()
                .with_text_response(r#"{"passed": true, "reasoning": "ok"}"#)
                .with_text_response(r#"{"passed": false, "reasoning": "bad"}"#),
        );
        let judge = LlmJudge::new(mock);
        let criteria = vec![criterion("a", "first"), criterion("b", "second")];
        let results = judge.evaluate(&criteria, "output", &json!({})).await;

        assert_eq!(results.len(), 2);
        assert!(results[0].passed);
        assert!(!results[1].passed);
    }
}
