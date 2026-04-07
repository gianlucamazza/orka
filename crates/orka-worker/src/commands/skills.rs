use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

use async_trait::async_trait;
use orka_core::{CommandArgs, Envelope, OutboundMessage, Result, Session};
use orka_skills::SkillRegistry;

use super::ServerCommand;

/// Command that lists all registered skills (`/skills [name]`).
pub struct SkillsCommand {
    skills: Arc<SkillRegistry>,
}

impl SkillsCommand {
    /// Create the command with access to the skill registry.
    pub fn new(skills: Arc<SkillRegistry>) -> Self {
        Self { skills }
    }
}

#[async_trait]
impl ServerCommand for SkillsCommand {
    fn name(&self) -> &'static str {
        "skills"
    }
    fn description(&self) -> &'static str {
        "List available skills"
    }
    fn usage(&self) -> &'static str {
        "/skills [name]"
    }

    async fn execute(
        &self,
        args: &CommandArgs,
        envelope: &Envelope,
        _session: &Session,
    ) -> Result<Vec<OutboundMessage>> {
        let text = if let Some(skill_name) = args.positional(0) {
            // Detail view for a specific skill.
            match self.skills.get(skill_name) {
                None => format!("Unknown skill: **{skill_name}**"),
                Some(skill) => {
                    let schema = serde_json::to_string_pretty(&skill.schema().parameters)
                        .unwrap_or_else(|_| "{}".to_string());
                    format!(
                        "**{skill_name}**\n{}\n\nCategory: `{}`\n\n**Input schema:**\n```json\n{schema}\n```",
                        skill.description(),
                        skill.category(),
                    )
                }
            }
        } else {
            // List view grouped by category.
            let infos = self.skills.list_info();
            if infos.is_empty() {
                "No skills registered.".to_string()
            } else {
                // Skills with open circuit breakers are excluded from list_available().
                let available_set: HashSet<&str> =
                    self.skills.list_available().into_iter().collect();

                // Group by category preserving alphabetical order.
                let mut by_cat: BTreeMap<&str, Vec<(&str, &str, bool)>> = BTreeMap::new();
                for (name, skill, _circuit) in &infos {
                    let available = available_set.contains(name);
                    by_cat.entry(skill.category()).or_default().push((
                        name,
                        skill.description(),
                        available,
                    ));
                }

                let mut lines = vec!["**Available skills:**".to_string()];
                for (cat, skills) in &by_cat {
                    lines.push(format!("\n_{cat}_"));
                    for (name, desc, available) in skills {
                        let status = if *available { "✓" } else { "✗" };
                        lines.push(format!("• **{name}** {status} — {desc}"));
                    }
                }
                lines.push("\nUse `/skills <name>` for details and input schema.".to_string());
                lines.join("\n")
            }
        };

        let mut msg = OutboundMessage::text(
            envelope.channel.clone(),
            envelope.session_id,
            text,
            Some(envelope.id),
        );
        msg.metadata.clone_from(&envelope.metadata);
        envelope.platform_context.clone_into(&mut msg.platform_context);
        Ok(vec![msg])
    }
}
