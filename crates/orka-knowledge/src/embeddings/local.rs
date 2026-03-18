use async_trait::async_trait;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use orka_core::Result;
use std::sync::Arc;
use tokio::sync::Mutex;

use super::EmbeddingProvider;

/// Local embedding provider using fastembed (ONNX runtime).
pub struct LocalEmbeddingProvider {
    model: Arc<Mutex<TextEmbedding>>,
    dims: usize,
}

impl LocalEmbeddingProvider {
    /// Load the named ONNX embedding model and initialise the provider.
    pub fn new(model_name: &str, dimensions: u32) -> Result<Self> {
        let embedding_model = match model_name {
            "BAAI/bge-small-en-v1.5" => EmbeddingModel::BGESmallENV15,
            "BAAI/bge-base-en-v1.5" => EmbeddingModel::BGEBaseENV15,
            "BAAI/bge-large-en-v1.5" => EmbeddingModel::BGELargeENV15,
            _ => EmbeddingModel::BGESmallENV15,
        };

        let model = TextEmbedding::try_new(
            InitOptions::new(embedding_model).with_show_download_progress(true),
        )
        .map_err(|e| orka_core::Error::Knowledge(format!("failed to init embedding model: {e}")))?;

        Ok(Self {
            model: Arc::new(Mutex::new(model)),
            dims: dimensions as usize,
        })
    }
}

#[async_trait]
impl EmbeddingProvider for LocalEmbeddingProvider {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let texts = texts.to_vec();
        let model = self.model.clone();

        tokio::task::spawn_blocking(move || {
            let model = model.blocking_lock();
            model
                .embed(texts, None)
                .map_err(|e| orka_core::Error::Knowledge(format!("embedding failed: {e}")))
        })
        .await
        .map_err(|e| orka_core::Error::Knowledge(format!("embedding task failed: {e}")))?
    }

    fn dimensions(&self) -> usize {
        self.dims
    }
}
