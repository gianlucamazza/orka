use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use orka_core::traits::Skill;
use orka_core::{Error, Result, SkillInput, SkillOutput, SkillSchema};
use tracing::debug;

use crate::cache::WebCache;
use crate::extract;

/// Skill that fetches and reads a web page, extracting readable text.
pub struct WebReadSkill {
    client: reqwest::Client,
    cache: Arc<WebCache>,
    max_chars: usize,
}

impl WebReadSkill {
    pub fn new(
        cache: Arc<WebCache>,
        max_chars: usize,
        timeout_secs: u64,
        user_agent: &str,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .user_agent(user_agent)
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .unwrap_or_default();

        Self {
            client,
            cache,
            max_chars,
        }
    }
}

#[async_trait]
impl Skill for WebReadSkill {
    fn name(&self) -> &str {
        "web_read"
    }

    fn description(&self) -> &str {
        "Fetch and read a web page. Returns extracted readable text from the URL. Use start_index to paginate through long pages."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema {
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The URL to fetch and read"
                    },
                    "start_index": {
                        "type": "integer",
                        "description": "Character offset for pagination (default: 0)",
                        "default": 0,
                        "minimum": 0
                    }
                },
                "required": ["url"]
            }),
        }
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let url = input
            .args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Skill("missing 'url' argument".into()))?;

        let start_index = input
            .args
            .get("start_index")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;

        // Validate URL
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(Error::Skill(
                "url must start with http:// or https://".into(),
            ));
        }

        // Check cache for full page content
        let (full_text, title) = if let Some(cached) = self.cache.get("read", url) {
            debug!(url, "web_read cache hit");
            let cached_data: serde_json::Value = serde_json::from_str(&cached)
                .map_err(|e| Error::Skill(format!("cache parse error: {e}")))?;
            (
                cached_data["text"].as_str().unwrap_or("").to_string(),
                cached_data["title"].as_str().map(String::from),
            )
        } else {
            let resp = self
                .client
                .get(url)
                .send()
                .await
                .map_err(|e| Error::Skill(format!("fetch failed: {e}")))?;

            if !resp.status().is_success() {
                let status = resp.status();
                return Err(Error::Skill(format!("HTTP {status} for {url}")));
            }

            let content_type = resp
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();

            if !content_type.contains("text/html")
                && !content_type.contains("text/plain")
                && !content_type.contains("application/json")
            {
                return Err(Error::Skill(format!(
                    "unsupported content type: {content_type}"
                )));
            }

            let body = resp
                .text()
                .await
                .map_err(|e| Error::Skill(format!("failed to read body: {e}")))?;

            let (text, title) = if content_type.contains("text/html") {
                (extract::extract_text(&body), extract::extract_title(&body))
            } else {
                (body, None)
            };

            // Cache the full content
            let cache_data = serde_json::json!({
                "text": text,
                "title": title,
            });
            if let Ok(json) = serde_json::to_string(&cache_data) {
                self.cache.set("read", url, json);
            }

            (text, title)
        };

        let content_length = full_text.len();

        // Apply start_index pagination
        let remaining = if start_index < content_length {
            &full_text[full_text.floor_char_boundary(start_index)..]
        } else {
            ""
        };

        let (text, truncated) = extract::truncate(remaining, self.max_chars);

        Ok(SkillOutput {
            data: serde_json::json!({
                "url": url,
                "title": title,
                "content": text,
                "truncated": truncated,
                "total_length": content_length,
                "start_index": start_index,
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_skill() -> WebReadSkill {
        WebReadSkill::new(
            Arc::new(crate::cache::WebCache::new(3600)),
            20_000,
            15,
            "test",
        )
    }

    #[test]
    fn schema_is_valid() {
        let skill = make_skill();
        let schema = skill.schema();
        assert_eq!(schema.parameters["required"][0], "url");
    }

    #[tokio::test]
    async fn rejects_missing_url() {
        let skill = make_skill();
        let input = SkillInput {
            args: HashMap::new(),
            context: None,
        };
        assert!(skill.execute(input).await.is_err());
    }

    #[tokio::test]
    async fn rejects_non_http_url() {
        let skill = make_skill();
        let mut args = HashMap::new();
        args.insert("url".into(), serde_json::json!("ftp://example.com"));
        let input = SkillInput {
            args,
            context: None,
        };
        let err = skill.execute(input).await.unwrap_err();
        assert!(err.to_string().contains("http://"));
    }
}
