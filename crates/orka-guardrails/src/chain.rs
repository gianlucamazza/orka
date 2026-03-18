use std::sync::Arc;

use async_trait::async_trait;
use orka_core::traits::{Guardrail, GuardrailDecision};
use orka_core::{Result, Session};

/// Chains multiple guardrails. Processes in order; first Block wins, Modify accumulates.
pub struct GuardrailChain {
    guardrails: Vec<Arc<dyn Guardrail>>,
}

impl GuardrailChain {
    /// Create an empty chain.
    pub fn new() -> Self {
        Self {
            guardrails: Vec::new(),
        }
    }

    /// Append a guardrail to the chain (builder pattern).
    #[allow(clippy::should_implement_trait)]
    pub fn add(mut self, guardrail: Arc<dyn Guardrail>) -> Self {
        self.guardrails.push(guardrail);
        self
    }
}

impl Default for GuardrailChain {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Guardrail for GuardrailChain {
    async fn check_input(&self, input: &str, session: &Session) -> Result<GuardrailDecision> {
        let mut current = input.to_string();
        let mut was_modified = false;

        for guard in &self.guardrails {
            match guard.check_input(&current, session).await? {
                GuardrailDecision::Allow => {}
                GuardrailDecision::Block(reason) => return Ok(GuardrailDecision::Block(reason)),
                GuardrailDecision::Modify(modified) => {
                    current = modified;
                    was_modified = true;
                }
                other => {
                    tracing::warn!(?other, "unhandled guardrail decision variant");
                }
            }
        }

        if was_modified {
            Ok(GuardrailDecision::Modify(current))
        } else {
            Ok(GuardrailDecision::Allow)
        }
    }

    async fn check_output(&self, output: &str, session: &Session) -> Result<GuardrailDecision> {
        let mut current = output.to_string();
        let mut was_modified = false;

        for guard in &self.guardrails {
            match guard.check_output(&current, session).await? {
                GuardrailDecision::Allow => {}
                GuardrailDecision::Block(reason) => return Ok(GuardrailDecision::Block(reason)),
                GuardrailDecision::Modify(modified) => {
                    current = modified;
                    was_modified = true;
                }
                other => {
                    tracing::warn!(?other, "unhandled guardrail decision variant");
                }
            }
        }

        if was_modified {
            Ok(GuardrailDecision::Modify(current))
        } else {
            Ok(GuardrailDecision::Allow)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{KeywordGuardrail, RegexGuardrail};

    fn test_session() -> Session {
        Session::new("test", "user1")
    }

    #[tokio::test]
    async fn chain_blocks_on_keyword() {
        let chain = GuardrailChain::new()
            .add(Arc::new(RegexGuardrail::new().with_pii_filter()))
            .add(Arc::new(KeywordGuardrail::new(vec!["forbidden".into()])));

        let decision = chain
            .check_input("this is forbidden content", &test_session())
            .await
            .unwrap();
        assert!(matches!(decision, GuardrailDecision::Block(_)));
    }

    #[tokio::test]
    async fn chain_redacts_then_allows() {
        let chain = GuardrailChain::new()
            .add(Arc::new(RegexGuardrail::new().with_pii_filter()))
            .add(Arc::new(KeywordGuardrail::new(vec!["forbidden".into()])));

        let decision = chain
            .check_output("email: test@example.com", &test_session())
            .await
            .unwrap();
        match decision {
            GuardrailDecision::Modify(text) => assert!(text.contains("[EMAIL]")),
            _ => panic!("expected Modify"),
        }
    }
}
