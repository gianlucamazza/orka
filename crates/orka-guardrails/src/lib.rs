//! Input/output guardrails for content safety filtering.
//!
//! - [`GuardrailChain`] — composable chain of [`Guardrail`] checks
//! - [`KeywordGuardrail`] — blocklist-based keyword filter
//! - [`RegexGuardrail`] — regex-based block/redact filter with PII support

#![warn(missing_docs)]

#[allow(missing_docs)]
pub mod chain;
#[allow(missing_docs)]
pub mod keyword;
#[allow(missing_docs)]
pub mod regex_filter;

pub use chain::GuardrailChain;
pub use keyword::KeywordGuardrail;
pub use regex_filter::RegexGuardrail;

use orka_core::config::GuardrailsConfig;
use orka_core::traits::Guardrail;
use std::sync::Arc;

/// Build a guardrail chain from config. Returns None if no rules configured.
pub fn create_guardrail(config: &GuardrailsConfig) -> Option<Arc<dyn Guardrail>> {
    let mut chain = GuardrailChain::new();
    let mut has_rules = false;

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
