//! Web search and read configuration.

use serde::Deserialize;

use crate::config::defaults;

/// Web search and read configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct WebConfig {
    /// Search backend to use (`"tavily"`, `"brave"`, `"searxng"`, or `"none"`).
    #[serde(default = "defaults::default_web_search_provider")]
    pub search_provider: String,
    /// Direct API key for the search provider (prefer `api_key_env` in
    /// production).
    #[serde(default)]
    pub api_key: Option<String>,
    /// Environment variable name containing the search provider API key.
    #[serde(default)]
    pub api_key_env: Option<String>,
    /// Base URL for a SearXNG instance (required when `search_provider =
    /// "searxng"`).
    #[serde(default)]
    pub searxng_base_url: Option<String>,
    /// Maximum number of search results to return per query.
    #[serde(default = "defaults::default_web_max_results")]
    pub max_results: usize,
    /// Maximum characters to read from a single web page.
    #[serde(default = "defaults::default_web_max_read_chars")]
    pub max_read_chars: usize,
    /// Maximum characters to include in search results content.
    #[serde(default = "defaults::default_web_max_content_chars")]
    pub max_content_chars: usize,
    /// Cache TTL for web content in seconds.
    #[serde(default = "defaults::default_web_cache_ttl_secs")]
    pub cache_ttl_secs: u64,
    /// Timeout for web requests in seconds.
    #[serde(default = "defaults::default_web_read_timeout_secs")]
    pub read_timeout_secs: u64,
    /// User agent string for web requests.
    #[serde(default = "defaults::default_web_user_agent")]
    pub user_agent: String,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            search_provider: defaults::default_web_search_provider(),
            api_key: None,
            api_key_env: None,
            searxng_base_url: None,
            max_results: defaults::default_web_max_results(),
            max_read_chars: defaults::default_web_max_read_chars(),
            max_content_chars: defaults::default_web_max_content_chars(),
            cache_ttl_secs: defaults::default_web_cache_ttl_secs(),
            read_timeout_secs: defaults::default_web_read_timeout_secs(),
            user_agent: defaults::default_web_user_agent(),
        }
    }
}
