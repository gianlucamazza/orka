#![allow(missing_docs)]

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use orka_core::{SkillInput, SkillOutput, SkillSchema, testing::EchoSkill, traits::Skill};
use orka_skills::SkillRegistry;

// ---------------------------------------------------------------------------
// StrictSkill – requires a "name" field, no extra properties allowed
// ---------------------------------------------------------------------------

struct StrictSkill;

#[async_trait]
impl Skill for StrictSkill {
    fn name(&self) -> &'static str {
        "strict"
    }

    fn description(&self) -> &'static str {
        "Skill with strict schema"
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" }
            },
            "required": ["name"],
            "additionalProperties": false
        }))
    }

    async fn execute(&self, input: SkillInput) -> orka_core::Result<SkillOutput> {
        Ok(SkillOutput::new(serde_json::to_value(input.args).unwrap()))
    }
}

#[tokio::test]
async fn register_and_invoke() {
    let mut registry = SkillRegistry::new();
    registry.register(Arc::new(EchoSkill));

    let input = SkillInput::new(
        [("msg".to_string(), serde_json::json!("hi"))]
            .into_iter()
            .collect(),
    );
    let output = registry.invoke("echo", input).await.unwrap();
    assert_eq!(output.data, serde_json::json!({"msg": "hi"}));
}

#[tokio::test]
async fn invoke_unknown_returns_error() {
    let registry = SkillRegistry::new();
    let input = SkillInput::new(Default::default());
    let result = registry.invoke("nonexistent", input).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn list_skills() {
    let mut registry = SkillRegistry::new();
    assert!(registry.list().is_empty());

    registry.register(Arc::new(EchoSkill));
    let names = registry.list();
    assert_eq!(names.len(), 1);
    assert!(names.contains(&"echo"));
}

// ---------------------------------------------------------------------------
// Schema-validation tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn valid_input_passes_schema() {
    let mut registry = SkillRegistry::new();
    registry.register(Arc::new(StrictSkill));

    let input = SkillInput::new(HashMap::from([(
        "name".to_string(),
        serde_json::json!("test"),
    )]));
    let result = registry.invoke("strict", input).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap().data, serde_json::json!({"name": "test"}));
}

#[tokio::test]
async fn invalid_input_fails_schema() {
    let mut registry = SkillRegistry::new();
    registry.register(Arc::new(StrictSkill));

    // Missing required "name" field
    let input = SkillInput::new(HashMap::new());
    let result = registry.invoke("strict", input).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("validation failed"),
        "expected validation error, got: {err}"
    );
}

#[tokio::test]
async fn permissive_schema_accepts_anything() {
    let mut registry = SkillRegistry::new();
    registry.register(Arc::new(EchoSkill));

    let input = SkillInput::new(HashMap::from([
        ("foo".to_string(), serde_json::json!(42)),
        ("bar".to_string(), serde_json::json!([1, 2, 3])),
        ("baz".to_string(), serde_json::json!({"nested": true})),
    ]));
    let result = registry.invoke("echo", input).await;
    assert!(result.is_ok());
}
