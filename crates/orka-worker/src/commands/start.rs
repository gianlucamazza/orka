use std::sync::Arc;

use async_trait::async_trait;
use orka_core::{CommandArgs, Envelope, Error, OutboundMessage, Result, Session, config::AgentConfig};
use orka_workspace::WorkspaceRegistry;

use super::ServerCommand;

/// Command sent automatically by Telegram when a user opens the chat for the
/// first time (`/start`).
pub struct StartCommand {
    workspace_registry: Arc<WorkspaceRegistry>,
    agent_config: AgentConfig,
}

impl StartCommand {
    /// Create the command with access to workspace and agent config.
    pub fn new(workspace_registry: Arc<WorkspaceRegistry>, agent_config: AgentConfig) -> Self {
        Self {
            workspace_registry,
            agent_config,
        }
    }
}

#[async_trait]
impl ServerCommand for StartCommand {
    fn name(&self) -> &'static str {
        "start"
    }
    fn description(&self) -> &'static str {
        "Start a conversation"
    }
    fn usage(&self) -> &'static str {
        "/start"
    }

    async fn execute(
        &self,
        _args: &CommandArgs,
        envelope: &Envelope,
        _session: &Session,
    ) -> Result<Vec<OutboundMessage>> {
        let state_lock = self
            .workspace_registry
            .default_state()
            .ok_or_else(|| Error::Workspace("default workspace not registered".into()))?;
        let state = state_lock.read().await;

        let agent_name = state
            .soul
            .as_ref()
            .and_then(|s| s.frontmatter.name.as_deref())
            .unwrap_or("Orka Agent");

        let description = state
            .soul
            .as_ref()
            .and_then(|s| s.frontmatter.description.as_deref())
            .unwrap_or("an AI assistant");

        let model = self.agent_config.model.as_str();

        let text = format!(
            "👋 Welcome! I'm **{agent_name}** — {description}.\n\n\
            Model: `{model}`\n\n\
            **Commands:**\n\
            • **/help** — List all available commands\n\
            • **/status** — Show session info\n\
            • **/skills** — List available skills\n\
            • **/reset** — Clear conversation history\n\n\
            Just send me a message to get started!"
        );

        let mut msg = OutboundMessage::text(
            envelope.channel.clone(),
            envelope.session_id,
            text,
            Some(envelope.id),
        );
        msg.metadata.clone_from(&envelope.metadata);
        Ok(vec![msg])
    }
}
