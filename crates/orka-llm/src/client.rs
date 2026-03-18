use std::pin::Pin;

use async_trait::async_trait;
use orka_core::Result;
use serde::{Deserialize, Serialize};

/// Internal error type used by LLM provider retry logic to distinguish
/// transient (retryable) from fatal (non-retryable) errors.
pub(crate) enum RetryableError {
    Transient(String),
    Fatal(String),
}

/// Message role in a chat conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// System instructions.
    System,
    /// Human user turn.
    User,
    /// Model assistant turn.
    Assistant,
    /// Tool result turn.
    Tool,
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Role::System => write!(f, "system"),
            Role::User => write!(f, "user"),
            Role::Assistant => write!(f, "assistant"),
            Role::Tool => write!(f, "tool"),
        }
    }
}

/// A chat message with a typed role and structured content.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ChatMessage {
    /// Message role.
    pub role: Role,
    /// Message content — either plain text or a list of content blocks.
    pub content: ChatContent,
}

impl ChatMessage {
    /// Create a new chat message.
    pub fn new(role: Role, content: ChatContent) -> Self {
        Self { role, content }
    }

    /// Convenience: create a user text message.
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: ChatContent::Text(content.into()),
        }
    }

    /// Convenience: create an assistant text message.
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: ChatContent::Text(content.into()),
        }
    }

    /// Convenience: create a system text message.
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: ChatContent::Text(content.into()),
        }
    }
}

/// Backward-compatible alias: `ChatMessageExt` is now the same as `ChatMessage`.
pub type ChatMessageExt = ChatMessage;

impl ToolDefinition {
    /// Create a new tool definition.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: serde_json::Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            input_schema,
        }
    }
}

impl ToolCall {
    /// Create a new tool call.
    pub fn new(id: impl Into<String>, name: impl Into<String>, input: serde_json::Value) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            input,
        }
    }
}

impl ToolResult {
    /// Create a new tool result.
    pub fn new(tool_use_id: impl Into<String>, content: impl Into<String>, is_error: bool) -> Self {
        Self {
            tool_use_id: tool_use_id.into(),
            content: content.into(),
            is_error,
        }
    }
}

impl Usage {
    /// Create a new usage with the given token counts.
    pub fn new(input_tokens: u32, output_tokens: u32) -> Self {
        Self {
            input_tokens,
            output_tokens,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        }
    }
}

impl CompletionResponse {
    /// Create a new completion response.
    pub fn new(blocks: Vec<ContentBlock>, usage: Usage, stop_reason: Option<StopReason>) -> Self {
        Self {
            blocks,
            usage,
            stop_reason,
        }
    }
}

/// A stream of text chunks from an LLM response.
pub type LlmStream = Pin<Box<dyn futures_util::Stream<Item = Result<String>> + Send>>;

/// Structured output format.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ResponseFormat {
    /// Request JSON output matching this schema.
    JsonSchema {
        /// Name used to identify the schema in the request.
        name: String,
        /// JSON Schema object describing the expected output structure.
        schema: serde_json::Value,
    },
    /// Just request JSON output without a specific schema.
    Json,
}

/// Per-call overrides for model, token limit, and output format.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct CompletionOptions {
    /// Override the model name for this request.
    pub model: Option<String>,
    /// Override the maximum output tokens for this request.
    pub max_tokens: Option<u32>,
    /// Optional JSON Schema for structured/constrained output.
    pub response_format: Option<ResponseFormat>,
}

/// A tool definition sent to the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ToolDefinition {
    /// Unique tool name used to identify calls by the model.
    pub name: String,
    /// Human-readable description of what the tool does.
    pub description: String,
    /// JSON Schema describing the tool's input parameters.
    pub input_schema: serde_json::Value,
}

/// A tool call requested by the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ToolCall {
    /// Unique ID for this tool invocation, used to correlate with the result.
    pub id: String,
    /// Name of the tool being called.
    pub name: String,
    /// JSON-encoded arguments for the tool.
    pub input: serde_json::Value,
}

/// A tool result to feed back to the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ToolResult {
    /// ID of the tool call this result belongs to.
    pub tool_use_id: String,
    /// Serialised output from the tool execution.
    pub content: String,
    /// Whether the tool execution produced an error.
    pub is_error: bool,
}

/// A content block in an LLM response — either text or a tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ContentBlock {
    /// A plain-text response fragment.
    Text(String),
    /// A tool invocation requested by the model.
    ToolUse(ToolCall),
}

/// Content can be simple text or a list of content blocks (for tool results).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(untagged)]
pub enum ChatContent {
    /// A simple plain-text message.
    Text(String),
    /// A sequence of typed content blocks (text, tool use, tool results).
    Blocks(Vec<ContentBlockInput>),
}

/// Input content block for messages with tool use/results.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(tag = "type")]
pub enum ContentBlockInput {
    /// A plain-text block.
    #[serde(rename = "text")]
    Text {
        /// The text content.
        text: String,
    },
    /// A tool invocation block sent by the model.
    #[serde(rename = "tool_use")]
    ToolUse {
        /// Unique ID for this tool call.
        id: String,
        /// Name of the tool to invoke.
        name: String,
        /// JSON-encoded input arguments.
        input: serde_json::Value,
    },
    /// The result of a tool call, returned by the client.
    #[serde(rename = "tool_result")]
    ToolResult {
        /// ID of the tool call being answered.
        tool_use_id: String,
        /// Output content from the tool.
        content: String,
        /// Whether the tool produced an error result.
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        is_error: bool,
    },
}

/// Token usage from an LLM response.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Usage {
    /// Number of tokens in the prompt (input).
    pub input_tokens: u32,
    /// Number of tokens generated (output).
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
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// Model finished naturally.
    EndTurn,
    /// Output token limit was reached.
    MaxTokens,
    /// Model requested one or more tool calls.
    ToolUse,
    /// A stop sequence was encountered.
    StopSequence,
}

/// Full response from `complete_with_tools`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CompletionResponse {
    /// Ordered content blocks (text and/or tool calls) produced by the model.
    pub blocks: Vec<ContentBlock>,
    /// Token usage reported by the provider.
    pub usage: Usage,
    /// Reason the model stopped generating, if provided.
    pub stop_reason: Option<StopReason>,
}

/// A streaming event from an LLM response with tool support.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum StreamEvent {
    /// An incremental text chunk.
    TextDelta(String),
    /// The model is beginning a tool call.
    ToolUseStart {
        /// Unique ID for this tool call.
        id: String,
        /// Name of the tool being invoked.
        name: String,
    },
    /// An incremental chunk of the tool call's JSON input.
    ToolUseInputDelta(String),
    /// The tool call's input is fully assembled.
    ToolUseEnd {
        /// Unique ID for this tool call.
        id: String,
        /// Fully parsed tool input arguments.
        input: serde_json::Value,
    },
    /// Token usage snapshot, emitted when the stream ends.
    Usage(Usage),
    /// The model has stopped generating.
    Stop(StopReason),
}

/// A stream of tool-aware events from an LLM response.
pub type LlmToolStream = Pin<Box<dyn futures_util::Stream<Item = Result<StreamEvent>> + Send>>;

/// Async LLM client supporting text completion, streaming, and tool use.
#[async_trait]
pub trait LlmClient: Send + Sync + 'static {
    /// Complete a chat conversation, returning the assistant's reply.
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
            tracing::warn!(
                "complete_with_options called with options but default impl ignores them — override this method"
            );
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
        messages: &[ChatMessageExt],
        system: &str,
        tools: &[ToolDefinition],
        options: CompletionOptions,
    ) -> Result<CompletionResponse> {
        if !tools.is_empty() {
            tracing::warn!(
                tool_count = tools.len(),
                "complete_with_tools called on backend that does not support tools; tools will be ignored"
            );
        }
        let simple_messages: Vec<ChatMessage> = messages
            .iter()
            .filter_map(|m| match &m.content {
                ChatContent::Text(_) => Some(m.clone()),
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
        messages: &[ChatMessageExt],
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
