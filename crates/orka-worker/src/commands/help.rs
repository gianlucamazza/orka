use async_trait::async_trait;
use orka_core::{CommandArgs, Envelope, OutboundMessage, Result, Session};

use super::ServerCommand;

/// Command that lists available commands and their usage (`/help [command]`).
pub struct HelpCommand {
    /// Snapshot of `(name, description, usage)` taken at registration time.
    entries: Vec<(String, String, String)>,
}

impl HelpCommand {
    /// Create the command with a snapshot of all registered command entries.
    pub fn new(entries: Vec<(String, String, String)>) -> Self {
        let mut entries = entries;
        // Add ourselves to the list.
        entries.push((
            "help".to_string(),
            "Show available commands".to_string(),
            "/help [command]".to_string(),
        ));
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        Self { entries }
    }

    fn make_reply(envelope: &Envelope, text: String) -> OutboundMessage {
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
        msg
    }
}

#[async_trait]
impl ServerCommand for HelpCommand {
    fn name(&self) -> &'static str {
        "help"
    }
    fn description(&self) -> &'static str {
        "Show available commands"
    }
    fn usage(&self) -> &'static str {
        "/help [command]"
    }

    async fn execute(
        &self,
        args: &CommandArgs,
        envelope: &Envelope,
        _session: &Session,
    ) -> Result<Vec<OutboundMessage>> {
        if let Some(cmd_name) = args.positional(0) {
            // Show detailed help for a specific command.
            if let Some((_, desc, usage)) =
                self.entries.iter().find(|(name, _, _)| name == cmd_name)
            {
                let text = format!("**/{cmd_name}**\n{desc}\n\nUsage: `{usage}`");
                return Ok(vec![Self::make_reply(envelope, text)]);
            }
            return Ok(vec![Self::make_reply(
                envelope,
                format!("Unknown command: /{cmd_name}\n\n{}", self.full_help()),
            )]);
        }

        Ok(vec![Self::make_reply(envelope, self.full_help())])
    }
}

impl HelpCommand {
    fn full_help(&self) -> String {
        let mut lines = vec!["**Available commands:**".to_string()];
        for (name, desc, _) in &self.entries {
            lines.push(format!("• **/{name}** — {desc}"));
        }
        lines.join("\n")
    }
}
