use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use orka_core::Result;
use tracing::debug;

use crate::client::{
    ChatMessage, ChatMessageExt, CompletionOptions, CompletionResponse, LlmClient, LlmStream,
    LlmToolStream, ToolDefinition,
};

/// Routes LLM requests to the appropriate provider based on model name prefix.
pub struct LlmRouter {
    /// Default provider used when no prefix matches.
    default_provider: Arc<dyn LlmClient>,
    /// Map of provider name -> client (e.g., "anthropic" -> AnthropicClient).
    providers: HashMap<String, Arc<dyn LlmClient>>,
    /// Map of model prefix -> provider name (e.g., "claude" -> "anthropic", "gpt" -> "openai").
    prefix_map: HashMap<String, String>,
}

impl LlmRouter {
    pub fn new(default_provider: Arc<dyn LlmClient>) -> Self {
        Self {
            default_provider,
            providers: HashMap::new(),
            prefix_map: HashMap::new(),
        }
    }

    pub fn add_provider(
        mut self,
        name: impl Into<String>,
        client: Arc<dyn LlmClient>,
        prefixes: Vec<String>,
    ) -> Self {
        let name = name.into();
        self.providers.insert(name.clone(), client);
        for prefix in prefixes {
            self.prefix_map.insert(prefix, name.clone());
        }
        self
    }

    fn resolve(&self, model: Option<&str>) -> &dyn LlmClient {
        if let Some(model_name) = model {
            // Check prefix map
            for (prefix, provider_name) in &self.prefix_map {
                if model_name.starts_with(prefix) {
                    if let Some(client) = self.providers.get(provider_name) {
                        debug!(
                            model = model_name,
                            provider = provider_name,
                            "routing to provider"
                        );
                        return client.as_ref();
                    }
                }
            }
            // Check if model name matches a provider name directly
            if let Some(client) = self.providers.get(model_name) {
                return client.as_ref();
            }
        }
        self.default_provider.as_ref()
    }
}

#[async_trait]
impl LlmClient for LlmRouter {
    async fn complete(&self, messages: Vec<ChatMessage>, system: &str) -> Result<String> {
        self.default_provider.complete(messages, system).await
    }

    async fn complete_with_options(
        &self,
        messages: Vec<ChatMessage>,
        system: &str,
        options: CompletionOptions,
    ) -> Result<String> {
        let provider = self.resolve(options.model.as_deref());
        provider
            .complete_with_options(messages, system, options)
            .await
    }

    async fn complete_stream(&self, messages: Vec<ChatMessage>, system: &str) -> Result<LlmStream> {
        self.default_provider
            .complete_stream(messages, system)
            .await
    }

    async fn complete_with_tools(
        &self,
        messages: Vec<ChatMessageExt>,
        system: &str,
        tools: &[ToolDefinition],
        options: CompletionOptions,
    ) -> Result<CompletionResponse> {
        let provider = self.resolve(options.model.as_deref());
        provider
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
        let provider = self.resolve(options.model.as_deref());
        provider
            .complete_stream_with_tools(messages, system, tools, options)
            .await
    }
}
