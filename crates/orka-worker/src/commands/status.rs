use std::sync::Arc;

use async_trait::async_trait;
use orka_core::config::AgentConfig;
use orka_core::{Envelope, OutboundMessage, Payload, Result, Session};
use orka_workspace::WorkspaceRegistry;

use super::ServerCommand;

pub struct StatusCommand {
    workspace_registry: Arc<WorkspaceRegistry>,
    agent_config: AgentConfig,
}

impl StatusCommand {
    pub fn new(workspace_registry: Arc<WorkspaceRegistry>, agent_config: AgentConfig) -> Self {
        Self {
            workspace_registry,
            agent_config,
        }
    }
}

#[async_trait]
impl ServerCommand for StatusCommand {
    fn name(&self) -> &str {
        "status"
    }
    fn description(&self) -> &str {
        "Show session info"
    }
    fn usage(&self) -> &str {
        "/status"
    }

    async fn execute(
        &self,
        _args: &[String],
        envelope: &Envelope,
        session: &Session,
    ) -> Result<Vec<OutboundMessage>> {
        let state = self.workspace_registry.default_state();
        let state = state.read().await;

        let model = self.agent_config.model.as_deref().unwrap_or("(default)");

        let name = state
            .soul
            .as_ref()
            .and_then(|s| s.frontmatter.name.as_deref())
            .unwrap_or("(unnamed)");

        let workspace_name = self.workspace_registry.default_name();
        let workspaces = self.workspace_registry.list_names().join(", ");

        let text = format!(
            "Session status:\n  Session ID: {}\n  Channel: {}\n  User: {}\n  Agent: {name}\n  Model: {model}\n  Workspace: {workspace_name}\n  Available workspaces: {workspaces}",
            session.id, session.channel, session.user_id
        );

        Ok(vec![OutboundMessage {
            channel: envelope.channel.clone(),
            session_id: envelope.session_id.clone(),
            payload: Payload::Text(text),
            reply_to: Some(envelope.id.clone()),
            metadata: envelope.metadata.clone(),
        }])
    }
}
