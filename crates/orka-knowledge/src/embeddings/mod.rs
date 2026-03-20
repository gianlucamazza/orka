/// Local ONNX-based embedding provider using fastembed.
pub mod local;
/// In-memory embedding provider for testing.
pub mod memory;
/// OpenAI-compatible embedding provider via REST API.
pub mod openai;

use async_trait::async_trait;
use orka_core::Result;

/// Trait for generating embeddings from text.
#[async_trait]
pub trait EmbeddingProvider: Send + Sync + 'static {
    /// Generate embeddings for a batch of texts.
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;

    /// Return the dimensionality of the embeddings.
    fn dimensions(&self) -> usize;
}
