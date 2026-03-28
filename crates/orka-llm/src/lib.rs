//! LLM client abstractions and provider implementations.
//!
//! - [`LlmClient`] — async trait for chat completions (text, streaming, tool
//!   use)
//! - [`AnthropicClient`], [`OpenAiClient`], [`OllamaClient`] — provider clients
//! - [`LlmRouter`] — model-prefix routing with per-provider circuit breakers
//! - [`SwappableLlmClient`] — lock-free hot-swappable client wrapper
//! - [`context`] — token estimation and history truncation utilities
//! - [`error`] — structured [`LlmError`] type for provider failures

#![warn(missing_docs)]

/// Anthropic Messages API client with retry and streaming support.
#[cfg(feature = "anthropic")]
pub mod anthropic;
/// Core LLM types: [`LlmClient`] trait, message structs, streaming types.
pub mod client;
/// LLM configuration types.
pub mod config;
/// Token estimation and history truncation utilities.
pub mod context;
/// Structured error types for LLM provider failures.
pub mod error;
/// Ollama client — delegates to OpenAI-compatible local API.
#[cfg(feature = "ollama")]
pub mod ollama;
/// `OpenAI` Chat Completions API client with retry and streaming support.
#[cfg(feature = "openai")]
pub mod openai;
/// [`LlmRouter`] — model-prefix routing with per-provider circuit breakers.
pub mod router;
/// Streaming response consumer — bridges [`LlmToolStream`] into the core
/// [`StreamRegistry`].
pub mod stream_consumer;
/// [`SwappableLlmClient`] — lock-free hot-swappable client wrapper.
pub mod swappable;
/// Test doubles for LLM client and related types ([`testing::MockLlmClient`]).
pub mod testing;

/// Anthropic Messages API version header value sent with every request.
pub const ANTHROPIC_API_VERSION: &str = "2023-06-01";

/// Infer LLM provider name from a model string, for observability.
///
/// Returns one of `"anthropic"`, `"openai"`, `"moonshot"`, `"google"`, or
/// `"unknown"`.
pub fn infer_provider(model: &str) -> String {
    if model.contains("claude") {
        "anthropic".into()
    } else if model.contains("kimi") || model.contains("moonshot/") {
        "moonshot".into()
    } else if model.contains("gpt") || model.contains("o1") || model.contains("o3") {
        "openai".into()
    } else if model.contains("gemini") {
        "google".into()
    } else {
        "unknown".into()
    }
}

#[cfg(feature = "anthropic")]
pub use anthropic::{AnthropicAuthKind, AnthropicClient};
pub use client::{
    ChatContent, ChatMessage, CompletionOptions, CompletionResponse, ContentBlock,
    ContentBlockInput, LlmClient, LlmStream, LlmToolStream, ReasoningEffort, Role, StopReason,
    StreamEvent, ThinkingConfig, ThinkingEffort, ToolCall, ToolDefinition, ToolResult, Usage,
};
pub use config::{LlmAuthKind, LlmConfig, LlmProviderConfig};
pub use context::TokenizerHint;
pub use error::LlmError;
#[cfg(feature = "ollama")]
pub use ollama::OllamaClient;
#[cfg(feature = "openai")]
pub use openai::OpenAiClient;
pub use router::LlmRouter;
pub use stream_consumer::consume_stream;
pub use swappable::SwappableLlmClient;

#[cfg(test)]
mod tests {
    use super::infer_provider;

    #[test]
    fn infer_provider_detects_moonshot_models() {
        assert_eq!(infer_provider("kimi-k2-thinking"), "moonshot");
        assert_eq!(
            infer_provider("moonshot/kimi-k2-thinking-turbo"),
            "moonshot"
        );
    }
}
