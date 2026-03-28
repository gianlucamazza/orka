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
    /// Outbound text parse mode: "HTML" (default), "`MarkdownV2`", or "none".
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
            .finish_non_exhaustive()
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
