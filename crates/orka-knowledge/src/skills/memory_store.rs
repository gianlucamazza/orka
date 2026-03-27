use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use orka_core::{MemoryScope, Result, SkillInput, SkillOutput, SkillSchema, traits::Skill};

use crate::fact_store::FactStore;

/// Skill that stores an explicit semantic fact for later retrieval.
pub struct RememberFactSkill {
    facts: Arc<FactStore>,
}

impl RememberFactSkill {
    /// Create the skill with the given fact store.
    pub fn new(facts: Arc<FactStore>) -> Self {
        Self { facts }
    }
}

#[async_trait]
impl Skill for RememberFactSkill {
    fn name(&self) -> &'static str {
        "remember_fact"
    }

    fn category(&self) -> &'static str {
        "memory"
    }

    fn description(&self) -> &'static str {
        "Store a durable semantic fact for later retrieval."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "The durable fact to remember"
                },
                "scope": {
                    "type": "string",
                    "enum": ["workspace", "user", "global"],
                    "default": "workspace",
                    "description": "Retention scope for the fact"
                },
                "source": {
                    "type": "string",
                    "description": "Origin of the fact (optional, defaults to user)"
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

        let scope = input
            .args
            .get("scope")
            .and_then(|v| v.as_str())
            .unwrap_or("workspace")
            .parse::<MemoryScope>()
            .map_err(|_| orka_core::Error::SkillCategorized {
                message: "scope must be one of: workspace, user, global".into(),
                category: orka_core::ErrorCategory::Input,
            })?;

        let source = input
            .args
            .get("source")
            .and_then(|v| v.as_str())
            .unwrap_or("user");

        let mut metadata = HashMap::new();
        if let Some(meta) = input.args.get("metadata").and_then(|v| v.as_object()) {
            for (k, v) in meta {
                if let Some(s) = v.as_str() {
                    metadata.insert(k.clone(), s.to_string());
                }
            }
        }
        let id = self
            .facts
            .store_fact(content, scope, source, metadata)
            .await?;

        Ok(SkillOutput::new(serde_json::json!({
            "stored": true,
            "id": id,
            "scope": scope.to_string(),
        })))
    }
}
