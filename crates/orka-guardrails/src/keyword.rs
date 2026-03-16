use async_trait::async_trait;
use orka_core::traits::{Guardrail, GuardrailDecision};
use orka_core::{Result, Session};

/// Simple keyword blocklist guardrail.
pub struct KeywordGuardrail {
    blocked_words: Vec<String>,
}

impl KeywordGuardrail {
    pub fn new(blocked_words: Vec<String>) -> Self {
        Self {
            blocked_words: blocked_words
                .into_iter()
                .map(|w| w.to_lowercase())
                .collect(),
        }
    }
}

#[async_trait]
impl Guardrail for KeywordGuardrail {
    async fn check_input(&self, input: &str, _session: &Session) -> Result<GuardrailDecision> {
        let lower = input.to_lowercase();
        for word in &self.blocked_words {
            if lower.contains(word.as_str()) {
                return Ok(GuardrailDecision::Block(
                    "Input contains blocked keyword".to_string(),
                ));
            }
        }
        Ok(GuardrailDecision::Allow)
    }

    async fn check_output(&self, output: &str, _session: &Session) -> Result<GuardrailDecision> {
        let lower = output.to_lowercase();
        for word in &self.blocked_words {
            if lower.contains(word.as_str()) {
                return Ok(GuardrailDecision::Block(
                    "Output contains blocked keyword".to_string(),
                ));
            }
        }
        Ok(GuardrailDecision::Allow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_session() -> Session {
        Session::new("test", "user1")
    }

    #[tokio::test]
    async fn allows_clean_input() {
        let guard = KeywordGuardrail::new(vec!["secret".into()]);
        let decision = guard
            .check_input("hello world", &test_session())
            .await
            .unwrap();
        assert!(matches!(decision, GuardrailDecision::Allow));
    }

    #[tokio::test]
    async fn blocks_keyword() {
        let guard = KeywordGuardrail::new(vec!["secret".into()]);
        let decision = guard
            .check_input("tell me the secret", &test_session())
            .await
            .unwrap();
        assert!(matches!(decision, GuardrailDecision::Block(_)));
    }

    #[tokio::test]
    async fn case_insensitive() {
        let guard = KeywordGuardrail::new(vec!["secret".into()]);
        let decision = guard
            .check_input("the SECRET is here", &test_session())
            .await
            .unwrap();
        assert!(matches!(decision, GuardrailDecision::Block(_)));
    }
}
