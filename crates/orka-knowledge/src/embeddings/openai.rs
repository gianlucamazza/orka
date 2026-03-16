use async_trait::async_trait;
use orka_core::Result;
use serde::{Deserialize, Serialize};

use super::EmbeddingProvider;

/// OpenAI-compatible embedding provider.
pub struct OpenAiEmbeddingProvider {
    client: reqwest::Client,
    api_key: String,
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
    pub fn new(api_key: String, model: String, dimensions: u32) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
            model,
            dims: dimensions as usize,
            base_url: "https://api.openai.com/v1".into(),
        }
    }

    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = url;
        self
    }
}

#[async_trait]
impl EmbeddingProvider for OpenAiEmbeddingProvider {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let request = EmbeddingRequest {
            model: self.model.clone(),
            input: texts.to_vec(),
        };

        let response = self
            .client
            .post(format!("{}/embeddings", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                orka_core::Error::Knowledge(format!("OpenAI embedding request failed: {e}"))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(orka_core::Error::Knowledge(format!(
                "OpenAI embedding API error {status}: {body}"
            )));
        }

        let resp: EmbeddingResponse = response.json().await.map_err(|e| {
            orka_core::Error::Knowledge(format!("failed to parse embedding response: {e}"))
        })?;

        Ok(resp.data.into_iter().map(|d| d.embedding).collect())
    }

    fn dimensions(&self) -> usize {
        self.dims
    }
}
