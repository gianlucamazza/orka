use std::sync::Mutex;
use std::time::{Duration, Instant};

use orka_core::{Error, Result};
use reqwest::Client;

use crate::config::McpOAuthConfig;

struct CachedToken {
    token: String,
    expires_at: Instant,
}

/// OAuth 2.1 Client Credentials token provider with in-memory cache.
pub struct OAuthClient {
    http: Client,
    token_url: String,
    client_id: String,
    client_secret: String,
    scopes: String,
    cached: Mutex<Option<CachedToken>>,
}

impl OAuthClient {
    /// Build from config, reading the client secret from the environment.
    pub fn from_config(http: Client, config: &McpOAuthConfig) -> Result<Self> {
        let client_secret = std::env::var(&config.client_secret_env).map_err(|_| {
            Error::Config(format!(
                "MCP OAuth: environment variable '{}' is not set",
                config.client_secret_env
            ))
        })?;
        Ok(Self {
            http,
            token_url: config.token_url.clone(),
            client_id: config.client_id.clone(),
            client_secret,
            scopes: config.scopes.join(" "),
            cached: Mutex::new(None),
        })
    }

    /// Return a valid access token, refreshing if the cached one has expired.
    pub async fn get_token(&self) -> Result<String> {
        // Fast path: valid cached token.
        {
            let guard = self.cached.lock().expect("oauth cache lock poisoned");
            if let Some(ref cached) = *guard
                && cached.expires_at > Instant::now()
            {
                return Ok(cached.token.clone());
            }
        }

        // Fetch a new token via Client Credentials.
        let resp = self
            .http
            .post(&self.token_url)
            .form(&[
                ("grant_type", "client_credentials"),
                ("client_id", &self.client_id),
                ("client_secret", &self.client_secret),
                ("scope", &self.scopes),
            ])
            .send()
            .await
            .map_err(|e| Error::Other(format!("OAuth token request failed: {e}")))?;

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Other(format!("OAuth token response parse failed: {e}")))?;

        let token = body["access_token"]
            .as_str()
            .ok_or_else(|| Error::Other("OAuth response missing 'access_token'".into()))?
            .to_string();

        // Cache with a 30-second safety margin.
        let expires_in = body["expires_in"].as_u64().unwrap_or(3600);
        let ttl = expires_in.saturating_sub(30);

        let mut guard = self.cached.lock().expect("oauth cache lock poisoned");
        *guard = Some(CachedToken {
            token: token.clone(),
            expires_at: Instant::now() + Duration::from_secs(ttl),
        });

        Ok(token)
    }
}
