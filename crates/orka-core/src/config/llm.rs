//! LLM (Large Language Model) configuration.

use serde::Deserialize;

use crate::config::defaults;

/// Authentication mode for an LLM provider.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum LlmAuthKind {
    /// Backward-compatible mode: infer the auth transport from explicit auth
    /// settings first, then fall back to legacy token-shape detection.
    #[default]
    Auto,
    /// Use API key semantics (for example `X-Api-Key` for Anthropic).
    ApiKey,
    /// Use bearer-token semantics (`Authorization: Bearer ...`).
    AuthToken,
    /// Subscription / setup-token style auth. Runtime maps this to the
    /// provider's bearer-compatible code paths.
    Subscription,
    /// Delegate to a CLI-backed provider instead of direct HTTP.
    Cli,
}

/// LLM provider configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct LlmConfig {
    /// Default agent model identifier.
    #[serde(default = "defaults::default_model")]
    pub default_model: String,
    /// Default temperature for generation.
    #[serde(default = "defaults::default_temperature")]
    pub default_temperature: f32,
    /// Default max tokens for generation.
    #[serde(default = "defaults::default_max_tokens")]
    pub default_max_tokens: u32,
    /// Available LLM providers.
    #[serde(default)]
    pub providers: Vec<LlmProviderConfig>,
}

impl LlmConfig {
    /// Create a new LLM config with defaults.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply default provider settings if not specified.
    pub fn apply_defaults(&mut self) {
        for provider in &mut self.providers {
            if provider.temperature.is_none() {
                provider.temperature = Some(self.default_temperature);
            }
            if provider.max_tokens.is_none() {
                provider.max_tokens = Some(self.default_max_tokens);
            }
        }
    }

    /// Find a provider by name.
    #[must_use]
    pub fn find_provider(&self, name: &str) -> Option<&LlmProviderConfig> {
        self.providers.iter().find(|p| p.name == name)
    }

    /// Validate the LLM configuration.
    pub fn validate(&self) -> crate::Result<()> {
        for provider in &self.providers {
            provider.validate()?;
        }
        Ok(())
    }
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            default_model: defaults::default_model(),
            default_temperature: defaults::default_temperature(),
            default_max_tokens: defaults::default_max_tokens(),
            providers: Vec::new(),
        }
    }
}

/// Individual LLM provider configuration.
#[derive(Clone, Default, Deserialize)]
#[non_exhaustive]
pub struct LlmProviderConfig {
    /// Provider identifier.
    pub name: String,
    /// Provider type.
    pub provider: String,
    /// Authentication mode for this provider.
    #[serde(default)]
    pub auth_kind: LlmAuthKind,
    /// Base URL for the API.
    pub base_url: Option<String>,
    /// Default model for this provider.
    pub model: Option<String>,
    /// API key.
    pub api_key: Option<String>,
    /// Environment variable containing the API key.
    pub api_key_env: Option<String>,
    /// Secret store path for the API key.
    pub api_key_secret: Option<String>,
    /// Bearer/auth token.
    pub auth_token: Option<String>,
    /// Environment variable containing the bearer/auth token.
    pub auth_token_env: Option<String>,
    /// Secret store path for the bearer/auth token.
    pub auth_token_secret: Option<String>,
    /// Temperature for generation.
    pub temperature: Option<f32>,
    /// Maximum tokens to generate.
    pub max_tokens: Option<u32>,
    /// Top-p sampling parameter.
    pub top_p: Option<f32>,
    /// Request timeout in seconds.
    pub timeout_secs: Option<u64>,
    /// Maximum retries for failed requests.
    pub max_retries: Option<u32>,
    /// Additional provider-specific parameters.
    #[serde(flatten)]
    pub extra: serde_json::Value,
}

impl std::fmt::Debug for LlmProviderConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmProviderConfig")
            .field("name", &self.name)
            .field("provider", &self.provider)
            .field("auth_kind", &self.auth_kind)
            .field("api_key", &self.api_key.as_ref().map(|_| "***"))
            .field(
                "api_key_secret",
                &self.api_key_secret.as_ref().map(|_| "***"),
            )
            .field("api_key_env", &self.api_key_env)
            .field("auth_token", &self.auth_token.as_ref().map(|_| "***"))
            .field(
                "auth_token_secret",
                &self.auth_token_secret.as_ref().map(|_| "***"),
            )
            .field("auth_token_env", &self.auth_token_env)
            .field("model", &self.model)
            .field("temperature", &self.temperature)
            .field("max_tokens", &self.max_tokens)
            .field("top_p", &self.top_p)
            .field("timeout_secs", &self.timeout_secs)
            .field("max_retries", &self.max_retries)
            .field("base_url", &self.base_url)
            .field("extra", &self.extra)
            .finish()
    }
}

impl LlmProviderConfig {
    /// Create a minimal provider config with the given name and provider type.
    #[must_use]
    pub fn for_provider(name: impl Into<String>, provider: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            provider: provider.into(),
            ..Self::default()
        }
    }

    /// Validate the provider configuration.
    pub fn validate(&self) -> crate::Result<()> {
        if self.name.is_empty() {
            return Err(crate::Error::Config(
                "llm.providers[].name must not be empty".into(),
            ));
        }
        Ok(())
    }
}
