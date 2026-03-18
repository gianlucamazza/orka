//! Web search and content reading skills.
//!
//! Provides `web_search` and `web_read` skills with configurable search providers
//! and HTML-to-markdown extraction.

#![warn(missing_docs)]

mod cache;
pub(crate) mod extract;
mod provider;
mod read;
mod search;
/// Shared types for web search results and configuration.
pub mod types;

use std::sync::Arc;

use orka_core::Result;
use orka_core::traits::Skill;
use tracing::info;

pub use types::{SearchProviderKind, WebConfig};

use cache::WebCache;
use provider::{BraveProvider, SearchProvider, SearxngProvider, TavilyProvider};
use read::WebReadSkill;
use search::WebSearchSkill;

/// Create web skills (web_search + web_read) from config.
///
/// Returns an empty vec if the provider is set to `none`.
/// The API key is resolved from: config.api_key > env var (config.api_key_env) > provider-specific env var.
pub fn create_web_skills(config: &WebConfig) -> Result<Vec<Arc<dyn Skill>>> {
    if config.search_provider == SearchProviderKind::None {
        return Ok(Vec::new());
    }

    let cache = Arc::new(WebCache::new(config.cache_ttl_secs));

    // Resolve API key
    let api_key = config
        .api_key
        .clone()
        .or_else(|| {
            config
                .api_key_env
                .as_deref()
                .and_then(|env| std::env::var(env).ok())
        })
        .or_else(|| match config.search_provider {
            SearchProviderKind::Tavily => std::env::var("TAVILY_API_KEY").ok(),
            SearchProviderKind::Brave => std::env::var("BRAVE_API_KEY").ok(),
            SearchProviderKind::Searxng | SearchProviderKind::None => None,
        });

    let provider: Arc<dyn SearchProvider> = match config.search_provider {
        SearchProviderKind::Tavily => {
            let key = api_key.ok_or_else(|| {
                orka_core::Error::Config(
                    "web.api_key or TAVILY_API_KEY required for tavily provider".into(),
                )
            })?;
            Arc::new(TavilyProvider::new(key, config.read_timeout_secs))
        }
        SearchProviderKind::Brave => {
            let key = api_key.ok_or_else(|| {
                orka_core::Error::Config(
                    "web.api_key or BRAVE_API_KEY required for brave provider".into(),
                )
            })?;
            Arc::new(BraveProvider::new(
                key,
                config.read_timeout_secs,
                &config.user_agent,
            ))
        }
        SearchProviderKind::Searxng => {
            let base_url = config
                .searxng_base_url
                .clone()
                .unwrap_or_else(|| "http://localhost:8080".into());
            Arc::new(SearxngProvider::new(
                base_url,
                config.read_timeout_secs,
                &config.user_agent,
            ))
        }
        SearchProviderKind::None => {
            return Err(orka_core::Error::Skill("no search provider configured".into()));
        }
    };

    let search_skill: Arc<dyn Skill> = Arc::new(WebSearchSkill::new(
        provider,
        cache.clone(),
        config.max_results,
        config.max_content_chars,
    ));

    let read_skill: Arc<dyn Skill> = Arc::new(WebReadSkill::new(
        cache,
        config.max_read_chars,
        config.read_timeout_secs,
        &config.user_agent,
    ));

    info!(
        provider = ?config.search_provider,
        "web skills initialized (web_search, web_read)"
    );

    Ok(vec![search_skill, read_skill])
}
