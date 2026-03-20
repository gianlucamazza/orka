use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use orka_circuit_breaker::{CircuitBreaker, CircuitBreakerConfig, CircuitState};
use orka_core::traits::Skill;
use orka_core::{Error, ErrorCategory, Result, SkillInput, SkillOutput};

/// Per-skill entry combining the skill implementation with its circuit breaker.
struct SkillEntry {
    skill: Arc<dyn Skill>,
    circuit: CircuitBreaker,
}

/// Thread-safe registry that maps skill names to their [`Skill`] implementations.
///
/// Each registered skill is paired with a [`CircuitBreaker`] that opens after
/// repeated environmental failures, preventing the LLM from repeatedly calling
/// a tool that cannot work in the current environment.
pub struct SkillRegistry {
    skills: HashMap<String, SkillEntry>,
}

/// Circuit breaker configuration for environmental errors:
/// opens after 3 consecutive failures, stays open for 5 minutes.
const ENV_CIRCUIT_CONFIG: CircuitBreakerConfig = CircuitBreakerConfig {
    failure_threshold: 3,
    success_threshold: 1,
    open_duration: Duration::from_secs(300),
};

impl SkillRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            skills: HashMap::new(),
        }
    }

    /// Register a skill, replacing any existing skill with the same name.
    pub fn register(&mut self, skill: Arc<dyn Skill>) {
        let name = skill.name().to_string();
        self.skills.insert(
            name,
            SkillEntry {
                skill,
                circuit: CircuitBreaker::new(ENV_CIRCUIT_CONFIG),
            },
        );
    }

    /// Register a skill and call its `init()` lifecycle hook.
    pub async fn register_with_init(&mut self, skill: Arc<dyn Skill>) -> Result<()> {
        let skill_ref: &dyn Skill = skill.as_ref();
        skill_ref.init().await?;
        let name = skill.name().to_string();
        self.skills.insert(
            name,
            SkillEntry {
                skill,
                circuit: CircuitBreaker::new(ENV_CIRCUIT_CONFIG),
            },
        );
        Ok(())
    }

    /// Call `cleanup()` on every registered skill. Errors are logged but not propagated.
    pub async fn cleanup_all(&self) {
        for (name, entry) in &self.skills {
            let skill_ref: &dyn Skill = entry.skill.as_ref();
            if let Err(e) = skill_ref.cleanup().await {
                tracing::warn!(skill = name, %e, "skill cleanup failed");
            }
        }
    }

    /// Look up a skill by name.
    pub fn get(&self, name: &str) -> Option<&Arc<dyn Skill>> {
        self.skills.get(name).map(|e| &e.skill)
    }

    /// Return the names of all registered skills (including those with open circuits).
    pub fn list(&self) -> Vec<&str> {
        self.skills.keys().map(|s| s.as_str()).collect()
    }

    /// Return the names of skills whose circuit breaker is Closed or HalfOpen.
    ///
    /// Use this when building the tool list for an LLM call so that skills
    /// with open circuits (persistent environmental failures) are not offered.
    pub fn list_available(&self) -> Vec<&str> {
        self.skills
            .iter()
            .filter(|(_, e)| e.circuit.state() != CircuitState::Open)
            .map(|(name, _)| name.as_str())
            .collect()
    }

    /// Force the circuit breaker for a skill to the Open state.
    ///
    /// Used by the experience system to structurally disable a skill after
    /// detecting persistent environmental failures.
    pub fn force_open(&self, skill_name: &str) {
        if let Some(entry) = self.skills.get(skill_name) {
            // Record failures up to the threshold to trip the circuit
            for _ in 0..ENV_CIRCUIT_CONFIG.failure_threshold {
                entry.circuit.record_failure();
            }
        }
    }

    /// Invoke a skill, checking that the caller has the required scope.
    /// Scope format: `skill:<name>` or `skill:*`.
    pub async fn invoke_with_scopes(
        &self,
        name: &str,
        input: SkillInput,
        caller_scopes: &[String],
    ) -> Result<SkillOutput> {
        // If scopes are provided, check for authorization
        if !caller_scopes.is_empty() {
            let required = format!("skill:{name}");
            let has_scope = caller_scopes
                .iter()
                .any(|s| s == &required || s == "skill:*" || s == "*");
            if !has_scope {
                return Err(Error::Auth(format!(
                    "missing scope '{required}' to invoke skill '{name}'"
                )));
            }
        }

        self.invoke(name, input).await
    }

    /// Invoke a skill by name after validating the input against its JSON schema.
    ///
    /// Environmental errors increment the circuit breaker failure counter.
    /// After `failure_threshold` consecutive environmental failures the circuit opens
    /// and this method returns an error immediately without executing the skill.
    pub async fn invoke(&self, name: &str, input: SkillInput) -> Result<SkillOutput> {
        let entry = self
            .skills
            .get(name)
            .ok_or_else(|| Error::Skill(format!("unknown skill: {name}")))?;

        // Reject immediately if circuit is open
        if entry.circuit.state() == CircuitState::Open {
            return Err(Error::SkillCategorized {
                message: format!("skill '{name}' is temporarily disabled (circuit open)"),
                category: ErrorCategory::Environmental,
            });
        }

        // Validate input against schema
        let schema = entry.skill.schema();
        let input_value = serde_json::to_value(&input.args)
            .map_err(|e| Error::Skill(format!("failed to serialize input: {e}")))?;

        if !jsonschema::is_valid(&schema.parameters, &input_value) {
            return Err(Error::Skill(format!(
                "skill '{name}' input validation failed"
            )));
        }

        let result = entry.skill.execute(input).await;

        // Update circuit breaker based on result
        match &result {
            Err(e) if e.category() == ErrorCategory::Environmental => {
                entry.circuit.record_failure();
                if entry.circuit.state() == CircuitState::Open {
                    tracing::warn!(
                        skill = name,
                        "circuit breaker opened after environmental failure"
                    );
                }
            }
            Ok(_) => {
                entry.circuit.record_success();
            }
            Err(_) => {
                // Non-environmental errors do not count against the circuit breaker
            }
        }

        result
    }
}

impl Default for SkillRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orka_core::testing::EchoSkill;

    /// A skill that always fails with an environmental error.
    struct FailingSkill;

    #[async_trait::async_trait]
    impl Skill for FailingSkill {
        fn name(&self) -> &str {
            "failing"
        }
        fn description(&self) -> &str {
            "always fails"
        }
        fn schema(&self) -> orka_core::SkillSchema {
            orka_core::SkillSchema::new(
                serde_json::json!({"type": "object", "additionalProperties": true}),
            )
        }
        async fn execute(&self, _input: SkillInput) -> Result<SkillOutput> {
            Err(Error::SkillCategorized {
                message: "env failure".into(),
                category: ErrorCategory::Environmental,
            })
        }
    }

    fn empty_input() -> SkillInput {
        SkillInput::new(Default::default())
    }

    #[test]
    fn register_and_get() {
        let mut reg = SkillRegistry::new();
        reg.register(Arc::new(EchoSkill));
        assert!(reg.get("echo").is_some());
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn list_skills() {
        let mut reg = SkillRegistry::new();
        reg.register(Arc::new(EchoSkill));
        let names = reg.list();
        assert!(names.contains(&"echo"));
    }

    #[tokio::test]
    async fn invoke_valid_skill() {
        let mut reg = SkillRegistry::new();
        reg.register(Arc::new(EchoSkill));
        let input = SkillInput::new(
            [("msg".into(), serde_json::json!("hi"))]
                .into_iter()
                .collect(),
        );
        let output = reg.invoke("echo", input).await.unwrap();
        assert_eq!(output.data, serde_json::json!({"msg": "hi"}));
    }

    #[tokio::test]
    async fn invoke_unknown_skill_errors() {
        let reg = SkillRegistry::new();
        let result = reg.invoke("nonexistent", empty_input()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn invoke_with_scopes_allowed() {
        let mut reg = SkillRegistry::new();
        reg.register(Arc::new(EchoSkill));
        let scopes = vec!["skill:echo".to_string()];
        let result = reg.invoke_with_scopes("echo", empty_input(), &scopes).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn invoke_with_scopes_denied() {
        let mut reg = SkillRegistry::new();
        reg.register(Arc::new(EchoSkill));
        let scopes = vec!["skill:other".to_string()];
        let result = reg.invoke_with_scopes("echo", empty_input(), &scopes).await;
        assert!(matches!(result, Err(Error::Auth(_))));
    }

    #[tokio::test]
    async fn invoke_with_wildcard_scope() {
        let mut reg = SkillRegistry::new();
        reg.register(Arc::new(EchoSkill));
        let scopes = vec!["skill:*".to_string()];
        let result = reg.invoke_with_scopes("echo", empty_input(), &scopes).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn invoke_with_empty_scopes_allows() {
        let mut reg = SkillRegistry::new();
        reg.register(Arc::new(EchoSkill));
        let scopes: Vec<String> = vec![];
        let result = reg.invoke_with_scopes("echo", empty_input(), &scopes).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn circuit_breaker_opens_after_failures() {
        let mut reg = SkillRegistry::new();
        reg.register(Arc::new(FailingSkill));

        // ENV_CIRCUIT_CONFIG has failure_threshold = 3
        for _ in 0..3 {
            let _ = reg.invoke("failing", empty_input()).await;
        }

        // Circuit should now be open
        let result = reg.invoke("failing", empty_input()).await;
        assert!(matches!(
            result,
            Err(Error::SkillCategorized {
                category: ErrorCategory::Environmental,
                ..
            })
        ));
    }

    #[test]
    fn list_available_excludes_open_circuits() {
        let mut reg = SkillRegistry::new();
        reg.register(Arc::new(EchoSkill));
        reg.register(Arc::new(FailingSkill));

        assert_eq!(reg.list_available().len(), 2);

        reg.force_open("failing");

        let available = reg.list_available();
        assert!(available.contains(&"echo"));
        assert!(!available.contains(&"failing"));
    }

    #[tokio::test]
    async fn register_with_init_calls_lifecycle() {
        let mut reg = SkillRegistry::new();
        reg.register_with_init(Arc::new(EchoSkill)).await.unwrap();
        assert!(reg.get("echo").is_some());
    }

    #[tokio::test]
    async fn invoke_validates_schema() {
        let mut reg = SkillRegistry::new();

        /// Skill with strict schema requiring a "query" field.
        struct StrictSkill;
        #[async_trait::async_trait]
        impl Skill for StrictSkill {
            fn name(&self) -> &str {
                "strict"
            }
            fn description(&self) -> &str {
                "strict schema"
            }
            fn schema(&self) -> orka_core::SkillSchema {
                orka_core::SkillSchema::new(serde_json::json!({
                    "type": "object",
                    "properties": { "query": { "type": "string" } },
                    "required": ["query"]
                }))
            }
            async fn execute(&self, _input: SkillInput) -> Result<SkillOutput> {
                Ok(SkillOutput::new(serde_json::json!(null)))
            }
        }

        reg.register(Arc::new(StrictSkill));

        // Empty input should fail schema validation
        let result = reg.invoke("strict", empty_input()).await;
        assert!(result.is_err());

        // Valid input should succeed
        let input = SkillInput::new(
            [("query".into(), serde_json::json!("test"))]
                .into_iter()
                .collect(),
        );
        let result = reg.invoke("strict", input).await;
        assert!(result.is_ok());
    }
}
