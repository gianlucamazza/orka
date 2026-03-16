use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use orka_core::traits::SecretManager;
use orka_core::{Envelope, OutboundMessage, Payload, Result, Session, SkillContext, SkillInput};
use orka_skills::SkillRegistry;

use super::ServerCommand;

pub struct SkillCommand {
    skills: Arc<SkillRegistry>,
    secrets: Arc<dyn SecretManager>,
}

impl SkillCommand {
    pub fn new(skills: Arc<SkillRegistry>, secrets: Arc<dyn SecretManager>) -> Self {
        Self { skills, secrets }
    }

    fn make_reply(&self, envelope: &Envelope, text: String) -> OutboundMessage {
        OutboundMessage {
            channel: envelope.channel.clone(),
            session_id: envelope.session_id.clone(),
            payload: Payload::Text(text),
            reply_to: Some(envelope.id.clone()),
            metadata: envelope.metadata.clone(),
        }
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

        let input = SkillInput {
            args: skill_args,
            context: Some(SkillContext {
                secrets: self.secrets.clone(),
            }),
        };

        match self.skills.invoke(skill_name, input).await {
            Ok(output) => Ok(vec![
                self.make_reply(envelope, format!("[{skill_name}] {}", output.data))
            ]),
            Err(e) => Ok(vec![self.make_reply(
                envelope,
                format!("Skill error: {e}"),
            )]),
        }
    }
}
