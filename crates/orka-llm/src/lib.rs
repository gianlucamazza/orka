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
pub mod anthropic;
/// Core LLM types: [`LlmClient`] trait, message structs, streaming types.
pub mod client;
/// Token estimation and history truncation utilities.
pub mod context;
/// Structured error types for LLM provider failures.
pub mod error;
/// Ollama client — delegates to OpenAI-compatible local API.
pub mod ollama;
/// OpenAI Chat Completions API client with retry and streaming support.
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
/// Returns one of `"anthropic"`, `"openai"`, `"google"`, or `"unknown"`.
pub fn infer_provider(model: &str) -> String {
    if model.contains("claude") {
        "anthropic".into()
    } else if model.contains("gpt") || model.contains("o1") || model.contains("o3") {
        "openai".into()
    } else if model.contains("gemini") {
        "google".into()
    } else {
        "unknown".into()
    }
}

pub use anthropic::AnthropicClient;
pub use client::{
    ChatContent, ChatMessage, CompletionOptions, CompletionResponse, ContentBlock,
    ContentBlockInput, LlmClient, LlmStream, LlmToolStream, ReasoningEffort, Role, StopReason,
    StreamEvent, ThinkingConfig, ThinkingEffort, ToolCall, ToolDefinition, ToolResult, Usage,
};
pub use context::TokenizerHint;
pub use error::LlmError;
pub use ollama::OllamaClient;
pub use openai::OpenAiClient;
pub use router::LlmRouter;
pub use stream_consumer::consume_stream;
pub use swappable::SwappableLlmClient;
