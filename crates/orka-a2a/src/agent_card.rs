use orka_skills::SkillRegistry;

use crate::types::{AgentCapabilities, AgentCard, AgentSkill};

/// Build an [`AgentCard`] for this agent by introspecting its skill registry.
pub fn build_agent_card(
    name: &str,
    description: &str,
    base_url: &str,
    skills: &SkillRegistry,
) -> AgentCard {
    let skill_list: Vec<AgentSkill> = skills
        .list()
        .iter()
        .filter_map(|skill_name| {
            let skill = skills.get(skill_name)?;
            Some(AgentSkill {
                id: skill.name().to_string(),
                name: skill.name().to_string(),
                description: skill.description().to_string(),
                input_schema: Some(skill.schema().parameters),
            })
        })
        .collect();

    AgentCard {
        name: name.to_string(),
        description: description.to_string(),
        url: format!("{base_url}/a2a"),
        version: env!("CARGO_PKG_VERSION").to_string(),
        capabilities: AgentCapabilities {
            streaming: true,
            push_notifications: false,
            state_transition_history: true,
        },
        skills: skill_list,
        default_input_modes: vec!["text/plain".to_string()],
        default_output_modes: vec!["text/plain".to_string()],
        authentication: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use orka_core::testing::EchoSkill;

    #[test]
    fn build_agent_card_empty_registry() {
        let skills = SkillRegistry::new();
        let card = build_agent_card("bot", "A bot", "http://localhost", &skills);
        assert_eq!(card.name, "bot");
        assert_eq!(card.url, "http://localhost/a2a");
        assert!(card.skills.is_empty());
    }

    #[test]
    fn build_agent_card_with_skill() {
        let mut skills = SkillRegistry::new();
        skills.register(Arc::new(EchoSkill));
        let card = build_agent_card("bot", "A bot", "http://localhost", &skills);
        assert_eq!(card.skills.len(), 1);
        assert_eq!(card.skills[0].id, "echo");
        assert!(!card.skills[0].description.is_empty());
    }

    #[test]
    fn build_agent_card_capabilities() {
        let skills = SkillRegistry::new();
        let card = build_agent_card("bot", "desc", "http://x", &skills);
        assert!(card.capabilities.streaming);
        assert!(!card.capabilities.push_notifications);
        assert!(card.capabilities.state_transition_history);
        assert_eq!(card.default_input_modes, vec!["text/plain"]);
        assert_eq!(card.default_output_modes, vec!["text/plain"]);
        assert!(card.authentication.is_none());
    }
}
