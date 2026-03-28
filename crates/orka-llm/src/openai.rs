use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use orka_core::{Error, Result, retry::retry_with_backoff};
use reqwest::Client;
use serde_json::json;
use tracing::debug;

use crate::client::{
    ChatContent, ChatMessage, CompletionOptions, CompletionResponse, ContentBlock, LlmClient,
    LlmStream, LlmToolStream, RetryableError, StopReason, StreamEvent, ToolCall, ToolDefinition,
    Usage,
};

struct OpenAiToolAccum {
    id: String,
    _name: String,
    arguments: String,
}

struct OpenAiSseState {
    buffer: String,
    tool_calls: std::collections::HashMap<u64, OpenAiToolAccum>,
}

/// `OpenAI` Chat Completions API client with retry and streaming support.
pub struct OpenAiClient {
    client: Client,
    api_key: String,
    model: String,
    max_tokens: u32,
    max_retries: u32,
    base_url: String,
}

impl OpenAiClient {
    /// Create a client with default settings (30s timeout, 4096 max tokens, 2
    /// retries).
    pub fn new(api_key: String, model: String) -> Self {
        Self::with_options(
            api_key,
            model,
            30,
            4096,
            2,
            "https://api.openai.com/v1".into(),
        )
    }

    /// Create a client with full configuration.
    pub fn with_options(
        api_key: String,
        model: String,
        timeout_secs: u64,
        max_tokens: u32,
        max_retries: u32,
        base_url: String,
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
            base_url,
        }
    }

    /// Returns true if the model requires `max_completion_tokens` instead of
    /// `max_tokens` (o-series, gpt-5+, chatgpt-4o-latest).
    fn uses_max_completion_tokens(model: &str) -> bool {
        let m = model.to_lowercase();
        m.starts_with("o1")
            || m.starts_with("o3")
            || m.starts_with("o4")
            || m.starts_with("gpt-5")
            || m.starts_with("chatgpt-4o")
    }

    /// Returns true if the model supports `reasoning_effort`.
    fn supports_reasoning(model: &str) -> bool {
        let m = model.to_lowercase();
        m.starts_with("o1")
            || m.starts_with("o3")
            || m.starts_with("o4")
    }

    /// Insert the appropriate max tokens key into the request body.
    fn insert_max_tokens(body: &mut serde_json::Value, model: &str, max_tokens: u32) {
        if Self::uses_max_completion_tokens(model) {
            body["max_completion_tokens"] = json!(max_tokens);
        } else {
            body["max_tokens"] = json!(max_tokens);
        }
    }

    /// Conditionally insert reasoning_effort if the model supports it.
    fn maybe_insert_reasoning(
        body: &mut serde_json::Value,
        model: &str,
        thinking: &Option<crate::client::ThinkingConfig>,
    ) {
        if !Self::supports_reasoning(model) {
            return;
        }
        match thinking {
            Some(crate::client::ThinkingConfig::ReasoningEffort(effort)) => {
                body["reasoning_effort"] = json!(effort.as_str());
            }
            Some(crate::client::ThinkingConfig::Adaptive { effort }) => {
                let openai_effort = match effort {
                    crate::client::ThinkingEffort::Low => "low",
                    crate::client::ThinkingEffort::Medium => "medium",
                    crate::client::ThinkingEffort::High | crate::client::ThinkingEffort::Max => {
                        "high"
                    }
                };
                body["reasoning_effort"] = json!(openai_effort);
            }
            _ => {}
        }
    }

    /// Send a request with retry logic for 429/5xx and transient errors.
    /// Returns the raw successful HTTP response.
    async fn send_request_with_retry(&self, body: &serde_json::Value) -> Result<reqwest::Response> {
        let url = format!("{}/chat/completions", self.base_url);
        retry_with_backoff(
            self.max_retries,
            500,
            30_000,
            || async {
                let result = self
                    .client
                    .post(&url)
                    .header("Authorization", format!("Bearer {}", self.api_key))
                    .header("Content-Type", "application/json")
                    .json(body)
                    .send()
                    .await;

                match result {
                    Ok(response) => {
                        let status = response.status();
                        if status == 429 || status.is_server_error() {
                            let text = response.text().await.unwrap_or_default();
                            Err(RetryableError::Transient(format!(
                                "OpenAI API error {status}: {text}"
                            )))
                        } else if !status.is_success() {
                            let text = response.text().await.unwrap_or_default();
                            Err(RetryableError::Fatal(format!(
                                "OpenAI API error {status}: {text}"
                            )))
                        } else {
                            Ok(response)
                        }
                    }
                    Err(e) if e.is_timeout() || e.is_connect() => Err(RetryableError::Transient(
                        format!("OpenAI API request failed: {e}"),
                    )),
                    Err(e) => Err(RetryableError::Fatal(format!(
                        "OpenAI API request failed: {e}"
                    ))),
                }
            },
            |e| matches!(e, RetryableError::Transient(_)),
        )
        .await
        .map_err(|e| match e {
            RetryableError::Transient(msg) | RetryableError::Fatal(msg) => Error::Other(msg),
        })
    }

    /// Send a request with retry and parse the JSON response.
    async fn send_with_retry(&self, body: &serde_json::Value) -> Result<serde_json::Value> {
        let response = self.send_request_with_retry(body).await?;
        response
            .json()
            .await
            .map_err(|e| Error::Other(format!("failed to parse OpenAI response: {e}")))
    }

    fn parse_response(resp: &serde_json::Value) -> (Vec<ContentBlock>, Usage, Option<StopReason>) {
        let Some(choice) = resp["choices"].as_array().and_then(|a| a.first()) else {
            return (Vec::new(), Usage::default(), None);
        };
        let message = &choice["message"];

        let mut blocks = Vec::new();

        // Text content
        if let Some(content) = message["content"].as_str()
            && !content.is_empty()
        {
            blocks.push(ContentBlock::Text(content.to_string()));
        }

        // Tool calls
        if let Some(tool_calls) = message["tool_calls"].as_array() {
            for tc in tool_calls {
                let id = tc["id"].as_str().unwrap_or("").to_string();
                let name = tc["function"]["name"].as_str().unwrap_or("").to_string();
                if name.is_empty() {
                    tracing::warn!(id, "ignoring tool call with empty function name");
                    continue;
                }
                let input: serde_json::Value = tc["function"]["arguments"]
                    .as_str()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or(serde_json::Value::Object(serde_json::Map::default()));
                blocks.push(ContentBlock::ToolUse(ToolCall { id, name, input }));
            }
        }

        let reasoning_tokens = resp["usage"]["completion_tokens_details"]["reasoning_tokens"]
            .as_u64()
            .unwrap_or(0) as u32;
        let usage = Usage {
            input_tokens: resp["usage"]["prompt_tokens"].as_u64().unwrap_or(0) as u32,
            output_tokens: resp["usage"]["completion_tokens"].as_u64().unwrap_or(0) as u32,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
            reasoning_tokens,
        };

        let stop_reason = match choice["finish_reason"].as_str() {
            Some("stop") => Some(StopReason::EndTurn),
            Some("length") => Some(StopReason::MaxTokens),
            Some("tool_calls") => Some(StopReason::ToolUse),
            _ => None,
        };

        (blocks, usage, stop_reason)
    }

    fn build_messages(messages: &[ChatMessage]) -> Vec<serde_json::Value> {
        let mut out = Vec::new();
        for m in messages {
            match &m.content {
                ChatContent::Text(t) => {
                    out.push(json!({"role": m.role, "content": t}));
                }
                ChatContent::Blocks(blocks) => {
                    // User messages with image blocks → OpenAI vision content array.
                    let has_image = blocks
                        .iter()
                        .any(|b| matches!(b, crate::client::ContentBlockInput::Image { .. }));
                    if m.role == crate::client::Role::User && has_image {
                        let mut content_parts: Vec<serde_json::Value> = Vec::new();
                        for block in blocks {
                            match block {
                                crate::client::ContentBlockInput::Text { text } => {
                                    content_parts.push(json!({"type": "text", "text": text}));
                                }
                                crate::client::ContentBlockInput::Image { source } => {
                                    let url = match source {
                                        crate::client::ImageSource::Url { url } => url.clone(),
                                        crate::client::ImageSource::Base64 { media_type, data } => {
                                            format!("data:{media_type};base64,{data}")
                                        }
                                    };
                                    content_parts.push(json!({
                                        "type": "image_url",
                                        "image_url": {"url": url}
                                    }));
                                }
                                _ => {}
                            }
                        }
                        out.push(json!({"role": "user", "content": content_parts}));
                        continue;
                    }
                    // Collect text and tool_use blocks into a single assistant message,
                    // and tool_result blocks into separate tool messages.
                    let mut text_parts = Vec::new();
                    let mut tool_calls = Vec::new();
                    for block in blocks {
                        match block {
                            crate::client::ContentBlockInput::Text { text } => {
                                text_parts.push(text.clone());
                            }
                            crate::client::ContentBlockInput::ToolUse { id, name, input } => {
                                // Skip tool calls with empty names — OpenAI rejects them
                                if name.is_empty() {
                                    tracing::warn!(id, "skipping tool call with empty name");
                                    continue;
                                }
                                tool_calls.push(json!({
                                    "id": id,
                                    "type": "function",
                                    "function": {
                                        "name": name,
                                        "arguments": serde_json::to_string(input).unwrap_or_default(),
                                    }
                                }));
                            }
                            crate::client::ContentBlockInput::ToolResult {
                                tool_use_id,
                                content,
                                ..
                            } => {
                                out.push(json!({
                                    "role": "tool",
                                    "tool_call_id": tool_use_id,
                                    "content": content,
                                }));
                            }
                            // Thinking and Image blocks are handled above; skip for this path.
                            crate::client::ContentBlockInput::Thinking { .. }
                            | crate::client::ContentBlockInput::Image { .. }
                            | crate::client::ContentBlockInput::Unknown => {}
                        }
                    }
                    // Emit assistant message with text and/or tool_calls
                    if !text_parts.is_empty() || !tool_calls.is_empty() {
                        let mut msg = json!({"role": "assistant"});
                        if !text_parts.is_empty() {
                            msg["content"] = json!(text_parts.join("\n"));
                        }
                        if !tool_calls.is_empty() {
                            msg["tool_calls"] = json!(tool_calls);
                        }
                        out.push(msg);
                    }
                }
            }
        }
        out
    }

    fn build_tools(tools: &[ToolDefinition]) -> Vec<serde_json::Value> {
        tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.input_schema,
                    }
                })
            })
            .collect()
    }
}

#[async_trait]
impl LlmClient for OpenAiClient {
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

        let mut api_messages = vec![json!({"role": "system", "content": system})];
        for m in &messages {
            let text = match &m.content {
                ChatContent::Text(t) => t.clone(),
                _ => String::new(),
            };
            api_messages.push(json!({"role": m.role, "content": text}));
        }

        let mut body = json!({
            "model": model,
            "messages": api_messages,
        });
        Self::insert_max_tokens(&mut body, model, max_tokens);

        if let Some(temp) = options.temperature {
            body["temperature"] = json!(temp);
        }
        Self::maybe_insert_reasoning(&mut body, model, &options.thinking);

        debug!(model, messages = messages.len(), "calling OpenAI API");
        let resp = self.send_with_retry(&body).await?;

        let text = resp["choices"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(|c| c["message"]["content"].as_str())
            .unwrap_or("")
            .to_string();

        Ok(text)
    }

    async fn complete_stream(&self, messages: Vec<ChatMessage>, system: &str) -> Result<LlmStream> {
        let mut api_messages = vec![json!({"role": "system", "content": system})];
        for m in &messages {
            let text = match &m.content {
                ChatContent::Text(t) => t.clone(),
                _ => String::new(),
            };
            api_messages.push(json!({"role": m.role, "content": text}));
        }

        let mut body = json!({
            "model": &self.model,
            "messages": api_messages,
            "stream": true,
        });
        Self::insert_max_tokens(&mut body, &self.model, self.max_tokens);

        debug!(model = %self.model, messages = messages.len(), "calling OpenAI API (streaming)");

        let response = self.send_request_with_retry(&body).await?;
        let byte_stream = response.bytes_stream();

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
                                    && let Some(t) =
                                        event["choices"][0]["delta"]["content"].as_str()
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

    #[allow(clippy::too_many_lines)]
    async fn complete_with_tools(
        &self,
        messages: &[ChatMessage],
        system: &str,
        tools: &[ToolDefinition],
        options: CompletionOptions,
    ) -> Result<CompletionResponse> {
        let model = options.model.as_deref().unwrap_or(&self.model);
        let max_tokens = options.max_tokens.unwrap_or(self.max_tokens);

        let mut api_messages = vec![json!({"role": "system", "content": system})];
        api_messages.extend(Self::build_messages(messages));

        let mut body = json!({
            "model": model,
            "messages": api_messages,
        });
        Self::insert_max_tokens(&mut body, model, max_tokens);

        if !tools.is_empty() {
            body["tools"] = json!(Self::build_tools(tools));
        }

        if let Some(ref format) = options.response_format {
            match format {
                crate::client::ResponseFormat::Json => {
                    body["response_format"] = serde_json::json!({"type": "json_object"});
                }
                crate::client::ResponseFormat::JsonSchema { name, schema } => {
                    body["response_format"] = serde_json::json!({
                        "type": "json_schema",
                        "json_schema": {
                            "name": name,
                            "schema": schema,
                        }
                    });
                }
            }
        }

        if let Some(temp) = options.temperature {
            body["temperature"] = json!(temp);
        }
        Self::maybe_insert_reasoning(&mut body, model, &options.thinking);

        debug!(
            model,
            messages = messages.len(),
            tools = tools.len(),
            "calling OpenAI API with tools"
        );
        let resp = self.send_with_retry(&body).await?;
        let (blocks, usage, stop_reason) = Self::parse_response(&resp);

        Ok(CompletionResponse {
            blocks,
            usage,
            stop_reason,
        })
    }

    #[allow(clippy::too_many_lines)]
    async fn complete_stream_with_tools(
        &self,
        messages: &[ChatMessage],
        system: &str,
        tools: &[ToolDefinition],
        options: CompletionOptions,
    ) -> Result<LlmToolStream> {
        let model = options.model.as_deref().unwrap_or(&self.model);
        let max_tokens = options.max_tokens.unwrap_or(self.max_tokens);

        let mut api_messages = vec![json!({"role": "system", "content": system})];
        api_messages.extend(Self::build_messages(messages));

        let mut body = json!({
            "model": model,
            "messages": api_messages,
            "stream": true,
        });
        Self::insert_max_tokens(&mut body, model, max_tokens);

        if !tools.is_empty() {
            body["tools"] = json!(Self::build_tools(tools));
        }

        if let Some(temp) = options.temperature {
            body["temperature"] = json!(temp);
        }
        Self::maybe_insert_reasoning(&mut body, model, &options.thinking);

        debug!(
            model,
            messages = messages.len(),
            tools = tools.len(),
            "calling OpenAI API with tools (streaming)"
        );

        let response = self.send_request_with_retry(&body).await?;
        let byte_stream = response.bytes_stream();

        let state = OpenAiSseState {
            buffer: String::new(),
            tool_calls: std::collections::HashMap::new(),
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

                            let Some(choice) = event["choices"].as_array().and_then(|a| a.first())
                            else {
                                continue;
                            };
                            let delta = &choice["delta"];

                            // Text content delta
                            if let Some(content) = delta["content"].as_str()
                                && !content.is_empty()
                            {
                                events.push(StreamEvent::TextDelta(content.to_string()));
                            }

                            // Tool call deltas
                            if let Some(tool_calls) = delta["tool_calls"].as_array() {
                                for tc in tool_calls {
                                    let index = tc["index"].as_u64().unwrap_or(0);
                                    let func = &tc["function"];

                                    if let Some(name) = func["name"].as_str() {
                                        // First chunk for this tool call
                                        let id = tc["id"].as_str().unwrap_or("").to_string();
                                        events.push(StreamEvent::ToolUseStart {
                                            id: id.clone(),
                                            name: name.to_string(),
                                        });
                                        state.tool_calls.insert(
                                            index,
                                            OpenAiToolAccum {
                                                id,
                                                _name: name.to_string(),
                                                arguments: String::new(),
                                            },
                                        );
                                    }

                                    // Accumulate arguments
                                    if let Some(args) = func["arguments"].as_str()
                                        && let Some(accum) = state.tool_calls.get_mut(&index)
                                    {
                                        accum.arguments.push_str(args);
                                        events
                                            .push(StreamEvent::ToolUseInputDelta(args.to_string()));
                                    }
                                }
                            }

                            // Finish reason
                            if let Some(reason) = choice["finish_reason"].as_str() {
                                if reason == "tool_calls" || reason == "stop" {
                                    // Emit ToolUseEnd for all accumulated tool calls
                                    let mut indices: Vec<u64> =
                                        state.tool_calls.keys().copied().collect();
                                    indices.sort_unstable();
                                    for idx in indices {
                                        if let Some(accum) = state.tool_calls.remove(&idx) {
                                            let input: serde_json::Value = serde_json::from_str(
                                                &accum.arguments,
                                            )
                                            .unwrap_or(serde_json::Value::Object(
                                                serde_json::Map::default(),
                                            ));
                                            events.push(StreamEvent::ToolUseEnd {
                                                id: accum.id,
                                                input,
                                            });
                                        }
                                    }

                                    let stop = match reason {
                                        "tool_calls" => StopReason::ToolUse,
                                        "length" => StopReason::MaxTokens,
                                        _ => StopReason::EndTurn,
                                    };
                                    events.push(StreamEvent::Stop(stop));
                                } else if reason == "length" {
                                    events.push(StreamEvent::Stop(StopReason::MaxTokens));
                                }
                            }

                            // Usage (some OpenAI responses include it in the final chunk)
                            if let Some(usage) = event["usage"].as_object() {
                                let reasoning_tokens =
                                    event["usage"]["completion_tokens_details"]["reasoning_tokens"]
                                        .as_u64()
                                        .unwrap_or(0) as u32;
                                events.push(StreamEvent::Usage(Usage {
                                    input_tokens: usage
                                        .get("prompt_tokens")
                                        .and_then(serde_json::Value::as_u64)
                                        .unwrap_or(0)
                                        as u32,
                                    output_tokens: usage
                                        .get("completion_tokens")
                                        .and_then(serde_json::Value::as_u64)
                                        .unwrap_or(0)
                                        as u32,
                                    cache_read_input_tokens: 0,
                                    cache_creation_input_tokens: 0,
                                    reasoning_tokens,
                                }));
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
    #![allow(clippy::expect_used)]

    use super::*;

    #[test]
    fn parse_response_text_only() {
        let resp = json!({
            "choices": [{
                "message": {"content": "Hello world"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5}
        });
        let (blocks, usage, stop_reason) = OpenAiClient::parse_response(&resp);
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            ContentBlock::Text(t) => assert_eq!(t, "Hello world"),
            _ => panic!("expected text block"),
        }
        assert_eq!(usage.input_tokens, 10);
        assert_eq!(usage.output_tokens, 5);
        assert_eq!(stop_reason, Some(StopReason::EndTurn));
    }

    #[test]
    fn parse_response_tool_calls() {
        let resp = json!({
            "choices": [{
                "message": {
                    "content": null,
                    "tool_calls": [{
                        "id": "call_123",
                        "type": "function",
                        "function": {
                            "name": "search",
                            "arguments": "{\"query\": \"test\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 20, "completion_tokens": 10}
        });
        let (blocks, _usage, stop_reason) = OpenAiClient::parse_response(&resp);
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            ContentBlock::ToolUse(call) => {
                assert_eq!(call.id, "call_123");
                assert_eq!(call.name, "search");
                assert_eq!(call.input["query"], "test");
            }
            _ => panic!("expected tool_use block"),
        }
        assert_eq!(stop_reason, Some(StopReason::ToolUse));
    }

    #[test]
    fn parse_response_max_tokens() {
        let resp = json!({
            "choices": [{
                "message": {"content": "truncated"},
                "finish_reason": "length"
            }],
            "usage": {"prompt_tokens": 5, "completion_tokens": 100}
        });
        let (_blocks, _usage, stop_reason) = OpenAiClient::parse_response(&resp);
        assert_eq!(stop_reason, Some(StopReason::MaxTokens));
    }

    #[test]
    fn build_tools_format() {
        let tools = vec![ToolDefinition {
            name: "search".into(),
            description: "Search the web".into(),
            input_schema: json!({"type": "object", "properties": {"q": {"type": "string"}}}),
        }];
        let api_tools = OpenAiClient::build_tools(&tools);
        assert_eq!(api_tools.len(), 1);
        assert_eq!(api_tools[0]["type"], "function");
        assert_eq!(api_tools[0]["function"]["name"], "search");
    }

    #[test]
    fn build_messages_image_url_produces_vision_content_array() {
        use crate::client::{ChatContent, ChatMessage, ContentBlockInput, ImageSource, Role};

        let messages = vec![ChatMessage {
            role: Role::User,
            content: ChatContent::Blocks(vec![
                ContentBlockInput::Text {
                    text: "What is in this image?".into(),
                },
                ContentBlockInput::Image {
                    source: ImageSource::Url {
                        url: "https://example.com/photo.png".into(),
                    },
                },
            ]),
        }];

        let built = OpenAiClient::build_messages(&messages);
        assert_eq!(built.len(), 1);

        let msg = &built[0];
        assert_eq!(msg["role"], "user");

        let parts = msg["content"].as_array().expect("content should be array");
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0]["type"], "text");
        assert_eq!(parts[0]["text"], "What is in this image?");
        assert_eq!(parts[1]["type"], "image_url");
        assert_eq!(
            parts[1]["image_url"]["url"],
            "https://example.com/photo.png"
        );
    }

    #[test]
    fn build_messages_image_base64_produces_data_url() {
        use crate::client::{ChatContent, ChatMessage, ContentBlockInput, ImageSource, Role};

        let messages = vec![ChatMessage {
            role: Role::User,
            content: ChatContent::Blocks(vec![ContentBlockInput::Image {
                source: ImageSource::Base64 {
                    media_type: "image/jpeg".into(),
                    data: "abc123".into(),
                },
            }]),
        }];

        let built = OpenAiClient::build_messages(&messages);
        assert_eq!(built.len(), 1);

        let parts = built[0]["content"]
            .as_array()
            .expect("content should be array");
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0]["type"], "image_url");
        assert_eq!(
            parts[0]["image_url"]["url"],
            "data:image/jpeg;base64,abc123"
        );
    }

    #[test]
    fn uses_max_completion_tokens_for_new_models() {
        assert!(OpenAiClient::uses_max_completion_tokens("o1-mini"));
        assert!(OpenAiClient::uses_max_completion_tokens("o3-mini"));
        assert!(OpenAiClient::uses_max_completion_tokens("o4-mini"));
        assert!(OpenAiClient::uses_max_completion_tokens("gpt-5-mini"));
        assert!(OpenAiClient::uses_max_completion_tokens("gpt-5.4"));
        assert!(OpenAiClient::uses_max_completion_tokens("chatgpt-4o-latest"));
        // Legacy models use max_tokens
        assert!(!OpenAiClient::uses_max_completion_tokens("gpt-4.1-mini"));
        assert!(!OpenAiClient::uses_max_completion_tokens("gpt-4o"));
        assert!(!OpenAiClient::uses_max_completion_tokens("gpt-4-turbo"));
    }

    #[test]
    fn supports_reasoning_only_for_o_series() {
        assert!(OpenAiClient::supports_reasoning("o1-mini"));
        assert!(OpenAiClient::supports_reasoning("o3-mini"));
        assert!(OpenAiClient::supports_reasoning("o4-mini"));
        assert!(!OpenAiClient::supports_reasoning("gpt-4.1-mini"));
        assert!(!OpenAiClient::supports_reasoning("gpt-5-mini"));
    }

    #[test]
    fn insert_max_tokens_uses_correct_key() {
        let mut body = json!({"model": "gpt-5-mini"});
        OpenAiClient::insert_max_tokens(&mut body, "gpt-5-mini", 4096);
        assert!(body.get("max_completion_tokens").is_some());
        assert!(body.get("max_tokens").is_none());

        let mut body2 = json!({"model": "gpt-4.1-mini"});
        OpenAiClient::insert_max_tokens(&mut body2, "gpt-4.1-mini", 4096);
        assert!(body2.get("max_tokens").is_some());
        assert!(body2.get("max_completion_tokens").is_none());
    }

    #[test]
    fn parse_response_skips_empty_tool_names() {
        let resp = json!({
            "choices": [{
                "message": {
                    "content": null,
                    "tool_calls": [
                        {"id": "call_1", "type": "function", "function": {"name": "shell_exec", "arguments": "{\"cmd\":\"whoami\"}"}},
                        {"id": "call_2", "type": "function", "function": {"name": "", "arguments": "{}"}},
                    ]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 20}
        });
        let (blocks, _usage, _stop) = OpenAiClient::parse_response(&resp);
        // Only the valid tool call should be included
        let tool_count = blocks.iter().filter(|b| matches!(b, ContentBlock::ToolUse(_))).count();
        assert_eq!(tool_count, 1);
    }
}
