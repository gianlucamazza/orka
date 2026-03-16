use std::pin::Pin;

use async_trait::async_trait;
use orka_core::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String, // "user" or "assistant"
    pub content: String,
}

/// A stream of text chunks from an LLM response.
pub type LlmStream = Pin<Box<dyn futures_util::Stream<Item = Result<String>> + Send>>;

/// Structured output format.
#[derive(Debug, Clone)]
pub enum ResponseFormat {
    /// Request JSON output matching this schema.
    JsonSchema {
        name: String,
        schema: serde_json::Value,
    },
    /// Just request JSON output without a specific schema.
    Json,
}

#[derive(Debug, Clone, Default)]
pub struct CompletionOptions {
    pub model: Option<String>,
    pub max_tokens: Option<u32>,
    /// Optional JSON Schema for structured/constrained output.
    pub response_format: Option<ResponseFormat>,
}

/// A tool definition sent to the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// A tool call requested by the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

/// A tool result to feed back to the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_use_id: String,
    pub content: String,
    pub is_error: bool,
}

/// A content block in an LLM response — either text or a tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ContentBlock {
    Text(String),
    ToolUse(ToolCall),
}

/// Extended chat message supporting tool results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessageExt {
    pub role: String,
    pub content: ChatContent,
}

/// Content can be simple text or a list of content blocks (for tool results).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ChatContent {
    Text(String),
    Blocks(Vec<ContentBlockInput>),
}

/// Input content block for messages with tool use/results.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlockInput {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        is_error: bool,
    },
}

/// Token usage from an LLM response.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    /// Tokens read from cache (Anthropic prompt caching).
    #[serde(default)]
    pub cache_read_input_tokens: u32,
    /// Tokens written to cache.
    #[serde(default)]
    pub cache_creation_input_tokens: u32,
}

/// Why the model stopped generating.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    ToolUse,
    StopSequence,
}

/// Full response from `complete_with_tools`.
#[derive(Debug, Clone)]
pub struct CompletionResponse {
    pub blocks: Vec<ContentBlock>,
    pub usage: Usage,
    pub stop_reason: Option<StopReason>,
}

/// A streaming event from an LLM response with tool support.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    TextDelta(String),
    ToolUseStart { id: String, name: String },
    ToolUseInputDelta(String),
    ToolUseEnd { id: String, input: serde_json::Value },
    Usage(Usage),
    Stop(StopReason),
}

/// A stream of tool-aware events from an LLM response.
pub type LlmToolStream = Pin<Box<dyn futures_util::Stream<Item = Result<StreamEvent>> + Send>>;

#[async_trait]
pub trait LlmClient: Send + Sync + 'static {
    async fn complete(&self, messages: Vec<ChatMessage>, system: &str) -> Result<String>;

    /// Complete with per-call overrides for model/max_tokens.
    /// Default implementation ignores options and calls `complete()`.
    async fn complete_with_options(
        &self,
        messages: Vec<ChatMessage>,
        system: &str,
        _options: CompletionOptions,
    ) -> Result<String> {
        if _options.model.is_some() || _options.max_tokens.is_some() {
            tracing::warn!("complete_with_options called with options but default impl ignores them — override this method");
        }
        self.complete(messages, system).await
    }

    /// Streaming variant — yields text chunks as they arrive.
    /// Default implementation calls `complete()` and yields the full response.
    async fn complete_stream(&self, messages: Vec<ChatMessage>, system: &str) -> Result<LlmStream> {
        let result = self.complete(messages, system).await?;
        Ok(Box::pin(futures_util::stream::once(async { Ok(result) })))
    }

    /// Complete with tool definitions. Returns content blocks, usage, and stop reason.
    /// Default implementation ignores tools and returns text only.
    async fn complete_with_tools(
        &self,
        messages: Vec<ChatMessageExt>,
        system: &str,
        tools: &[ToolDefinition],
        options: CompletionOptions,
    ) -> Result<CompletionResponse> {
        let _ = tools;
        let simple_messages: Vec<ChatMessage> = messages
            .into_iter()
            .filter_map(|m| match m.content {
                ChatContent::Text(t) => Some(ChatMessage {
                    role: m.role,
                    content: t,
                }),
                _ => None,
            })
            .collect();
        let text = self
            .complete_with_options(simple_messages, system, options)
            .await?;
        Ok(CompletionResponse {
            blocks: vec![ContentBlock::Text(text)],
            usage: Usage::default(),
            stop_reason: Some(StopReason::EndTurn),
        })
    }

    /// Streaming variant with tool support — yields StreamEvent as they arrive.
    /// Default implementation calls `complete_with_tools()` and yields events from the full response.
    async fn complete_stream_with_tools(
        &self,
        messages: Vec<ChatMessageExt>,
        system: &str,
        tools: &[ToolDefinition],
        options: CompletionOptions,
    ) -> Result<LlmToolStream> {
        let resp = self
            .complete_with_tools(messages, system, tools, options)
            .await?;
        let mut events: Vec<Result<StreamEvent>> = Vec::new();
        for block in &resp.blocks {
            match block {
                ContentBlock::Text(t) => events.push(Ok(StreamEvent::TextDelta(t.clone()))),
                ContentBlock::ToolUse(call) => {
                    events.push(Ok(StreamEvent::ToolUseStart {
                        id: call.id.clone(),
                        name: call.name.clone(),
                    }));
                    events.push(Ok(StreamEvent::ToolUseEnd {
                        id: call.id.clone(),
                        input: call.input.clone(),
                    }));
                }
            }
        }
        events.push(Ok(StreamEvent::Usage(resp.usage)));
        if let Some(reason) = resp.stop_reason {
            events.push(Ok(StreamEvent::Stop(reason)));
        }
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}
