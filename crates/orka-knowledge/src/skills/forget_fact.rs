use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use orka_core::{Result, SkillInput, SkillOutput, SkillSchema, traits::Skill};

use crate::fact_store::FactStore;

/// Skill that deletes stored semantic facts.
pub struct ForgetFactSkill {
    facts: Arc<FactStore>,
}

impl ForgetFactSkill {
    /// Create the skill with the given fact store.
    pub fn new(facts: Arc<FactStore>) -> Self {
        Self { facts }
    }
}

#[async_trait]
impl Skill for ForgetFactSkill {
    fn name(&self) -> &'static str {
        "forget_fact"
    }

    fn category(&self) -> &'static str {
        "memory"
    }

    fn description(&self) -> &'static str {
        "Delete semantic facts by id or metadata filter."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Fact identifier to delete"
                },
                "filter": {
                    "type": "object",
                    "additionalProperties": { "type": "string" },
                    "description": "Optional metadata filter key-value pairs"
                }
            }
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let mut filter = input
            .args
            .get("filter")
            .and_then(|v| v.as_object())
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect::<HashMap<String, String>>()
            })
            .unwrap_or_default();

        if let Some(id) = input.args.get("id").and_then(|v| v.as_str()) {
            filter.insert("id".into(), id.to_string());
        }

        if filter.is_empty() {
            return Err(orka_core::Error::SkillCategorized {
                message: "either 'id' or 'filter' is required".into(),
                category: orka_core::ErrorCategory::Input,
            });
        }

        let deleted = self.facts.forget(filter).await?;

        Ok(SkillOutput::new(serde_json::json!({
            "deleted": deleted,
        })))
    }
}
