use std::sync::Arc;

use arc_swap::ArcSwap;
use async_trait::async_trait;
use orka_core::Result;

use crate::client::{
    ChatMessage, ChatMessageExt, CompletionOptions, CompletionResponse, LlmClient, LlmStream,
    LlmToolStream, ToolDefinition,
};

/// An `LlmClient` wrapper that allows atomic hot-swapping of the underlying client.
///
/// Uses `ArcSwap` for lock-free reads on the hot path. Call [`swap`] to atomically
/// replace the inner client (e.g., after an API key rotation).
pub struct SwappableLlmClient {
    inner: ArcSwap<Arc<dyn LlmClient>>,
}

impl SwappableLlmClient {
    pub fn new(client: Arc<dyn LlmClient>) -> Self {
        Self {
            inner: ArcSwap::from_pointee(client),
        }
    }

    /// Atomically replace the inner client.
    pub fn swap(&self, new: Arc<dyn LlmClient>) {
        self.inner.store(Arc::new(new));
    }

    fn load(&self) -> Arc<Arc<dyn LlmClient>> {
        self.inner.load_full()
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
        options: CompletionOptions,
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
        messages: Vec<ChatMessageExt>,
        system: &str,
        tools: &[ToolDefinition],
        options: CompletionOptions,
    ) -> Result<CompletionResponse> {
        self.load()
            .complete_with_tools(messages, system, tools, options)
            .await
    }

    async fn complete_stream_with_tools(
        &self,
        messages: Vec<ChatMessageExt>,
        system: &str,
        tools: &[ToolDefinition],
        options: CompletionOptions,
    ) -> Result<LlmToolStream> {
        self.load()
            .complete_stream_with_tools(messages, system, tools, options)
            .await
    }
}
