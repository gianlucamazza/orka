use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use orka_core::traits::SecretManager;
use orka_core::{Envelope, OutboundMessage, Result, Session, SkillContext, SkillInput};
use orka_skills::SkillRegistry;

use super::ServerCommand;

/// Command that invokes a named skill directly (`/skill <name> [args]`).
pub struct SkillCommand {
    skills: Arc<SkillRegistry>,
    secrets: Arc<dyn SecretManager>,
}

impl SkillCommand {
    /// Create the command with access to the skill registry and secret manager.
    pub fn new(skills: Arc<SkillRegistry>, secrets: Arc<dyn SecretManager>) -> Self {
        Self { skills, secrets }
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
}

#[async_trait]
impl ServerCommand for SkillCommand {
    fn name(&self) -> &str {
        "skill"
    }
    fn description(&self) -> &str {
        "Invoke a skill directly"
    }
    fn usage(&self) -> &str {
        "/skill <name> [key=val ...]"
    }

    async fn execute(
        &self,
        args: &[String],
        envelope: &Envelope,
        _session: &Session,
    ) -> Result<Vec<OutboundMessage>> {
        if args.is_empty() {
            let available = self.skills.list().join(", ");
            return Ok(vec![self.make_reply(
                envelope,
                format!("Usage: {}\nAvailable skills: {available}", self.usage()),
            )]);
        }

        let skill_name = &args[0];

        if self.skills.get(skill_name).is_none() {
            let available = self.skills.list().join(", ");
            return Ok(vec![self.make_reply(
                envelope,
                format!("Unknown skill: {skill_name}\nAvailable skills: {available}"),
            )]);
        }

        let mut skill_args = HashMap::new();
        for arg in &args[1..] {
            if let Some((k, v)) = arg.split_once('=') {
                skill_args.insert(k.to_string(), serde_json::Value::String(v.to_string()));
            }
        }

        let input =
            SkillInput::new(skill_args).with_context(SkillContext::new(self.secrets.clone(), None));

        match self.skills.invoke(skill_name, input).await {
            Ok(output) => {
                Ok(vec![self.make_reply(
                    envelope,
                    format!("[{skill_name}] {}", output.data),
                )])
            }
            Err(e) => Ok(vec![self.make_reply(envelope, format!("Skill error: {e}"))]),
        }
    }
}
