use std::sync::Arc;

use async_trait::async_trait;
use orka_core::{Envelope, OutboundMessage, Payload, Result, Session};
use orka_workspace::state::WorkspaceState;
use tokio::sync::RwLock;

use super::ServerCommand;

pub struct StatusCommand {
    workspace_state: Arc<RwLock<WorkspaceState>>,
}

impl StatusCommand {
    pub fn new(workspace_state: Arc<RwLock<WorkspaceState>>) -> Self {
        Self { workspace_state }
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
        let state = self.workspace_state.read().await;

        let model = state
            .soul
            .as_ref()
            .and_then(|s| s.frontmatter.model.as_deref())
            .unwrap_or("(default)");

        let name = state
            .soul
            .as_ref()
            .and_then(|s| s.frontmatter.name.as_deref())
            .unwrap_or("(unnamed)");

        let text = format!(
            "Session status:\n  Session ID: {}\n  Channel: {}\n  User: {}\n  Agent: {name}\n  Model: {model}",
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
