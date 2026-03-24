use async_trait::async_trait;
use orka_core::Result;
use serde_json::Value;

use super::types::{PrincipleContext, SessionContext, WorkspaceContext};

/// Trait for context providers that supply data for prompt building.
///
/// Implementors can fetch data from various sources (memory, external APIs,
/// etc.) and provide it to the prompt building pipeline.
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
}
