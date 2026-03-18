//! LLM client abstractions and provider implementations.
//!
//! - [`LlmClient`] — async trait for chat completions (text, streaming, tool use)
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
/// [`SwappableLlmClient`] — lock-free hot-swappable client wrapper.
pub mod swappable;

pub use anthropic::AnthropicClient;
pub use client::{
    ChatContent, ChatMessage, ChatMessageExt, CompletionOptions, CompletionResponse, ContentBlock,
    ContentBlockInput, LlmClient, LlmStream, LlmToolStream, ReasoningEffort, Role, StopReason,
    StreamEvent, ThinkingConfig, ToolCall, ToolDefinition, ToolResult, Usage,
};
pub use context::TokenizerHint;
pub use error::LlmError;
pub use ollama::OllamaClient;
pub use openai::OpenAiClient;
pub use router::LlmRouter;
pub use swappable::SwappableLlmClient;
