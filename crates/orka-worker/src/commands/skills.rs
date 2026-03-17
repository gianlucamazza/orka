use std::sync::Arc;

use async_trait::async_trait;
use orka_core::{Envelope, OutboundMessage, Result, Session};
use orka_skills::SkillRegistry;

use super::ServerCommand;

pub struct SkillsCommand {
    skills: Arc<SkillRegistry>,
}

impl SkillsCommand {
    pub fn new(skills: Arc<SkillRegistry>) -> Self {
        Self { skills }
    }
}

#[async_trait]
impl ServerCommand for SkillsCommand {
    fn name(&self) -> &str {
        "skills"
    }
    fn description(&self) -> &str {
        "List available skills"
    }
    fn usage(&self) -> &str {
        "/skills"
    }

    async fn execute(
        &self,
        _args: &[String],
        envelope: &Envelope,
        _session: &Session,
    ) -> Result<Vec<OutboundMessage>> {
        let names = self.skills.list();
        let text = if names.is_empty() {
            "No skills registered.".to_string()
        } else {
            let mut lines = vec!["Available skills:".to_string()];
            for name in names {
                if let Some(skill) = self.skills.get(name) {
                    lines.push(format!("  {name} — {}", skill.description()));
                }
            }
            lines.join("\n")
        };

        let mut msg = OutboundMessage::text(
            envelope.channel.clone(),
            envelope.session_id,
            text,
            Some(envelope.id),
        );
        msg.metadata = envelope.metadata.clone();
        Ok(vec![msg])
    }
}
