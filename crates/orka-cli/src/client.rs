use std::time::Duration;

use serde_json::json;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub struct OrkaClient {
    http: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
}

impl OrkaClient {
    pub fn new(base_url: &str, api_key: Option<&str>) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.map(String::from),
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub async fn send_message(&self, text: &str, session_id: &str) -> Result<serde_json::Value> {
        let url = format!("{}/api/v1/message", self.base_url);
        let payload = json!({
            "text": text,
            "session_id": session_id,
        });
        let mut req = self.http.post(&url).json(&payload);
        if let Some(key) = &self.api_key {
            req = req.header("X-Api-Key", key);
        }
        let resp = req.send().await?;
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
        let url = format!("{}{path}", self.base_url);
        let mut req = self.http.get(&url);
        if let Some(key) = &self.api_key {
            req = req.header("X-Api-Key", key);
        }
        req.send().await
    }

    pub async fn post(
        &self,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> std::result::Result<reqwest::Response, reqwest::Error> {
        let url = format!("{}{path}", self.base_url);
        let mut req = self.http.post(&url);
        if let Some(key) = &self.api_key {
            req = req.header("X-Api-Key", key);
        }
        if let Some(b) = body {
            req = req.json(&b);
        }
        req.send().await
    }

    pub async fn delete(
        &self,
        path: &str,
    ) -> std::result::Result<reqwest::Response, reqwest::Error> {
        let url = format!("{}{path}", self.base_url);
        let mut req = self.http.delete(&url);
        if let Some(key) = &self.api_key {
            req = req.header("X-Api-Key", key);
        }
        req.send().await
    }

    /// Resolve an optional session ID, generating a new UUID v4 if absent.
    pub fn resolve_session_id(session_id: Option<&str>) -> String {
        session_id
            .map(String::from)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
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
        let mut url = format!("{ws_base}/api/v1/ws?session_id={session_id}");
        if let Some(key) = &self.api_key {
            url.push_str(&format!("&api_key={key}"));
        }
        url
    }
}
