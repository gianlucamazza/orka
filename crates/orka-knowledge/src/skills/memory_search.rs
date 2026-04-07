use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use orka_core::{Result, SkillInput, SkillOutput, SkillSchema, traits::Skill};

use crate::fact_store::FactStore;

/// Skill that searches stored semantic facts.
pub struct SearchFactsSkill {
    facts: Arc<FactStore>,
}

impl SearchFactsSkill {
    /// Create the skill with the given fact store.
    pub fn new(facts: Arc<FactStore>) -> Self {
        Self { facts }
    }
}

#[async_trait]
impl Skill for SearchFactsSkill {
    fn name(&self) -> &'static str {
        "search_facts"
    }

    fn category(&self) -> &'static str {
        "memory"
    }

    fn description(&self) -> &'static str {
        "Search semantic facts remembered by the agent."
    }

    fn budget_cost(&self) -> f32 {
        0.5
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query"
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
                "scope": {
                    "type": "string",
                    "enum": ["session", "workspace", "user", "global"],
                    "description": "Optional scope filter"
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

        let limit = input
            .args
            .get("limit")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(5) as usize;

        let score_threshold = input
            .args
            .get("score_threshold")
            .and_then(serde_json::Value::as_f64)
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

        let mut filter = filter.unwrap_or_default();
        if let Some(scope) = input.args.get("scope").and_then(|v| v.as_str()) {
            filter.insert("memory_scope".into(), scope.to_string());
        }

        let results = self
            .facts
            .search(
                query,
                limit,
                score_threshold,
                if filter.is_empty() {
                    None
                } else {
                    Some(filter)
                },
            )
            .await?;

        Ok(SkillOutput::new(serde_json::json!({
            "results": results,
            "count": results.len(),
        })))
    }
}
