use async_trait::async_trait;
use orka_core::Result;
use tracing::debug;

use crate::client::{
    ChatMessage, ChatMessageExt, CompletionOptions, CompletionResponse, LlmClient, LlmStream,
    ToolDefinition,
};
use crate::openai::OpenAiClient;

/// Ollama client — delegates to OpenAI-compatible API.
pub struct OllamaClient {
    inner: OpenAiClient,
}

impl OllamaClient {
    pub fn new(model: String) -> Self {
        Self::with_options(model, 120, 4096, 1, "http://localhost:11434/v1".into())
    }

    pub fn with_options(
        model: String,
        timeout_secs: u64,
        max_tokens: u32,
        max_retries: u32,
        base_url: String,
    ) -> Self {
        // Ollama doesn't require an API key
        let inner = OpenAiClient::with_options(
            String::new(),
            model,
            timeout_secs,
            max_tokens,
            max_retries,
            base_url,
        );
        Self { inner }
    }
}

#[async_trait]
impl LlmClient for OllamaClient {
    async fn complete(&self, messages: Vec<ChatMessage>, system: &str) -> Result<String> {
        debug!("Ollama: delegating to OpenAI-compatible API");
        self.inner.complete(messages, system).await
    }

    async fn complete_with_options(
        &self,
        messages: Vec<ChatMessage>,
        system: &str,
        options: CompletionOptions,
    ) -> Result<String> {
        self.inner
            .complete_with_options(messages, system, options)
            .await
    }

    async fn complete_stream(
        &self,
        messages: Vec<ChatMessage>,
        system: &str,
    ) -> Result<LlmStream> {
        self.inner.complete_stream(messages, system).await
    }

    async fn complete_with_tools(
        &self,
        messages: Vec<ChatMessageExt>,
        system: &str,
        tools: &[ToolDefinition],
        options: CompletionOptions,
    ) -> Result<CompletionResponse> {
        self.inner
            .complete_with_tools(messages, system, tools, options)
            .await
    }
}
