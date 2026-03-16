use async_trait::async_trait;
use orka_core::{Envelope, OutboundMessage, Payload, Result, Session};
use std::collections::HashMap;

#[async_trait]
pub trait AgentHandler: Send + Sync + 'static {
    async fn handle(&self, envelope: &Envelope, session: &Session) -> Result<Vec<OutboundMessage>>;
}

/// Echo handler for testing - echoes back the text payload.
pub struct EchoHandler;

#[async_trait]
impl AgentHandler for EchoHandler {
    async fn handle(
        &self,
        envelope: &Envelope,
        _session: &Session,
    ) -> Result<Vec<OutboundMessage>> {
        let reply_text = match &envelope.payload {
            Payload::Text(t) => format!("echo: {t}"),
            _ => "echo: [non-text payload]".into(),
        };

        Ok(vec![OutboundMessage {
            channel: envelope.channel.clone(),
            session_id: envelope.session_id.clone(),
            payload: Payload::Text(reply_text),
            reply_to: Some(envelope.id.clone()),
            metadata: HashMap::new(),
        }])
    }
}
