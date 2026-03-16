use orka_skills::SkillRegistry;

use crate::types::{AgentCapabilities, AgentCard, AgentSkill};

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
