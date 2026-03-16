use std::sync::Arc;

use async_trait::async_trait;
use orka_core::traits::Skill;
use orka_core::{Error, Result, SkillInput, SkillOutput, SkillSchema};
use tracing::debug;

use crate::cache::WebCache;
use crate::provider::SearchProvider;

/// Skill that searches the web using a configured provider.
pub struct WebSearchSkill {
    provider: Arc<dyn SearchProvider>,
    cache: Arc<WebCache>,
    max_results: usize,
}

impl WebSearchSkill {
    pub fn new(
        provider: Arc<dyn SearchProvider>,
        cache: Arc<WebCache>,
        max_results: usize,
    ) -> Self {
        Self {
            provider,
            cache,
            max_results,
        }
    }
}

#[async_trait]
impl Skill for WebSearchSkill {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the web for information. Returns a list of results with title, URL, and snippet."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema {
            parameters: serde_json::json!({
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
                    }
                },
                "required": ["query"]
            }),
        }
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let query = input
            .args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Skill("missing 'query' argument".into()))?;

        let max_results = input
            .args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).min(10).max(1))
            .unwrap_or(self.max_results);

        // Check cache
        let cache_key = format!("{query}:{max_results}");
        if let Some(cached) = self.cache.get("search", &cache_key) {
            debug!(query, "web_search cache hit");
            let data: serde_json::Value = serde_json::from_str(&cached)
                .map_err(|e| Error::Skill(format!("cache deserialization error: {e}")))?;
            return Ok(SkillOutput { data });
        }

        let results = self.provider.search(query, max_results).await?;

        let data = serde_json::to_value(&results)
            .map_err(|e| Error::Skill(format!("serialization error: {e}")))?;

        // Store in cache
        if let Ok(json) = serde_json::to_string(&data) {
            self.cache.set("search", &cache_key, json);
        }

        Ok(SkillOutput { data })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SearchResult;
    use std::collections::HashMap;

    struct MockProvider {
        results: Vec<SearchResult>,
    }

    #[async_trait]
    impl SearchProvider for MockProvider {
        async fn search(&self, _query: &str, max_results: usize) -> Result<Vec<SearchResult>> {
            Ok(self.results.iter().take(max_results).cloned().collect())
        }
    }

    fn make_skill(results: Vec<SearchResult>) -> WebSearchSkill {
        WebSearchSkill::new(
            Arc::new(MockProvider { results }),
            Arc::new(WebCache::new(3600)),
            5,
        )
    }

    #[tokio::test]
    async fn search_returns_results() {
        let skill = make_skill(vec![SearchResult {
            title: "Rust Lang".into(),
            url: "https://rust-lang.org".into(),
            snippet: "A systems programming language".into(),
            score: Some(0.95),
            published_date: None,
        }]);

        let mut args = HashMap::new();
        args.insert("query".into(), serde_json::json!("rust"));
        let input = SkillInput {
            args,
            context: None,
        };
        let output = skill.execute(input).await.unwrap();

        let results = output.data.as_array().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["title"], "Rust Lang");
    }

    #[tokio::test]
    async fn search_respects_max_results() {
        let skill = make_skill(vec![
            SearchResult {
                title: "A".into(),
                url: "https://a.com".into(),
                snippet: "a".into(),
                score: None,
                published_date: None,
            },
            SearchResult {
                title: "B".into(),
                url: "https://b.com".into(),
                snippet: "b".into(),
                score: None,
                published_date: None,
            },
        ]);

        let mut args = HashMap::new();
        args.insert("query".into(), serde_json::json!("test"));
        args.insert("max_results".into(), serde_json::json!(1));
        let input = SkillInput {
            args,
            context: None,
        };
        let output = skill.execute(input).await.unwrap();
        assert_eq!(output.data.as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn search_missing_query_errors() {
        let skill = make_skill(vec![]);
        let input = SkillInput {
            args: HashMap::new(),
            context: None,
        };
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
        }]);

        let mut args = HashMap::new();
        args.insert("query".into(), serde_json::json!("cache test"));
        let input = SkillInput {
            args: args.clone(),
            context: None,
        };
        let _ = skill.execute(input).await.unwrap();

        // Second call should hit cache
        let input2 = SkillInput {
            args,
            context: None,
        };
        let output2 = skill.execute(input2).await.unwrap();
        assert_eq!(output2.data.as_array().unwrap()[0]["title"], "Cached");
    }

    #[test]
    fn schema_is_valid() {
        let skill = make_skill(vec![]);
        let schema = skill.schema();
        assert_eq!(schema.parameters["required"][0], "query");
    }
}
