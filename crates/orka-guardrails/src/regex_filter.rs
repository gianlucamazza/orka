use async_trait::async_trait;
use orka_core::{
    Result, Session,
    traits::{Guardrail, GuardrailDecision},
};
use regex::Regex;
use tracing::warn;

/// Regex-based guardrail that can redact or block matching patterns.
pub struct RegexGuardrail {
    patterns: Vec<(Regex, RegexAction)>,
}

/// Action to take when a regex pattern matches.
#[derive(Debug, Clone)]
pub enum RegexAction {
    /// Reject the content entirely.
    Block,
    /// Replace the matched text with the given string.
    Redact(String),
}

impl RegexGuardrail {
    /// Create an empty regex guardrail with no patterns.
    pub fn new() -> Self {
        Self {
            patterns: Vec::new(),
        }
    }

    /// Add a regex pattern that will cause the guardrail to block matching
    /// content.
    #[must_use]
    pub fn add_block_pattern(mut self, pattern: &str) -> Self {
        match Regex::new(pattern) {
            Ok(re) => self.patterns.push((re, RegexAction::Block)),
            Err(e) => warn!(pattern, %e, "guardrail: invalid block regex — pattern skipped"),
        }
        self
    }

    /// Add a regex pattern whose matches will be replaced with `replacement`.
    #[must_use]
    pub fn add_redact_pattern(mut self, pattern: &str, replacement: &str) -> Self {
        match Regex::new(pattern) {
            Ok(re) => self
                .patterns
                .push((re, RegexAction::Redact(replacement.to_string()))),
            Err(e) => warn!(pattern, %e, "guardrail: invalid redact regex — pattern skipped"),
        }
        self
    }

    /// Pre-built PII filter: emails, phone numbers, SSNs.
    #[must_use]
    pub fn with_pii_filter(self) -> Self {
        self.add_redact_pattern(
            r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Z|a-z]{2,}\b",
            "[EMAIL]",
        )
        .add_redact_pattern(r"\b\d{3}[-.]?\d{3}[-.]?\d{4}\b", "[PHONE]")
        .add_redact_pattern(r"\b\d{3}-\d{2}-\d{4}\b", "[SSN]")
    }

    fn apply(&self, text: &str) -> GuardrailDecision {
        let mut result = text.to_string();
        let mut modified = false;

        for (re, action) in &self.patterns {
            match action {
                RegexAction::Block => {
                    if re.is_match(&result) {
                        return GuardrailDecision::Block(
                            "Content matches blocked pattern".to_string(),
                        );
                    }
                }
                RegexAction::Redact(replacement) => {
                    let new = re.replace_all(&result, replacement.as_str()).to_string();
                    if new != result {
                        modified = true;
                        result = new;
                    }
                }
            }
        }

        if modified {
            GuardrailDecision::Modify(result)
        } else {
            GuardrailDecision::Allow
        }
    }
}

impl Default for RegexGuardrail {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Guardrail for RegexGuardrail {
    async fn check_input(&self, input: &str, _session: &Session) -> Result<GuardrailDecision> {
        Ok(self.apply(input))
    }

    async fn check_output(&self, output: &str, _session: &Session) -> Result<GuardrailDecision> {
        Ok(self.apply(output))
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

    fn test_session() -> Session {
        Session::new("test", "user1")
    }

    #[tokio::test]
    async fn pii_redaction() {
        let guard = RegexGuardrail::new().with_pii_filter();
        let decision = guard
            .check_output(
                "Contact me at test@example.com or 555-123-4567",
                &test_session(),
            )
            .await
            .unwrap();
        match decision {
            GuardrailDecision::Modify(text) => {
                assert!(text.contains("[EMAIL]"));
                assert!(text.contains("[PHONE]"));
                assert!(!text.contains("test@example.com"));
            }
            _ => panic!("expected Modify"),
        }
    }

    #[tokio::test]
    async fn block_pattern() {
        let guard = RegexGuardrail::new().add_block_pattern(r"(?i)password\s*[:=]");
        let decision = guard
            .check_input("the password: hunter2", &test_session())
            .await
            .unwrap();
        assert!(matches!(decision, GuardrailDecision::Block(_)));
    }

    #[tokio::test]
    async fn allow_clean() {
        let guard = RegexGuardrail::new().with_pii_filter();
        let decision = guard
            .check_input("just a normal message", &test_session())
            .await
            .unwrap();
        assert!(matches!(decision, GuardrailDecision::Allow));
    }
}
