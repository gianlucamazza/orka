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
            reasoning_tokens: 0,
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

/// Extended thinking / reasoning configuration.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ThinkingConfig {
    /// Anthropic extended thinking with a token budget.
    Enabled {
        /// Maximum tokens the model may spend on thinking.
        budget_tokens: u32,
    },
    /// OpenAI o-series reasoning effort.
    ReasoningEffort(ReasoningEffort),
}

/// OpenAI o-series reasoning effort level.
#[derive(Debug, Clone, Copy)]
pub enum ReasoningEffort {
    /// Low effort — fastest, least thorough.
    Low,
    /// Medium effort — balanced.
    Medium,
    /// High effort — most thorough.
    High,
}

impl ReasoningEffort {
    /// Return the string value expected by the OpenAI API.
    pub fn as_str(self) -> &'static str {
        match self {
            ReasoningEffort::Low => "low",
            ReasoningEffort::Medium => "medium",
            ReasoningEffort::High => "high",
        }
    }
}

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
    /// Sampling temperature (0.0–2.0). Note: Anthropic requires temperature=1 when thinking is enabled.
    pub temperature: Option<f32>,
    /// Extended thinking / reasoning configuration.
    pub thinking: Option<ThinkingConfig>,
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

/// A content block in an LLM response — either text, a tool call, or reasoning.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ContentBlock {
    /// A plain-text response fragment.
    Text(String),
    /// A tool invocation requested by the model.
    ToolUse(ToolCall),
    /// Extended thinking/reasoning text produced before the response.
    Thinking(String),
}

/// Content can be simple text or a list of content blocks (for tool results).
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
#[serde(untagged)]
pub enum ChatContent {
    /// A simple plain-text message.
    Text(String),
    /// A sequence of typed content blocks (text, tool use, tool results).
    Blocks(Vec<ContentBlockInput>),
}

impl<'de> Deserialize<'de> for ChatContent {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::Null => Ok(ChatContent::Text(String::new())),
            serde_json::Value::String(s) => Ok(ChatContent::Text(s)),
            serde_json::Value::Array(_) => {
                let blocks = serde_json::from_value(value).map_err(serde::de::Error::custom)?;
                Ok(ChatContent::Blocks(blocks))
            }
            other => Err(serde::de::Error::custom(format!(
                "unexpected ChatContent value: {other}"
            ))),
        }
    }
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
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        is_error: bool,
    },
    /// A thinking/reasoning block returned by reasoning-capable models.
    #[serde(rename = "thinking")]
    Thinking {
        /// The model's internal reasoning text.
        thinking: String,
    },
    /// Unknown block type — ignored gracefully to avoid deserialization failures
    /// when the API introduces new block types.
    #[serde(other)]
    Unknown,
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
    /// Tokens consumed by extended thinking / reasoning (Anthropic + OpenAI o-series).
    #[serde(default)]
    pub reasoning_tokens: u32,
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
    /// An incremental thinking/reasoning chunk (extended thinking models).
    ThinkingDelta(String),
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
        messages: &[ChatMessage],
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
        messages: &[ChatMessage],
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
                ContentBlock::Thinking(t) => events.push(Ok(StreamEvent::ThinkingDelta(t.clone()))),
            }
        }
        events.push(Ok(StreamEvent::Usage(resp.usage)));
        if let Some(reason) = resp.stop_reason {
            events.push(Ok(StreamEvent::Stop(reason)));
        }
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_content_deserializes_null_as_empty_text() {
        let c: ChatContent = serde_json::from_str("null").unwrap();
        assert!(matches!(c, ChatContent::Text(s) if s.is_empty()));
    }

    #[test]
    fn chat_content_deserializes_string() {
        let c: ChatContent = serde_json::from_str("\"hello\"").unwrap();
        assert!(matches!(c, ChatContent::Text(s) if s == "hello"));
    }

    #[test]
    fn chat_content_deserializes_known_blocks() {
        let json =
            r#"[{"type":"text","text":"hi"},{"type":"tool_use","id":"1","name":"foo","input":{}}]"#;
        let c: ChatContent = serde_json::from_str(json).unwrap();
        assert!(matches!(c, ChatContent::Blocks(_)));
    }

    #[test]
    fn content_block_input_thinking_type_is_deserialized() {
        let json = r#"{"type":"thinking","thinking":"some internal thought"}"#;
        let b: ContentBlockInput = serde_json::from_str(json).unwrap();
        assert!(
            matches!(b, ContentBlockInput::Thinking { thinking } if thinking == "some internal thought")
        );
    }

    #[test]
    fn content_block_input_unknown_type_is_ignored() {
        let json = r#"{"type":"unknown_future_block","data":42}"#;
        let b: ContentBlockInput = serde_json::from_str(json).unwrap();
        assert!(matches!(b, ContentBlockInput::Unknown));
    }

    #[test]
    fn chat_content_with_mixed_blocks_including_thinking() {
        let json = r#"[
            {"type":"text","text":"hello"},
            {"type":"thinking","thinking":"internal"},
            {"type":"tool_use","id":"x","name":"bar","input":{}}
        ]"#;
        let c: ChatContent = serde_json::from_str(json).unwrap();
        let ChatContent::Blocks(blocks) = c else {
            panic!("expected Blocks")
        };
        assert_eq!(blocks.len(), 3);
        assert!(matches!(&blocks[0], ContentBlockInput::Text { .. }));
        assert!(matches!(&blocks[1], ContentBlockInput::Thinking { .. }));
        assert!(matches!(&blocks[2], ContentBlockInput::ToolUse { .. }));
    }

    #[test]
    fn role_display_variants() {
        assert_eq!(Role::System.to_string(), "system");
        assert_eq!(Role::User.to_string(), "user");
        assert_eq!(Role::Assistant.to_string(), "assistant");
        assert_eq!(Role::Tool.to_string(), "tool");
    }

    #[test]
    fn chat_message_user_constructor() {
        let msg = ChatMessage::user("hi");
        assert_eq!(msg.role, Role::User);
        assert!(matches!(msg.content, ChatContent::Text(s) if s == "hi"));
    }

    #[test]
    fn chat_message_assistant_constructor() {
        let msg = ChatMessage::assistant("reply");
        assert_eq!(msg.role, Role::Assistant);
        assert!(matches!(msg.content, ChatContent::Text(s) if s == "reply"));
    }

    #[test]
    fn chat_message_system_constructor() {
        let msg = ChatMessage::system("instructions");
        assert_eq!(msg.role, Role::System);
        assert!(matches!(msg.content, ChatContent::Text(s) if s == "instructions"));
    }

    #[test]
    fn tool_definition_new() {
        let td = ToolDefinition::new(
            "search",
            "web search",
            serde_json::json!({"type": "object"}),
        );
        assert_eq!(td.name, "search");
        assert_eq!(td.description, "web search");
        assert_eq!(td.input_schema["type"], "object");
    }

    #[test]
    fn tool_call_new() {
        let tc = ToolCall::new("id-1", "search", serde_json::json!({"q": "rust"}));
        assert_eq!(tc.id, "id-1");
        assert_eq!(tc.name, "search");
        assert_eq!(tc.input["q"], "rust");
    }

    #[test]
    fn tool_result_new() {
        let tr = ToolResult::new("id-1", "found it", true);
        assert_eq!(tr.tool_use_id, "id-1");
        assert_eq!(tr.content, "found it");
        assert!(tr.is_error);
    }

    #[test]
    fn reasoning_effort_as_str() {
        assert_eq!(ReasoningEffort::Low.as_str(), "low");
        assert_eq!(ReasoningEffort::Medium.as_str(), "medium");
        assert_eq!(ReasoningEffort::High.as_str(), "high");
    }

    #[test]
    fn usage_new_sets_fields() {
        let u = Usage::new(100, 50);
        assert_eq!(u.input_tokens, 100);
        assert_eq!(u.output_tokens, 50);
        assert_eq!(u.cache_read_input_tokens, 0);
        assert_eq!(u.cache_creation_input_tokens, 0);
        assert_eq!(u.reasoning_tokens, 0);
    }

    #[test]
    fn stop_reason_serde_roundtrip() {
        for (reason, expected_json) in [
            (StopReason::EndTurn, "\"end_turn\""),
            (StopReason::MaxTokens, "\"max_tokens\""),
            (StopReason::ToolUse, "\"tool_use\""),
            (StopReason::StopSequence, "\"stop_sequence\""),
        ] {
            let json = serde_json::to_string(&reason).unwrap();
            assert_eq!(json, expected_json);
            let back: StopReason = serde_json::from_str(&json).unwrap();
            assert_eq!(back, reason);
        }
    }

    #[test]
    fn tool_result_serde_round_trip_success() {
        // is_error=false is skip_serializing, so the field is absent in JSON.
        // Without #[serde(default)] this would fail to deserialize — regression guard.
        let msg = ChatMessage::new(
            Role::User,
            ChatContent::Blocks(vec![ContentBlockInput::ToolResult {
                tool_use_id: "t1".into(),
                content: "output".into(),
                is_error: false,
            }]),
        );
        let json = serde_json::to_value(&msg).unwrap();
        // Confirm is_error is absent
        let blocks = json["content"].as_array().unwrap();
        assert!(blocks[0].get("is_error").is_none());
        // Must round-trip without error
        let back: ChatMessage = serde_json::from_value(json).unwrap();
        assert!(matches!(
            back.content,
            ChatContent::Blocks(ref b) if matches!(b[0], ContentBlockInput::ToolResult { is_error: false, .. })
        ));
    }

    #[test]
    fn tool_result_serde_round_trip_error() {
        let msg = ChatMessage::new(
            Role::User,
            ChatContent::Blocks(vec![ContentBlockInput::ToolResult {
                tool_use_id: "t2".into(),
                content: "err".into(),
                is_error: true,
            }]),
        );
        let json = serde_json::to_value(&msg).unwrap();
        // is_error=true must be present in JSON
        assert_eq!(json["content"][0]["is_error"], true);
        // Must round-trip without error
        let back: ChatMessage = serde_json::from_value(json).unwrap();
        assert!(matches!(
            back.content,
            ChatContent::Blocks(ref b) if matches!(b[0], ContentBlockInput::ToolResult { is_error: true, .. })
        ));
    }
}
