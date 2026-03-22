//! Input/output guardrails for content safety filtering.
//!
//! - [`GuardrailChain`] — composable chain of [`Guardrail`] checks
//! - [`KeywordGuardrail`] — blocklist-based keyword filter
//! - [`RegexGuardrail`] — regex-based block/redact filter with PII support
//! - [`CodeGuardrail`] — blocks dangerous code patterns before sandbox execution

#![warn(missing_docs)]

/// Composable guardrail chain that runs checks in order.
pub mod chain;
/// Code execution safety guardrail.
pub mod code_filter;
/// Simple keyword blocklist guardrail.
pub mod keyword;
/// Regex-based block and redact guardrail.
pub mod regex_filter;

pub use chain::GuardrailChain;
pub use code_filter::CodeGuardrail;
pub use keyword::KeywordGuardrail;
pub use regex_filter::RegexGuardrail;

use orka_core::config::GuardrailsConfig;
use orka_core::traits::Guardrail;
use std::sync::Arc;

/// Build a guardrail chain from config. Returns None if no rules configured.
pub fn create_guardrail(config: &GuardrailsConfig) -> Option<Arc<dyn Guardrail>> {
    let mut chain = GuardrailChain::new();
    let mut has_rules = false;

    // Code execution safety filter (default: on)
    if config.code_filter {
        chain = chain.add(Arc::new(CodeGuardrail::new()));
        has_rules = true;
    }

    // PII filter
    if config.pii_filter {
        chain = chain.add(Arc::new(RegexGuardrail::new().with_pii_filter()));
        has_rules = true;
    }

    // Regex block/redact patterns
    if !config.block_patterns.is_empty() || !config.redact_patterns.is_empty() {
        let mut rg = RegexGuardrail::new();
        for p in &config.block_patterns {
            rg = rg.add_block_pattern(p);
        }
        for rp in &config.redact_patterns {
            rg = rg.add_redact_pattern(&rp.pattern, &rp.replacement);
        }
        chain = chain.add(Arc::new(rg));
        has_rules = true;
    }

    // Keyword blocklist
    if !config.blocked_keywords.is_empty() {
        chain = chain.add(Arc::new(KeywordGuardrail::new(
            config.blocked_keywords.clone(),
        )));
        has_rules = true;
    }

    if has_rules {
        Some(Arc::new(chain))
    } else {
        None
    }
}
