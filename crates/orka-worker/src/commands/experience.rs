use std::sync::Arc;

use async_trait::async_trait;
use orka_core::{CommandArgs, Envelope, OutboundMessage, Result, Session};
use orka_experience::ExperienceService;
use orka_workspace::WorkspaceRegistry;

use super::ServerCommand;

/// Command to inspect and manage the self-learning experience system (`/experience`).
pub struct ExperienceCommand {
    experience: Arc<ExperienceService>,
    workspace_registry: Arc<WorkspaceRegistry>,
}

impl ExperienceCommand {
    /// Create the command.  Returns `None` when the experience system is disabled.
    pub fn new_if_enabled(
        experience: Arc<ExperienceService>,
        workspace_registry: Arc<WorkspaceRegistry>,
    ) -> Option<Self> {
        if experience.is_enabled() {
            Some(Self {
                experience,
                workspace_registry,
            })
        } else {
            None
        }
    }

    fn make_reply(&self, envelope: &Envelope, text: String) -> OutboundMessage {
        let mut msg = OutboundMessage::text(
            envelope.channel.clone(),
            envelope.session_id,
            text,
            Some(envelope.id),
        );
        msg.metadata = envelope.metadata.clone();
        msg
    }

    fn usage_text() -> &'static str {
        "**Usage:**\n\
        • `/experience` — Show this help\n\
        • `/experience status` — Show system status\n\
        • `/experience principles <query>` — Search learned principles\n\
        • `/experience distill` — Trigger offline distillation\n\n\
        _Note: reflection happens automatically after each task._"
    }
}

#[async_trait]
impl ServerCommand for ExperienceCommand {
    fn name(&self) -> &str {
        "experience"
    }
    fn description(&self) -> &str {
        "Inspect the self-learning experience system"
    }
    fn usage(&self) -> &str {
        "/experience [status|principles <query>|distill]"
    }

    async fn execute(
        &self,
        args: &CommandArgs,
        envelope: &Envelope,
        _session: &Session,
    ) -> Result<Vec<OutboundMessage>> {
        let subcommand = args.positional(0).unwrap_or("help");

        let text = match subcommand {
            "status" => {
                let workspace = self.workspace_registry.default_name();
                format!(
                    "**Experience system**\n\
                    **Status:** enabled ✓\n\
                    **Default workspace:** {workspace}"
                )
            }
            "principles" => {
                let query = args
                    .positional_args()
                    .iter()
                    .skip(1)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" ");
                if query.is_empty() {
                    return Ok(vec![
                        self.make_reply(
                            envelope,
                            "Usage: `/experience principles <query>`\n\
                        Example: `/experience principles error handling`"
                                .to_string(),
                        ),
                    ]);
                }
                let workspace = self.workspace_registry.default_name();
                match self.experience.retrieve_principles(&query, workspace).await {
                    Ok(principles) if principles.is_empty() => {
                        format!("No principles found for: _{query}_")
                    }
                    Ok(principles) => {
                        let mut lines = vec![format!("**Principles matching _{query}_:**\n")];
                        for (i, p) in principles.iter().enumerate() {
                            lines.push(format!("{}. {}", i + 1, p.text));
                        }
                        lines.join("\n")
                    }
                    Err(e) => format!("Error retrieving principles: {e}"),
                }
            }
            "distill" => {
                let workspace = self.workspace_registry.default_name();
                match self.experience.distill(workspace).await {
                    Ok(0) => "No new principles distilled (no recent trajectories).".to_string(),
                    Ok(n) => format!("Distillation complete: {n} principle(s) created or updated."),
                    Err(e) => format!("Distillation error: {e}"),
                }
            }
            _ => Self::usage_text().to_string(),
        };

        Ok(vec![self.make_reply(envelope, text)])
    }
}
