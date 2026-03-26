use serde::{Deserialize, Serialize};

/// A single search result from a web search provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// Page title.
    pub title: String,
    /// Canonical URL of the result.
    pub url: String,
    /// Short snippet from the search engine.
    pub snippet: String,
    /// Relevance score, if provided by the search backend.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    /// Publication date as an ISO-8601 string, if available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_date: Option<String>,
    /// Fetched page content, if `max_content_chars > 0`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

/// Extracted readable content from a web page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebContent {
    /// Source URL.
    pub url: String,
    /// Page title extracted from the HTML `<title>` tag.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Extracted plain text content.
    pub text: String,
    /// Whether the content was cut off at `max_read_chars`.
    pub truncated: bool,
    /// Total character count before truncation.
    pub content_length: usize,
}

/// Configuration for web search and read skills.
#[derive(Debug, Clone, Deserialize)]
pub struct WebConfig {
    /// Search backend to use.
    #[serde(default = "default_search_provider")]
    pub search_provider: SearchProviderKind,
    /// API key (takes precedence over `api_key_env`).
    #[serde(default)]
    pub api_key: Option<String>,
    /// Environment variable that holds the API key.
    #[serde(default)]
    pub api_key_env: Option<String>,
    /// Base URL for a self-hosted `SearXNG` instance.
    #[serde(default)]
    pub searxng_base_url: Option<String>,
    /// Maximum number of search results returned per query.
    #[serde(default = "default_max_results")]
    pub max_results: usize,
    /// Maximum characters fetched by `web_read`.
    #[serde(default = "default_max_read_chars")]
    pub max_read_chars: usize,
    /// Result cache TTL in seconds.
    #[serde(default = "default_cache_ttl_secs")]
    pub cache_ttl_secs: u64,
    /// Maximum characters of page content included in search results.
    #[serde(default = "default_max_content_chars")]
    pub max_content_chars: usize,
    /// HTTP read timeout in seconds.
    #[serde(default = "default_read_timeout_secs")]
    pub read_timeout_secs: u64,
    /// `User-Agent` header sent with HTTP requests.
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

/// Supported web search backends.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchProviderKind {
    /// Tavily AI search API.
    Tavily,
    /// Brave Search API.
    Brave,
    /// Self-hosted `SearXNG` instance.
    Searxng,
    /// Web skills disabled.
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
    format!("Orka/{} (Web Agent)", env!("CARGO_PKG_VERSION"))
}
