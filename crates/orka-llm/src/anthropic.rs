use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::Client;
use serde_json::json;
use tracing::{debug, warn};

use crate::client::{
    ChatContent, ChatMessage, ChatMessageExt, CompletionOptions, CompletionResponse, ContentBlock,
    LlmClient, LlmStream, LlmToolStream, StopReason, StreamEvent, ToolCall, ToolDefinition, Usage,
};
use orka_core::{Error, Result};

const API_URL: &str = "https://api.anthropic.com/v1/messages";

pub struct AnthropicClient {
    client: Client,
    api_key: String,
    model: String,
    max_tokens: u32,
    max_retries: u32,
    api_version: String,
}

impl AnthropicClient {
    pub fn new(api_key: String, model: String) -> Self {
        Self::with_options(api_key, model, 30, 4096, 2, "2023-06-01".into())
    }

    pub fn with_options(
        api_key: String,
        model: String,
        timeout_secs: u64,
        max_tokens: u32,
        max_retries: u32,
        api_version: String,
    ) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .unwrap_or_default();
        Self {
            client,
            api_key,
            model,
            max_tokens,
            max_retries,
            api_version,
        }
    }

    /// Send a request with retry logic for 429/5xx and transient errors.
    /// Returns the raw successful HTTP response.
    async fn send_request_with_retry(&self, body: &serde_json::Value) -> Result<reqwest::Response> {
        let mut last_err = None;
        for attempt in 0..=self.max_retries {
            if attempt > 0 {
                let delay = Duration::from_millis(500 * 2u64.pow(attempt - 1));
                warn!(
                    attempt,
                    delay_ms = delay.as_millis() as u64,
                    "retrying Anthropic API call"
                );
                tokio::time::sleep(delay).await;
            }

            let result = self
                .client
                .post(API_URL)
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", &self.api_version)
                .header("content-type", "application/json")
                .json(body)
                .send()
                .await;

            match result {
                Ok(response) => {
                    let status = response.status();

                    if status == 429 || status.is_server_error() {
                        let body_text = response.text().await.unwrap_or_default();
                        last_err = Some(format!("Anthropic API error {status}: {body_text}"));
                        continue;
                    }

                    if !status.is_success() {
                        let body_text = response.text().await.unwrap_or_default();
                        return Err(Error::Other(format!(
                            "Anthropic API error {status}: {body_text}"
                        )));
                    }

                    return Ok(response);
                }
                Err(e) => {
                    if e.is_timeout() || e.is_connect() {
                        last_err = Some(format!("Anthropic API request failed: {e}"));
                        continue;
                    }
                    return Err(Error::Other(format!("Anthropic API request failed: {e}")));
                }
            }
        }

        Err(Error::Other(last_err.unwrap_or_else(|| {
            "Anthropic API request failed after retries".into()
        })))
    }

    /// Send a request with retry and parse the JSON response.
    async fn send_with_retry(&self, body: &serde_json::Value) -> Result<serde_json::Value> {
        let response = self.send_request_with_retry(body).await?;
        response
            .json()
            .await
            .map_err(|e| Error::Other(format!("failed to parse Anthropic response: {e}")))
    }

    /// Parse usage from Anthropic response JSON.
    fn parse_usage(resp: &serde_json::Value) -> Usage {
        let usage = &resp["usage"];
        Usage {
            input_tokens: usage["input_tokens"].as_u64().unwrap_or(0) as u32,
            output_tokens: usage["output_tokens"].as_u64().unwrap_or(0) as u32,
            cache_read_input_tokens: usage["cache_read_input_tokens"].as_u64().unwrap_or(0) as u32,
            cache_creation_input_tokens: usage["cache_creation_input_tokens"].as_u64().unwrap_or(0)
                as u32,
        }
    }

    /// Parse stop_reason from Anthropic response JSON.
    fn parse_stop_reason(resp: &serde_json::Value) -> Option<StopReason> {
        match resp["stop_reason"].as_str() {
            Some("end_turn") => Some(StopReason::EndTurn),
            Some("max_tokens") => Some(StopReason::MaxTokens),
            Some("tool_use") => Some(StopReason::ToolUse),
            Some("stop_sequence") => Some(StopReason::StopSequence),
            _ => None,
        }
    }

    /// Parse content blocks from Anthropic response JSON.
    fn parse_content_blocks(resp: &serde_json::Value) -> Vec<ContentBlock> {
        resp["content"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|block| match block["type"].as_str() {
                        Some("text") => {
                            let text = block["text"].as_str().unwrap_or("").to_string();
                            Some(ContentBlock::Text(text))
                        }
                        Some("tool_use") => {
                            let id = block["id"].as_str().unwrap_or("").to_string();
                            let name = block["name"].as_str().unwrap_or("").to_string();
                            let input = block["input"].clone();
                            Some(ContentBlock::ToolUse(ToolCall { id, name, input }))
                        }
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Build API messages from ChatMessageExt.
    fn build_ext_messages(messages: &[ChatMessageExt]) -> Vec<serde_json::Value> {
        messages
            .iter()
            .map(|m| match &m.content {
                ChatContent::Text(t) => json!({"role": m.role, "content": t}),
                ChatContent::Blocks(blocks) => {
                    let blocks_json: Vec<serde_json::Value> = blocks
                        .iter()
                        .map(|b| serde_json::to_value(b).unwrap_or_default())
                        .collect();
                    json!({"role": m.role, "content": blocks_json})
                }
            })
            .collect()
    }

    /// Build API messages from ChatMessage.
    fn build_simple_messages(messages: &[ChatMessage]) -> Vec<serde_json::Value> {
        messages
            .iter()
            .map(|m| json!({"role": m.role, "content": m.content}))
            .collect()
    }
}

#[async_trait]
impl LlmClient for AnthropicClient {
    async fn complete(&self, messages: Vec<ChatMessage>, system: &str) -> Result<String> {
        self.complete_with_options(messages, system, CompletionOptions::default())
            .await
    }

    async fn complete_with_options(
        &self,
        messages: Vec<ChatMessage>,
        system: &str,
        options: CompletionOptions,
    ) -> Result<String> {
        let model = options.model.as_deref().unwrap_or(&self.model);
        let max_tokens = options.max_tokens.unwrap_or(self.max_tokens);
        let api_messages = Self::build_simple_messages(&messages);

        let body = json!({
            "model": model,
            "max_tokens": max_tokens,
            "system": system,
            "messages": api_messages,
        });

        debug!(model, messages = messages.len(), "calling Anthropic API");

        let resp = self.send_with_retry(&body).await?;
        let text = resp["content"]
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|block| block["text"].as_str())
            .unwrap_or("")
            .to_string();

        debug!(response_len = text.len(), "Anthropic API response received");
        Ok(text)
    }

    async fn complete_with_tools(
        &self,
        messages: Vec<ChatMessageExt>,
        system: &str,
        tools: &[ToolDefinition],
        options: CompletionOptions,
    ) -> Result<CompletionResponse> {
        let model = options.model.as_deref().unwrap_or(&self.model);
        let max_tokens = options.max_tokens.unwrap_or(self.max_tokens);
        let api_messages = Self::build_ext_messages(&messages);

        let api_tools: Vec<serde_json::Value> = tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.input_schema,
                })
            })
            .collect();

        let mut body = json!({
            "model": model,
            "max_tokens": max_tokens,
            "system": system,
            "messages": api_messages,
        });
        if !api_tools.is_empty() {
            body["tools"] = json!(api_tools);
        }

        // Structured output support (not all providers support this)
        // Anthropic doesn't have native response_format, but we can add it to system prompt
        if let Some(ref format) = options.response_format {
            match format {
                crate::client::ResponseFormat::Json => {
                    let system_with_json =
                        format!("{system}\n\nIMPORTANT: Respond ONLY with valid JSON.");
                    body["system"] = serde_json::Value::String(system_with_json);
                }
                crate::client::ResponseFormat::JsonSchema { name, schema } => {
                    let system_with_schema = format!(
                        "{system}\n\nIMPORTANT: Respond ONLY with valid JSON matching this schema named '{name}':\n```json\n{}\n```",
                        serde_json::to_string_pretty(schema).unwrap_or_default()
                    );
                    body["system"] = serde_json::Value::String(system_with_schema);
                }
            }
        }

        debug!(
            model,
            messages = messages.len(),
            tools = tools.len(),
            "calling Anthropic API with tools"
        );

        let resp = self.send_with_retry(&body).await?;
        let blocks = Self::parse_content_blocks(&resp);
        let usage = Self::parse_usage(&resp);
        let stop_reason = Self::parse_stop_reason(&resp);

        debug!(
            blocks = blocks.len(),
            input_tokens = usage.input_tokens,
            output_tokens = usage.output_tokens,
            ?stop_reason,
            "Anthropic API response with tools received"
        );

        Ok(CompletionResponse {
            blocks,
            usage,
            stop_reason,
        })
    }

    async fn complete_stream(&self, messages: Vec<ChatMessage>, system: &str) -> Result<LlmStream> {
        let api_messages = Self::build_simple_messages(&messages);

        let body = json!({
            "model": &self.model,
            "max_tokens": self.max_tokens,
            "system": system,
            "messages": api_messages,
            "stream": true,
        });

        debug!(model = %self.model, messages = messages.len(), "calling Anthropic API (streaming)");

        // Retry the initial connection (not mid-stream)
        let response = self.send_request_with_retry(&body).await?;

        let byte_stream = response.bytes_stream();

        // Buffer incomplete SSE lines across chunk boundaries to avoid
        // corrupting JSON payloads that span multiple TCP segments.
        let stream = byte_stream
            .scan(String::new(), |buffer, chunk_result| {
                let result: Result<String> = match chunk_result {
                    Err(e) => Err(Error::Other(format!("stream read error: {e}"))),
                    Ok(bytes) => {
                        buffer.push_str(&String::from_utf8_lossy(&bytes));
                        let mut text = String::new();
                        while let Some(pos) = buffer.find('\n') {
                            let line = buffer[..pos].trim_end_matches('\r').to_string();
                            buffer.drain(..=pos);
                            if let Some(data) = line.strip_prefix("data: ") {
                                if data == "[DONE]" {
                                    continue;
                                }
                                if let Ok(event) = serde_json::from_str::<serde_json::Value>(data)
                                    && event["type"] == "content_block_delta"
                                    && let Some(t) = event["delta"]["text"].as_str()
                                {
                                    text.push_str(t);
                                }
                            }
                        }
                        Ok(text)
                    }
                };
                futures_util::future::ready(Some(result))
            })
            .filter(|r: &Result<String>| {
                let keep = match r {
                    Ok(s) => !s.is_empty(),
                    Err(_) => true,
                };
                async move { keep }
            });

        Ok(Box::pin(stream))
    }

    async fn complete_stream_with_tools(
        &self,
        messages: Vec<ChatMessageExt>,
        system: &str,
        tools: &[ToolDefinition],
        options: CompletionOptions,
    ) -> Result<LlmToolStream> {
        let model = options.model.as_deref().unwrap_or(&self.model);
        let max_tokens = options.max_tokens.unwrap_or(self.max_tokens);
        let api_messages = Self::build_ext_messages(&messages);

        let api_tools: Vec<serde_json::Value> = tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.input_schema,
                })
            })
            .collect();

        let mut body = json!({
            "model": model,
            "max_tokens": max_tokens,
            "system": system,
            "messages": api_messages,
            "stream": true,
        });
        if !api_tools.is_empty() {
            body["tools"] = json!(api_tools);
        }

        debug!(
            model,
            messages = messages.len(),
            tools = tools.len(),
            "calling Anthropic API with tools (streaming)"
        );

        let response = self.send_request_with_retry(&body).await?;
        let byte_stream = response.bytes_stream();

        struct SseState {
            buffer: String,
            active_tool_id: Option<String>,
            active_tool_name: Option<String>,
            tool_input_buffer: String,
        }

        let state = SseState {
            buffer: String::new(),
            active_tool_id: None,
            active_tool_name: None,
            tool_input_buffer: String::new(),
        };

        let stream = byte_stream
            .scan(state, |state, chunk_result| {
                let events: Result<Vec<StreamEvent>> = match chunk_result {
                    Err(e) => Err(Error::Other(format!("stream read error: {e}"))),
                    Ok(bytes) => {
                        state.buffer.push_str(&String::from_utf8_lossy(&bytes));
                        let mut events = Vec::new();
                        while let Some(pos) = state.buffer.find('\n') {
                            let line = state.buffer[..pos].trim_end_matches('\r').to_string();
                            state.buffer.drain(..=pos);
                            let data = match line.strip_prefix("data: ") {
                                Some(d) if d != "[DONE]" => d,
                                _ => continue,
                            };
                            let event: serde_json::Value = match serde_json::from_str(data) {
                                Ok(v) => v,
                                Err(_) => continue,
                            };
                            let event_type = event["type"].as_str().unwrap_or("");
                            match event_type {
                                "message_start" => {
                                    // Extract initial usage if present
                                    let msg = &event["message"];
                                    if msg["usage"].is_object() {
                                        let usage = Usage {
                                            input_tokens: msg["usage"]["input_tokens"]
                                                .as_u64()
                                                .unwrap_or(0)
                                                as u32,
                                            output_tokens: msg["usage"]["output_tokens"]
                                                .as_u64()
                                                .unwrap_or(0)
                                                as u32,
                                            cache_read_input_tokens:
                                                msg["usage"]["cache_read_input_tokens"]
                                                    .as_u64()
                                                    .unwrap_or(0)
                                                    as u32,
                                            cache_creation_input_tokens:
                                                msg["usage"]["cache_creation_input_tokens"]
                                                    .as_u64()
                                                    .unwrap_or(0)
                                                    as u32,
                                        };
                                        events.push(StreamEvent::Usage(usage));
                                    }
                                }
                                "content_block_start" => {
                                    let block = &event["content_block"];
                                    match block["type"].as_str() {
                                        Some("tool_use") => {
                                            let id = block["id"].as_str().unwrap_or("").to_string();
                                            let name =
                                                block["name"].as_str().unwrap_or("").to_string();
                                            events.push(StreamEvent::ToolUseStart {
                                                id: id.clone(),
                                                name: name.clone(),
                                            });
                                            state.active_tool_id = Some(id);
                                            state.active_tool_name = Some(name);
                                            state.tool_input_buffer.clear();
                                        }
                                        _ => {
                                            // text block start — deltas come next
                                        }
                                    }
                                }
                                "content_block_delta" => {
                                    let delta = &event["delta"];
                                    match delta["type"].as_str() {
                                        Some("text_delta") => {
                                            if let Some(text) = delta["text"].as_str() {
                                                events
                                                    .push(StreamEvent::TextDelta(text.to_string()));
                                            }
                                        }
                                        Some("input_json_delta") => {
                                            if let Some(json_str) = delta["partial_json"].as_str() {
                                                state.tool_input_buffer.push_str(json_str);
                                                events.push(StreamEvent::ToolUseInputDelta(
                                                    json_str.to_string(),
                                                ));
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                                "content_block_stop" => {
                                    // If we were accumulating tool input, emit ToolUseEnd
                                    if let Some(id) = state.active_tool_id.take() {
                                        let input: serde_json::Value =
                                            serde_json::from_str(&state.tool_input_buffer)
                                                .unwrap_or(serde_json::Value::Object(
                                                    Default::default(),
                                                ));
                                        events.push(StreamEvent::ToolUseEnd { id, input });
                                        state.active_tool_name = None;
                                        state.tool_input_buffer.clear();
                                    }
                                }
                                "message_delta" => {
                                    let delta = &event["delta"];
                                    if let Some(reason) = delta["stop_reason"].as_str() {
                                        let stop = match reason {
                                            "end_turn" => Some(StopReason::EndTurn),
                                            "max_tokens" => Some(StopReason::MaxTokens),
                                            "tool_use" => Some(StopReason::ToolUse),
                                            "stop_sequence" => Some(StopReason::StopSequence),
                                            _ => None,
                                        };
                                        if let Some(s) = stop {
                                            events.push(StreamEvent::Stop(s));
                                        }
                                    }
                                    if let Some(usage) = event["usage"].as_object() {
                                        events.push(StreamEvent::Usage(Usage {
                                            input_tokens: usage
                                                .get("input_tokens")
                                                .and_then(|v| v.as_u64())
                                                .unwrap_or(0)
                                                as u32,
                                            output_tokens: usage
                                                .get("output_tokens")
                                                .and_then(|v| v.as_u64())
                                                .unwrap_or(0)
                                                as u32,
                                            cache_read_input_tokens: 0,
                                            cache_creation_input_tokens: 0,
                                        }));
                                    }
                                }
                                "message_stop" => {
                                    // Stream complete
                                }
                                _ => {}
                            }
                        }
                        Ok(events)
                    }
                };
                futures_util::future::ready(Some(events))
            })
            .flat_map(|result| {
                let items: Vec<Result<StreamEvent>> = match result {
                    Ok(events) => events.into_iter().map(Ok).collect(),
                    Err(e) => vec![Err(e)],
                };
                futures_util::stream::iter(items)
            });

        Ok(Box::pin(stream))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::ContentBlockInput;

    #[test]
    fn parse_usage_from_response() {
        let resp = json!({
            "usage": {
                "input_tokens": 100,
                "output_tokens": 50,
                "cache_read_input_tokens": 20,
                "cache_creation_input_tokens": 10,
            }
        });
        let usage = AnthropicClient::parse_usage(&resp);
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 50);
        assert_eq!(usage.cache_read_input_tokens, 20);
        assert_eq!(usage.cache_creation_input_tokens, 10);
    }

    #[test]
    fn parse_usage_missing_fields() {
        let resp = json!({"usage": {"input_tokens": 42}});
        let usage = AnthropicClient::parse_usage(&resp);
        assert_eq!(usage.input_tokens, 42);
        assert_eq!(usage.output_tokens, 0);
        assert_eq!(usage.cache_read_input_tokens, 0);
    }

    #[test]
    fn parse_usage_no_usage_key() {
        let resp = json!({"content": []});
        let usage = AnthropicClient::parse_usage(&resp);
        assert_eq!(usage.input_tokens, 0);
        assert_eq!(usage.output_tokens, 0);
    }

    #[test]
    fn parse_stop_reason_variants() {
        assert_eq!(
            AnthropicClient::parse_stop_reason(&json!({"stop_reason": "end_turn"})),
            Some(StopReason::EndTurn)
        );
        assert_eq!(
            AnthropicClient::parse_stop_reason(&json!({"stop_reason": "max_tokens"})),
            Some(StopReason::MaxTokens)
        );
        assert_eq!(
            AnthropicClient::parse_stop_reason(&json!({"stop_reason": "tool_use"})),
            Some(StopReason::ToolUse)
        );
        assert_eq!(
            AnthropicClient::parse_stop_reason(&json!({"stop_reason": "stop_sequence"})),
            Some(StopReason::StopSequence)
        );
        assert_eq!(
            AnthropicClient::parse_stop_reason(&json!({"stop_reason": "unknown"})),
            None
        );
        assert_eq!(AnthropicClient::parse_stop_reason(&json!({})), None);
    }

    #[test]
    fn parse_content_blocks_text_only() {
        let resp = json!({
            "content": [
                {"type": "text", "text": "Hello world"}
            ]
        });
        let blocks = AnthropicClient::parse_content_blocks(&resp);
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            ContentBlock::Text(t) => assert_eq!(t, "Hello world"),
            _ => panic!("expected text block"),
        }
    }

    #[test]
    fn parse_content_blocks_tool_use() {
        let resp = json!({
            "content": [
                {"type": "text", "text": "Let me check."},
                {
                    "type": "tool_use",
                    "id": "tool_123",
                    "name": "search",
                    "input": {"query": "test"}
                }
            ]
        });
        let blocks = AnthropicClient::parse_content_blocks(&resp);
        assert_eq!(blocks.len(), 2);
        match &blocks[1] {
            ContentBlock::ToolUse(call) => {
                assert_eq!(call.id, "tool_123");
                assert_eq!(call.name, "search");
                assert_eq!(call.input["query"], "test");
            }
            _ => panic!("expected tool_use block"),
        }
    }

    #[test]
    fn parse_content_blocks_empty() {
        let resp = json!({"content": []});
        assert!(AnthropicClient::parse_content_blocks(&resp).is_empty());
    }

    #[test]
    fn parse_content_blocks_no_content() {
        let resp = json!({});
        assert!(AnthropicClient::parse_content_blocks(&resp).is_empty());
    }

    #[test]
    fn parse_content_blocks_skips_unknown_types() {
        let resp = json!({
            "content": [
                {"type": "text", "text": "hello"},
                {"type": "thinking", "text": "..."},
            ]
        });
        let blocks = AnthropicClient::parse_content_blocks(&resp);
        assert_eq!(blocks.len(), 1);
    }

    #[test]
    fn build_simple_messages_format() {
        let messages = vec![
            ChatMessage {
                role: "user".into(),
                content: "hello".into(),
            },
            ChatMessage {
                role: "assistant".into(),
                content: "hi".into(),
            },
        ];
        let api_msgs = AnthropicClient::build_simple_messages(&messages);
        assert_eq!(api_msgs.len(), 2);
        assert_eq!(api_msgs[0]["role"], "user");
        assert_eq!(api_msgs[0]["content"], "hello");
        assert_eq!(api_msgs[1]["role"], "assistant");
        assert_eq!(api_msgs[1]["content"], "hi");
    }

    #[test]
    fn build_ext_messages_text() {
        let messages = vec![ChatMessageExt {
            role: "user".into(),
            content: ChatContent::Text("hello".into()),
        }];
        let api_msgs = AnthropicClient::build_ext_messages(&messages);
        assert_eq!(api_msgs[0]["role"], "user");
        assert_eq!(api_msgs[0]["content"], "hello");
    }

    #[test]
    fn build_ext_messages_tool_result() {
        let messages = vec![ChatMessageExt {
            role: "user".into(),
            content: ChatContent::Blocks(vec![ContentBlockInput::ToolResult {
                tool_use_id: "t1".into(),
                content: "result".into(),
                is_error: false,
            }]),
        }];
        let api_msgs = AnthropicClient::build_ext_messages(&messages);
        assert_eq!(api_msgs[0]["role"], "user");
        let content = api_msgs[0]["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "tool_result");
        assert_eq!(content[0]["tool_use_id"], "t1");
        assert_eq!(content[0]["content"], "result");
    }
}
