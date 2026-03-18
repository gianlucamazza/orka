use async_trait::async_trait;
use orka_core::traits::Skill;
use orka_core::{Result, SkillInput, SkillOutput, SkillSchema};
use std::sync::Arc;

use crate::vector_store::VectorStore;

/// Skill that lists documents ingested into a knowledge base collection.
pub struct DocListSkill {
    store: Arc<dyn VectorStore>,
    default_collection: String,
}

impl DocListSkill {
    /// Create the skill with the given vector store and default collection name.
    pub fn new(store: Arc<dyn VectorStore>, default_collection: String) -> Self {
        Self {
            store,
            default_collection,
        }
    }
}

#[async_trait]
impl Skill for DocListSkill {
    fn name(&self) -> &str {
        "doc_list"
    }

    fn description(&self) -> &str {
        "List documents that have been ingested into the knowledge base"
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "collection": {
                    "type": "string",
                    "description": "Collection to list (optional, uses default)"
                },
                "limit": {
                    "type": "integer",
                    "default": 100,
                    "description": "Maximum number of documents to list"
                }
            }
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let collection = input
            .args
            .get("collection")
            .and_then(|v| v.as_str())
            .unwrap_or(&self.default_collection);

        let limit = input
            .args
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(100) as usize;

        let documents = self.store.list_documents(collection, limit, None).await?;

        Ok(SkillOutput::new(serde_json::json!({
            "documents": documents,
            "count": documents.len(),
            "collection": collection,
        })))
    }
}
