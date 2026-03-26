use async_trait::async_trait;
use orka_core::{CommandArgs, Envelope, OutboundMessage, Result, Session};

use super::ServerCommand;

/// Command to request cancellation of the session's current operation
/// (`/cancel`).
///
/// The actual cancellation is handled at the worker-pool level, which
/// intercepts `/cancel` before acquiring the session lock.  This command exists
/// so that `/cancel` appears in `/help` output.
pub struct CancelCommand;

impl CancelCommand {
    /// Create the command.
    pub fn new() -> Self {
        Self
    }
}

impl Default for CancelCommand {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ServerCommand for CancelCommand {
    fn name(&self) -> &'static str {
        "cancel"
    }
    fn description(&self) -> &'static str {
        "Cancel the current operation"
    }
    fn usage(&self) -> &'static str {
        "/cancel"
    }

    async fn execute(
        &self,
        _args: &CommandArgs,
        envelope: &Envelope,
        _session: &Session,
    ) -> Result<Vec<OutboundMessage>> {
        // If we reach here the worker pool did not intercept the command (no active
        // operation).
        let mut msg = OutboundMessage::text(
            envelope.channel.clone(),
            envelope.session_id,
            "No active operation to cancel.".to_string(),
            Some(envelope.id),
        );
        msg.metadata.clone_from(&envelope.metadata);
        Ok(vec![msg])
    }
}
