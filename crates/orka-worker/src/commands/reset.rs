use std::sync::Arc;

use async_trait::async_trait;
use orka_core::{
    CommandArgs, Envelope, MemoryEntry, OutboundMessage, Result, Session, traits::MemoryStore,
};

use super::ServerCommand;

/// Command that clears the session's memory store (`/reset`).
pub struct ResetCommand {
    memory: Arc<dyn MemoryStore>,
}

impl ResetCommand {
    /// Create the command backed by the given memory store.
    pub fn new(memory: Arc<dyn MemoryStore>) -> Self {
        Self { memory }
    }
}

#[async_trait]
impl ServerCommand for ResetCommand {
    fn name(&self) -> &'static str {
        "reset"
    }
    fn description(&self) -> &'static str {
        "Clear conversation history"
    }
    fn usage(&self) -> &'static str {
        "/reset"
    }

    async fn execute(
        &self,
        _args: &CommandArgs,
        envelope: &Envelope,
        _session: &Session,
    ) -> Result<Vec<OutboundMessage>> {
        let history_key = format!("conversation:{}", envelope.session_id);
        let token_key = format!("tokens:{}", envelope.session_id);
        let summary_key = format!("conversation_summary:{}", envelope.session_id);
        let override_key = format!("workspace_override:{}", envelope.session_id);

        self.memory
            .store(
                &history_key,
                MemoryEntry::new(history_key.clone(), serde_json::json!([])),
                None,
            )
            .await?;

        self.memory
            .store(
                &token_key,
                MemoryEntry::new(
                    token_key.clone(),
                    serde_json::json!({"input": 0, "output": 0}),
                ),
                None,
            )
            .await?;

        self.memory
            .store(
                &summary_key,
                MemoryEntry::new(summary_key.clone(), serde_json::json!("")),
                None,
            )
            .await?;

        self.memory
            .store(
                &override_key,
                MemoryEntry::new(override_key.clone(), serde_json::json!({})),
                None,
            )
            .await?;

        let mut msg = OutboundMessage::text(
            envelope.channel.clone(),
            envelope.session_id,
            "Conversation history, token counters, and workspace override cleared.",
            Some(envelope.id),
        );
        msg.metadata.clone_from(&envelope.metadata);
        envelope
            .platform_context
            .clone_into(&mut msg.platform_context);
        Ok(vec![msg])
    }
}
