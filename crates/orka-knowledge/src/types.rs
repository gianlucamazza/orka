use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// A chunk of text with metadata, ready for embedding and storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    /// Unique identifier for this chunk.
    pub id: String,
    /// The text content of this chunk.
    pub content: String,
    /// ID of the parent document, if any.
    pub document_id: Option<String>,
    /// Zero-based position of this chunk within the document.
    pub chunk_index: usize,
    /// Arbitrary key-value metadata attached to this chunk.
    pub metadata: HashMap<String, String>,
}

/// A document that has been ingested.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    /// Unique identifier for this document.
    pub id: String,
    /// Original file path or URL the document was loaded from.
    pub source: String,
    /// Detected format (e.g. `"pdf"`, `"html"`, `"md"`, `"txt"`).
    pub format: String,
    /// Number of chunks the document was split into.
    pub chunk_count: usize,
    /// Vector store collection this document belongs to.
    pub collection: String,
    /// RFC 3339 timestamp of when the document was ingested.
    pub ingested_at: String,
    /// Arbitrary key-value metadata attached to this document.
    pub metadata: HashMap<String, String>,
}

/// A search result from the vector store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// The text content of the matching chunk.
    pub content: String,
    /// Cosine similarity score in the range \[0, 1\].
    pub score: f32,
    /// ID of the parent document, if available.
    pub document_id: Option<String>,
    /// Metadata payload returned alongside the vector.
    pub metadata: HashMap<String, String>,
}
