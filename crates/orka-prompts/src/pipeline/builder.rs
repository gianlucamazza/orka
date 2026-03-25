use std::{collections::HashMap, sync::Arc};

use orka_core::{Error, Result};

use super::{config::PipelineConfig, section::PromptSection};
use crate::template::TemplateRegistry;

/// Context passed during prompt building.
///
/// Contains all data needed to render prompt sections.
/// This is the unified `BuildContext` used by both the pipeline and context
/// providers.
#[derive(Clone, Default)]
pub struct BuildContext {
    /// Agent display name.
    pub agent_name: String,

    /// Agent persona/body content.
    pub persona: String,

    /// Tool instructions content.
    pub tool_instructions: String,

    /// Workspace name.
    pub workspace_name: String,

    /// Available workspaces.
    pub available_workspaces: Vec<String>,

    /// Current working directory.
    pub cwd: Option<String>,

    /// Learned principles to inject.
    pub principles: Vec<serde_json::Value>,

    /// Conversation summary (if history was truncated).
    pub conversation_summary: Option<String>,

    /// Additional dynamic sections.
    pub dynamic_sections: HashMap<String, String>,

    /// Template registry for rendering.
    pub template_registry: Option<Arc<TemplateRegistry>>,

    /// Pipeline configuration.
    pub config: PipelineConfig,

    /// Current datetime (ISO 8601 format).
    pub datetime: String,

    /// Timezone for datetime display.
    pub timezone: String,
}

impl BuildContext {
    /// Create a new build context with the given agent name.
    pub fn new(agent_name: impl Into<String>) -> Self {
        Self {
            agent_name: agent_name.into(),
            ..Default::default()
        }
    }

    /// Set the persona content.
    #[must_use]
    pub fn with_persona(mut self, persona: impl Into<String>) -> Self {
        self.persona = persona.into();
        self
    }

    /// Set the tool instructions.
    #[must_use]
    pub fn with_tool_instructions(mut self, instructions: impl Into<String>) -> Self {
        self.tool_instructions = instructions.into();
        self
    }

    /// Set the workspace context.
    #[must_use]
    pub fn with_workspace(mut self, name: impl Into<String>, available: Vec<String>) -> Self {
        self.workspace_name = name.into();
        self.available_workspaces = available;
        self
    }

    /// Set the current working directory.
    #[must_use]
    pub fn with_cwd(mut self, cwd: impl Into<String>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    /// Set the principles.
    #[must_use]
    pub fn with_principles(mut self, principles: Vec<serde_json::Value>) -> Self {
        self.principles = principles;
        self
    }

    /// Set the conversation summary.
    #[must_use]
    pub fn with_summary(mut self, summary: impl Into<String>) -> Self {
        self.conversation_summary = Some(summary.into());
        self
    }

    /// Add a dynamic section.
    #[must_use]
    pub fn with_dynamic_section(
        mut self,
        name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        self.dynamic_sections.insert(name.into(), content.into());
        self
    }

    /// Set the template registry.
    #[must_use]
    pub fn with_templates(mut self, registry: Arc<TemplateRegistry>) -> Self {
        self.template_registry = Some(registry);
        self
    }

    /// Set the pipeline configuration.
    #[must_use]
    pub fn with_config(mut self, config: PipelineConfig) -> Self {
        self.config = config;
        self
    }

    /// Set the datetime string.
    #[must_use]
    pub fn with_datetime(mut self, datetime: impl Into<String>) -> Self {
        self.datetime = datetime.into();
        self
    }

    /// Set the timezone.
    #[must_use]
    pub fn with_timezone(mut self, timezone: impl Into<String>) -> Self {
        self.timezone = timezone.into();
        self
    }
}

/// Configurable pipeline for building system prompts.
///
/// Composes multiple sections into a complete prompt according to
/// the configured order and separators.
///
/// # Example
///
/// ```rust
/// use orka_prompts::pipeline::{BuildContext, PipelineConfig, SystemPromptPipeline};
///
/// async fn example() {
///     let config = PipelineConfig::default();
///     let pipeline = SystemPromptPipeline::from_config(&config);
///
///     let ctx = BuildContext::new("MyAgent").with_persona("I am a helpful assistant.");
///
///     let prompt = pipeline.build(&ctx).await.unwrap();
///     println!("{}", prompt);
/// }
/// ```
pub struct SystemPromptPipeline {
    sections: Vec<Box<dyn PromptSection>>,
    config: PipelineConfig,
}

impl SystemPromptPipeline {
    /// Create a pipeline from configuration.
    ///
    /// Automatically configures standard sections based on the config.
    pub fn from_config(config: &PipelineConfig) -> Self {
        let sections: Vec<Box<dyn PromptSection>> = config
            .sections
            .iter()
            .filter_map(|name| Self::create_section(name, config))
            .collect();

        Self {
            sections,
            config: config.clone(),
        }
    }

    /// Create a pipeline with explicit sections.
    pub fn with_sections(sections: Vec<Box<dyn PromptSection>>, config: PipelineConfig) -> Self {
        Self { sections, config }
    }

    /// Build the complete prompt.
    ///
    /// Iterates through all sections in order, renders each one,
    /// and joins them with the configured separator.
    pub async fn build(&self, ctx: &BuildContext) -> Result<String> {
        let mut parts = Vec::new();

        for section in &self.sections {
            match section.render(ctx).await {
                Ok(Some(content)) => {
                    if !content.is_empty() {
                        parts.push(content);
                    }
                }
                Ok(None) => {
                    // Section skipped
                }
                Err(e) => {
                    tracing::warn!(
                        section = section.name(),
                        error = %e,
                        "failed to render section"
                    );
                    if section.is_required() {
                        return Err(e);
                    }
                }
            }
        }

        Ok(parts.join(&self.config.section_separator))
    }

    fn create_section(name: &str, config: &PipelineConfig) -> Option<Box<dyn PromptSection>> {
        use crate::defaults::{
            SECTION_DATETIME, SECTION_DYNAMIC, SECTION_PERSONA, SECTION_PRINCIPLES,
            SECTION_SUMMARY, SECTION_TOOLS, SECTION_WORKSPACE,
        };

        match name {
            SECTION_PERSONA => Some(Box::new(PersonaSection)),
            SECTION_DATETIME if config.include_datetime => Some(Box::new(DateTimeSection {
                timezone: config.timezone.clone(),
            })),
            SECTION_WORKSPACE => Some(Box::new(WorkspaceSection)),
            SECTION_TOOLS => Some(Box::new(ToolsSection)),
            SECTION_DYNAMIC => Some(Box::new(DynamicSectionsSection)),
            SECTION_PRINCIPLES => Some(Box::new(PrinciplesSection {
                max_principles: config.max_principles,
            })),
            SECTION_SUMMARY if config.include_summary => Some(Box::new(SummarySection)),
            _ => None,
        }
    }
}

// Built-in section implementations

use async_trait::async_trait;

struct PersonaSection;

#[async_trait]
impl PromptSection for PersonaSection {
    fn name(&self) -> &'static str {
        "persona"
    }

    fn is_required(&self) -> bool {
        true
    }

    async fn render(&self, ctx: &BuildContext) -> Result<Option<String>> {
        let content = if ctx.persona.is_empty() {
            format!("You are {}.", ctx.agent_name)
        } else {
            format!("You are {}.\n\n{}", ctx.agent_name, ctx.persona)
        };
        Ok(Some(content))
    }
}

struct DateTimeSection {
    timezone: String,
}

#[async_trait]
impl PromptSection for DateTimeSection {
    fn name(&self) -> &'static str {
        "datetime"
    }

    fn is_required(&self) -> bool {
        false
    }

    async fn render(&self, ctx: &BuildContext) -> Result<Option<String>> {
        if let Some(registry) = &ctx.template_registry
            && registry.has_template("sections/datetime").await
        {
            let now = chrono::Utc::now();
            let tz: chrono_tz::Tz = self.timezone.parse().unwrap_or(chrono_tz::UTC);
            let local_time = now.with_timezone(&tz);

            let data = serde_json::json!({
                "date": local_time.format("%Y-%m-%d").to_string(),
                "time": local_time.format("%H:%M:%S").to_string(),
                "timezone": self.timezone,
                "datetime": local_time.to_rfc3339(),
            });

            return registry
                .render("sections/datetime", &data)
                .await
                .map(Some)
                .map_err(|e| Error::Other(format!("template error: {e}")));
        }

        // Fallback
        let now = chrono::Utc::now();
        let tz: chrono_tz::Tz = self.timezone.parse().unwrap_or(chrono_tz::UTC);
        let local_time = now.with_timezone(&tz);

        Ok(Some(format!(
            "Current date and time: {}",
            local_time.format("%Y-%m-%d %H:%M:%S %:z")
        )))
    }
}

struct WorkspaceSection;

#[async_trait]
impl PromptSection for WorkspaceSection {
    fn name(&self) -> &'static str {
        "workspace"
    }

    fn is_required(&self) -> bool {
        false
    }

    async fn render(&self, ctx: &BuildContext) -> Result<Option<String>> {
        if ctx.workspace_name.is_empty() {
            return Ok(None);
        }

        if let Some(registry) = &ctx.template_registry
            && registry.has_template("sections/workspace").await
        {
            let data = serde_json::json!({
                "workspace_name": ctx.workspace_name,
                "available_workspaces": ctx.available_workspaces,
                "cwd": ctx.cwd,
            });

            return registry
                .render("sections/workspace", &data)
                .await
                .map(Some)
                .map_err(|e| Error::Other(format!("template error: {e}")));
        }

        // Fallback
        let mut parts = Vec::new();

        let ws_list = ctx.available_workspaces.join(", ");
        parts.push(format!(
            "You are currently operating in workspace \"{}\".\n\
             Available workspaces: {}.\n\
             You can use the workspace_info tool to get details and workspace_switch to change workspace.",
            ctx.workspace_name, ws_list
        ));

        if let Some(cwd) = &ctx.cwd {
            parts.push(format!(
                "The user's current working directory is: {cwd}\n\
                When the user asks to create, read, or modify files without specifying an absolute \
                path, resolve them relative to this directory. Use this directory as the default \
                working directory for shell commands."
            ));
        }

        Ok(Some(parts.join("\n\n")))
    }
}

struct ToolsSection;

#[async_trait]
impl PromptSection for ToolsSection {
    fn name(&self) -> &'static str {
        "tools"
    }

    fn is_required(&self) -> bool {
        false
    }

    async fn render(&self, ctx: &BuildContext) -> Result<Option<String>> {
        if ctx.tool_instructions.is_empty() {
            return Ok(None);
        }

        if let Some(registry) = &ctx.template_registry
            && registry.has_template("sections/tools").await
        {
            let data = serde_json::json!({
                "instructions": ctx.tool_instructions,
            });

            return registry
                .render("sections/tools", &data)
                .await
                .map(Some)
                .map_err(|e| Error::Other(format!("template error: {e}")));
        }

        Ok(Some(ctx.tool_instructions.clone()))
    }
}

struct PrinciplesSection {
    max_principles: usize,
}

struct DynamicSectionsSection;

#[async_trait]
impl PromptSection for DynamicSectionsSection {
    fn name(&self) -> &'static str {
        "dynamic"
    }

    fn is_required(&self) -> bool {
        false
    }

    async fn render(&self, ctx: &BuildContext) -> Result<Option<String>> {
        if ctx.dynamic_sections.is_empty() {
            return Ok(None);
        }

        let mut keys: Vec<_> = ctx.dynamic_sections.keys().cloned().collect();
        keys.sort();

        let sections: Vec<String> = keys
            .into_iter()
            .filter_map(|key| {
                ctx.dynamic_sections
                    .get(&key)
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
            })
            .collect();

        if sections.is_empty() {
            Ok(None)
        } else {
            Ok(Some(sections.join("\n\n")))
        }
    }
}

#[async_trait]
impl PromptSection for PrinciplesSection {
    fn name(&self) -> &'static str {
        "principles"
    }

    fn is_required(&self) -> bool {
        false
    }

    async fn render(&self, ctx: &BuildContext) -> Result<Option<String>> {
        use crate::defaults::{
            PRINCIPLE_PREFIX_AVOID, PRINCIPLE_PREFIX_DO, PRINCIPLES_SECTION_HEADER,
        };

        if ctx.principles.is_empty() {
            return Ok(None);
        }

        let principles: Vec<_> = ctx
            .principles
            .iter()
            .take(self.max_principles)
            .enumerate()
            .map(|(i, p)| {
                let mut obj = p.clone();
                if let Some(map) = obj.as_object_mut() {
                    map.insert("index".to_string(), serde_json::json!(i + 1));
                }
                obj
            })
            .collect();

        if let Some(registry) = &ctx.template_registry
            && registry.has_template("sections/principles").await
        {
            let data = serde_json::json!({ "principles": principles });

            return registry
                .render("sections/principles", &data)
                .await
                .map(Some)
                .map_err(|e| Error::Other(format!("template error: {e}")));
        }

        // Fallback
        let mut lines = vec![
            PRINCIPLES_SECTION_HEADER.to_string(),
            String::new(),
            "The following principles were learned from past interactions. Apply them when relevant:"
                .to_string(),
            String::new(),
        ];

        for (i, p) in principles.iter().enumerate() {
            let text = p.get("text").and_then(|v| v.as_str()).unwrap_or("");
            let kind = p.get("kind").and_then(|v| v.as_str()).unwrap_or("do");
            let prefix = if kind == "avoid" {
                PRINCIPLE_PREFIX_AVOID
            } else {
                PRINCIPLE_PREFIX_DO
            };
            lines.push(format!("{}. [{}] {}", i + 1, prefix, text));
        }

        Ok(Some(lines.join("\n")))
    }
}

struct SummarySection;

#[async_trait]
impl PromptSection for SummarySection {
    fn name(&self) -> &'static str {
        "summary"
    }

    fn is_required(&self) -> bool {
        false
    }

    async fn render(&self, ctx: &BuildContext) -> Result<Option<String>> {
        let summary = match &ctx.conversation_summary {
            Some(s) if !s.is_empty() => s,
            _ => return Ok(None),
        };

        if let Some(registry) = &ctx.template_registry
            && registry.has_template("sections/summary").await
        {
            let data = serde_json::json!({ "summary": summary });

            return registry
                .render("sections/summary", &data)
                .await
                .map(Some)
                .map_err(|e| Error::Other(format!("template error: {e}")));
        }

        // Fallback
        Ok(Some(format!("## Prior Conversation Context\n\n{summary}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_build_basic() {
        let config = PipelineConfig::default();
        let pipeline = SystemPromptPipeline::from_config(&config);

        let ctx = BuildContext::new("TestAgent").with_persona("I am helpful.");

        let prompt = pipeline.build(&ctx).await.unwrap();
        assert!(prompt.contains("You are TestAgent"));
        assert!(prompt.contains("I am helpful"));
    }

    #[tokio::test]
    async fn test_build_with_workspace() {
        let config = PipelineConfig::default();
        let pipeline = SystemPromptPipeline::from_config(&config);

        let ctx = BuildContext::new("TestAgent")
            .with_workspace("default", vec!["default".to_string(), "other".to_string()])
            .with_cwd("/home/user");

        let prompt = pipeline.build(&ctx).await.unwrap();
        assert!(prompt.contains("operating in workspace \"default\""));
        assert!(prompt.contains("/home/user"));
    }

    #[tokio::test]
    async fn test_build_empty_persona() {
        // Test with minimal config (only persona section)
        let config = PipelineConfig {
            sections: vec!["persona".to_string()],
            ..Default::default()
        };
        let pipeline = SystemPromptPipeline::from_config(&config);

        let ctx = BuildContext::new("TestAgent");

        let prompt = pipeline.build(&ctx).await.unwrap();
        assert!(prompt.contains("You are TestAgent."));
        assert!(!prompt.contains("You are TestAgent.\n\n")); // No extra newline after empty persona
    }

    #[tokio::test]
    async fn test_build_renders_dynamic_sections() {
        let config = PipelineConfig::default();
        let pipeline = SystemPromptPipeline::from_config(&config);

        let ctx = BuildContext::new("TestAgent")
            .with_dynamic_section("coding_runtime", "## Coding Runtime\n\nstatus")
            .with_dynamic_section("soft_skills", "## Soft Skills\n\nrules");

        let prompt = pipeline.build(&ctx).await.unwrap();
        assert!(prompt.contains("## Coding Runtime"));
        assert!(prompt.contains("## Soft Skills"));
    }
}
