use async_trait::async_trait;
use orka_core::traits::Skill;
use orka_core::{Result, SkillInput, SkillOutput, SkillSchema};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

use crate::embeddings::EmbeddingProvider;
use crate::vector_store::VectorStore;

pub struct MemoryStoreSkill {
    embeddings: Arc<dyn EmbeddingProvider>,
    store: Arc<dyn VectorStore>,
    default_collection: String,
}

impl MemoryStoreSkill {
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
    fn name(&self) -> &str {
        "memory_store"
    }

    fn description(&self) -> &str {
        "Store content with semantic embedding in the vector store for later retrieval"
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
            .ok_or_else(|| orka_core::Error::Skill("content is required".into()))?;

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

        let id = Uuid::new_v4().to_string();
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
