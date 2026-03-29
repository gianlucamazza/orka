//! Slack adapter configuration.

use serde::Deserialize;

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
    #[serde(default = "default_slack_port")]
    pub port: u16,
}

impl Default for SlackAdapterConfig {
    fn default() -> Self {
        Self {
            bot_token_secret: None,
            signing_secret_path: None,
            workspace: None,
            port: default_slack_port(),
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
            .finish_non_exhaustive()
    }
}

impl SlackAdapterConfig {
    /// Validate Slack adapter configuration.
    pub fn validate(&self) -> orka_core::Result<()> {
        Ok(())
    }
}

const fn default_slack_port() -> u16 {
    3001
}
