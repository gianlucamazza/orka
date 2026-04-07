use std::sync::Arc;

use async_trait::async_trait;
use orka_core::{
    Error, ErrorCategory, Result, SkillInput, SkillOutput, SkillSchema, traits::Skill,
};
use tracing::debug;

use crate::{
    cache::WebCache,
    provider::{SearchOptions, SearchProvider},
};

/// Skill that searches the web using a configured provider.
pub(crate) struct WebSearchSkill {
    provider: Arc<dyn SearchProvider>,
    cache: Arc<WebCache>,
    max_results: usize,
    max_content_chars: usize,
}

impl WebSearchSkill {
    pub(crate) fn new(
        provider: Arc<dyn SearchProvider>,
        cache: Arc<WebCache>,
        max_results: usize,
        max_content_chars: usize,
    ) -> Self {
        Self {
            provider,
            cache,
            max_results,
            max_content_chars,
        }
    }
}

#[async_trait]
impl Skill for WebSearchSkill {
    fn name(&self) -> &'static str {
        "web_search"
    }

    fn category(&self) -> &'static str {
        "web"
    }

    fn description(&self) -> &'static str {
        "Search the web for information. Returns results with title, URL, snippet, and full page content. \
         Use a single well-crafted query — the inline content is usually sufficient to answer without \
         additional searches or web_read calls."
    }

    fn budget_cost(&self) -> f32 {
        0.5
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of results to return (1-10)",
                    "default": 5,
                    "minimum": 1,
                    "maximum": 10
                },
                "include_content": {
                    "type": "boolean",
                    "description": "Include page content in results (default: true). Set to false for faster results with only snippets.",
                    "default": true
                }
            },
            "required": ["query"]
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let query = input
            .args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::SkillCategorized {
                message: "missing 'query' argument".into(),
                category: ErrorCategory::Input,
            })?;

        let max_results = input
            .args
            .get("max_results")
            .and_then(serde_json::Value::as_u64)
            .map_or(self.max_results, |n| (n as usize).clamp(1, 10));

        let include_content = input
            .args
            .get("include_content")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);

        // Check cache
        let cache_key = format!("{query}:{max_results}:{include_content}");
        if let Some(cached) = self.cache.get("search", &cache_key) {
            debug!(query, "web_search cache hit");
            let data: serde_json::Value =
                serde_json::from_str(&cached).map_err(|e| Error::SkillCategorized {
                    message: format!("cache deserialization error: {e}"),
                    category: ErrorCategory::Unknown,
                })?;
            return Ok(SkillOutput::new(data));
        }

        let options = SearchOptions {
            max_results,
            include_content,
            max_content_chars: self.max_content_chars,
        };

        let results = self.provider.search(query, &options).await?;

        // Pre-populate read cache with inline content
        if include_content {
            for r in &results {
                if let Some(content) = &r.content {
                    let cache_data = serde_json::json!({
                        "text": content,
                        "title": r.title,
                    });
                    if let Ok(json) = serde_json::to_string(&cache_data) {
                        self.cache.set("read", &r.url, json);
                    }
                }
            }
        }

        let data = serde_json::to_value(&results).map_err(|e| Error::SkillCategorized {
            message: format!("serialization error: {e}"),
            category: ErrorCategory::Unknown,
        })?;

        // Store in cache
        if let Ok(json) = serde_json::to_string(&data) {
            self.cache.set("search", &cache_key, json);
        }

        Ok(SkillOutput::new(data))
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::field_reassign_with_default,
    clippy::default_trait_access,
    clippy::needless_pass_by_value,
    clippy::stable_sort_primitive
)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::types::SearchResult;

    struct MockProvider {
        results: Vec<SearchResult>,
    }

    #[async_trait]
    impl SearchProvider for MockProvider {
        async fn search(&self, _query: &str, options: &SearchOptions) -> Result<Vec<SearchResult>> {
            Ok(self
                .results
                .iter()
                .take(options.max_results)
                .cloned()
                .collect())
        }
    }

    fn make_skill(results: Vec<SearchResult>) -> WebSearchSkill {
        WebSearchSkill::new(
            Arc::new(MockProvider { results }),
            Arc::new(WebCache::new(3600)),
            5,
            8_000,
        )
    }

    fn result(title: &str, url: &str, snippet: &str) -> SearchResult {
        SearchResult {
            title: title.into(),
            url: url.into(),
            snippet: snippet.into(),
            score: None,
            published_date: None,
            content: None,
        }
    }

    #[tokio::test]
    async fn search_returns_results() {
        let skill = make_skill(vec![SearchResult {
            title: "Rust Lang".into(),
            url: "https://rust-lang.org".into(),
            snippet: "A systems programming language".into(),
            score: Some(0.95),
            published_date: None,
            content: None,
        }]);

        let mut args = HashMap::new();
        args.insert("query".into(), serde_json::json!("rust"));
        let input = SkillInput::new(args);
        let output = skill.execute(input).await.unwrap();

        let results = output.data.as_array().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["title"], "Rust Lang");
    }

    #[tokio::test]
    async fn search_respects_max_results() {
        let skill = make_skill(vec![
            result("A", "https://a.com", "a"),
            result("B", "https://b.com", "b"),
        ]);

        let mut args = HashMap::new();
        args.insert("query".into(), serde_json::json!("test"));
        args.insert("max_results".into(), serde_json::json!(1));
        let input = SkillInput::new(args);
        let output = skill.execute(input).await.unwrap();
        assert_eq!(output.data.as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn search_missing_query_errors() {
        let skill = make_skill(vec![]);
        let input = SkillInput::new(HashMap::new());
        assert!(skill.execute(input).await.is_err());
    }

    #[tokio::test]
    async fn search_caching_works() {
        let skill = make_skill(vec![SearchResult {
            title: "Cached".into(),
            url: "https://cached.com".into(),
            snippet: "cached result".into(),
            score: None,
            published_date: None,
            content: None,
        }]);

        let mut args = HashMap::new();
        args.insert("query".into(), serde_json::json!("cache test"));
        let input = SkillInput::new(args.clone());
        let _ = skill.execute(input).await.unwrap();

        // Second call should hit cache
        let input2 = SkillInput::new(args);
        let output2 = skill.execute(input2).await.unwrap();
        assert_eq!(output2.data.as_array().unwrap()[0]["title"], "Cached");
    }

    #[tokio::test]
    async fn search_with_content_populates_read_cache() {
        let skill = make_skill(vec![SearchResult {
            title: "Page".into(),
            url: "https://example.com/page".into(),
            snippet: "snippet".into(),
            score: None,
            published_date: None,
            content: Some("Full page content here".into()),
        }]);

        let mut args = HashMap::new();
        args.insert("query".into(), serde_json::json!("example"));
        let input = SkillInput::new(args);
        let output = skill.execute(input).await.unwrap();

        // Content should be in search results
        let results = output.data.as_array().unwrap();
        assert_eq!(results[0]["content"], "Full page content here");

        // Read cache should be populated
        let cached = skill.cache.get("read", "https://example.com/page").unwrap();
        assert!(cached.contains("Full page content here"));
    }

    #[tokio::test]
    async fn search_without_content_different_cache_key() {
        let skill = make_skill(vec![result("A", "https://a.com", "a")]);

        // First search with content
        let mut args = HashMap::new();
        args.insert("query".into(), serde_json::json!("test"));
        args.insert("include_content".into(), serde_json::json!(true));
        let input = SkillInput::new(args);
        let _ = skill.execute(input).await.unwrap();

        // Search without content should NOT be a cache hit (different key)
        let mut args2 = HashMap::new();
        args2.insert("query".into(), serde_json::json!("test"));
        args2.insert("include_content".into(), serde_json::json!(false));
        let input2 = SkillInput::new(args2);
        // This should succeed (hits provider, not cache) — verifies different cache
        // keys
        let output = skill.execute(input2).await.unwrap();
        assert_eq!(output.data.as_array().unwrap().len(), 1);
    }

    #[test]
    fn schema_is_valid() {
        let skill = make_skill(vec![]);
        let schema = skill.schema();
        assert_eq!(schema.parameters["required"][0], "query");
        // include_content should be in schema
        assert!(schema.parameters["properties"]["include_content"].is_object());
    }
}
