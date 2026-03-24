//! HTTP client, webhook, and prompt configuration.

use serde::Deserialize;

/// HTTP client and webhook configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct HttpClientConfig {
    /// Request timeout in seconds.
    #[serde(default = "default_http_timeout_secs")]
    pub timeout_secs: u64,
    /// Maximum redirects to follow.
    #[serde(default = "default_max_redirects")]
    pub max_redirects: usize,
    /// User agent string.
    #[serde(default)]
    pub user_agent: Option<String>,
    /// Custom headers.
    #[serde(default)]
    pub default_headers: Vec<(String, String)>,
    /// Webhook configurations.
    #[serde(default)]
    pub webhooks: Vec<WebhookConfig>,
}

impl Default for HttpClientConfig {
    fn default() -> Self {
        Self {
            timeout_secs: default_http_timeout_secs(),
            max_redirects: default_max_redirects(),
            user_agent: None,
            default_headers: Vec::new(),
            webhooks: Vec::new(),
        }
    }
}

const fn default_http_timeout_secs() -> u64 {
    30
}

const fn default_max_redirects() -> usize {
    10
}

/// Webhook endpoint configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct WebhookConfig {
    /// Webhook name.
    pub name: String,
    /// Target URL.
    pub url: String,
    /// HTTP method.
    #[serde(default = "default_webhook_method")]
    pub method: String,
    /// Secret for HMAC signature.
    pub secret: Option<String>,
    /// Retry configuration.
    #[serde(default)]
    pub retry: WebhookRetryConfig,
}

impl Default for WebhookConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            url: String::new(),
            method: default_webhook_method(),
            secret: None,
            retry: WebhookRetryConfig::default(),
        }
    }
}

fn default_webhook_method() -> String {
    "POST".to_string()
}

/// Webhook retry configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct WebhookRetryConfig {
    /// Maximum retry attempts.
    #[serde(default = "default_webhook_max_retries")]
    pub max_retries: u32,
    /// Base delay between retries (seconds).
    #[serde(default = "default_webhook_retry_delay_secs")]
    pub delay_secs: u64,
}

impl Default for WebhookRetryConfig {
    fn default() -> Self {
        Self {
            max_retries: default_webhook_max_retries(),
            delay_secs: default_webhook_retry_delay_secs(),
        }
    }
}

const fn default_webhook_max_retries() -> u32 {
    3
}

const fn default_webhook_retry_delay_secs() -> u64 {
    5
}

impl HttpClientConfig {
    /// Validate the HTTP client configuration.
    pub fn validate(&self) -> crate::Result<()> {
        if self.timeout_secs == 0 {
            return Err(crate::Error::Config(
                "http.timeout_secs must be greater than 0".into(),
            ));
        }
        Ok(())
    }
}
