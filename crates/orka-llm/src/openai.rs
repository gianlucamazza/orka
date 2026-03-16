use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use serde_json::json;
use tracing::{debug, warn};

use crate::client::{
    ChatContent, ChatMessage, ChatMessageExt, CompletionOptions, CompletionResponse, ContentBlock,
    LlmClient, StopReason, ToolCall, ToolDefinition, Usage,
};
use orka_core::{Error, Result};

pub struct OpenAiClient {
    client: Client,
    api_key: String,
    model: String,
    max_tokens: u32,
    max_retries: u32,
    base_url: String,
}

impl OpenAiClient {
    pub fn new(api_key: String, model: String) -> Self {
        Self::with_options(api_key, model, 30, 4096, 2, "https://api.openai.com/v1".into())
    }

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

    async fn send_with_retry(&self, body: &serde_json::Value) -> Result<serde_json::Value> {
        let url = format!("{}/chat/completions", self.base_url);
        let mut last_err = None;
        for attempt in 0..=self.max_retries {
            if attempt > 0 {
                let delay = Duration::from_millis(500 * 2u64.pow(attempt - 1));
                warn!(attempt, delay_ms = delay.as_millis() as u64, "retrying OpenAI API call");
                tokio::time::sleep(delay).await;
            }

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
                        let body_text = response.text().await.unwrap_or_default();
                        last_err = Some(format!("OpenAI API error {status}: {body_text}"));
                        continue;
                    }
                    if !status.is_success() {
                        let body_text = response.text().await.unwrap_or_default();
                        return Err(Error::Other(format!(
                            "OpenAI API error {status}: {body_text}"
                        )));
                    }
                    return response.json().await.map_err(|e| {
                        Error::Other(format!("failed to parse OpenAI response: {e}"))
                    });
                }
                Err(e) => {
                    if e.is_timeout() || e.is_connect() {
                        last_err = Some(format!("OpenAI API request failed: {e}"));
                        continue;
                    }
                    return Err(Error::Other(format!("OpenAI API request failed: {e}")));
                }
            }
        }
        Err(Error::Other(
            last_err.unwrap_or_else(|| "OpenAI API request failed after retries".into()),
        ))
    }

    fn parse_response(resp: &serde_json::Value) -> (Vec<ContentBlock>, Usage, Option<StopReason>) {
        let choice = &resp["choices"][0];
        let message = &choice["message"];

        let mut blocks = Vec::new();

        // Text content
        if let Some(content) = message["content"].as_str() {
            if !content.is_empty() {
                blocks.push(ContentBlock::Text(content.to_string()));
            }
        }

        // Tool calls
        if let Some(tool_calls) = message["tool_calls"].as_array() {
            for tc in tool_calls {
                let id = tc["id"].as_str().unwrap_or("").to_string();
                let name = tc["function"]["name"].as_str().unwrap_or("").to_string();
                let input: serde_json::Value = tc["function"]["arguments"]
                    .as_str()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or(serde_json::Value::Object(Default::default()));
                blocks.push(ContentBlock::ToolUse(ToolCall { id, name, input }));
            }
        }

        let usage = Usage {
            input_tokens: resp["usage"]["prompt_tokens"].as_u64().unwrap_or(0) as u32,
            output_tokens: resp["usage"]["completion_tokens"].as_u64().unwrap_or(0) as u32,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        };

        let stop_reason = match choice["finish_reason"].as_str() {
            Some("stop") => Some(StopReason::EndTurn),
            Some("length") => Some(StopReason::MaxTokens),
            Some("tool_calls") => Some(StopReason::ToolUse),
            _ => None,
        };

        (blocks, usage, stop_reason)
    }

    fn build_messages(messages: &[ChatMessageExt]) -> Vec<serde_json::Value> {
        messages
            .iter()
            .map(|m| match &m.content {
                ChatContent::Text(t) => json!({"role": m.role, "content": t}),
                ChatContent::Blocks(blocks) => {
                    // Convert tool results to OpenAI format
                    let mut msgs = Vec::new();
                    for block in blocks {
                        match block {
                            crate::client::ContentBlockInput::ToolResult {
                                tool_use_id,
                                content,
                                ..
                            } => {
                                msgs.push(json!({
                                    "role": "tool",
                                    "tool_call_id": tool_use_id,
                                    "content": content,
                                }));
                            }
                        }
                    }
                    // Return first or empty
                    msgs.into_iter()
                        .next()
                        .unwrap_or(json!({"role": "user", "content": ""}))
                }
            })
            .collect()
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
            api_messages.push(json!({"role": m.role, "content": m.content}));
        }

        let body = json!({
            "model": model,
            "max_tokens": max_tokens,
            "messages": api_messages,
        });

        debug!(model, messages = messages.len(), "calling OpenAI API");
        let resp = self.send_with_retry(&body).await?;

        let text = resp["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();

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

        let mut api_messages = vec![json!({"role": "system", "content": system})];
        api_messages.extend(Self::build_messages(&messages));

        let mut body = json!({
            "model": model,
            "max_tokens": max_tokens,
            "messages": api_messages,
        });

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

        debug!(model, messages = messages.len(), tools = tools.len(), "calling OpenAI API with tools");
        let resp = self.send_with_retry(&body).await?;
        let (blocks, usage, stop_reason) = Self::parse_response(&resp);

        Ok(CompletionResponse {
            blocks,
            usage,
            stop_reason,
        })
    }
}

#[cfg(test)]
mod tests {
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
}
