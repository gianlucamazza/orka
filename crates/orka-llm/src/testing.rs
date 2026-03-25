//! Test doubles for LLM client and related types.
//!
//! Provides a configurable mock LLM client for unit and integration tests.
//! [`MockLlmClient`] overrides `complete` and `complete_with_tools`; the
//! `complete_stream_with_tools` method intentionally relies on the default
//! trait implementation so that stream events are always derived correctly from
//! the queued [`CompletionResponse`].

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use orka_core::{Error, Result};

use crate::client::{
    ChatMessage, CompletionOptions, CompletionResponse, ContentBlock, LlmClient, StopReason,
    ToolCall, ToolDefinition, Usage,
};

/// Mock LLM client for testing that returns predefined responses.
///
/// Use [`MockLlmClient::with_text_response`] for simple `complete` paths and
/// [`MockLlmClient::with_tool_response`] for the full tool-loop paths used by
/// `run_agent_node`.
///
/// # Example
///
/// ```no_run
/// use orka_llm::{
///     client::{LlmClient, StopReason},
///     testing::{CompletionResponseBuilder, MockLlmClient},
/// };
///
/// let mock = MockLlmClient::new().with_tool_response(
///     CompletionResponseBuilder::new()
///         .text("Hello, world!")
///         .stop_reason(StopReason::EndTurn)
///         .build(),
/// );
/// ```
pub struct MockLlmClient {
    text_responses: Arc<Mutex<VecDeque<String>>>,
    tool_responses: Arc<Mutex<VecDeque<CompletionResponse>>>,
    error_after: Arc<Mutex<Option<usize>>>,
    call_count: Arc<Mutex<usize>>,
}

impl MockLlmClient {
    /// Create a new mock client with no predefined responses.
    pub fn new() -> Self {
        Self {
            text_responses: Arc::new(Mutex::new(VecDeque::new())),
            tool_responses: Arc::new(Mutex::new(VecDeque::new())),
            error_after: Arc::new(Mutex::new(None)),
            call_count: Arc::new(Mutex::new(0)),
        }
    }

    /// Enqueue a plain-text reply returned by `complete`.
    pub fn with_text_response(self, response: impl Into<String>) -> Self {
        self.text_responses
            .lock()
            .unwrap()
            .push_back(response.into());
        self
    }

    /// Enqueue a [`CompletionResponse`] returned by `complete_with_tools`
    /// (and transitively by the default `complete_stream_with_tools`).
    pub fn with_tool_response(self, response: CompletionResponse) -> Self {
        self.tool_responses.lock().unwrap().push_back(response);
        self
    }

    /// Configure the mock to return an error after N successful calls.
    pub fn error_after(self, n: usize) -> Self {
        *self.error_after.lock().unwrap() = Some(n);
        self
    }

    /// Return the number of times the mock has been called.
    pub fn call_count(&self) -> usize {
        *self.call_count.lock().unwrap()
    }

    fn check_error(&self) -> Result<()> {
        if let Some(limit) = *self.error_after.lock().unwrap()
            && *self.call_count.lock().unwrap() > limit
        {
            return Err(Error::Other(format!(
                "Mock error triggered after {limit} calls"
            )));
        }
        Ok(())
    }

    fn next_text_response(&self) -> String {
        self.text_responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| "mock response".to_string())
    }

    fn next_tool_response(&self) -> CompletionResponse {
        if let Some(resp) = self.tool_responses.lock().unwrap().pop_front() {
            resp
        } else {
            // Fall back to the text queue, wrapped as a single Text block.
            let text = self.next_text_response();
            CompletionResponse {
                blocks: vec![ContentBlock::Text(text)],
                usage: Usage::default(),
                stop_reason: Some(StopReason::EndTurn),
            }
        }
    }
}

impl Default for MockLlmClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl LlmClient for MockLlmClient {
    async fn complete(&self, _messages: Vec<ChatMessage>, _system: &str) -> Result<String> {
        *self.call_count.lock().unwrap() += 1;
        self.check_error()?;
        Ok(self.next_text_response())
    }

    async fn complete_with_tools(
        &self,
        _messages: &[ChatMessage],
        _system: &str,
        _tools: &[ToolDefinition],
        _options: CompletionOptions,
    ) -> Result<CompletionResponse> {
        *self.call_count.lock().unwrap() += 1;
        self.check_error()?;
        Ok(self.next_tool_response())
    }

    // `complete_stream_with_tools` uses the default trait implementation, which
    // delegates to `complete_with_tools` and converts the response into the
    // correct `StreamEvent` sequence.  Overriding it here would duplicate that
    // logic and risk drift.
}

// ---------------------------------------------------------------------------
// CompletionResponseBuilder
// ---------------------------------------------------------------------------

/// Fluent builder for [`CompletionResponse`] objects used in tests.
///
/// # Example
///
/// ```
/// use orka_llm::{client::StopReason, testing::CompletionResponseBuilder};
///
/// let response = CompletionResponseBuilder::new()
///     .text("Hello!")
///     .stop_reason(StopReason::EndTurn)
///     .build();
/// ```
pub struct CompletionResponseBuilder {
    blocks: Vec<ContentBlock>,
    usage: Usage,
    stop_reason: Option<StopReason>,
}

impl CompletionResponseBuilder {
    /// Create a new builder with empty blocks and zero usage.
    pub fn new() -> Self {
        Self {
            blocks: Vec::new(),
            usage: Usage::default(),
            stop_reason: None,
        }
    }

    /// Append a [`ContentBlock::Text`] block.
    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.blocks.push(ContentBlock::Text(text.into()));
        self
    }

    /// Append a [`ContentBlock::ToolUse`] block.
    pub fn tool_use(
        mut self,
        id: impl Into<String>,
        name: impl Into<String>,
        input: serde_json::Value,
    ) -> Self {
        self.blocks
            .push(ContentBlock::ToolUse(ToolCall::new(id, name, input)));
        self
    }

    /// Set the input/output token usage.
    pub fn usage(mut self, input_tokens: u32, output_tokens: u32) -> Self {
        self.usage = Usage::new(input_tokens, output_tokens);
        self
    }

    /// Set the stop reason.
    pub fn stop_reason(mut self, reason: StopReason) -> Self {
        self.stop_reason = Some(reason);
        self
    }

    /// Consume the builder and return the [`CompletionResponse`].
    pub fn build(self) -> CompletionResponse {
        CompletionResponse::new(self.blocks, self.usage, self.stop_reason)
    }
}

impl Default for CompletionResponseBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use futures_util::StreamExt;

    use super::*;
    use crate::client::{CompletionOptions, StreamEvent};

    #[tokio::test]
    async fn mock_returns_configured_text_response() {
        let mock = MockLlmClient::new().with_text_response("Hello!");
        let result = mock.complete(vec![], "").await.unwrap();
        assert_eq!(result, "Hello!");
        assert_eq!(mock.call_count(), 1);
    }

    #[tokio::test]
    async fn mock_returns_default_when_empty() {
        let mock = MockLlmClient::new();
        let result = mock.complete(vec![], "").await.unwrap();
        assert_eq!(result, "mock response");
    }

    #[tokio::test]
    async fn mock_returns_sequential_text_responses() {
        let mock = MockLlmClient::new()
            .with_text_response("first")
            .with_text_response("second");
        assert_eq!(mock.complete(vec![], "").await.unwrap(), "first");
        assert_eq!(mock.complete(vec![], "").await.unwrap(), "second");
        assert_eq!(mock.call_count(), 2);
    }

    #[tokio::test]
    async fn mock_returns_tool_response() {
        let response = CompletionResponseBuilder::new()
            .text("Using tool...")
            .tool_use("call_1", "web_search", serde_json::json!({"query": "test"}))
            .usage(10, 20)
            .stop_reason(StopReason::ToolUse)
            .build();

        let mock = MockLlmClient::new().with_tool_response(response);
        let result = mock
            .complete_with_tools(&[], "", &[], CompletionOptions::default())
            .await
            .unwrap();

        assert_eq!(result.blocks.len(), 2);
        assert!(matches!(&result.blocks[0], ContentBlock::Text(t) if t == "Using tool..."));
        assert!(matches!(&result.blocks[1], ContentBlock::ToolUse(c) if c.name == "web_search"));
        assert_eq!(result.usage.input_tokens, 10);
        assert_eq!(result.usage.output_tokens, 20);
        assert_eq!(result.stop_reason, Some(StopReason::ToolUse));
    }

    #[tokio::test]
    async fn tool_response_falls_back_to_text_when_queue_empty() {
        let mock = MockLlmClient::new().with_text_response("fallback");
        let result = mock
            .complete_with_tools(&[], "", &[], CompletionOptions::default())
            .await
            .unwrap();
        assert!(matches!(&result.blocks[0], ContentBlock::Text(t) if t == "fallback"));
    }

    #[tokio::test]
    async fn mock_errors_after_n_calls() {
        let mock = MockLlmClient::new().with_text_response("ok").error_after(2);
        assert!(mock.complete(vec![], "").await.is_ok());
        assert!(mock.complete(vec![], "").await.is_ok());
        assert!(mock.complete(vec![], "").await.is_err());
    }

    #[tokio::test]
    async fn stream_with_tools_emits_text_delta_via_default_impl() {
        let response = CompletionResponseBuilder::new()
            .text("stream reply")
            .stop_reason(StopReason::EndTurn)
            .build();

        let mock = MockLlmClient::new().with_tool_response(response);
        let mut stream = mock
            .complete_stream_with_tools(&[], "", &[], CompletionOptions::default())
            .await
            .unwrap();

        let mut text_seen = false;
        while let Some(event) = stream.next().await {
            if let Ok(StreamEvent::TextDelta(t)) = event {
                assert_eq!(t, "stream reply");
                text_seen = true;
            }
        }
        assert!(text_seen, "expected at least one TextDelta event");
    }

    #[tokio::test]
    async fn stream_with_tools_emits_tool_use_events() {
        let response = CompletionResponseBuilder::new()
            .tool_use("id1", "search", serde_json::json!({"q": "rust"}))
            .stop_reason(StopReason::ToolUse)
            .build();

        let mock = MockLlmClient::new().with_tool_response(response);
        let events: Vec<_> = mock
            .complete_stream_with_tools(&[], "", &[], CompletionOptions::default())
            .await
            .unwrap()
            .collect()
            .await;

        let has_start = events
            .iter()
            .any(|e| matches!(e, Ok(StreamEvent::ToolUseStart { name, .. }) if name == "search"));
        assert!(has_start, "expected ToolUseStart for 'search'");
    }

    #[test]
    fn builder_constructs_response_correctly() {
        let resp = CompletionResponseBuilder::new()
            .text("hello")
            .tool_use("id1", "search", serde_json::json!({"q": "rust"}))
            .usage(5, 10)
            .stop_reason(StopReason::ToolUse)
            .build();

        assert_eq!(resp.blocks.len(), 2);
        assert_eq!(resp.usage.input_tokens, 5);
        assert_eq!(resp.usage.output_tokens, 10);
        assert_eq!(resp.stop_reason, Some(StopReason::ToolUse));
    }
}
