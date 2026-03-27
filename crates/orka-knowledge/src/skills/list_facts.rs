use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use orka_core::{Result, SkillInput, SkillOutput, SkillSchema, traits::Skill};

use crate::fact_store::FactStore;

/// Skill that lists stored semantic facts.
pub struct ListFactsSkill {
    facts: Arc<FactStore>,
}

impl ListFactsSkill {
    /// Create the skill with the given fact store.
    pub fn new(facts: Arc<FactStore>) -> Self {
        Self { facts }
    }
}

#[async_trait]
impl Skill for ListFactsSkill {
    fn name(&self) -> &'static str {
        "list_facts"
    }

    fn category(&self) -> &'static str {
        "memory"
    }

    fn description(&self) -> &'static str {
        "List semantic facts remembered by the agent."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "limit": {
                    "type": "integer",
                    "default": 25,
                    "description": "Maximum number of facts to list"
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
        let limit = input
            .args
            .get("limit")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(25) as usize;

        let filter = input
            .args
            .get("filter")
            .and_then(|v| v.as_object())
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect::<HashMap<String, String>>()
            });

        let facts = self.facts.list(limit, filter).await?;
        let facts_json: Vec<serde_json::Value> = facts
            .into_iter()
            .map(|fact| {
                serde_json::json!({
                    "id": fact.id,
                    "content": fact.content,
                    "scope": fact.scope.to_string(),
                    "source": fact.source,
                    "metadata": fact.metadata,
                })
            })
            .collect();

        Ok(SkillOutput::new(serde_json::json!({
            "facts": facts_json,
            "count": facts_json.len(),
        })))
    }
}
