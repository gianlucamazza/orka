//! Knowledge base and RAG configuration.

use serde::Deserialize;

use crate::config::{
    defaults,
    primitives::{EmbeddingProvider, VectorStoreBackend},
};

/// Knowledge base and RAG configuration.
#[derive(Debug, Clone, Default, Deserialize)]
#[non_exhaustive]
pub struct KnowledgeConfig {
    /// Enable knowledge base.
    #[serde(default)]
    pub enabled: bool,
    /// Vector store configuration.
    #[serde(default)]
    pub vector_store: VectorStoreConfig,
    /// Embeddings configuration.
    #[serde(default)]
    pub embeddings: EmbeddingsConfig,
    /// Text chunking configuration.
    #[serde(default)]
    pub chunking: ChunkingConfig,
    /// Retrieval configuration.
    #[serde(default)]
    pub retrieval: RetrievalConfig,
}

impl KnowledgeConfig {
    /// Validate the knowledge configuration.
    pub fn validate(&self) -> crate::Result<()> {
        if self.chunking.chunk_overlap >= self.chunking.chunk_size {
            return Err(crate::Error::Config(format!(
                "knowledge.chunking.chunk_overlap ({}) must be less than chunk_size ({})",
                self.chunking.chunk_overlap, self.chunking.chunk_size
            )));
        }
        Ok(())
    }
}

/// Vector store configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct VectorStoreConfig {
    /// Backend type.
    #[serde(default)]
    pub backend: VectorStoreBackend,
    /// Qdrant URL.
    #[serde(default)]
    pub url: Option<String>,
    /// Collection name.
    #[serde(default = "defaults::default_collection_name")]
    pub collection_name: String,
    /// Vector dimension.
    #[serde(default = "defaults::default_vector_dimension")]
    pub dimension: usize,
    /// Distance metric: "cosine", "euclidean", "dot".
    #[serde(default = "default_distance_metric")]
    pub distance_metric: String,
}

impl Default for VectorStoreConfig {
    fn default() -> Self {
        Self {
            backend: VectorStoreBackend::default(),
            url: None,
            collection_name: defaults::default_collection_name(),
            dimension: defaults::default_vector_dimension(),
            distance_metric: default_distance_metric(),
        }
    }
}

fn default_distance_metric() -> String {
    "cosine".to_string()
}

/// Embeddings configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct EmbeddingsConfig {
    /// Provider type.
    #[serde(default)]
    pub provider: EmbeddingProvider,
    /// Model name (for local) or API model.
    #[serde(default = "default_embedding_model")]
    pub model: String,
    /// API key (if using cloud provider).
    pub api_key: Option<String>,
    /// Batch size for embedding generation.
    #[serde(default = "default_embedding_batch_size")]
    pub batch_size: usize,
}

impl Default for EmbeddingsConfig {
    fn default() -> Self {
        Self {
            provider: EmbeddingProvider::default(),
            model: default_embedding_model(),
            api_key: None,
            batch_size: default_embedding_batch_size(),
        }
    }
}

fn default_embedding_model() -> String {
    "BAAI/bge-small-en-v1.5".to_string()
}

const fn default_embedding_batch_size() -> usize {
    32
}

/// Text chunking configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct ChunkingConfig {
    /// Chunk size in tokens.
    #[serde(default = "defaults::default_chunk_size")]
    pub chunk_size: usize,
    /// Chunk overlap in tokens.
    #[serde(default = "defaults::default_chunk_overlap")]
    pub chunk_overlap: usize,
    /// Split on specific separators.
    #[serde(default)]
    pub separators: Vec<String>,
}

impl Default for ChunkingConfig {
    fn default() -> Self {
        Self {
            chunk_size: defaults::default_chunk_size(),
            chunk_overlap: defaults::default_chunk_overlap(),
            separators: Vec::new(),
        }
    }
}

/// Retrieval configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct RetrievalConfig {
    /// Number of documents to retrieve.
    #[serde(default = "defaults::default_top_k")]
    pub top_k: usize,
    /// Minimum similarity score threshold.
    #[serde(default = "defaults::default_score_threshold")]
    pub score_threshold: f32,
    /// Rerank results.
    #[serde(default)]
    pub rerank: bool,
}

impl Default for RetrievalConfig {
    fn default() -> Self {
        Self {
            top_k: defaults::default_top_k(),
            score_threshold: defaults::default_score_threshold(),
            rerank: false,
        }
    }
}
