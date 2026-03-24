use std::sync::Arc;

use async_trait::async_trait;
use orka_core::{
    Error, ErrorCategory, Result, SkillInput, SkillOutput, SkillSchema, traits::Skill,
};

use crate::guard::PermissionGuard;

// ── env_get ──

/// Skill that reads the value of a single environment variable.
pub struct EnvGetSkill {
    guard: Arc<PermissionGuard>,
}

impl EnvGetSkill {
    /// Create a new `env_get` skill with the given permission guard.
    pub fn new(guard: Arc<PermissionGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
impl Skill for EnvGetSkill {
    fn name(&self) -> &str {
        "env_get"
    }

    fn category(&self) -> &str {
        "system"
    }

    fn description(&self) -> &str {
        "Get the value of an environment variable."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Environment variable name" }
            },
            "required": ["name"]
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let name = input
            .args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::SkillCategorized {
                message: "missing 'name' argument".into(),
                category: ErrorCategory::Input,
            })?;

        self.guard.check_env_var(name)?;

        let value = std::env::var(name).ok();

        Ok(SkillOutput::new(serde_json::json!({
            "name": name,
            "value": value,
            "exists": value.is_some(),
        })))
    }
}

// ── env_list ──

/// Skill that lists all environment variables (sensitive values masked).
pub struct EnvListSkill {
    guard: Arc<PermissionGuard>,
}

impl EnvListSkill {
    /// Create a new `env_list` skill with the given permission guard.
    pub fn new(guard: Arc<PermissionGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
impl Skill for EnvListSkill {
    fn name(&self) -> &str {
        "env_list"
    }

    fn category(&self) -> &str {
        "system"
    }

    fn description(&self) -> &str {
        "List environment variables. Sensitive values are masked."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "filter": { "type": "string", "description": "Filter by name substring (case-insensitive)" }
            },
            "required": []
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let filter = input.args.get("filter").and_then(|v| v.as_str());
        let filter_lower = filter.map(|f| f.to_lowercase());

        let mut vars: Vec<serde_json::Value> = std::env::vars()
            .filter(|(name, _)| {
                if let Some(ref f) = filter_lower {
                    name.to_lowercase().contains(f)
                } else {
                    true
                }
            })
            .map(|(name, value)| {
                let masked = if self.guard.is_sensitive_env(&name) {
                    "***".to_string()
                } else {
                    value
                };
                serde_json::json!({
                    "name": name,
                    "value": masked,
                })
            })
            .collect();

        vars.sort_by(|a, b| {
            a["name"]
                .as_str()
                .unwrap_or("")
                .cmp(b["name"].as_str().unwrap_or(""))
        });

        Ok(SkillOutput::new(serde_json::json!({
            "variables": vars,
            "count": vars.len(),
        })))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn make_guard() -> Arc<PermissionGuard> {
        use orka_core::config::OsConfig;
        Arc::new(PermissionGuard::new(&OsConfig::default()))
    }

    #[test]
    fn env_get_schema_valid() {
        let skill = EnvGetSkill::new(make_guard());
        let schema = skill.schema();
        assert_eq!(schema.parameters["required"][0], "name");
    }

    #[tokio::test]
    async fn env_get_home() {
        let skill = EnvGetSkill::new(make_guard());
        let mut args = HashMap::new();
        args.insert("name".into(), serde_json::json!("HOME"));
        let output = skill.execute(SkillInput::new(args)).await.unwrap();
        assert!(output.data["exists"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn env_get_sensitive_blocked() {
        let skill = EnvGetSkill::new(make_guard());
        let mut args = HashMap::new();
        args.insert("name".into(), serde_json::json!("API_KEY"));
        let result = skill.execute(SkillInput::new(args)).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn env_list_returns_data() {
        let skill = EnvListSkill::new(make_guard());
        let input = SkillInput::new(HashMap::new());
        let output = skill.execute(input).await.unwrap();
        assert!(output.data["count"].as_u64().unwrap() > 0);
    }

    #[tokio::test]
    async fn env_list_filter() {
        let skill = EnvListSkill::new(make_guard());
        let mut args = HashMap::new();
        args.insert("filter".into(), serde_json::json!("HOME"));
        let output = skill.execute(SkillInput::new(args)).await.unwrap();
        assert!(output.data["count"].as_u64().unwrap() >= 1);
    }

    #[tokio::test]
    async fn env_list_masks_sensitive() {
        // Set a test env var that matches sensitive pattern
        // SAFETY: test runs single-threaded; no other thread reads this var.
        unsafe { std::env::set_var("TEST_API_KEY", "super_secret") };
        let skill = EnvListSkill::new(make_guard());
        let mut args = HashMap::new();
        args.insert("filter".into(), serde_json::json!("TEST_API_KEY"));
        let output = skill.execute(SkillInput::new(args)).await.unwrap();
        let vars = output.data["variables"].as_array().unwrap();
        if let Some(var) = vars.iter().find(|v| v["name"] == "TEST_API_KEY") {
            assert_eq!(var["value"], "***");
        }
        // SAFETY: test runs single-threaded; no other thread reads this var.
        unsafe { std::env::remove_var("TEST_API_KEY") };
    }
}
