//! Test doubles for LLM client and related types.
//!
//! Provides a configurable mock LLM client for unit and integration tests.

use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures_util::Stream;

use crate::client::{
    ChatMessage, CompletionOptions, CompletionResponse, LlmClient, LlmStream, LlmToolStream,
    StopReason, StreamEvent, ToolCall, ToolDefinition, Usage,
};
use crate::Result;

/// Mock LLM client for testing that returns predefined responses.
///
/// Supports both simple text responses and full completion responses with tools.
///
/// # Example
///
/// ```
/// use orka_llm::testing::MockLlmClient;
/// use orka_llm::client::LlmClient;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let mock = MockLlmClient::new()
///     .with_response("Hello, world!");
///
/// let result = mock.complete(vec![], "system").await?;
/// assert_eq!(result, "Hello, world!");
/// # Ok(())
/// # }
/// ```
pub struct MockLlmClient {
    responses: Arc<Mutex<VecDeque<String>>>,
    tool_responses: Arc<Mutex<VecDeque<CompletionResponse>>>,
    stream_responses: Arc<Mutex<VecDeque<Vec<String>>>>,
    error_after: Arc<Mutex<Option<usize>>>,
    call_count: Arc<Mutex<usize>>,
}

impl MockLlmClient {
    /// Create a new mock client with no predefined responses.
    pub fn new() -> Self {
        Self {
            responses: Arc::new(Mutex::new(VecDeque::new())),
            tool_responses: Arc::new(Mutex::new(VecDeque::new())),
            stream_responses: Arc::new(Mutex::new(VecDeque::new())),
            error_after: Arc::new(Mutex::new(None)),
            call_count: Arc::new(Mutex::new(0)),
        }
    }

    /// Set a single text response for all `complete` calls.
    pub fn with_response(self, response: impl Into<String>) -> Self {
        self.responses.lock().unwrap().push_back(response.into());
        self
    }

    /// Set multiple text responses that will be returned in sequence.
    pub fn with_responses(self, responses: Vec<String>) -> Self {
        *self.responses.lock().unwrap() = responses.into();
        self
    }

    /// Set a single tool response for `complete_with_tools` calls.
    pub fn with_tool_response(self, response: CompletionResponse) -> Self {
        self.tool_responses.lock().unwrap().push_back(response);
        self
    }

    /// Set multiple tool responses that will be returned in sequence.
    pub fn with_tool_responses(self, responses: Vec<CompletionResponse>) -> Self {
        *self.tool_responses.lock().unwrap() = responses.into();
        self
    }

    /// Set stream chunks for `complete_stream` calls.
    pub fn with_stream_chunks(self, chunks: Vec<String>) -> Self {
        self.stream_responses.lock().unwrap().push_back(chunks);
        self
    }

    /// Configure the mock to return an error after N calls.
    pub fn error_after(self, n: usize) -> Self {
        *self.error_after.lock().unwrap() = Some(n);
        self
    }

    /// Get the number of times the mock was called.
    pub fn call_count(&self) -> usize {
        *self.call_count.lock().unwrap()
    }

    /// Reset all responses and call count.
    pub fn reset(&self) {
        self.responses.lock().unwrap().clear();
        self.tool_responses.lock().unwrap().clear();
        self.stream_responses.lock().unwrap().clear();
        *self.call_count.lock().unwrap() = 0;
        *self.error_after.lock().unwrap() = None;
    }

    fn check_error(&self) -> Result<()> {
        if let Some(error_after) = *self.error_after.lock().unwrap() {
            let count = *self.call_count.lock().unwrap();
            if count >= error_after {
                return Err(crate::Error::Other(format!(
                    "Mock error triggered after {} calls",
                    error_after
                )));
            }
        }
        Ok(())
    }

    fn next_response(&self) -> String {
        let mut responses = self.responses.lock().unwrap();
        responses
            .pop_front()
            .unwrap_or_else(|| "mock response".to_string())
    }

    fn next_tool_response(&self) -> CompletionResponse {
        let mut responses = self.tool_responses.lock().unwrap();
        responses.pop_front().unwrap_or_else(|| CompletionResponse {
            content: Some("mock response".to_string()),
            tool_calls: Vec::new(),
            usage: Usage {
                prompt_tokens: 10,
                completion_tokens: 10,
                total_tokens: 20,
            },
            stop_reason: StopReason::Stop,
        })
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
        Ok(self.next_response())
    }

    async fn complete_with_options(
        &self,
        messages: Vec<ChatMessage>,
        system: &str,
        _options: CompletionOptions,
    ) -> Result<String> {
        self.complete(messages, system).await
    }

    async fn complete_stream(
        &self,
        _messages: Vec<ChatMessage>,
        _system: &str,
    ) -> Result<LlmStream> {
        *self.call_count.lock().unwrap() += 1;
        self.check_error()?;

        let chunks = self
            .stream_responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| vec![self.next_response()]);

        let stream = futures_util::stream::iter(chunks.into_iter().map(Ok));
        Ok(Box::pin(stream))
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

    async fn complete_stream_with_tools(
        &self,
        _messages: &[ChatMessage],
        _system: &str,
        _tools: &[ToolDefinition],
        _options: CompletionOptions,
    ) -> Result<LlmToolStream> {
        *self.call_count.lock().unwrap() += 1;
        self.check_error()?;

        let response = self.next_tool_response();
        let events = vec![
            Ok(StreamEvent::Text(response.content.unwrap_or_default())),
            Ok(StreamEvent::Usage(response.usage)),
            Ok(StreamEvent::Stop(response.stop_reason)),
        ];

        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}

/// Builder for constructing `CompletionResponse` objects in tests.
///
/// # Example
///
/// ```
/// use orka_llm::testing::CompletionResponseBuilder;
///
/// let response = CompletionResponseBuilder::new()
///     .content("Hello!")
///     .tool_call("web_search", "search_1", r#"{"query": "test"}"#)
///     .build();
/// ```
pub struct CompletionResponseBuilder {
    content: Option<String>,
    tool_calls: Vec<ToolCall>,
    usage: Usage,
    stop_reason: StopReason,
}

impl CompletionResponseBuilder {
    /// Create a new builder with default values.
    pub fn new() -> Self {
        Self {
            content: None,
            tool_calls: Vec::new(),
            usage: Usage {
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
            },
            stop_reason: StopReason::Stop,
        }
    }

    /// Set the response content.
    pub fn content(mut self, content: impl Into<String>) -> Self {
        self.content = Some(content.into());
        self
    }

    /// Add a tool call to the response.
    pub fn tool_call(
        mut self,
        name: impl Into<String>,
        id: impl Into<String>,
        input: impl Into<String>,
    ) -> Self {
        self.tool_calls.push(ToolCall {
            name: name.into(),
            id: id.into(),
            input: serde_json::from_str(&input.into()).unwrap_or_default(),
        });
        self
    }

    /// Set token usage.
    pub fn usage(mut self, prompt: u32, completion: u32) -> Self {
        self.usage = Usage {
            prompt_tokens: prompt,
            completion_tokens: completion,
            total_tokens: prompt + completion,
        };
        self
    }

    /// Set the stop reason.
    pub fn stop_reason(mut self, reason: StopReason) -> Self {
        self.stop_reason = reason;
        self
    }

    /// Build the final `CompletionResponse`.
    pub fn build(self) -> CompletionResponse {
        CompletionResponse {
            content: self.content,
            tool_calls: self.tool_calls,
            usage: self.usage,
            stop_reason: self.stop_reason,
        }
    }
}

impl Default for CompletionResponseBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_llm_returns_configured_response() {
        let mock = MockLlmClient::new().with_response("Hello!");
        let result = mock.complete(vec![], "system").await.unwrap();
        assert_eq!(result, "Hello!");
        assert_eq!(mock.call_count(), 1);
    }

    #[tokio::test]
    async fn mock_llm_returns_sequential_responses() {
        let mock = MockLlmClient::new().with_responses(vec![
            "First".to_string(),
            "Second".to_string(),
        ]);
        assert_eq!(mock.complete(vec![], "").await.unwrap(), "First");
        assert_eq!(mock.complete(vec![], "").await.unwrap(), "Second");
        assert_eq!(mock.call_count(), 2);
    }

    #[tokio::test]
    async fn mock_llm_returns_default_when_empty() {
        let mock = MockLlmClient::new();
        let result = mock.complete(vec![], "").await.unwrap();
        assert_eq!(result, "mock response");
    }

    #[tokio::test]
    async fn mock_llm_streams_chunks() {
        let mock = MockLlmClient::new().with_stream_chunks(vec![
            "Hello".to_string(),
            " ".to_string(),
            "world".to_string(),
        ]);
        let mut stream = mock.complete_stream(vec![], "").await.unwrap();
        let mut result = String::new();
        while let Some(chunk) = stream.next().await {
            result.push_str(&chunk.unwrap());
        }
        assert_eq!(result, "Hello world");
    }

    #[tokio::test]
    async fn mock_llm_returns_tool_response() {
        let response = CompletionResponseBuilder::new()
            .content("Using tool...")
            .tool_call("web_search", "call_1", r#"{"query": "test"}"#)
            .usage(10, 20)
            .build();

        let mock = MockLlmClient::new().with_tool_response(response);
        let result = mock
            .complete_with_tools(&[], "", &[], CompletionOptions::default())
            .await
            .unwrap();

        assert_eq!(result.content, Some("Using tool...".to_string()));
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "web_search");
        assert_eq!(result.usage.total_tokens, 30);
    }

    #[tokio::test]
    async fn mock_llm_errors_after_n_calls() {
        let mock = MockLlmClient::new()
            .with_response("success")
            .error_after(2);

        assert!(mock.complete(vec![], "").await.is_ok());
        assert!(mock.complete(vec![], "").await.is_ok());
        assert!(mock.complete(vec![], "").await.is_err());
    }

    #[tokio::test]
    async fn mock_llm_resets_properly() {
        let mock = MockLlmClient::new().with_response("test");
        let _ = mock.complete(vec![], "").await;
        assert_eq!(mock.call_count(), 1);

        mock.reset();
        assert_eq!(mock.call_count(), 0);
        assert_eq!(mock.complete(vec![], "").await.unwrap(), "mock response");
    }
}
