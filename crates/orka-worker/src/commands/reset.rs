use std::sync::Arc;

use async_trait::async_trait;
use orka_core::traits::MemoryStore;
use orka_core::{Envelope, MemoryEntry, OutboundMessage, Result, Session};

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
    fn name(&self) -> &str {
        "reset"
    }
    fn description(&self) -> &str {
        "Clear conversation history"
    }
    fn usage(&self) -> &str {
        "/reset"
    }

    async fn execute(
        &self,
        _args: &[String],
        envelope: &Envelope,
        _session: &Session,
    ) -> Result<Vec<OutboundMessage>> {
        let history_key = format!("conversation:{}", envelope.session_id);
        let token_key = format!("tokens:{}", envelope.session_id);

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

        let mut msg = OutboundMessage::text(
            envelope.channel.clone(),
            envelope.session_id,
            "Conversation history cleared.",
            Some(envelope.id),
        );
        msg.metadata = envelope.metadata.clone();
        Ok(vec![msg])
    }
}
