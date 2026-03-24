//! Channel adapter configurations (Telegram, Discord, Slack, WhatsApp, Custom).

use crate::config::defaults;
use serde::Deserialize;
use std::collections::HashMap;

/// Channel adapter configuration (Telegram, Discord, Slack, WhatsApp, custom).
#[derive(Debug, Clone, Default, Deserialize)]
#[non_exhaustive]
pub struct AdapterConfig {
    /// Custom HTTP adapter configuration.
    pub custom: Option<CustomAdapterConfig>,
    /// Telegram bot adapter configuration.
    pub telegram: Option<TelegramAdapterConfig>,
    /// Discord bot adapter configuration.
    pub discord: Option<DiscordAdapterConfig>,
    /// Slack bot adapter configuration.
    pub slack: Option<SlackAdapterConfig>,
    /// WhatsApp Cloud API adapter configuration.
    pub whatsapp: Option<WhatsAppAdapterConfig>,
}

/// Telegram bot adapter configuration.
#[derive(Clone, Default, Deserialize)]
#[non_exhaustive]
pub struct TelegramAdapterConfig {
    /// Secret store path for the Telegram bot token.
    pub bot_token_secret: Option<String>,
    /// Workspace name to route messages to (uses default if unset).
    #[serde(default)]
    pub workspace: Option<String>,
    /// Receive mode: "polling" (default) or "webhook".
    #[serde(default)]
    pub mode: Option<String>,
    /// Public HTTPS URL for webhook mode.
    #[serde(default)]
    pub webhook_url: Option<String>,
    /// Local port to listen on in webhook mode (default 8443).
    #[serde(default)]
    pub webhook_port: Option<u16>,
    /// Outbound text parse mode: "HTML" (default), "MarkdownV2", or "none".
    #[serde(default)]
    pub parse_mode: Option<String>,
    /// Enable streaming via editMessageText (default false).
    #[serde(default)]
    pub streaming: Option<bool>,
}

impl std::fmt::Debug for TelegramAdapterConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TelegramAdapterConfig")
            .field("workspace", &self.workspace)
            .field("mode", &self.mode)
            .field("streaming", &self.streaming)
            .field("bot_token_secret", &"<redacted>")
            .finish()
    }
}

impl TelegramAdapterConfig {
    /// Returns true if webhook mode is enabled.
    #[must_use]
    pub fn is_webhook(&self) -> bool {
        self.mode.as_deref() == Some("webhook")
    }

    /// Returns the webhook port, defaulting to 8443.
    #[must_use]
    pub fn webhook_port_or_default(&self) -> u16 {
        self.webhook_port.unwrap_or(8443)
    }
}

/// Discord bot adapter configuration.
#[derive(Clone, Default, Deserialize)]
#[non_exhaustive]
pub struct DiscordAdapterConfig {
    /// Secret store path for the Discord bot token.
    pub bot_token_secret: Option<String>,
    /// Workspace name to route messages to.
    #[serde(default)]
    pub workspace: Option<String>,
}

impl std::fmt::Debug for DiscordAdapterConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiscordAdapterConfig")
            .field("workspace", &self.workspace)
            .field("bot_token_secret", &"<redacted>")
            .finish()
    }
}

/// Slack bot adapter configuration.
#[derive(Clone, Deserialize)]
#[non_exhaustive]
pub struct SlackAdapterConfig {
    /// Secret store path for the Slack bot token.
    pub bot_token_secret: Option<String>,
    /// Secret store path for the Slack signing secret.
    pub signing_secret_path: Option<String>,
    /// Workspace name to route messages to.
    #[serde(default)]
    pub workspace: Option<String>,
    /// Port to listen on for Slack events.
    #[serde(default = "defaults::default_slack_port")]
    pub port: u16,
}

impl Default for SlackAdapterConfig {
    fn default() -> Self {
        Self {
            bot_token_secret: None,
            signing_secret_path: None,
            workspace: None,
            port: defaults::default_slack_port(),
        }
    }
}

impl std::fmt::Debug for SlackAdapterConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlackAdapterConfig")
            .field("workspace", &self.workspace)
            .field("port", &self.port)
            .field("bot_token_secret", &"<redacted>")
            .field("signing_secret_path", &"<redacted>")
            .finish()
    }
}

/// WhatsApp Cloud API adapter configuration.
#[derive(Clone, Deserialize)]
#[non_exhaustive]
pub struct WhatsAppAdapterConfig {
    /// Secret store path for the WhatsApp access token.
    pub access_token_secret: Option<String>,
    /// Secret store path for the WhatsApp app secret.
    pub app_secret_path: Option<String>,
    /// WhatsApp phone number ID.
    pub phone_number_id: Option<String>,
    /// WhatsApp business account ID.
    pub business_account_id: Option<String>,
    /// Workspace name to route messages to.
    #[serde(default)]
    pub workspace: Option<String>,
    /// Port to listen on for webhooks.
    #[serde(default = "defaults::default_whatsapp_port")]
    pub port: u16,
    /// Verify token for webhook setup.
    pub verify_token: Option<String>,
}

impl Default for WhatsAppAdapterConfig {
    fn default() -> Self {
        Self {
            access_token_secret: None,
            app_secret_path: None,
            phone_number_id: None,
            business_account_id: None,
            workspace: None,
            port: defaults::default_whatsapp_port(),
            verify_token: None,
        }
    }
}

impl std::fmt::Debug for WhatsAppAdapterConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WhatsAppAdapterConfig")
            .field("workspace", &self.workspace)
            .field("port", &self.port)
            .field("access_token_secret", &"<redacted>")
            .field("app_secret_path", &"<redacted>")
            .finish()
    }
}

/// Custom HTTP/WS adapter configuration.
#[derive(Clone, Deserialize)]
#[non_exhaustive]
pub struct CustomAdapterConfig {
    /// Host to bind the adapter webhook server on.
    #[serde(default = "defaults::default_custom_host")]
    pub host: String,
    /// Port to listen on.
    #[serde(default = "defaults::default_custom_port")]
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
            host: defaults::default_custom_host().to_string(),
            port: defaults::default_custom_port(),
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
            .finish()
    }
}

impl CustomAdapterConfig {
    /// Validate the custom adapter configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if the port is 0.
    pub fn validate(&self) -> crate::Result<()> {
        if self.port == 0 {
            return Err(crate::Error::Config(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telegram_webhook_detection() {
        let polling = TelegramAdapterConfig {
            mode: Some("polling".to_string()),
            ..Default::default()
        };
        assert!(!polling.is_webhook());

        let webhook = TelegramAdapterConfig {
            mode: Some("webhook".to_string()),
            ..Default::default()
        };
        assert!(webhook.is_webhook());
    }

    #[test]
    fn telegram_default_webhook_port() {
        let config = TelegramAdapterConfig::default();
        assert_eq!(config.webhook_port_or_default(), 8443);
    }

    #[test]
    fn custom_adapter_validates_port() {
        let invalid = CustomAdapterConfig {
            port: 0,
            ..Default::default()
        };
        assert!(invalid.validate().is_err());

        let valid = CustomAdapterConfig {
            port: 8080,
            ..Default::default()
        };
        assert!(valid.validate().is_ok());
    }

    #[test]
    fn slack_default_port() {
        let config = SlackAdapterConfig::default();
        assert_eq!(config.port, 3000);
    }

    #[test]
    fn whatsapp_default_port() {
        let config = WhatsAppAdapterConfig::default();
        assert_eq!(config.port, 3000);
    }
}
