//! Concrete context providers for Orka.
//!
//! These providers fetch data from various sources (experience service,
//! shell history, workspace registry) and make it available to the
//! prompt building pipeline.

use super::types::{BuildContext, PrincipleContext, SessionContext, WorkspaceContext};
use async_trait::async_trait;
use orka_core::Result;
use serde_json::Value;
use std::sync::Arc;

/// Provider that fetches learned principles from experience service.
pub struct ExperienceContextProvider {
    experience: Arc<dyn ExperienceService>,
    workspace: String,
}

/// Trait abstracting the experience service.
#[async_trait]
pub trait ExperienceService: Send + Sync {
    /// Retrieve principles relevant to the query.
    async fn retrieve_principles(&self, query: &str, workspace: &str) -> Result<Vec<Principle>>;
}

/// Principle from experience service.
pub struct Principle {
    pub text: String,
    pub kind: PrincipleKind,
}

/// Kind of principle.
/// Types of principles
pub enum PrincipleKind {
    /// Something the agent should do
    Do,
    /// Something the agent should avoid
    Avoid,
}

impl ExperienceContextProvider {
    /// Create a new experience context provider.
    pub fn new(experience: Arc<dyn ExperienceService>, workspace: impl Into<String>) -> Self {
        Self {
            experience,
            workspace: workspace.into(),
        }
    }
}

#[async_trait]
impl super::provider::ContextProvider for ExperienceContextProvider {
    fn provider_id(&self) -> &str {
        "experience"
    }

    async fn provide(&self, ctx: &SessionContext) -> Result<Value> {
        let principles = self
            .experience
            .retrieve_principles(&ctx.user_message, &self.workspace)
            .await?;

        let contexts: Vec<PrincipleContext> = principles
            .into_iter()
            .enumerate()
            .map(|(i, p)| PrincipleContext {
                index: Some(i + 1),
                text: p.text,
                kind: match p.kind {
                    PrincipleKind::Do => "do".to_string(),
                    PrincipleKind::Avoid => "avoid".to_string(),
                },
            })
            .collect();

        Ok(serde_json::json!({ "principles": contexts }))
    }
}

/// Provider for shell command history.
pub struct ShellContextProvider;

impl ShellContextProvider {
    /// Create a new shell context provider.
    pub fn new() -> Self {
        Self
    }
}

impl Default for ShellContextProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl super::provider::ContextProvider for ShellContextProvider {
    fn provider_id(&self) -> &str {
        "shell"
    }

    async fn provide(&self, ctx: &SessionContext) -> Result<Value> {
        // Extract recent commands from metadata
        let commands = ctx
            .metadata
            .get("shell:recent_commands")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty());

        match commands {
            Some(cmds) => Ok(serde_json::json!({
                "shell_commands": {
                    "content": format!("## Recent local shell commands\n{cmds}"),
                    "raw": cmds,
                }
            })),
            None => Ok(serde_json::json!({})),
        }
    }
}

/// Provider for soft skills.
pub struct SoftSkillsContextProvider {
    registry: Arc<dyn SoftSkillRegistry>,
    selection_mode: SoftSkillSelectionMode,
}

/// Trait for soft skill registry.
pub trait SoftSkillRegistry: Send + Sync {
    /// Build prompt section for selected skills.
    fn build_prompt_section(&self, names: &[&str]) -> String;
    /// List all skill names.
    fn list(&self) -> Vec<&str>;
    /// Filter skills by message content.
    fn filter_by_message(&self, message: &str) -> Vec<&str>;
}

/// Selection mode for soft skills.
pub enum SoftSkillSelectionMode {
    All,
    Keyword,
}

impl SoftSkillsContextProvider {
    /// Create a new soft skills context provider.
    pub fn new(registry: Arc<dyn SoftSkillRegistry>, selection_mode: SoftSkillSelectionMode) -> Self {
        Self {
            registry,
            selection_mode,
        }
    }
}

#[async_trait]
impl super::provider::ContextProvider for SoftSkillsContextProvider {
    fn provider_id(&self) -> &str {
        "soft_skills"
    }

    async fn provide(&self, ctx: &SessionContext) -> Result<Value> {
        let selected: Vec<&str> = match self.selection_mode {
            SoftSkillSelectionMode::Keyword => {
                self.registry.filter_by_message(&ctx.user_message)
            }
            SoftSkillSelectionMode::All => self.registry.list(),
        };

        let section = self.registry.build_prompt_section(&selected);

        if section.is_empty() {
            Ok(serde_json::json!({}))
        } else {
            Ok(serde_json::json!({
                "soft_skills": {
                    "content": section,
                    "selected": selected,
                }
            }))
        }
    }
}

/// Provider for static/dynamic sections.
pub struct SectionsContextProvider {
    sections: std::collections::HashMap<String, String>,
}

impl SectionsContextProvider {
    /// Create a new sections provider.
    pub fn new(sections: std::collections::HashMap<String, String>) -> Self {
        Self { sections }
    }
}

#[async_trait]
impl super::provider::ContextProvider for SectionsContextProvider {
    fn provider_id(&self) -> &str {
        "sections"
    }

    async fn provide(&self, _ctx: &SessionContext) -> Result<Value> {
        let obj: serde_json::Map<String, Value> = self
            .sections
            .iter()
            .map(|(k, v)| (k.clone(), Value::String(v.clone())))
            .collect();

        Ok(Value::Object(obj))
    }
}

/// A coordinator that builds BuildContext using registered providers.
pub struct ContextCoordinator {
    base_context: BuildContext,
    providers: Vec<Box<dyn super::provider::ContextProvider>>,
}

impl ContextCoordinator {
    /// Create a new coordinator with base context.
    pub fn new(base_context: BuildContext) -> Self {
        Self {
            base_context,
            providers: Vec::new(),
        }
    }

    /// Add a provider.
    pub fn with_provider(mut self, provider: Box<dyn super::provider::ContextProvider>) -> Self {
        self.providers.push(provider);
        self
    }

    /// Build the final context by running all providers.
    pub async fn build(mut self, session: &SessionContext) -> Result<BuildContext> {
        // Set datetime
        self.base_context.datetime = chrono::Utc::now().to_rfc3339();

        // Run all providers and collect their data
        let mut all_data: Vec<(String, Value)> = Vec::new();
        for provider in &self.providers {
            match provider.provide(session).await {
                Ok(data) => {
                    if let Some(obj) = data.as_object() {
                        for (k, v) in obj {
                            all_data.push((k.clone(), v.clone()));
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        provider = provider.provider_id(),
                        error = %e,
                        "context provider failed, continuing without its data"
                    );
                }
            }
        }

        // Merge collected data
        for (key, value) in all_data {
            self.merge_single(&key, &value);
        }

        Ok(self.base_context)
    }

    fn merge_single(&mut self, key: &str, value: &Value) {
        match key {
            "principles" => {
                if let Ok(principles) = serde_json::from_value(value.clone()) {
                    self.base_context.principles = principles;
                }
            }
            "workspace" => {
                if let Ok(ws) = serde_json::from_value(value.clone()) {
                    self.base_context.workspace = ws;
                }
            }
            "shell_commands" => {
                if let Some(content) = value.get("content").and_then(|v| v.as_str()) {
                    self.base_context
                        .dynamic_sections
                        .insert("shell_commands".to_string(), content.to_string());
                }
            }
            "soft_skills" => {
                if let Some(content) = value.get("content").and_then(|v| v.as_str()) {
                    self.base_context
                        .dynamic_sections
                        .insert("soft_skills".to_string(), content.to_string());
                }
            }
            // Any other key goes to dynamic_sections
            _ => {
                if let Some(s) = value.as_str() {
                    self.base_context.dynamic_sections.insert(key.to_string(), s.to_string());
                }
            }
        }
    }
}
