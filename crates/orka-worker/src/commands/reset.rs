use std::sync::Arc;

use async_trait::async_trait;
use orka_core::traits::MemoryStore;
use orka_core::{Envelope, MemoryEntry, OutboundMessage, Payload, Result, Session};

use super::ServerCommand;

pub struct ResetCommand {
    memory: Arc<dyn MemoryStore>,
}

impl ResetCommand {
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
        let now = chrono::Utc::now();

        self.memory
            .store(
                &history_key,
                MemoryEntry {
                    key: history_key.clone(),
                    value: serde_json::json!([]),
                    created_at: now,
                    updated_at: now,
                    tags: vec![],
                },
                None,
            )
            .await?;

        self.memory
            .store(
                &token_key,
                MemoryEntry {
                    key: token_key.clone(),
                    value: serde_json::json!({"input": 0, "output": 0}),
                    created_at: now,
                    updated_at: now,
                    tags: vec![],
                },
                None,
            )
            .await?;

        Ok(vec![OutboundMessage {
            channel: envelope.channel.clone(),
            session_id: envelope.session_id.clone(),
            payload: Payload::Text("Conversation history cleared.".to_string()),
            reply_to: Some(envelope.id.clone()),
            metadata: envelope.metadata.clone(),
        }])
    }
}
