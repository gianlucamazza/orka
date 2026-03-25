use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use orka_core::{Result, SkillInput, SkillOutput, SkillSchema, traits::Skill};
use uuid::Uuid;

use crate::{embeddings::EmbeddingProvider, vector_store::VectorStore};

/// Skill that embeds a piece of text and stores it in the vector store for
/// later retrieval.
pub struct MemoryStoreSkill {
    embeddings: Arc<dyn EmbeddingProvider>,
    store: Arc<dyn VectorStore>,
    default_collection: String,
}

impl MemoryStoreSkill {
    /// Create the skill with the given embedding provider, vector store, and
    /// default collection.
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
impl Skill for MemoryStoreSkill {
    fn name(&self) -> &'static str {
        "memory_store"
    }

    fn category(&self) -> &'static str {
        "knowledge"
    }

    fn description(&self) -> &'static str {
        "Store content with semantic embedding in the vector store for later retrieval."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "The text content to store"
                },
                "collection": {
                    "type": "string",
                    "description": "Collection name (optional, uses default)"
                },
                "metadata": {
                    "type": "object",
                    "additionalProperties": { "type": "string" },
                    "description": "Optional metadata key-value pairs"
                }
            },
            "required": ["content"]
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let content = input
            .args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| orka_core::Error::SkillCategorized {
                message: "content is required".into(),
                category: orka_core::ErrorCategory::Input,
            })?;

        let collection = input
            .args
            .get("collection")
            .and_then(|v| v.as_str())
            .unwrap_or(&self.default_collection);

        let mut metadata = HashMap::new();
        if let Some(meta) = input.args.get("metadata").and_then(|v| v.as_object()) {
            for (k, v) in meta {
                if let Some(s) = v.as_str() {
                    metadata.insert(k.clone(), s.to_string());
                }
            }
        }
        metadata.insert("content".into(), content.to_string());

        // Ensure collection exists
        self.store
            .ensure_collection(collection, self.embeddings.dimensions())
            .await?;

        // Generate embedding
        let embeddings = self.embeddings.embed(&[content.to_string()]).await?;

        let id = Uuid::now_v7().to_string();
        self.store
            .upsert(
                collection,
                std::slice::from_ref(&id),
                &embeddings,
                &[metadata],
            )
            .await?;

        Ok(SkillOutput::new(serde_json::json!({
            "stored": true,
            "id": id,
            "collection": collection,
        })))
    }
}
