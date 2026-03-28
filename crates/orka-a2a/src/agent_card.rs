use std::collections::HashMap;

use orka_auth::AuthConfig;
use orka_skills::SkillRegistry;

use crate::types::{
    AgentCard, AgentSkill, InterfaceCapabilities, SecurityScheme, SupportedInterface,
};

/// Build an [`AgentCard`] by introspecting the skill registry (A2A v1.0).
///
/// The card is served at `GET /.well-known/agent.json` and advertises a single
/// JSON-RPC interface at `{base_url}/a2a` with protocol version `"1.0"`.
///
/// When `auth_config` is provided the interface's `security_schemes` field is
/// populated from the configured authenticators.
pub fn build_agent_card(
    name: &str,
    description: &str,
    base_url: &str,
    skills: &SkillRegistry,
) -> AgentCard {
    build_agent_card_with_auth(name, description, base_url, skills, None)
}

/// Build an [`AgentCard`] with optional auth configuration.
pub fn build_agent_card_with_auth(
    name: &str,
    description: &str,
    base_url: &str,
    skills: &SkillRegistry,
    auth_config: Option<&AuthConfig>,
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
                output_schema: None,
                security: Vec::new(),
            })
        })
        .collect();

    let security_schemes = build_security_schemes(auth_config);

    AgentCard {
        name: name.to_string(),
        description: description.to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        supported_interfaces: vec![SupportedInterface {
            uri: format!("{base_url}/a2a"),
            protocol_version: "1.0".to_string(),
            capabilities: InterfaceCapabilities {
                streaming: true,
                push_notifications: true,
                state_transition_history: true,
            },
            security_schemes,
        }],
        skills: skill_list,
        default_input_modes: vec!["text/plain".to_string()],
        default_output_modes: vec!["text/plain".to_string()],
        metadata: None,
    }
}

/// Build security schemes map from the auth configuration.
fn build_security_schemes(auth_config: Option<&AuthConfig>) -> HashMap<String, SecurityScheme> {
    let Some(auth) = auth_config else {
        return HashMap::new();
    };

    let mut schemes = HashMap::new();

    if !auth.api_keys.is_empty() {
        schemes.insert(
            "apiKey".to_string(),
            SecurityScheme::ApiKey {
                name: "X-Api-Key".to_string(),
                location: "header".to_string(),
            },
        );
    }

    if auth.jwt.is_some() {
        schemes.insert(
            "bearerAuth".to_string(),
            SecurityScheme::Http {
                scheme: "bearer".to_string(),
                bearer_format: Some("JWT".to_string()),
            },
        );
    }

    schemes
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::sync::Arc;

    use orka_core::testing::EchoSkill;

    use super::*;

    #[test]
    fn build_agent_card_empty_registry() {
        let skills = SkillRegistry::new();
        let card = build_agent_card("bot", "A bot", "http://localhost", &skills);
        assert_eq!(card.name, "bot");
        assert_eq!(card.supported_interfaces[0].uri, "http://localhost/a2a");
        assert_eq!(card.supported_interfaces[0].protocol_version, "1.0");
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
        assert!(card.skills[0].input_schema.is_some());
        assert!(card.skills[0].output_schema.is_none());
    }

    #[test]
    fn build_agent_card_capabilities() {
        let skills = SkillRegistry::new();
        let card = build_agent_card("bot", "desc", "http://x", &skills);
        let caps = &card.supported_interfaces[0].capabilities;
        assert!(caps.streaming);
        assert!(caps.push_notifications);
        assert!(caps.state_transition_history);
        assert_eq!(card.default_input_modes, vec!["text/plain"]);
        assert_eq!(card.default_output_modes, vec!["text/plain"]);
        assert!(card.metadata.is_none());
    }

    #[test]
    fn build_agent_card_no_security_schemes_by_default() {
        let skills = SkillRegistry::new();
        let card = build_agent_card("bot", "desc", "http://x", &skills);
        assert!(card.supported_interfaces[0].security_schemes.is_empty());
    }

    #[test]
    fn build_agent_card_with_auth_api_key() {
        use orka_auth::ApiKeyEntry;

        let skills = SkillRegistry::new();
        let mut auth = AuthConfig::default();
        auth.api_keys = vec![ApiKeyEntry::new("k", "hash", vec![])];
        let card = build_agent_card_with_auth("bot", "desc", "http://x", &skills, Some(&auth));
        let schemes = &card.supported_interfaces[0].security_schemes;
        assert!(schemes.contains_key("apiKey"), "should have apiKey scheme");
        assert!(
            !schemes.contains_key("bearerAuth"),
            "should not have bearerAuth"
        );
    }

    #[test]
    fn build_agent_card_with_auth_jwt() {
        use orka_auth::JwtAuthConfig;

        let skills = SkillRegistry::new();
        let mut auth = AuthConfig::default();
        auth.jwt = Some(serde_json::from_str::<JwtAuthConfig>(r#"{"secret":"s3cr3t"}"#).unwrap());
        let card = build_agent_card_with_auth("bot", "desc", "http://x", &skills, Some(&auth));
        let schemes = &card.supported_interfaces[0].security_schemes;
        assert!(
            schemes.contains_key("bearerAuth"),
            "should have bearerAuth scheme"
        );
        assert!(!schemes.contains_key("apiKey"), "should not have apiKey");
    }

    #[test]
    fn build_agent_card_with_both_auth_methods() {
        use orka_auth::{ApiKeyEntry, JwtAuthConfig};

        let skills = SkillRegistry::new();
        let mut auth = AuthConfig::default();
        auth.api_keys = vec![ApiKeyEntry::new("k", "hash", vec![])];
        auth.jwt = Some(serde_json::from_str::<JwtAuthConfig>(r#"{"secret":"s"}"#).unwrap());
        let card = build_agent_card_with_auth("bot", "desc", "http://x", &skills, Some(&auth));
        let schemes = &card.supported_interfaces[0].security_schemes;
        assert!(schemes.contains_key("apiKey"));
        assert!(schemes.contains_key("bearerAuth"));
    }
}
