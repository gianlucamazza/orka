use std::sync::Arc;

use async_trait::async_trait;
use orka_core::{
    CommandArgs, Envelope, Error, OutboundMessage, Result, Session, config::AgentConfig,
};
use orka_workspace::WorkspaceRegistry;

use super::ServerCommand;

/// Command that prints agent status information (`/status`).
pub struct StatusCommand {
    workspace_registry: Arc<WorkspaceRegistry>,
    agent_config: AgentConfig,
}

impl StatusCommand {
    /// Create the command with access to the workspace registry and agent
    /// config.
    pub fn new(workspace_registry: Arc<WorkspaceRegistry>, agent_config: AgentConfig) -> Self {
        Self {
            workspace_registry,
            agent_config,
        }
    }
}

#[async_trait]
impl ServerCommand for StatusCommand {
    fn name(&self) -> &'static str {
        "status"
    }
    fn description(&self) -> &'static str {
        "Show session info"
    }
    fn usage(&self) -> &'static str {
        "/status"
    }

    async fn execute(
        &self,
        _args: &CommandArgs,
        envelope: &Envelope,
        session: &Session,
    ) -> Result<Vec<OutboundMessage>> {
        let state_lock = self
            .workspace_registry
            .default_state()
            .await
            .ok_or_else(|| Error::Workspace("default workspace not registered".into()))?;
        let state = state_lock.read().await;

        let model = self.agent_config.model.as_str();

        let name = state
            .soul
            .as_ref()
            .and_then(|s| s.frontmatter.name.as_deref())
            .unwrap_or("(unnamed)");

        let workspace_name = self.workspace_registry.default_name();
        let workspaces = self.workspace_registry.list_names().await.join(", ");

        let text = format!(
            "**Session status**\n\
            **Session ID:** `{}`\n\
            **Channel:** {}\n\
            **User:** {}\n\
            **Agent:** {name}\n\
            **Model:** `{model}`\n\
            **Workspace:** {workspace_name}\n\
            **Available workspaces:** {workspaces}",
            session.id, session.channel, session.user_id
        );

        let mut msg = OutboundMessage::text(
            envelope.channel.clone(),
            envelope.session_id,
            text,
            Some(envelope.id),
        );
        msg.metadata.clone_from(&envelope.metadata);
        envelope
            .platform_context
            .clone_into(&mut msg.platform_context);
        Ok(vec![msg])
    }
}
