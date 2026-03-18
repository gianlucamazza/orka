/// `/reset` command — clears session memory.
pub mod reset;
/// `/skill` command — invokes a named skill directly.
pub mod skill;
/// `/skills` command — lists registered skills.
pub mod skills;
/// `/status` command — shows agent configuration and workspace info.
pub mod status;

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use orka_core::config::AgentConfig;
use orka_core::traits::{MemoryStore, SecretManager};
use orka_core::{Envelope, OutboundMessage, Result, Session};
use orka_skills::SkillRegistry;
use orka_workspace::WorkspaceRegistry;

/// A slash command that can be invoked by users (e.g. `/reset`, `/skills`).
#[async_trait]
pub trait ServerCommand: Send + Sync {
    /// The command keyword (without the leading `/`).
    fn name(&self) -> &str;
    /// One-line description shown in `/help`.
    fn description(&self) -> &str;
    /// Usage string shown in `/help <command>`.
    fn usage(&self) -> &str;
    /// Execute the command with the parsed argument tokens.
    async fn execute(
        &self,
        args: &[String],
        envelope: &Envelope,
        session: &Session,
    ) -> Result<Vec<OutboundMessage>>;
}

/// Registry of named slash commands.
pub struct CommandRegistry {
    commands: HashMap<String, Arc<dyn ServerCommand>>,
}

impl CommandRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            commands: HashMap::new(),
        }
    }

    /// Register a command, replacing any existing command with the same name.
    pub fn register(&mut self, cmd: Arc<dyn ServerCommand>) {
        self.commands.insert(cmd.name().to_string(), cmd);
    }

    /// Look up a command by name.
    pub fn get(&self, name: &str) -> Option<&Arc<dyn ServerCommand>> {
        self.commands.get(name)
    }

    /// Return `(name, description)` pairs for all registered commands, sorted by name.
    pub fn list(&self) -> Vec<(&str, &str)> {
        let mut items: Vec<_> = self
            .commands
            .values()
            .map(|c| (c.name(), c.description()))
            .collect();
        items.sort_by_key(|(name, _)| *name);
        items
    }

    /// Build a formatted help string listing all registered commands.
    pub fn help_text(&self) -> String {
        let mut lines = vec!["Available server commands:".to_string()];
        for (name, desc) in self.list() {
            lines.push(format!("  /{name} — {desc}"));
        }
        lines.join("\n")
    }
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Register all built-in server commands.
pub fn register_all(
    registry: &mut CommandRegistry,
    skills: Arc<SkillRegistry>,
    memory: Arc<dyn MemoryStore>,
    secrets: Arc<dyn SecretManager>,
    workspace_registry: Arc<WorkspaceRegistry>,
    agent_config: &AgentConfig,
) {
    registry.register(Arc::new(skill::SkillCommand::new(skills.clone(), secrets)));
    registry.register(Arc::new(skills::SkillsCommand::new(skills)));
    registry.register(Arc::new(reset::ResetCommand::new(memory)));
    registry.register(Arc::new(status::StatusCommand::new(
        workspace_registry,
        agent_config.clone(),
    )));
}
