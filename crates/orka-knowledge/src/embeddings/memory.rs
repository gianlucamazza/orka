use async_trait::async_trait;
use orka_core::Result;

use super::EmbeddingProvider;

/// In-memory [`EmbeddingProvider`] for tests. Returns deterministic vectors
/// based on a simple hash of the input text.
pub struct InMemoryEmbeddingProvider {
    dimensions: usize,
}

impl InMemoryEmbeddingProvider {
    /// Create a provider with the given output dimensionality.
    pub fn new(dimensions: usize) -> Self {
        Self { dimensions }
    }
}

impl Default for InMemoryEmbeddingProvider {
    fn default() -> Self {
        Self::new(64)
    }
}

/// Simple deterministic hash-based embedding for testing.
fn text_to_vector(text: &str, dimensions: usize) -> Vec<f32> {
    let mut vec = vec![0.0f32; dimensions];
    for (i, byte) in text.bytes().enumerate() {
        vec[i % dimensions] += byte as f32;
    }
    // Normalize
    let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for v in &mut vec {
            *v /= norm;
        }
    }
    vec
}

#[async_trait]
impl EmbeddingProvider for InMemoryEmbeddingProvider {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(texts
            .iter()
            .map(|t| text_to_vector(t, self.dimensions))
            .collect())
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn embed_returns_correct_dimensions() {
        let provider = InMemoryEmbeddingProvider::new(32);
        let embeddings = provider.embed(&["hello world".into()]).await.unwrap();
        assert_eq!(embeddings.len(), 1);
        assert_eq!(embeddings[0].len(), 32);
    }

    #[tokio::test]
    async fn embed_is_deterministic() {
        let provider = InMemoryEmbeddingProvider::new(16);
        let a = provider.embed(&["test".into()]).await.unwrap();
        let b = provider.embed(&["test".into()]).await.unwrap();
        assert_eq!(a, b);
    }

    #[tokio::test]
    async fn embed_batch() {
        let provider = InMemoryEmbeddingProvider::new(8);
        let embeddings = provider
            .embed(&["one".into(), "two".into(), "three".into()])
            .await
            .unwrap();
        assert_eq!(embeddings.len(), 3);
    }

    #[tokio::test]
    async fn embed_vectors_are_normalized() {
        let provider = InMemoryEmbeddingProvider::new(16);
        let embeddings = provider.embed(&["hello world".into()]).await.unwrap();
        let norm: f32 = embeddings[0].iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.01);
    }

    #[tokio::test]
    async fn similar_texts_produce_similar_vectors() {
        let provider = InMemoryEmbeddingProvider::new(64);
        let embeddings = provider
            .embed(&[
                "hello world".into(),
                "hello worlds".into(),
                "completely different xyz".into(),
            ])
            .await
            .unwrap();

        // Cosine similarity between similar texts should be higher
        let sim_similar: f32 = embeddings[0]
            .iter()
            .zip(&embeddings[1])
            .map(|(a, b)| a * b)
            .sum();
        let sim_different: f32 = embeddings[0]
            .iter()
            .zip(&embeddings[2])
            .map(|(a, b)| a * b)
            .sum();
        assert!(sim_similar > sim_different);
    }
}
