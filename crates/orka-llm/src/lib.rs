//! LLM client abstractions and provider implementations.
//!
//! - [`LlmClient`] — async trait for chat completions (text, streaming, tool use)
//! - [`AnthropicClient`], [`OpenAiClient`], [`OllamaClient`] — provider clients
//! - [`LlmRouter`] — model-prefix routing with per-provider circuit breakers
//! - [`SwappableLlmClient`] — lock-free hot-swappable client wrapper
//! - [`context`] — token estimation and history truncation utilities

#![warn(missing_docs)]

#[allow(missing_docs)]
pub mod anthropic;
#[allow(missing_docs)]
pub mod client;
#[allow(missing_docs)]
pub mod context;
#[allow(missing_docs)]
pub mod ollama;
#[allow(missing_docs)]
pub mod openai;
#[allow(missing_docs)]
pub mod router;
#[allow(missing_docs)]
pub mod swappable;

pub use anthropic::AnthropicClient;
pub use client::{
    ChatContent, ChatMessage, ChatMessageExt, CompletionOptions, CompletionResponse, ContentBlock,
    ContentBlockInput, LlmClient, LlmStream, LlmToolStream, StopReason, StreamEvent, ToolCall,
    ToolDefinition, ToolResult, Usage,
};
pub use context::TokenizerHint;
pub use ollama::OllamaClient;
pub use openai::OpenAiClient;
pub use router::LlmRouter;
pub use swappable::SwappableLlmClient;
