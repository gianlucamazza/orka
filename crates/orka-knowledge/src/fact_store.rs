use std::{collections::HashMap, sync::Arc};

use chrono::Utc;
use orka_core::{MemoryScope, Result};

use crate::{
    embeddings::EmbeddingProvider,
    types::{SearchResult, StoredRecord},
    vector_store::VectorStore,
};

/// A stored semantic fact.
#[derive(Debug, Clone)]
pub struct FactRecord {
    /// Stable fact identifier.
    pub id: String,
    /// Stored fact content.
    pub content: String,
    /// Retention scope.
    pub scope: MemoryScope,
    /// Origin marker.
    pub source: String,
    /// Arbitrary metadata stored with the fact.
    pub metadata: HashMap<String, String>,
}

/// Semantic fact storage with explicit memory semantics.
pub struct FactStore {
    embeddings: Arc<dyn EmbeddingProvider>,
    vector_store: Arc<dyn VectorStore>,
    collection: String,
    initialized: tokio::sync::OnceCell<()>,
}

impl FactStore {
    /// Create a new fact store backed by the given embeddings and vector store.
    pub fn new(
        embeddings: Arc<dyn EmbeddingProvider>,
        vector_store: Arc<dyn VectorStore>,
        collection: String,
    ) -> Self {
        Self {
            embeddings,
            vector_store,
            collection,
            initialized: tokio::sync::OnceCell::new(),
        }
    }

    async fn ensure_init(&self) -> Result<()> {
        self.initialized
            .get_or_try_init(|| async {
                self.vector_store
                    .ensure_collection(&self.collection, self.embeddings.dimensions())
                    .await
            })
            .await?;
        Ok(())
    }

    /// Store a semantic fact and return its stable identifier.
    pub async fn store_fact(
        &self,
        content: &str,
        scope: MemoryScope,
        source: &str,
        mut metadata: HashMap<String, String>,
    ) -> Result<String> {
        self.ensure_init().await?;

        let embeddings = self.embeddings.embed(&[content.to_string()]).await?;
        let vector = embeddings
            .into_iter()
            .next()
            .ok_or_else(|| orka_core::Error::Knowledge("empty embedding result".into()))?;

        let id = uuid::Uuid::now_v7().to_string();
        metadata.insert("id".into(), id.clone());
        metadata.insert("content".into(), content.to_string());
        metadata.insert("memory_kind".into(), "semantic".into());
        metadata.insert("memory_scope".into(), scope.to_string());
        metadata.insert("source".into(), source.to_string());
        metadata.insert("stored_at".into(), Utc::now().to_rfc3339());

        self.vector_store
            .upsert(
                &self.collection,
                std::slice::from_ref(&id),
                &[vector],
                &[metadata],
            )
            .await?;

        Ok(id)
    }

    /// Search for relevant facts.
    pub async fn search(
        &self,
        query: &str,
        limit: usize,
        score_threshold: Option<f32>,
        filter: Option<HashMap<String, String>>,
    ) -> Result<Vec<SearchResult>> {
        self.ensure_init().await?;
        let embeddings = self.embeddings.embed(&[query.to_string()]).await?;
        let vector = embeddings
            .into_iter()
            .next()
            .ok_or_else(|| orka_core::Error::Knowledge("empty embedding result".into()))?;

        self.vector_store
            .search(&self.collection, &vector, limit, score_threshold, filter)
            .await
    }

    /// List stored facts using metadata filters.
    pub async fn list(
        &self,
        limit: usize,
        filter: Option<HashMap<String, String>>,
    ) -> Result<Vec<FactRecord>> {
        self.ensure_init().await?;
        let records = self
            .vector_store
            .list_records(&self.collection, limit, filter)
            .await?;
        Ok(records.into_iter().map(Self::to_fact_record).collect())
    }

    /// Delete facts matching the given metadata filter.
    pub async fn forget(&self, filter: HashMap<String, String>) -> Result<usize> {
        self.ensure_init().await?;
        self.vector_store
            .delete_records(&self.collection, filter)
            .await
    }

    fn to_fact_record(record: StoredRecord) -> FactRecord {
        let scope = record
            .metadata
            .get("memory_scope")
            .and_then(|s| s.parse().ok())
            .unwrap_or(MemoryScope::Global);
        let content = record.metadata.get("content").cloned().unwrap_or_default();
        let source = record
            .metadata
            .get("source")
            .cloned()
            .unwrap_or_else(|| "unknown".into());
        FactRecord {
            id: if record.id.is_empty() {
                record.metadata.get("id").cloned().unwrap_or_default()
            } else {
                record.id
            },
            content,
            scope,
            source,
            metadata: record.metadata,
        }
    }
}
