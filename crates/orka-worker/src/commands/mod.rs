/// `/cancel` command — abort the current operation.
pub mod cancel;
/// `/experience` command — inspect the self-learning system.
pub mod experience;
/// `/help` command — shows available commands and usage.
pub mod help;
/// `/reset` command — clears session memory.
pub mod reset;
/// `/skill` command — invokes a named skill directly.
pub mod skill;
/// `/skills` command — lists registered skills.
pub mod skills;
/// `/start` command — welcome message for new users.
pub mod start;
/// `/status` command — shows agent configuration and workspace info.
pub mod status;
/// `/workspace` command — list or switch workspaces.
pub mod workspace;

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use orka_core::config::AgentConfig;
use orka_core::traits::{MemoryStore, SecretManager};
use orka_core::{CommandArgs, Envelope, OutboundMessage, Result, Session};
use orka_experience::ExperienceService;
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
    /// Execute the command with the parsed arguments.
    async fn execute(
        &self,
        args: &CommandArgs,
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
    experience: Option<Arc<ExperienceService>>,
) {
    registry.register(Arc::new(cancel::CancelCommand::new()));
    registry.register(Arc::new(skill::SkillCommand::new(skills.clone(), secrets)));
    registry.register(Arc::new(skills::SkillsCommand::new(skills)));
    registry.register(Arc::new(reset::ResetCommand::new(memory.clone())));
    registry.register(Arc::new(status::StatusCommand::new(
        workspace_registry.clone(),
        agent_config.clone(),
    )));
    registry.register(Arc::new(start::StartCommand::new(
        workspace_registry.clone(),
        agent_config.clone(),
    )));
    registry.register(Arc::new(workspace::WorkspaceCommand::new(
        workspace_registry.clone(),
        memory,
    )));

    // `/experience` is only registered when the experience system is enabled.
    if let Some(exp) = experience
        && let Some(cmd) = experience::ExperienceCommand::new_if_enabled(exp, workspace_registry)
    {
        registry.register(Arc::new(cmd));
    }

    // `/help` must be registered last so its snapshot includes all other commands.
    let entries: Vec<(String, String, String)> = registry
        .list()
        .into_iter()
        .map(|(name, desc)| {
            let usage = registry
                .get(name)
                .map(|c| c.usage().to_string())
                .unwrap_or_default();
            (name.to_string(), desc.to_string(), usage)
        })
        .collect();
    registry.register(Arc::new(help::HelpCommand::new(entries)));
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockCommand {
        cmd_name: &'static str,
        cmd_desc: &'static str,
    }

    #[async_trait]
    impl ServerCommand for MockCommand {
        fn name(&self) -> &str {
            self.cmd_name
        }
        fn description(&self) -> &str {
            self.cmd_desc
        }
        fn usage(&self) -> &str {
            ""
        }
        async fn execute(
            &self,
            _args: &CommandArgs,
            _envelope: &Envelope,
            _session: &Session,
        ) -> Result<Vec<OutboundMessage>> {
            Ok(vec![])
        }
    }

    fn mock_cmd(name: &'static str, desc: &'static str) -> Arc<dyn ServerCommand> {
        Arc::new(MockCommand {
            cmd_name: name,
            cmd_desc: desc,
        })
    }

    #[test]
    fn registry_new_is_empty() {
        let reg = CommandRegistry::new();
        assert!(reg.list().is_empty());
    }

    #[test]
    fn register_and_get() {
        let mut reg = CommandRegistry::new();
        reg.register(mock_cmd("test", "a test command"));
        assert!(reg.get("test").is_some());
        assert_eq!(reg.get("test").unwrap().name(), "test");
    }

    #[test]
    fn get_missing_returns_none() {
        let reg = CommandRegistry::new();
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn list_sorted_alphabetically() {
        let mut reg = CommandRegistry::new();
        reg.register(mock_cmd("zebra", "z"));
        reg.register(mock_cmd("alpha", "a"));
        reg.register(mock_cmd("mid", "m"));
        let names: Vec<&str> = reg.list().iter().map(|(n, _)| *n).collect();
        assert_eq!(names, vec!["alpha", "mid", "zebra"]);
    }

    #[test]
    fn help_text_includes_all_commands() {
        let mut reg = CommandRegistry::new();
        reg.register(mock_cmd("skills", "list skills"));
        reg.register(mock_cmd("reset", "clear memory"));
        let help = reg.help_text();
        assert!(help.contains("/skills"));
        assert!(help.contains("/reset"));
        assert!(help.contains("list skills"));
    }
}
