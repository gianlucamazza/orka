use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A chunk of text with metadata, ready for embedding and storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    pub id: String,
    pub content: String,
    pub document_id: Option<String>,
    pub chunk_index: usize,
    pub metadata: HashMap<String, String>,
}

/// A document that has been ingested.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub id: String,
    pub source: String,
    pub format: String,
    pub chunk_count: usize,
    pub collection: String,
    pub ingested_at: String,
    pub metadata: HashMap<String, String>,
}

/// A search result from the vector store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub content: String,
    pub score: f32,
    pub document_id: Option<String>,
    pub metadata: HashMap<String, String>,
}
