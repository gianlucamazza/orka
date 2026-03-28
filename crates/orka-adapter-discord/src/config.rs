//! Discord adapter configuration.

use serde::Deserialize;

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
            .finish_non_exhaustive()
    }
}
