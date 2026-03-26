use futures_util::StreamExt;
use orka_core::{Error, Result};
use reqwest::Client;
use tokio::sync::Mutex;

use crate::oauth::OAuthClient;

/// Streamable HTTP transport for MCP (spec 2025-03-26).
///
/// Sends JSON-RPC 2.0 requests via HTTP POST and handles both
/// `application/json` and `text/event-stream` responses.
pub(crate) struct HttpTransport {
    http: Client,
    url: String,
    session_id: Mutex<Option<String>>,
    auth: Option<OAuthClient>,
}

impl HttpTransport {
    pub(crate) fn new(http: Client, url: String, auth: Option<OAuthClient>) -> Self {
        Self {
            http,
            url,
            session_id: Mutex::new(None),
            auth,
        }
    }

    /// Send a JSON-RPC request and return the `result` value.
    pub(crate) async fn send(&self, request: serde_json::Value) -> Result<serde_json::Value> {
        let body = serde_json::to_string(&request)
            .map_err(|e| Error::Other(format!("failed to serialize MCP request: {e}")))?;

        let mut builder = self
            .http
            .post(&self.url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .body(body);

        if let Some(ref oauth) = self.auth {
            let token = oauth.get_token().await?;
            builder = builder.bearer_auth(token);
        }

        {
            let guard = self.session_id.lock().await;
            if let Some(ref id) = *guard {
                builder = builder.header("Mcp-Session-Id", id.as_str());
            }
        }

        let resp = builder
            .send()
            .await
            .map_err(|e| Error::Other(format!("MCP HTTP request failed: {e}")))?;

        // Persist session ID for subsequent requests.
        if let Some(session) = resp.headers().get("Mcp-Session-Id")
            && let Ok(id) = session.to_str()
        {
            let mut guard = self.session_id.lock().await;
            *guard = Some(id.to_string());
        }

        let content_type = resp
            .headers()
            .get("Content-Type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        if content_type.contains("text/event-stream") {
            self.parse_sse(resp).await
        } else {
            let val: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| Error::Other(format!("failed to parse MCP HTTP response: {e}")))?;
            extract_result(&val)
        }
    }

    async fn parse_sse(&self, resp: reqwest::Response) -> Result<serde_json::Value> {
        let mut stream = resp.bytes_stream();
        let mut buf = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| Error::Other(format!("MCP SSE read error: {e}")))?;
            buf.push_str(&String::from_utf8_lossy(&chunk));

            // Events are delimited by double newlines.
            while let Some(end) = buf.find("\n\n") {
                let event = buf[..end].to_string();
                buf.drain(..end + 2);

                let mut event_type = "";
                let mut data = "";

                for line in event.lines() {
                    if let Some(v) = line.strip_prefix("event: ") {
                        event_type = v;
                    } else if let Some(v) = line.strip_prefix("data: ") {
                        data = v;
                    }
                }

                // Accept both typed "message" events and untyped data lines.
                if (event_type == "message" || event_type.is_empty())
                    && !data.is_empty()
                    && let Ok(val) = serde_json::from_str::<serde_json::Value>(data)
                {
                    // Only respond to the JSON-RPC reply (has an "id").
                    if val.get("id").is_some() {
                        return extract_result(&val);
                    }
                }
            }
        }

        Err(Error::Other(
            "MCP SSE stream ended without a JSON-RPC response".into(),
        ))
    }
}

fn extract_result(val: &serde_json::Value) -> Result<serde_json::Value> {
    if let Some(error) = val.get("error") {
        let msg = error["message"].as_str().unwrap_or("unknown error");
        let code = error["code"].as_i64().unwrap_or(-1);
        return Err(Error::Other(format!("MCP error {code}: {msg}")));
    }
    Ok(val
        .get("result")
        .cloned()
        .unwrap_or(serde_json::Value::Null))
}
