use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use orka_core::{
    CommandArgs, Envelope, OutboundMessage, Result, Session, SkillContext, SkillInput,
    traits::SecretManager,
};
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
}

#[async_trait]
impl ServerCommand for SkillCommand {
    fn name(&self) -> &'static str {
        "skill"
    }
    fn description(&self) -> &'static str {
        "Invoke a skill directly"
    }
    fn usage(&self) -> &'static str {
        "/skill <name> [key=val ...]"
    }

    async fn execute(
        &self,
        args: &CommandArgs,
        envelope: &Envelope,
        _session: &Session,
    ) -> Result<Vec<OutboundMessage>> {
        let Some(skill_name) = args.positional(0) else {
            let available = self.skills.list().join(", ");
            return Ok(vec![Self::make_reply(
                envelope,
                format!("Usage: {}\nAvailable skills: {available}", self.usage()),
            )]);
        };

        if self.skills.get(skill_name).is_none() {
            let available = self.skills.list().join(", ");
            return Ok(vec![Self::make_reply(
                envelope,
                format!("Unknown skill: {skill_name}\nAvailable skills: {available}"),
            )]);
        }

        // Named args (from text `key=val` parsing or structured adapter options).
        let mut skill_args: HashMap<String, serde_json::Value> = HashMap::new();
        for (k, v) in args.named_iter() {
            skill_args.insert(k.to_string(), v.clone());
        }

        // If there are positional tokens after the skill name and no named args were
        // provided, try to map them to a single required parameter in the
        // skill's schema.
        let extra_positional: Vec<&str> = args
            .positional_args()
            .iter()
            .skip(1)
            .map(String::as_str)
            .collect();
        if !extra_positional.is_empty()
            && skill_args.is_empty()
            && let Some(skill) = self.skills.get(skill_name)
        {
            let schema = skill.schema();
            let required = schema
                .parameters
                .get("required")
                .and_then(|r| r.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
                .unwrap_or_default();
            if required.len() == 1 {
                let param_name = required[0];
                skill_args.insert(
                    param_name.to_string(),
                    serde_json::Value::String(extra_positional.join(" ")),
                );
            }
        }

        let input =
            SkillInput::new(skill_args).with_context(SkillContext::new(self.secrets.clone(), None));

        match self.skills.invoke(skill_name, input).await {
            Ok(output) => Ok(vec![Self::make_reply(
                envelope,
                format!("[{skill_name}] {}", output.data),
            )]),
            Err(e) => Ok(vec![Self::make_reply(
                envelope,
                format!("Skill error: {e}"),
            )]),
        }
    }
}
