//! Input/output guardrails for content safety filtering.
//!
//! - [`GuardrailChain`] — composable chain of [`Guardrail`] checks
//! - [`KeywordGuardrail`] — blocklist-based keyword filter
//! - [`RegexGuardrail`] — regex-based block/redact filter with PII support
//! - [`CodeGuardrail`] — blocks dangerous code patterns before sandbox
//!   execution
//! - [`LlmModerationGuardrail`] — LLM-based content moderation (hate, violence,
//!   etc.)
//! - [`PromptInjectionGuardrail`] — detects prompt injection and jailbreak
//!   attempts

#![warn(missing_docs)]

/// Composable guardrail chain that runs checks in order.
pub mod chain;
/// Code execution safety guardrail.
pub mod code_filter;
/// Simple keyword blocklist guardrail.
pub mod keyword;
/// LLM-based content moderation guardrail.
pub mod llm_moderation;
/// Prompt injection detection guardrail.
pub mod prompt_injection;
/// Regex-based block and redact guardrail.
pub mod regex_filter;

use std::sync::Arc;

pub use chain::GuardrailChain;
pub use code_filter::CodeGuardrail;
pub use keyword::KeywordGuardrail;
pub use llm_moderation::LlmModerationGuardrail;
use orka_core::{config::GuardrailsConfig, traits::Guardrail};
use orka_llm::client::LlmClient;
pub use prompt_injection::PromptInjectionGuardrail;
pub use regex_filter::RegexGuardrail;
use tracing::warn;

/// Build a guardrail chain from config. Returns None if no rules configured.
pub fn create_guardrail(config: &GuardrailsConfig) -> Option<Arc<dyn Guardrail>> {
    if !config.enabled {
        return None;
    }

    let mut chain = GuardrailChain::new();

    // Always add prompt injection detection for input protection
    chain = chain.add(Arc::new(PromptInjectionGuardrail::new()));

    // Regex block/redact patterns (from input rules)
    if !config.input.blocked_patterns.is_empty() || !config.input.redact_patterns.is_empty() {
        let mut rg = RegexGuardrail::new();
        for p in &config.input.blocked_patterns {
            rg = rg.add_block_pattern(p);
        }
        for rp in &config.input.redact_patterns {
            rg = rg.add_redact_pattern(&rp.pattern, &rp.replacement);
        }
        chain = chain.add(Arc::new(rg));
    }

    // Keyword blocklist (from input rules)
    if !config.input.blocked_keywords.is_empty() {
        chain = chain.add(Arc::new(KeywordGuardrail::new(
            config.input.blocked_keywords.clone(),
        )));
    }

    // LLM-based content moderation
    if config.input.llm_moderation.enabled {
        warn!(
            "LLM moderation enabled but requires LLM client initialization - use create_guardrail_with_llm instead"
        );
    }

    Some(Arc::new(chain))
}

/// Build a guardrail chain with LLM moderation support.
pub fn create_guardrail_with_llm(
    config: &GuardrailsConfig,
    llm: Arc<dyn LlmClient>,
) -> Option<Arc<dyn Guardrail>> {
    if !config.enabled {
        return None;
    }

    let mut chain = GuardrailChain::new();

    // Always add prompt injection detection
    chain = chain.add(Arc::new(PromptInjectionGuardrail::new()));

    // Regex block/redact patterns (from input rules)
    if !config.input.blocked_patterns.is_empty() || !config.input.redact_patterns.is_empty() {
        let mut rg = RegexGuardrail::new();
        for p in &config.input.blocked_patterns {
            rg = rg.add_block_pattern(p);
        }
        for rp in &config.input.redact_patterns {
            rg = rg.add_redact_pattern(&rp.pattern, &rp.replacement);
        }
        chain = chain.add(Arc::new(rg));
    }

    // Keyword blocklist (from input rules)
    if !config.input.blocked_keywords.is_empty() {
        chain = chain.add(Arc::new(KeywordGuardrail::new(
            config.input.blocked_keywords.clone(),
        )));
    }

    // LLM-based content moderation
    if config.input.llm_moderation.enabled {
        let moderation = LlmModerationGuardrail::new(llm, config.input.llm_moderation.clone());
        chain = chain.add(Arc::new(moderation));
    }

    Some(Arc::new(chain))
}
