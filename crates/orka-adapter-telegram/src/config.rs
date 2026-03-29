//! Telegram adapter configuration.

use serde::Deserialize;

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
    /// Secret token registered with Telegram for webhook authentication
    /// (`X-Telegram-Bot-Api-Secret-Token` header).  When set, Telegram will
    /// include this token in every POST to the webhook endpoint and the
    /// adapter will reject requests that do not carry it.  Allowed characters:
    /// `A-Z`, `a-z`, `0-9`, `_` and `-`.  1–256 chars.
    #[serde(default)]
    pub webhook_secret: Option<String>,
    /// Outbound text parse mode: "HTML" (default), "`MarkdownV2`", or "none".
    #[serde(default)]
    pub parse_mode: Option<String>,
    /// Enable streaming via editMessageText (default false).
    #[serde(default)]
    pub streaming: Option<bool>,
    /// Restrict bot access to these Telegram user IDs. When set, messages
    /// from any other user are silently dropped. When empty or unset, all
    /// users are allowed.
    #[serde(default)]
    pub allowed_users: Vec<i64>,
}

impl std::fmt::Debug for TelegramAdapterConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TelegramAdapterConfig")
            .field("workspace", &self.workspace)
            .field("mode", &self.mode)
            .field("streaming", &self.streaming)
            .field("bot_token_secret", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl TelegramAdapterConfig {
    /// Validate Telegram adapter configuration.
    pub fn validate(&self) -> orka_core::Result<()> {
        if self.is_webhook() && self.webhook_url.is_none() {
            return Err(orka_core::Error::Config(
                "adapters.telegram.webhook_url is required when mode = \"webhook\"".into(),
            ));
        }
        if let Some(secret) = &self.webhook_secret {
            if secret.is_empty() || secret.len() > 256 {
                return Err(orka_core::Error::Config(
                    "adapters.telegram.webhook_secret must be 1–256 characters".into(),
                ));
            }
            if !secret
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                return Err(orka_core::Error::Config(
                    "adapters.telegram.webhook_secret must only contain A-Z, a-z, 0-9, _ or -"
                        .into(),
                ));
            }
        }
        Ok(())
    }

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
