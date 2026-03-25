/// In-memory vector store backend for testing.
pub mod memory;
/// Qdrant vector database backend implementation.
pub mod qdrant;

use std::collections::HashMap;

use async_trait::async_trait;
use orka_core::Result;

use crate::types::SearchResult;

/// Trait for vector storage backends.
#[async_trait]
pub trait VectorStore: Send + Sync + 'static {
    /// Ensure a collection exists with the given dimensionality.
    async fn ensure_collection(&self, name: &str, dimensions: usize) -> Result<()>;

    /// Upsert vectors with their payloads.
    async fn upsert(
        &self,
        collection: &str,
        ids: &[String],
        vectors: &[Vec<f32>],
        payloads: &[HashMap<String, String>],
    ) -> Result<()>;

    /// Search for similar vectors.
    async fn search(
        &self,
        collection: &str,
        vector: &[f32],
        limit: usize,
        score_threshold: Option<f32>,
        filter: Option<HashMap<String, String>>,
    ) -> Result<Vec<SearchResult>>;

    /// List all documents in a collection (by unique `document_id`).
    async fn list_documents(
        &self,
        collection: &str,
        limit: usize,
        filter: Option<HashMap<String, String>>,
    ) -> Result<Vec<HashMap<String, String>>>;
}
