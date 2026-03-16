use std::time::Duration;

use async_trait::async_trait;
use orka_core::Result;
use tracing::debug;

use crate::extract;
use crate::types::SearchResult;

/// Options passed to search providers.
pub struct SearchOptions {
    pub max_results: usize,
    pub include_content: bool,
    pub max_content_chars: usize,
}

/// Trait for pluggable search backends.
#[async_trait]
pub trait SearchProvider: Send + Sync {
    async fn search(&self, query: &str, options: &SearchOptions) -> Result<Vec<SearchResult>>;
}

/// Fetch a URL, extract text, and truncate. Returns None on any error.
async fn fetch_and_extract(
    client: &reqwest::Client,
    url: &str,
    max_chars: usize,
) -> Option<String> {
    let resp = client.get(url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    if !ct.contains("text/html") && !ct.contains("text/plain") {
        return None;
    }
    let body = resp.text().await.ok()?;
    let text = if ct.contains("text/html") {
        extract::extract_text(&body)
    } else {
        body
    };
    let (truncated, _) = extract::truncate(&text, max_chars);
    Some(truncated)
}

// ---------------------------------------------------------------------------
// Tavily
// ---------------------------------------------------------------------------

/// Tavily search provider — purpose-built for AI agents.
pub struct TavilyProvider {
    client: reqwest::Client,
    api_key: String,
}

impl TavilyProvider {
    pub fn new(api_key: String, timeout_secs: u64) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .unwrap_or_default();
        Self { client, api_key }
    }
}

#[async_trait]
impl SearchProvider for TavilyProvider {
    async fn search(&self, query: &str, options: &SearchOptions) -> Result<Vec<SearchResult>> {
        let mut body = serde_json::json!({
            "query": query,
            "max_results": options.max_results,
            "include_answer": false,
        });

        if options.include_content {
            body["include_raw_content"] = serde_json::json!(true);
        }

        let resp = self
            .client
            .post("https://api.tavily.com/search")
            .header("Content-Type", "application/json")
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| orka_core::Error::Skill(format!("tavily request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(orka_core::Error::Skill(format!(
                "tavily returned {status}: {text}"
            )));
        }

        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| orka_core::Error::Skill(format!("tavily parse error: {e}")))?;

        let results = data["results"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|r| {
                        let content = if options.include_content {
                            r["raw_content"].as_str().map(|raw| {
                                let (t, _) =
                                    extract::truncate(raw, options.max_content_chars);
                                t
                            })
                        } else {
                            None
                        };
                        SearchResult {
                            title: r["title"].as_str().unwrap_or("").to_string(),
                            url: r["url"].as_str().unwrap_or("").to_string(),
                            snippet: r["content"].as_str().unwrap_or("").to_string(),
                            score: r["score"].as_f64(),
                            published_date: r["published_date"].as_str().map(String::from),
                            content,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(results)
    }
}

// ---------------------------------------------------------------------------
// Brave
// ---------------------------------------------------------------------------

/// Brave Search API provider.
pub struct BraveProvider {
    client: reqwest::Client,
    /// Separate client with short timeout for page fetches.
    fetch_client: reqwest::Client,
}

impl BraveProvider {
    pub fn new(api_key: String, timeout_secs: u64, user_agent: &str) -> Self {
        let mut headers = reqwest::header::HeaderMap::new();
        if let Ok(val) = api_key.parse() {
            headers.insert("X-Subscription-Token", val);
        }
        headers.insert("Accept", "application/json".parse().unwrap());

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .default_headers(headers)
            .build()
            .unwrap_or_default();

        let fetch_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent(user_agent)
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .unwrap_or_default();

        Self {
            client,
            fetch_client,
        }
    }
}

#[async_trait]
impl SearchProvider for BraveProvider {
    async fn search(&self, query: &str, options: &SearchOptions) -> Result<Vec<SearchResult>> {
        let resp = self
            .client
            .get("https://api.search.brave.com/res/v1/web/search")
            .query(&[("q", query), ("count", &options.max_results.to_string())])
            .send()
            .await
            .map_err(|e| orka_core::Error::Skill(format!("brave request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(orka_core::Error::Skill(format!(
                "brave returned {status}: {text}"
            )));
        }

        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| orka_core::Error::Skill(format!("brave parse error: {e}")))?;

        let mut results: Vec<SearchResult> = data["web"]["results"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|r| SearchResult {
                        title: r["title"].as_str().unwrap_or("").to_string(),
                        url: r["url"].as_str().unwrap_or("").to_string(),
                        snippet: r["description"].as_str().unwrap_or("").to_string(),
                        score: None,
                        published_date: r["page_age"].as_str().map(String::from),
                        content: None,
                    })
                    .collect()
            })
            .unwrap_or_default();

        if options.include_content && !results.is_empty() {
            debug!(count = results.len(), "brave: fetching page content inline");
            let mut set = tokio::task::JoinSet::new();
            for (i, r) in results.iter().enumerate() {
                let client = self.fetch_client.clone();
                let url = r.url.clone();
                let max_chars = options.max_content_chars;
                set.spawn(async move {
                    let content = fetch_and_extract(&client, &url, max_chars).await;
                    (i, content)
                });
            }
            while let Some(Ok((i, content))) = set.join_next().await {
                if let Some(r) = results.get_mut(i) {
                    r.content = content;
                }
            }
        }

        Ok(results)
    }
}

// ---------------------------------------------------------------------------
// SearXNG
// ---------------------------------------------------------------------------

/// SearXNG provider — self-hosted, no API key required.
pub struct SearxngProvider {
    client: reqwest::Client,
    fetch_client: reqwest::Client,
    base_url: String,
}

impl SearxngProvider {
    pub fn new(base_url: String, timeout_secs: u64, user_agent: &str) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .unwrap_or_default();

        let fetch_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent(user_agent)
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .unwrap_or_default();

        Self {
            client,
            fetch_client,
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }
}

#[async_trait]
impl SearchProvider for SearxngProvider {
    async fn search(&self, query: &str, options: &SearchOptions) -> Result<Vec<SearchResult>> {
        let url = format!("{}/search", self.base_url);

        let resp = self
            .client
            .get(&url)
            .query(&[("q", query), ("format", "json"), ("pageno", "1")])
            .send()
            .await
            .map_err(|e| orka_core::Error::Skill(format!("searxng request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(orka_core::Error::Skill(format!(
                "searxng returned {status}: {text}"
            )));
        }

        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| orka_core::Error::Skill(format!("searxng parse error: {e}")))?;

        let mut results: Vec<SearchResult> = data["results"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .take(options.max_results)
                    .map(|r| SearchResult {
                        title: r["title"].as_str().unwrap_or("").to_string(),
                        url: r["url"].as_str().unwrap_or("").to_string(),
                        snippet: r["content"].as_str().unwrap_or("").to_string(),
                        score: r["score"].as_f64(),
                        published_date: r["publishedDate"].as_str().map(String::from),
                        content: None,
                    })
                    .collect()
            })
            .unwrap_or_default();

        if options.include_content && !results.is_empty() {
            debug!(count = results.len(), "searxng: fetching page content inline");
            let mut set = tokio::task::JoinSet::new();
            for (i, r) in results.iter().enumerate() {
                let client = self.fetch_client.clone();
                let url = r.url.clone();
                let max_chars = options.max_content_chars;
                set.spawn(async move {
                    let content = fetch_and_extract(&client, &url, max_chars).await;
                    (i, content)
                });
            }
            while let Some(Ok((i, content))) = set.join_next().await {
                if let Some(r) = results.get_mut(i) {
                    r.content = content;
                }
            }
        }

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn searxng_strips_trailing_slash() {
        let p = SearxngProvider::new("http://localhost:8888/".into(), 30, "test");
        assert_eq!(p.base_url, "http://localhost:8888");
    }
}
