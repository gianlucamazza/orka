use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use orka_core::{
    CommandArgs, Envelope, MemoryEntry, MemoryKind, MemoryScope, OutboundMessage, Result, Session,
    traits::MemoryStore,
};
use orka_experience::ExperienceService;
use orka_knowledge::FactStore;
use orka_workspace::WorkspaceRegistry;

use super::ServerCommand;

/// Command for inspecting and managing memory layers (`/memory`).
pub struct MemoryCommand {
    memory: Arc<dyn MemoryStore>,
    facts: Option<Arc<FactStore>>,
    experience: Option<Arc<ExperienceService>>,
    workspace_registry: Arc<WorkspaceRegistry>,
}

impl MemoryCommand {
    /// Create the command backed by the configured memory services.
    pub fn new(
        memory: Arc<dyn MemoryStore>,
        facts: Option<Arc<FactStore>>,
        experience: Option<Arc<ExperienceService>>,
        workspace_registry: Arc<WorkspaceRegistry>,
    ) -> Self {
        Self {
            memory,
            facts,
            experience,
            workspace_registry,
        }
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

    fn usage_text() -> String {
        "**Usage:**\n\
        • `/memory status` — Show counts per memory layer\n\
        • `/memory list <working|episodic|semantic|procedural>` — List records\n\
        • `/memory forget <working|episodic|semantic|procedural> <id>` — Delete one record\n\
        • `/memory clear` — Clear current session working + episodic memory"
            .to_string()
    }

    fn current_workspace(&self, envelope: &Envelope) -> String {
        envelope
            .metadata
            .get("workspace:name")
            .and_then(|v| v.as_str())
            .unwrap_or(self.workspace_registry.default_name())
            .to_string()
    }

    #[allow(clippy::unused_self)]
    fn current_session_entries(
        &self,
        entries: Vec<MemoryEntry>,
        session_id: &str,
    ) -> Vec<MemoryEntry> {
        entries
            .into_iter()
            .filter(|entry| {
                entry
                    .metadata
                    .get("session_id")
                    .is_some_and(|sid| sid == session_id)
                    || entry.key.ends_with(session_id)
            })
            .collect()
    }
}

#[async_trait]
#[allow(clippy::too_many_lines)]
impl ServerCommand for MemoryCommand {
    fn name(&self) -> &'static str {
        "memory"
    }

    fn description(&self) -> &'static str {
        "Inspect and manage working, episodic, semantic, and procedural memory"
    }

    fn usage(&self) -> &'static str {
        "/memory [status|list <layer>|forget <layer> <id>|clear]"
    }

    async fn execute(
        &self,
        args: &CommandArgs,
        envelope: &Envelope,
        _session: &Session,
    ) -> Result<Vec<OutboundMessage>> {
        let subcommand = args.positional(0).unwrap_or("status");
        let session_id = envelope.session_id.to_string();
        let workspace = self.current_workspace(envelope);

        let text = match subcommand {
            "status" => {
                let session_entries =
                    self.current_session_entries(self.memory.list(None, 256).await?, &session_id);
                let working = session_entries
                    .iter()
                    .filter(|entry| entry.kind == MemoryKind::Working)
                    .count();
                let episodic = session_entries
                    .iter()
                    .filter(|entry| entry.kind == MemoryKind::Episodic)
                    .count();

                let semantic = if let Some(facts) = &self.facts {
                    facts
                        .list(256, None)
                        .await?
                        .into_iter()
                        .filter(|fact| match fact.scope {
                            MemoryScope::Session => fact
                                .metadata
                                .get("session_id")
                                .is_some_and(|sid| sid == &session_id),
                            MemoryScope::Workspace => fact
                                .metadata
                                .get("workspace")
                                .is_some_and(|ws| ws == &workspace),
                            MemoryScope::Global => true,
                            // User-scoped and any future scopes are excluded from this listing
                            _ => false,
                        })
                        .count()
                } else {
                    0
                };

                let procedural = if let Some(exp) = &self.experience {
                    exp.list_principles(&workspace, 256).await?.len()
                } else {
                    0
                };

                format!(
                    "**Memory status**\n\
                    Session: `{session_id}`\n\
                    Workspace: `{workspace}`\n\n\
                    • Working: {working}\n\
                    • Episodic: {episodic}\n\
                    • Semantic: {semantic}\n\
                    • Procedural: {procedural}"
                )
            }
            "list" => match args.positional(1).unwrap_or("") {
                "working" | "episodic" => {
                    let target_kind = if args.positional(1) == Some("working") {
                        MemoryKind::Working
                    } else {
                        MemoryKind::Episodic
                    };
                    let entries = self
                        .current_session_entries(self.memory.list(None, 64).await?, &session_id)
                        .into_iter()
                        .filter(|entry| entry.kind == target_kind)
                        .collect::<Vec<_>>();
                    if entries.is_empty() {
                        format!("No {target_kind} memory entries for this session.")
                    } else {
                        let mut lines = vec![format!("**{} memory**", target_kind)];
                        for entry in entries {
                            lines.push(format!("• `{}`", entry.key));
                        }
                        lines.join("\n")
                    }
                }
                "semantic" => {
                    if let Some(facts) = &self.facts {
                        let facts = facts.list(32, None).await?;
                        if facts.is_empty() {
                            "No semantic facts stored.".to_string()
                        } else {
                            let mut lines = vec!["**Semantic memory**".to_string()];
                            for fact in facts {
                                lines.push(format!(
                                    "• `{}` [{}] {}",
                                    fact.id, fact.scope, fact.content
                                ));
                            }
                            lines.join("\n")
                        }
                    } else {
                        "Semantic memory is not enabled.".to_string()
                    }
                }
                "procedural" => {
                    if let Some(exp) = &self.experience {
                        let principles = exp.list_principles(&workspace, 32).await?;
                        if principles.is_empty() {
                            "No procedural memory stored.".to_string()
                        } else {
                            let mut lines = vec!["**Procedural memory**".to_string()];
                            for principle in principles {
                                lines.push(format!("• `{}` {}", principle.id, principle.text));
                            }
                            lines.join("\n")
                        }
                    } else {
                        "Procedural memory is not enabled.".to_string()
                    }
                }
                _ => Self::usage_text(),
            },
            "forget" => {
                let Some(layer) = args.positional(1) else {
                    return Ok(vec![Self::make_reply(envelope, Self::usage_text())]);
                };
                let Some(id) = args.positional(2) else {
                    return Ok(vec![Self::make_reply(envelope, Self::usage_text())]);
                };
                match layer {
                    "working" | "episodic" => {
                        let deleted = self.memory.delete(id).await?;
                        if deleted {
                            format!("Deleted `{id}` from {layer} memory.")
                        } else {
                            format!("No {layer} memory entry found for `{id}`.")
                        }
                    }
                    "semantic" => {
                        if let Some(facts) = &self.facts {
                            let deleted = facts
                                .forget(HashMap::from([("id".into(), id.to_string())]))
                                .await?;
                            if deleted > 0 {
                                format!("Deleted semantic fact `{id}`.")
                            } else {
                                format!("No semantic fact found for `{id}`.")
                            }
                        } else {
                            "Semantic memory is not enabled.".to_string()
                        }
                    }
                    "procedural" => {
                        if let Some(exp) = &self.experience {
                            if exp.forget_principle(id).await? {
                                format!("Deleted procedural memory `{id}`.")
                            } else {
                                format!("No procedural memory found for `{id}`.")
                            }
                        } else {
                            "Procedural memory is not enabled.".to_string()
                        }
                    }
                    _ => Self::usage_text(),
                }
            }
            "clear" => {
                let history_key = format!("conversation:{}", envelope.session_id);
                let token_key = format!("tokens:{}", envelope.session_id);
                let summary_key = format!("conversation_summary:{}", envelope.session_id);
                let override_key = format!("workspace_override:{}", envelope.session_id);

                let _ = self.memory.delete(&history_key).await?;
                let _ = self.memory.delete(&token_key).await?;
                let _ = self.memory.delete(&summary_key).await?;
                let _ = self.memory.delete(&override_key).await?;

                "Current session working and episodic memory cleared.".to_string()
            }
            _ => Self::usage_text(),
        };

        Ok(vec![Self::make_reply(envelope, text)])
    }
}
