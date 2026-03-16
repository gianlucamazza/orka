use std::collections::HashMap;
use std::sync::Arc;

use orka_core::traits::Skill;
use orka_core::{Error, Result, SkillInput, SkillOutput};

pub struct SkillRegistry {
    skills: HashMap<String, Arc<dyn Skill>>,
}

impl SkillRegistry {
    pub fn new() -> Self {
        Self {
            skills: HashMap::new(),
        }
    }

    pub fn register(&mut self, skill: Arc<dyn Skill>) {
        self.skills.insert(skill.name().to_string(), skill);
    }

    /// Register a skill and call its `init()` lifecycle hook.
    pub async fn register_with_init(&mut self, skill: Arc<dyn Skill>) -> Result<()> {
        let skill_ref: &dyn Skill = skill.as_ref();
        skill_ref.init().await?;
        self.skills.insert(skill.name().to_string(), skill);
        Ok(())
    }

    /// Call `cleanup()` on every registered skill. Errors are logged but not propagated.
    pub async fn cleanup_all(&self) {
        for (name, skill) in &self.skills {
            let skill_ref: &dyn Skill = skill.as_ref();
            if let Err(e) = skill_ref.cleanup().await {
                tracing::warn!(skill = name, %e, "skill cleanup failed");
            }
        }
    }

    pub fn get(&self, name: &str) -> Option<&Arc<dyn Skill>> {
        self.skills.get(name)
    }

    pub fn list(&self) -> Vec<&str> {
        self.skills.keys().map(|s| s.as_str()).collect()
    }

    /// Invoke a skill, checking that the caller has the required scope.
    /// Scope format: "skill:<name>" or "skill:*".
    pub async fn invoke_with_scopes(
        &self,
        name: &str,
        input: SkillInput,
        caller_scopes: &[String],
    ) -> Result<SkillOutput> {
        // If scopes are provided, check for authorization
        if !caller_scopes.is_empty() {
            let required = format!("skill:{name}");
            let has_scope =
                caller_scopes.iter().any(|s| s == &required || s == "skill:*" || s == "*");
            if !has_scope {
                return Err(Error::Auth(format!(
                    "missing scope '{required}' to invoke skill '{name}'"
                )));
            }
        }

        self.invoke(name, input).await
    }

    pub async fn invoke(&self, name: &str, input: SkillInput) -> Result<SkillOutput> {
        let skill = self
            .skills
            .get(name)
            .ok_or_else(|| Error::Skill(format!("unknown skill: {name}")))?;

        // Validate input against schema
        let schema = skill.schema();
        let input_value = serde_json::to_value(&input.args)
            .map_err(|e| Error::Skill(format!("failed to serialize input: {e}")))?;

        if !jsonschema::is_valid(&schema.parameters, &input_value) {
            return Err(Error::Skill(format!(
                "skill '{name}' input validation failed"
            )));
        }

        skill.execute(input).await
    }
}

impl Default for SkillRegistry {
    fn default() -> Self {
        Self::new()
    }
}
