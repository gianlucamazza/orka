use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use orka_core::Result;

use crate::client::{
    ChatMessage, CompletionOptions, CompletionResponse, LlmClient, LlmStream, LlmToolStream,
    ToolDefinition,
};

/// An `LlmClient` wrapper that allows atomic hot-swapping of the underlying
/// client without double indirection.
///
/// Swaps are rare (API-key rotation); reads use a shared `RwLock` whose
/// overhead is negligible compared to any LLM HTTP round-trip. Call
/// [`Self::swap`] to replace the inner client while existing in-flight calls
/// complete normally on the previous instance.
pub struct SwappableLlmClient {
    inner: RwLock<Arc<dyn LlmClient>>,
}

impl SwappableLlmClient {
    /// Wrap an existing client for hot-swapping.
    pub fn new(client: Arc<dyn LlmClient>) -> Self {
        Self {
            inner: RwLock::new(client),
        }
    }

    /// Replace the inner client. Ongoing calls on the previous client
    /// finish normally; new calls use the replacement.
    pub fn swap(&self, new: Arc<dyn LlmClient>) {
        if let Ok(mut guard) = self.inner.write() {
            *guard = new;
        }
    }

    fn load(&self) -> Arc<dyn LlmClient> {
        self.inner
            .read()
            .map_or_else(|e| Arc::clone(&*e.into_inner()), |g| Arc::clone(&*g))
    }
}

#[async_trait]
impl LlmClient for SwappableLlmClient {
    async fn complete(&self, messages: Vec<ChatMessage>, system: &str) -> Result<String> {
        self.load().complete(messages, system).await
    }

    async fn complete_with_options(
        &self,
        messages: Vec<ChatMessage>,
        system: &str,
        options: &CompletionOptions,
    ) -> Result<String> {
        self.load()
            .complete_with_options(messages, system, options)
            .await
    }

    async fn complete_stream(&self, messages: Vec<ChatMessage>, system: &str) -> Result<LlmStream> {
        self.load().complete_stream(messages, system).await
    }

    async fn complete_with_tools(
        &self,
        messages: &[ChatMessage],
        system: &str,
        tools: &[ToolDefinition],
        options: &CompletionOptions,
    ) -> Result<CompletionResponse> {
        self.load()
            .complete_with_tools(messages, system, tools, options)
            .await
    }

    async fn complete_stream_with_tools(
        &self,
        messages: &[ChatMessage],
        system: &str,
        tools: &[ToolDefinition],
        options: &CompletionOptions,
    ) -> Result<LlmToolStream> {
        self.load()
            .complete_stream_with_tools(messages, system, tools, options)
            .await
    }
}
