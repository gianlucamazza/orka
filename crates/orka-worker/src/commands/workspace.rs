use std::sync::Arc;

use async_trait::async_trait;
use orka_core::{
    CommandArgs, Envelope, MemoryEntry, MemoryScope, OutboundMessage, Result, Session,
    traits::MemoryStore,
};
use orka_workspace::WorkspaceRegistry;

use super::ServerCommand;

/// Command to list and switch workspaces (`/workspace [list|<name>|reset]`).
pub struct WorkspaceCommand {
    workspace_registry: Arc<WorkspaceRegistry>,
    memory: Arc<dyn MemoryStore>,
}

impl WorkspaceCommand {
    /// Create the command with access to the workspace registry and memory
    /// store.
    pub fn new(workspace_registry: Arc<WorkspaceRegistry>, memory: Arc<dyn MemoryStore>) -> Self {
        Self {
            workspace_registry,
            memory,
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
        msg
    }

    async fn current_workspace_name(&self, session: &Session) -> Option<String> {
        let override_key = format!("workspace_override:{}", session.id);
        let entry = self.memory.recall(&override_key).await.ok().flatten()?;
        entry
            .value
            .get("workspace_name")
            .and_then(|v| v.as_str())
            .map(str::to_string)
    }
}

#[async_trait]
impl ServerCommand for WorkspaceCommand {
    fn name(&self) -> &'static str {
        "workspace"
    }
    fn description(&self) -> &'static str {
        "List or switch workspaces"
    }
    fn usage(&self) -> &'static str {
        "/workspace [list|<name>|reset]"
    }

    async fn execute(
        &self,
        args: &CommandArgs,
        envelope: &Envelope,
        session: &Session,
    ) -> Result<Vec<OutboundMessage>> {
        let subcommand = args.positional(0).unwrap_or("list");

        match subcommand {
            "list" => {
                let current = self
                    .current_workspace_name(session)
                    .await
                    .unwrap_or_else(|| self.workspace_registry.default_name().to_string());
                let names = self.workspace_registry.list_names();
                let mut lines = vec!["**Available workspaces:**".to_string()];
                for name in &names {
                    if *name == current {
                        lines.push(format!("• **{name}** ✓ (current)"));
                    } else {
                        lines.push(format!("• {name}"));
                    }
                }
                lines.push("\nSwitch with: `/workspace <name>`".to_string());
                Ok(vec![Self::make_reply(envelope, lines.join("\n"))])
            }
            "reset" => {
                let override_key = format!("workspace_override:{}", session.id);
                self.memory
                    .store(
                        &override_key,
                        MemoryEntry::working(override_key.clone(), serde_json::json!({}))
                            .with_scope(MemoryScope::Session)
                            .with_source("workspace_command")
                            .with_metadata(std::collections::HashMap::from([(
                                "session_id".into(),
                                session.id.to_string(),
                            )])),
                        None,
                    )
                    .await?;
                let default = self.workspace_registry.default_name();
                Ok(vec![Self::make_reply(
                    envelope,
                    format!("Workspace reset to default: **{default}**"),
                )])
            }
            name => {
                // Try to switch to the named workspace.
                if self.workspace_registry.get(name).is_none() {
                    let available = self
                        .workspace_registry
                        .list_names()
                        .into_iter()
                        .map(|s| format!("`{s}`"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    return Ok(vec![Self::make_reply(
                        envelope,
                        format!("Unknown workspace: **{name}**\nAvailable: {available}"),
                    )]);
                }
                let override_key = format!("workspace_override:{}", session.id);
                self.memory
                    .store(
                        &override_key,
                        MemoryEntry::working(
                            override_key.clone(),
                            serde_json::json!({ "workspace_name": name }),
                        )
                        .with_scope(MemoryScope::Session)
                        .with_source("workspace_command")
                        .with_metadata(std::collections::HashMap::from([(
                            "session_id".into(),
                            session.id.to_string(),
                        )])),
                        None,
                    )
                    .await?;
                Ok(vec![Self::make_reply(
                    envelope,
                    format!("Switched to workspace: **{name}**"),
                )])
            }
        }
    }
}
