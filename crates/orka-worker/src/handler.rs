use async_trait::async_trait;
use orka_core::{Envelope, OutboundMessage, Payload, Result, Session};

/// Handler that processes an inbound envelope and produces outbound messages.
#[async_trait]
pub trait AgentHandler: Send + Sync + 'static {
    /// Process an envelope within a session, returning zero or more replies.
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

        Ok(vec![OutboundMessage::text(
            envelope.channel.clone(),
            envelope.session_id,
            reply_text,
            Some(envelope.id),
        )])
    }
}
