//! `WhatsApp` adapter configuration.

use serde::Deserialize;

/// `WhatsApp` Cloud API adapter configuration.
#[derive(Clone, Deserialize)]
#[non_exhaustive]
pub struct WhatsAppAdapterConfig {
    /// Secret store path for the `WhatsApp` access token.
    pub access_token_secret: Option<String>,
    /// Secret store path for the `WhatsApp` app secret.
    pub app_secret_path: Option<String>,
    /// `WhatsApp` phone number ID.
    pub phone_number_id: Option<String>,
    /// `WhatsApp` business account ID.
    pub business_account_id: Option<String>,
    /// Workspace name to route messages to.
    #[serde(default)]
    pub workspace: Option<String>,
    /// Port to listen on for webhooks.
    #[serde(default = "default_whatsapp_port")]
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
            port: default_whatsapp_port(),
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
            .finish_non_exhaustive()
    }
}

const fn default_whatsapp_port() -> u16 {
    3002
}
