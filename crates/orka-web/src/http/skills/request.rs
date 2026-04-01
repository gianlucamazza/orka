use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use orka_core::{ErrorCategory, Result, SkillInput, SkillOutput, SkillSchema, traits::Skill};
use tracing::debug;

use super::super::guard::SsrfGuard;

pub(crate) struct HttpRequestSkill {
    client: reqwest::Client,
    guard: Arc<SsrfGuard>,
    max_response_bytes: usize,
    default_timeout_secs: u64,
}

impl HttpRequestSkill {
    pub(crate) fn new(
        guard: Arc<SsrfGuard>,
        max_response_bytes: usize,
        default_timeout_secs: u64,
        user_agent: &str,
    ) -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(user_agent)
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .map_err(|e| orka_core::Error::http_client(e, "failed to build HTTP client"))?;

        Ok(Self {
            client,
            guard,
            max_response_bytes,
            default_timeout_secs,
        })
    }
}

#[allow(clippy::too_many_lines)]
#[async_trait]
impl Skill for HttpRequestSkill {
    fn name(&self) -> &'static str {
        "http_request"
    }

    fn category(&self) -> &'static str {
        "http"
    }

    fn description(&self) -> &'static str {
        "Make HTTP requests to external APIs and services."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to request"
                },
                "method": {
                    "type": "string",
                    "enum": ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"],
                    "default": "GET",
                    "description": "HTTP method"
                },
                "headers": {
                    "type": "object",
                    "additionalProperties": { "type": "string" },
                    "description": "Request headers"
                },
                "body": {
                    "description": "Request body (string or JSON object)",
                    "oneOf": [
                        { "type": "string" },
                        { "type": "object" }
                    ]
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Request timeout in seconds"
                },
                "auth_bearer_secret": {
                    "type": "string",
                    "description": "Name of secret containing Bearer token"
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
            .ok_or_else(|| orka_core::Error::SkillCategorized {
                message: "url is required".into(),
                category: ErrorCategory::Input,
            })?;

        // SSRF check
        self.guard
            .check(url)
            .map_err(|msg| orka_core::Error::SkillCategorized {
                message: msg,
                category: ErrorCategory::Input,
            })?;

        let method = input
            .args
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("GET");

        let timeout_secs = input
            .args
            .get("timeout_secs")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(self.default_timeout_secs);

        debug!(url, method, "http_request executing");

        let mut req = match method.to_uppercase().as_str() {
            "GET" => self.client.get(url),
            "POST" => self.client.post(url),
            "PUT" => self.client.put(url),
            "PATCH" => self.client.patch(url),
            "DELETE" => self.client.delete(url),
            "HEAD" => self.client.head(url),
            other => {
                return Err(orka_core::Error::SkillCategorized {
                    message: format!("unsupported method: {other}"),
                    category: ErrorCategory::Input,
                });
            }
        };

        req = req.timeout(Duration::from_secs(timeout_secs));

        // Add custom headers
        if let Some(headers) = input.args.get("headers").and_then(|v| v.as_object()) {
            for (key, value) in headers {
                if let Some(val) = value.as_str() {
                    req = req.header(key.as_str(), val);
                }
            }
        }

        // Bearer auth from secret
        if let Some(secret_name) = input
            .args
            .get("auth_bearer_secret")
            .and_then(|v| v.as_str())
            && let Some(ref ctx) = input.context
        {
            match ctx.secrets.get_secret(secret_name).await {
                Ok(secret) => {
                    let token = secret.expose_str().unwrap_or("").to_string();
                    if !token.is_empty() {
                        req = req.bearer_auth(token);
                    }
                }
                Err(e) => {
                    return Err(orka_core::Error::SkillCategorized {
                        message: format!("failed to read bearer secret '{secret_name}': {e}"),
                        category: ErrorCategory::Input,
                    });
                }
            }
        }

        // Add body
        if let Some(body) = input.args.get("body") {
            if let Some(s) = body.as_str() {
                req = req.body(s.to_string());
            } else {
                req = req.json(body);
            }
        }

        let response = req
            .send()
            .await
            .map_err(|e| orka_core::Error::http_client(e, "request failed"))?;

        let status = response.status().as_u16();
        let headers: serde_json::Map<String, serde_json::Value> = response
            .headers()
            .iter()
            .map(|(k, v)| {
                (
                    k.as_str().to_string(),
                    serde_json::Value::String(v.to_str().unwrap_or("").to_string()),
                )
            })
            .collect();

        let body_bytes = response
            .bytes()
            .await
            .map_err(|e| orka_core::Error::http_client(e, "failed to read response body"))?;

        let body = if body_bytes.len() > self.max_response_bytes {
            let truncated = String::from_utf8_lossy(&body_bytes[..self.max_response_bytes]);
            format!(
                "{truncated}... [truncated at {} bytes]",
                self.max_response_bytes
            )
        } else {
            String::from_utf8_lossy(&body_bytes).to_string()
        };

        // Try to parse as JSON
        let body_value = serde_json::from_str::<serde_json::Value>(&body)
            .unwrap_or(serde_json::Value::String(body));

        Ok(SkillOutput::new(serde_json::json!({
            "status": status,
            "headers": headers,
            "body": body_value,
        })))
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
    use super::*;

    #[test]
    fn schema_is_valid() {
        let guard = Arc::new(SsrfGuard::new(vec![]));
        let skill = HttpRequestSkill::new(guard, 1024, 30, "test").unwrap();
        let schema = skill.schema();
        assert!(schema.parameters.get("properties").is_some());
    }
}
