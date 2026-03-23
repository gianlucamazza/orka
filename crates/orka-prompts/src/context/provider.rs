use super::types::{BuildContext, PrincipleContext, SessionContext, WorkspaceContext};
use async_trait::async_trait;
use orka_core::Result;
use serde_json::Value;

/// Trait for context providers that supply data for prompt building.
///
/// Implementors can fetch data from various sources (memory, external APIs, etc.)
/// and provide it to the prompt building pipeline.
#[async_trait]
pub trait ContextProvider: Send + Sync {
    /// Returns the provider identifier.
    fn provider_id(&self) -> &str;

    /// Provides context data for the given session.
    ///
    /// Returns a JSON value that can be used in templates.
    async fn provide(&self, ctx: &SessionContext) -> Result<Value>;
}

/// Provider for workspace-related context.
pub struct WorkspaceProvider {
    available_workspaces: Vec<String>,
}

impl WorkspaceProvider {
    /// Create a new workspace provider.
    pub fn new(available: Vec<String>) -> Self {
        Self {
            available_workspaces: available,
        }
    }
}

#[async_trait]
impl ContextProvider for WorkspaceProvider {
    fn provider_id(&self) -> &str {
        "workspace"
    }

    async fn provide(&self, ctx: &SessionContext) -> Result<Value> {
        let workspace_ctx = WorkspaceContext {
            name: ctx.workspace.clone(),
            available: self.available_workspaces.clone(),
            cwd: ctx.cwd.clone(),
        };

        Ok(serde_json::to_value(workspace_ctx)?)
    }
}

/// Provider for learned principles.
pub struct PrinciplesProvider<F> {
    fetcher: F,
}

impl<F> PrinciplesProvider<F> {
    /// Create a new principles provider with the given fetcher function.
    pub fn new(fetcher: F) -> Self {
        Self { fetcher }
    }
}

#[async_trait]
impl<F, Fut> ContextProvider for PrinciplesProvider<F>
where
    F: Fn(&SessionContext) -> Fut + Send + Sync,
    Fut: std::future::Future<Output = Result<Vec<PrincipleContext>>> + Send,
{
    fn provider_id(&self) -> &str {
        "principles"
    }

    async fn provide(&self, ctx: &SessionContext) -> Result<Value> {
        let principles = (self.fetcher)(ctx).await?;
        Ok(serde_json::to_value(principles)?)
    }
}

/// Composite provider that aggregates multiple providers.
pub struct CompositeProvider {
    providers: Vec<Box<dyn ContextProvider>>,
}

impl CompositeProvider {
    /// Create a new composite provider.
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    /// Add a provider to the composite.
    pub fn add(&mut self, provider: Box<dyn ContextProvider>) {
        self.providers.push(provider);
    }
}

impl Default for CompositeProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ContextProvider for CompositeProvider {
    fn provider_id(&self) -> &str {
        "composite"
    }

    async fn provide(&self, ctx: &SessionContext) -> Result<Value> {
        let mut result = serde_json::Map::new();

        for provider in &self.providers {
            match provider.provide(ctx).await {
                Ok(value) => {
                    if let Some(obj) = value.as_object() {
                        for (k, v) in obj {
                            result.insert(k.clone(), v.clone());
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        provider = provider.provider_id(),
                        error = %e,
                        "context provider failed"
                    );
                }
            }
        }

        Ok(Value::Object(result))
    }
}

/// Builder for constructing BuildContext from SessionContext and providers.
pub struct ContextBuilder {
    providers: CompositeProvider,
    base_context: BuildContext,
}

impl ContextBuilder {
    /// Create a new context builder.
    pub fn new(agent_name: impl Into<String>) -> Self {
        Self {
            providers: CompositeProvider::new(),
            base_context: BuildContext {
                agent_name: agent_name.into(),
                ..Default::default()
            },
        }
    }

    /// Add a context provider.
    pub fn with_provider(mut self, provider: Box<dyn ContextProvider>) -> Self {
        self.providers.add(provider);
        self
    }

    /// Set the base persona.
    pub fn with_persona(mut self, persona: impl Into<String>) -> Self {
        self.base_context.persona = persona.into();
        self
    }

    /// Set the tool instructions.
    pub fn with_tool_instructions(mut self, instructions: impl Into<String>) -> Self {
        self.base_context.tool_instructions = instructions.into();
        self
    }

    /// Set the timezone.
    pub fn with_timezone(mut self, timezone: impl Into<String>) -> Self {
        self.base_context.timezone = timezone.into();
        self
    }

    /// Set the conversation summary.
    pub fn with_summary(mut self, summary: impl Into<String>) -> Self {
        self.base_context.conversation_summary = Some(summary.into());
        self
    }

    /// Build the final BuildContext.
    pub async fn build(self, session_ctx: &SessionContext) -> Result<BuildContext> {
        let mut ctx = self.base_context;

        // Set current datetime
        let now = chrono::Utc::now();
        ctx.datetime = now.to_rfc3339();

        // Apply providers
        let provider_data = self.providers.provide(session_ctx).await?;

        // Merge provider data into context
        if let Some(obj) = provider_data.as_object() {
            if let Some(workspace) = obj.get("workspace") {
                ctx.workspace = serde_json::from_value(workspace.clone()).unwrap_or_default();
            }
            if let Some(principles) = obj.get("principles") {
                ctx.principles = serde_json::from_value(principles.clone()).unwrap_or_default();
            }
        }

        Ok(ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_workspace_provider() {
        let provider = WorkspaceProvider::new(vec!["default".to_string(), "other".to_string()]);

        let session = SessionContext {
            workspace: "default".to_string(),
            cwd: Some("/home/user".to_string()),
            ..Default::default()
        };

        let value = provider.provide(&session).await.unwrap();
        let obj = value.as_object().unwrap();

        assert_eq!(obj.get("name").unwrap().as_str(), Some("default"));
        assert_eq!(obj.get("cwd").unwrap().as_str(), Some("/home/user"));
    }

    #[tokio::test]
    async fn test_composite_provider() {
        let mut composite = CompositeProvider::new();
        composite.add(Box::new(WorkspaceProvider::new(vec![
            "default".to_string(),
        ])));

        let session = SessionContext {
            workspace: "default".to_string(),
            ..Default::default()
        };

        let value = composite.provide(&session).await.unwrap();
        assert!(value.get("name").is_some());
    }

    #[tokio::test]
    async fn test_context_builder() {
        let builder = ContextBuilder::new("TestAgent")
            .with_persona("I am helpful.")
            .with_timezone("Europe/Rome");

        let session = SessionContext::default();
        let ctx = builder.build(&session).await.unwrap();

        assert_eq!(ctx.agent_name, "TestAgent");
        assert_eq!(ctx.persona, "I am helpful.");
        assert_eq!(ctx.timezone, "Europe/Rome");
        assert!(!ctx.datetime.is_empty());
    }
}
