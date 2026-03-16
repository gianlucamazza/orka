use async_trait::async_trait;
use orka_core::Result;

use crate::types::SearchResult;

/// Trait for pluggable search backends.
#[async_trait]
pub trait SearchProvider: Send + Sync {
    async fn search(&self, query: &str, max_results: usize) -> Result<Vec<SearchResult>>;
}

/// Tavily search provider — purpose-built for AI agents.
pub struct TavilyProvider {
    client: reqwest::Client,
    api_key: String,
}

impl TavilyProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
        }
    }
}

#[async_trait]
impl SearchProvider for TavilyProvider {
    async fn search(&self, query: &str, max_results: usize) -> Result<Vec<SearchResult>> {
        let body = serde_json::json!({
            "query": query,
            "max_results": max_results,
            "include_answer": false,
        });

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
                    .map(|r| SearchResult {
                        title: r["title"].as_str().unwrap_or("").to_string(),
                        url: r["url"].as_str().unwrap_or("").to_string(),
                        snippet: r["content"].as_str().unwrap_or("").to_string(),
                        score: r["score"].as_f64(),
                        published_date: r["published_date"].as_str().map(String::from),
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(results)
    }
}

/// Brave Search API provider.
pub struct BraveProvider {
    client: reqwest::Client,
    api_key: String,
}

impl BraveProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
        }
    }
}

#[async_trait]
impl SearchProvider for BraveProvider {
    async fn search(&self, query: &str, max_results: usize) -> Result<Vec<SearchResult>> {
        let resp = self
            .client
            .get("https://api.search.brave.com/res/v1/web/search")
            .header("X-Subscription-Token", &self.api_key)
            .header("Accept", "application/json")
            .query(&[("q", query), ("count", &max_results.to_string())])
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

        let results = data["web"]["results"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|r| SearchResult {
                        title: r["title"].as_str().unwrap_or("").to_string(),
                        url: r["url"].as_str().unwrap_or("").to_string(),
                        snippet: r["description"].as_str().unwrap_or("").to_string(),
                        score: None,
                        published_date: r["page_age"].as_str().map(String::from),
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(results)
    }
}

/// SearXNG provider — self-hosted, no API key required.
pub struct SearxngProvider {
    client: reqwest::Client,
    base_url: String,
}

impl SearxngProvider {
    pub fn new(base_url: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }
}

#[async_trait]
impl SearchProvider for SearxngProvider {
    async fn search(&self, query: &str, max_results: usize) -> Result<Vec<SearchResult>> {
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

        let results = data["results"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .take(max_results)
                    .map(|r| SearchResult {
                        title: r["title"].as_str().unwrap_or("").to_string(),
                        url: r["url"].as_str().unwrap_or("").to_string(),
                        snippet: r["content"].as_str().unwrap_or("").to_string(),
                        score: r["score"].as_f64(),
                        published_date: r["publishedDate"].as_str().map(String::from),
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn searxng_strips_trailing_slash() {
        let p = SearxngProvider::new("http://localhost:8888/".into());
        assert_eq!(p.base_url, "http://localhost:8888");
    }
}
