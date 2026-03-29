use async_trait::async_trait;
use orka_core::{Result, SecretStr};
use serde::{Deserialize, Serialize};
use tracing::warn;

use super::EmbeddingProvider;

/// Maximum number of texts per embedding API request.
/// `OpenAI` accepts up to 2048 inputs per request; smaller batches also reduce
/// the risk of hitting per-request token limits.
const MAX_BATCH_SIZE: usize = 2048;

/// OpenAI-compatible embedding provider.
pub struct OpenAiEmbeddingProvider {
    client: reqwest::Client,
    api_key: SecretStr,
    model: String,
    dims: usize,
    base_url: String,
}

#[derive(Serialize)]
struct EmbeddingRequest {
    model: String,
    input: Vec<String>,
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

impl OpenAiEmbeddingProvider {
    /// Create a provider targeting the standard `OpenAI` embeddings endpoint.
    pub fn new(api_key: SecretStr, model: String, dimensions: u32) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
            model,
            dims: dimensions as usize,
            base_url: "https://api.openai.com/v1".into(),
        }
    }

    /// Override the API base URL (useful for OpenAI-compatible proxies).
    #[must_use]
    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = url;
        self
    }
}

#[async_trait]
impl EmbeddingProvider for OpenAiEmbeddingProvider {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let url = format!("{}/embeddings", self.base_url);
        let mut all_embeddings: Vec<Vec<f32>> = Vec::with_capacity(texts.len());

        // Process in batches to stay within OpenAI's per-request input limit.
        for chunk in texts.chunks(MAX_BATCH_SIZE) {
            let request = EmbeddingRequest {
                model: self.model.clone(),
                input: chunk.to_vec(),
            };

            // Retry up to 3 times on 429 (rate limit) or 5xx.
            let mut last_err: Option<orka_core::Error> = None;
            let chunk_embeddings: Vec<Vec<f32>> = 'retry: {
                for attempt in 0..3u32 {
                    if attempt > 0 {
                        let ms = 500u64.saturating_mul(1u64 << (attempt - 1));
                        tokio::time::sleep(std::time::Duration::from_millis(ms.min(30_000))).await;
                    }
                    let result = self
                        .client
                        .post(&url)
                        .bearer_auth(self.api_key.expose())
                        .json(&request)
                        .send()
                        .await;
                    match result {
                        Err(e) if e.is_timeout() || e.is_connect() => {
                            warn!(%e, attempt, "OpenAI embedding transient error, retrying");
                            last_err = Some(orka_core::Error::Knowledge(format!(
                                "OpenAI embedding request failed: {e}"
                            )));
                        }
                        Err(e) => {
                            return Err(orka_core::Error::Knowledge(format!(
                                "OpenAI embedding request failed: {e}"
                            )));
                        }
                        Ok(r) if r.status() == 429 || r.status().is_server_error() => {
                            let status = r.status();
                            let body = r.text().await.unwrap_or_default();
                            warn!(
                                %status, attempt,
                                "OpenAI embedding API rate limited / server error, retrying"
                            );
                            last_err = Some(orka_core::Error::Knowledge(format!(
                                "OpenAI embedding API error {status}: {body}"
                            )));
                        }
                        Ok(r) if !r.status().is_success() => {
                            let status = r.status();
                            let body = r.text().await.unwrap_or_default();
                            return Err(orka_core::Error::Knowledge(format!(
                                "OpenAI embedding API error {status}: {body}"
                            )));
                        }
                        Ok(r) => {
                            let resp: EmbeddingResponse = r.json().await.map_err(|e| {
                                orka_core::Error::Knowledge(format!(
                                    "failed to parse embedding response: {e}"
                                ))
                            })?;
                            break 'retry resp.data.into_iter().map(|d| d.embedding).collect();
                        }
                    }
                }
                return Err(last_err.unwrap_or_else(|| {
                    orka_core::Error::Knowledge("OpenAI embedding failed after max retries".into())
                }));
            };

            all_embeddings.extend(chunk_embeddings);
        }

        Ok(all_embeddings)
    }

    fn dimensions(&self) -> usize {
        self.dims
    }
}
