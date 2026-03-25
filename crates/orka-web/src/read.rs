use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use orka_core::{
    Error, ErrorCategory, Result, SkillInput, SkillOutput, SkillSchema, traits::Skill,
};
use tracing::debug;

use crate::{cache::WebCache, extract};

/// Skill that fetches and reads a web page, extracting readable text.
pub(crate) struct WebReadSkill {
    client: reqwest::Client,
    cache: Arc<WebCache>,
    max_chars: usize,
}

impl WebReadSkill {
    pub(crate) fn new(
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

    fn category(&self) -> &'static str {
        "web"
    }

    fn description(&self) -> &str {
        "Fetch and read a web page. Returns extracted readable text from the URL. Use start_index to paginate through long pages."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
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
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let url = input
            .args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::SkillCategorized {
                message: "missing 'url' argument".into(),
                category: ErrorCategory::Input,
            })?;

        let start_index = input
            .args
            .get("start_index")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;

        // Validate URL
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(Error::SkillCategorized {
                message: "url must start with http:// or https://".into(),
                category: ErrorCategory::Input,
            });
        }

        // Check cache for full page content
        let (full_text, title) = if let Some(cached) = self.cache.get("read", url) {
            debug!(url, "web_read cache hit");
            let cached_data: serde_json::Value =
                serde_json::from_str(&cached).map_err(|e| Error::SkillCategorized {
                    message: format!("cache parse error: {e}"),
                    category: ErrorCategory::Unknown,
                })?;
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
                .map_err(|e| Error::SkillCategorized {
                    message: format!("fetch failed: {e}"),
                    category: ErrorCategory::Transient,
                })?;

            if !resp.status().is_success() {
                let status = resp.status();
                return Err(Error::SkillCategorized {
                    message: format!("HTTP {status} for {url}"),
                    category: ErrorCategory::Transient,
                });
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
                return Err(Error::SkillCategorized {
                    message: format!("unsupported content type: {content_type}"),
                    category: ErrorCategory::Input,
                });
            }

            let body = resp.text().await.map_err(|e| Error::SkillCategorized {
                message: format!("failed to read body: {e}"),
                category: ErrorCategory::Transient,
            })?;

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

        Ok(SkillOutput::new(serde_json::json!({
            "url": url,
            "title": title,
            "content": text,
            "truncated": truncated,
            "total_length": content_length,
            "start_index": start_index,
        })))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

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
        let input = SkillInput::new(HashMap::new());
        assert!(skill.execute(input).await.is_err());
    }

    #[tokio::test]
    async fn rejects_non_http_url() {
        let skill = make_skill();
        let mut args = HashMap::new();
        args.insert("url".into(), serde_json::json!("ftp://example.com"));
        let input = SkillInput::new(args);
        let err = skill.execute(input).await.unwrap_err();
        assert!(err.to_string().contains("http://"));
    }
}
