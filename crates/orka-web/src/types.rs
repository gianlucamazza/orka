use serde::{Deserialize, Serialize};

/// A single search result from a web search provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

/// Extracted readable content from a web page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebContent {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub text: String,
    pub truncated: bool,
    pub content_length: usize,
}

/// Configuration for web search and read skills.
#[derive(Debug, Clone, Deserialize)]
pub struct WebConfig {
    #[serde(default = "default_search_provider")]
    pub search_provider: SearchProviderKind,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub searxng_base_url: Option<String>,
    #[serde(default = "default_max_results")]
    pub max_results: usize,
    #[serde(default = "default_max_read_chars")]
    pub max_read_chars: usize,
    #[serde(default = "default_cache_ttl_secs")]
    pub cache_ttl_secs: u64,
    #[serde(default = "default_max_content_chars")]
    pub max_content_chars: usize,
    #[serde(default = "default_read_timeout_secs")]
    pub read_timeout_secs: u64,
    #[serde(default = "default_user_agent")]
    pub user_agent: String,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            search_provider: default_search_provider(),
            api_key: None,
            api_key_env: None,
            searxng_base_url: None,
            max_results: default_max_results(),
            max_read_chars: default_max_read_chars(),
            max_content_chars: default_max_content_chars(),
            cache_ttl_secs: default_cache_ttl_secs(),
            read_timeout_secs: default_read_timeout_secs(),
            user_agent: default_user_agent(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchProviderKind {
    Tavily,
    Brave,
    Searxng,
    None,
}

fn default_search_provider() -> SearchProviderKind {
    SearchProviderKind::None
}

fn default_max_results() -> usize {
    5
}

fn default_max_read_chars() -> usize {
    20_000
}

fn default_max_content_chars() -> usize {
    8_000
}

fn default_cache_ttl_secs() -> u64 {
    3600
}

fn default_read_timeout_secs() -> u64 {
    15
}

fn default_user_agent() -> String {
    "Orka/0.1 (Web Agent)".into()
}
