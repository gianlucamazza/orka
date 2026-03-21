use async_trait::async_trait;
use orka_core::traits::Skill;
use orka_core::{Result, SkillInput, SkillOutput, SkillSchema};
use std::collections::HashMap;
use std::sync::Arc;

use crate::embeddings::EmbeddingProvider;
use crate::vector_store::VectorStore;

/// Skill that performs semantic similarity search against a vector store collection.
pub struct MemorySearchSkill {
    embeddings: Arc<dyn EmbeddingProvider>,
    store: Arc<dyn VectorStore>,
    default_collection: String,
}

impl MemorySearchSkill {
    /// Create the skill with the given embedding provider, vector store, and default collection.
    pub fn new(
        embeddings: Arc<dyn EmbeddingProvider>,
        store: Arc<dyn VectorStore>,
        default_collection: String,
    ) -> Self {
        Self {
            embeddings,
            store,
            default_collection,
        }
    }
}

#[async_trait]
impl Skill for MemorySearchSkill {
    fn name(&self) -> &str {
        "memory_search"
    }

    fn category(&self) -> &str {
        "knowledge"
    }

    fn description(&self) -> &str {
        "Search for semantically similar content in the vector store."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query"
                },
                "collection": {
                    "type": "string",
                    "description": "Collection to search (optional, uses default)"
                },
                "limit": {
                    "type": "integer",
                    "default": 5,
                    "description": "Maximum number of results"
                },
                "score_threshold": {
                    "type": "number",
                    "description": "Minimum similarity score (0-1)"
                },
                "filter": {
                    "type": "object",
                    "additionalProperties": { "type": "string" },
                    "description": "Metadata filter key-value pairs"
                }
            },
            "required": ["query"]
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let query = input
            .args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| orka_core::Error::SkillCategorized {
                message: "query is required".into(),
                category: orka_core::ErrorCategory::Input,
            })?;

        let collection = input
            .args
            .get("collection")
            .and_then(|v| v.as_str())
            .unwrap_or(&self.default_collection);

        let limit = input
            .args
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(5) as usize;

        let score_threshold = input
            .args
            .get("score_threshold")
            .and_then(|v| v.as_f64())
            .map(|v| v as f32);

        let filter = input
            .args
            .get("filter")
            .and_then(|v| v.as_object())
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect::<HashMap<String, String>>()
            });

        // Generate query embedding
        let embeddings = self.embeddings.embed(&[query.to_string()]).await?;

        let vector = embeddings
            .into_iter()
            .next()
            .ok_or_else(|| orka_core::Error::Knowledge("no embedding generated".into()))?;

        let results = self
            .store
            .search(collection, &vector, limit, score_threshold, filter)
            .await?;

        Ok(SkillOutput::new(serde_json::json!({
            "results": results,
            "count": results.len(),
            "collection": collection,
        })))
    }
}
