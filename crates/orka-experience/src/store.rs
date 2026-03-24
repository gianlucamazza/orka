use std::{collections::HashMap, sync::Arc};

use chrono::Utc;
use orka_core::Result;
use orka_knowledge::{embeddings::EmbeddingProvider, vector_store::VectorStore};
use tracing::{debug, warn};

use crate::types::{Principle, PrincipleKind};

/// Stores and retrieves principles using a vector store + embedding provider.
pub struct PrincipleStore {
    embeddings: Arc<dyn EmbeddingProvider>,
    vector_store: Arc<dyn VectorStore>,
    collection: String,
    initialized: tokio::sync::OnceCell<()>,
}

impl PrincipleStore {
    /// Create a new principle store backed by the given vector store and
    /// embedding provider.
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

    /// Ensure the collection exists.
    async fn ensure_init(&self) -> Result<()> {
        self.initialized
            .get_or_try_init(|| async {
                let dims = self.embeddings.dimensions();
                self.vector_store
                    .ensure_collection(&self.collection, dims)
                    .await
            })
            .await?;
        Ok(())
    }

    /// Store a principle, reinforcing an existing similar one if found above
    /// the dedup threshold.
    ///
    /// Returns `true` if a new principle was created, `false` if an existing
    /// one was reinforced.
    pub async fn store(&self, principle: &Principle, dedup_threshold: f32) -> Result<bool> {
        self.ensure_init().await?;

        let embeddings = self
            .embeddings
            .embed(std::slice::from_ref(&principle.text))
            .await?;
        let vector = embeddings
            .into_iter()
            .next()
            .ok_or_else(|| orka_core::Error::Knowledge("empty embedding result".into()))?;

        // Check for a near-duplicate within the same scope
        let similar = self
            .vector_store
            .search(&self.collection, &vector, 1, Some(dedup_threshold), None)
            .await?;

        if let Some(hit) = similar.into_iter().next() {
            let hit_scope = hit
                .metadata
                .get("scope")
                .map(|s| s.as_str())
                .unwrap_or("global");
            if hit_scope == principle.scope {
                // Reinforce the existing principle
                let existing_id = hit
                    .metadata
                    .get("id")
                    .cloned()
                    .unwrap_or_else(|| principle.id.clone());
                let reinforcement_count: u32 = hit
                    .metadata
                    .get("reinforcement_count")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0)
                    + 1;

                let mut payload = hit.metadata.clone();
                payload.insert(
                    "reinforcement_count".to_string(),
                    reinforcement_count.to_string(),
                );

                self.vector_store
                    .upsert(
                        &self.collection,
                        std::slice::from_ref(&existing_id),
                        &[vector],
                        &[payload],
                    )
                    .await?;

                debug!(
                    id = %existing_id,
                    reinforcement_count,
                    score = hit.score,
                    "reinforced existing principle"
                );
                return Ok(false);
            }
        }

        // No duplicate found — store as new
        let mut payload = HashMap::new();
        payload.insert("id".to_string(), principle.id.clone());
        payload.insert("text".to_string(), principle.text.clone());
        payload.insert(
            "kind".to_string(),
            match principle.kind {
                PrincipleKind::Do => "do".to_string(),
                PrincipleKind::Avoid => "avoid".to_string(),
            },
        );
        payload.insert("scope".to_string(), principle.scope.clone());
        payload.insert("created_at".to_string(), principle.created_at.to_rfc3339());
        payload.insert(
            "reinforcement_count".to_string(),
            principle.reinforcement_count.to_string(),
        );

        self.vector_store
            .upsert(
                &self.collection,
                std::slice::from_ref(&principle.id),
                &[vector],
                &[payload],
            )
            .await?;

        debug!(id = %principle.id, "stored new principle");
        Ok(true)
    }

    /// Retrieve the top-K most relevant principles for the given query.
    pub async fn retrieve(
        &self,
        query: &str,
        limit: usize,
        min_score: f32,
        scope_filter: Option<&str>,
    ) -> Result<Vec<Principle>> {
        self.ensure_init().await?;

        let embeddings = self.embeddings.embed(&[query.to_string()]).await?;
        let vector = embeddings
            .into_iter()
            .next()
            .ok_or_else(|| orka_core::Error::Knowledge("empty embedding result".into()))?;

        let filter = scope_filter.map(|s| {
            let mut f = HashMap::new();
            f.insert("scope".to_string(), s.to_string());
            f
        });

        let results = self
            .vector_store
            .search(&self.collection, &vector, limit, Some(min_score), filter)
            .await?;

        let principles: Vec<Principle> = results
            .into_iter()
            .filter_map(|r| {
                let payload = &r.metadata;
                let id = payload.get("id")?.clone();
                let text = payload.get("text")?.clone();
                let kind = match payload.get("kind").map(|s| s.as_str()) {
                    Some("avoid") => PrincipleKind::Avoid,
                    _ => PrincipleKind::Do,
                };
                let scope = payload
                    .get("scope")
                    .cloned()
                    .unwrap_or_else(|| "global".to_string());
                let created_at = payload
                    .get("created_at")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or_else(Utc::now);
                let reinforcement_count = payload
                    .get("reinforcement_count")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);

                Some(Principle {
                    id,
                    text,
                    kind,
                    scope,
                    created_at,
                    reinforcement_count,
                    relevance_score: r.score,
                })
            })
            .collect();

        debug!(
            query_len = query.len(),
            results = principles.len(),
            "retrieved principles"
        );
        Ok(principles)
    }

    /// Store a batch of principles with deduplication.
    ///
    /// Returns the number of newly created principles (reinforced ones are not
    /// counted).
    pub async fn store_batch(
        &self,
        principles: &[Principle],
        dedup_threshold: f32,
    ) -> Result<usize> {
        let mut created = 0;
        for p in principles {
            match self.store(p, dedup_threshold).await {
                Ok(true) => created += 1,
                Ok(false) => {}
                Err(e) => warn!(id = %p.id, %e, "failed to store principle"),
            }
        }
        Ok(created)
    }
}
