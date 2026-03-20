use std::fmt::Write as _;
use std::time::Duration;

use serde_json::json;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Percent-encode a string for use as a URL query parameter value.
/// Passes through unreserved characters (A-Z a-z 0-9 - _ . ~) unchanged.
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
    out
}

pub struct OrkaClient {
    http: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
}

impl OrkaClient {
    pub fn new(base_url: &str, api_key: Option<&str>) -> Self {
        Self {
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(30))
                .build()
                .expect("failed to build HTTP client"),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.map(String::from),
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Build a request with the correct base URL and API key header attached.
    fn request_builder(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}{path}", self.base_url);
        let mut req = self.http.request(method, &url);
        if let Some(key) = &self.api_key {
            req = req.header("X-Api-Key", key);
        }
        req
    }

    pub async fn send_message(
        &self,
        text: &str,
        session_id: &str,
        metadata: Option<std::collections::HashMap<String, serde_json::Value>>,
    ) -> Result<serde_json::Value> {
        let mut payload = json!({
            "text": text,
            "session_id": session_id,
        });
        if let Some(meta) = metadata
            && !meta.is_empty()
        {
            payload["metadata"] = json!(meta);
        }
        let resp = self
            .request_builder(reqwest::Method::POST, "/api/v1/message")
            .json(&payload)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Server returned {status}: {body}").into());
        }
        let body = resp.json::<serde_json::Value>().await?;
        Ok(body)
    }

    pub async fn get_json(&self, path: &str) -> Result<serde_json::Value> {
        let resp = self.get(path).await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Server returned {status}: {body}").into());
        }
        let body = resp.json::<serde_json::Value>().await?;
        Ok(body)
    }

    pub async fn get(&self, path: &str) -> std::result::Result<reqwest::Response, reqwest::Error> {
        self.request_builder(reqwest::Method::GET, path)
            .send()
            .await
    }

    pub async fn post(
        &self,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> std::result::Result<reqwest::Response, reqwest::Error> {
        let mut req = self.request_builder(reqwest::Method::POST, path);
        if let Some(b) = body {
            req = req.json(&b);
        }
        req.send().await
    }

    pub async fn delete(
        &self,
        path: &str,
    ) -> std::result::Result<reqwest::Response, reqwest::Error> {
        self.request_builder(reqwest::Method::DELETE, path)
            .send()
            .await
    }

    /// Resolve an optional session ID, generating a new UUID v7 if absent.
    pub fn resolve_session_id(session_id: Option<&str>) -> String {
        session_id
            .map(String::from)
            .unwrap_or_else(|| uuid::Uuid::now_v7().to_string())
    }

    /// Poll the health endpoint until the server responds 200.
    pub async fn wait_for_ready(&self, retries: u32, interval: Duration) -> Result<()> {
        let per_request_timeout = Duration::from_secs(3);
        for attempt in 0..retries {
            let result =
                tokio::time::timeout(per_request_timeout, self.get("/api/v1/health")).await;
            match result {
                Ok(Ok(resp)) if resp.status().is_success() => return Ok(()),
                _ => {
                    if attempt == 0 {
                        eprintln!("Waiting for server at {} ...", self.base_url);
                    }
                    tokio::time::sleep(interval).await;
                }
            }
        }
        Err(format!(
            "Server at {} not ready after {} retries",
            self.base_url, retries
        )
        .into())
    }

    pub fn ws_url(&self, session_id: &str) -> String {
        let ws_base = if self.base_url.starts_with("https://") {
            self.base_url.replacen("https://", "wss://", 1)
        } else {
            self.base_url.replacen("http://", "ws://", 1)
        };
        let encoded = percent_encode(session_id);
        format!("{ws_base}/api/v1/ws?session_id={encoded}")
    }

    /// Connect a WebSocket to the given session, sending the API key as an
    /// `X-Api-Key` request header instead of a query-string parameter.
    pub async fn ws_connect(
        &self,
        session_id: &str,
    ) -> Result<WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>> {
        let url = self.ws_url(session_id);
        let mut req = url
            .clone()
            .into_client_request()
            .map_err(|e| format!("invalid WS URL: {e}"))?;
        if let Some(key) = &self.api_key {
            let val = HeaderValue::from_str(key)
                .map_err(|e| format!("invalid API key for WS header: {e}"))?;
            req.headers_mut().insert("X-Api-Key", val);
        }
        let (ws, _) = connect_async(req)
            .await
            .map_err(|e| format!("Failed to connect WebSocket to {url}: {e}"))?;
        Ok(ws)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_session_id_returns_provided_value() {
        let id = OrkaClient::resolve_session_id(Some("my-session"));
        assert_eq!(id, "my-session");
    }

    #[test]
    fn resolve_session_id_generates_uuid_when_none() {
        let id = OrkaClient::resolve_session_id(None);
        // UUID v7 is 36 chars: 8-4-4-4-12
        assert_eq!(id.len(), 36);
        assert!(id.contains('-'));
    }

    #[test]
    fn ws_url_converts_http_to_ws() {
        let client = OrkaClient::new("http://localhost:8080", None);
        let url = client.ws_url("abc");
        assert_eq!(url, "ws://localhost:8080/api/v1/ws?session_id=abc");
    }

    #[test]
    fn ws_url_converts_https_to_wss() {
        let client = OrkaClient::new("https://example.com", None);
        let url = client.ws_url("xyz");
        assert_eq!(url, "wss://example.com/api/v1/ws?session_id=xyz");
    }

    #[test]
    fn ws_url_handles_multiple_trailing_slashes() {
        let client = OrkaClient::new("http://localhost:8080///", None);
        let url = client.ws_url("sess1");
        assert_eq!(url, "ws://localhost:8080/api/v1/ws?session_id=sess1");
    }

    #[test]
    fn ws_url_encodes_special_chars_in_session_id() {
        let client = OrkaClient::new("http://localhost:8080", None);
        let url = client.ws_url("sess&id=1 2");
        assert_eq!(url, "ws://localhost:8080/api/v1/ws?session_id=sess%26id%3D1%202");
    }

    #[test]
    fn percent_encode_safe_chars_unchanged() {
        assert_eq!(percent_encode("abc-def_1.2~3"), "abc-def_1.2~3");
    }

    #[test]
    fn percent_encode_special_chars() {
        assert_eq!(percent_encode("a&b=c #"), "a%26b%3Dc%20%23");
    }

    #[test]
    fn resolve_session_id_uuid_format_valid() {
        let id = OrkaClient::resolve_session_id(None);
        // UUID v7: 8-4-4-4-12 format, all hex + hyphens
        assert_eq!(id.len(), 36);
        let parts: Vec<&str> = id.split('-').collect();
        assert_eq!(parts.len(), 5);
        assert_eq!(parts[0].len(), 8);
        assert_eq!(parts[1].len(), 4);
        assert_eq!(parts[2].len(), 4);
        assert_eq!(parts[3].len(), 4);
        assert_eq!(parts[4].len(), 12);
        // Version nibble (first char of 3rd group) should be '7' for UUID v7
        assert!(parts[2].starts_with('7'), "expected UUID v7, got {id}");
    }

    #[test]
    fn new_trims_trailing_slash_from_base_url() {
        let client = OrkaClient::new("http://localhost:8080/", None);
        assert_eq!(client.base_url(), "http://localhost:8080");
    }
}
