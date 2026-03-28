//! Custom adapter configuration.

use std::collections::HashMap;

use serde::Deserialize;

/// Custom HTTP/WS adapter configuration.
#[derive(Clone, Deserialize)]
#[non_exhaustive]
pub struct CustomAdapterConfig {
    /// Host to bind the adapter webhook server on.
    #[serde(default = "default_custom_host")]
    pub host: String,
    /// Port to listen on.
    #[serde(default = "default_custom_port")]
    pub port: u16,
    /// Path for the webhook endpoint (e.g., "/webhook").
    #[serde(default)]
    pub webhook_path: Option<String>,
    /// Optional bearer token for authenticating incoming webhooks.
    pub bearer_token_secret: Option<String>,
    /// Workspace name to route messages to.
    #[serde(default)]
    pub workspace: Option<String>,
    /// Custom HTTP headers to include in responses.
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

impl Default for CustomAdapterConfig {
    fn default() -> Self {
        Self {
            host: default_custom_host(),
            port: default_custom_port(),
            webhook_path: None,
            bearer_token_secret: None,
            workspace: None,
            headers: HashMap::new(),
        }
    }
}

impl std::fmt::Debug for CustomAdapterConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CustomAdapterConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("webhook_path", &self.webhook_path)
            .field("workspace", &self.workspace)
            .field("bearer_token_secret", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl CustomAdapterConfig {
    /// Validate the custom adapter configuration.
    pub fn validate(&self) -> orka_core::Result<()> {
        if self.port == 0 {
            return Err(orka_core::Error::Config(
                "adapters.custom.port must be in range 1-65535".into(),
            ));
        }
        Ok(())
    }

    /// Returns the full webhook URL path.
    #[must_use]
    pub fn webhook_path_or_default(&self) -> &str {
        self.webhook_path.as_deref().unwrap_or("/webhook")
    }
}

fn default_custom_host() -> String {
    "0.0.0.0".to_string()
}

const fn default_custom_port() -> u16 {
    3000
}
