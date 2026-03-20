use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A2A agent discovery card served at `/.well-known/agent.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCard {
    /// Human-readable agent name.
    pub name: String,
    /// Short description of what the agent does.
    pub description: String,
    /// Base URL where the A2A endpoint is reachable.
    pub url: String,
    /// Crate version string.
    pub version: String,
    /// Protocol features supported by this agent.
    pub capabilities: AgentCapabilities,
    /// Skills this agent can invoke.
    pub skills: Vec<AgentSkill>,
    /// MIME types accepted as input.
    pub default_input_modes: Vec<String>,
    /// MIME types produced as output.
    pub default_output_modes: Vec<String>,
    /// Optional authentication config.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authentication: Option<AuthConfig>,
}

/// Protocol features supported by this agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCapabilities {
    /// Whether the agent supports streaming responses.
    pub streaming: bool,
    /// Whether the agent supports push notifications.
    pub push_notifications: bool,
    /// Whether the agent records task state transition history.
    pub state_transition_history: bool,
}

/// A skill advertised in the agent card.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSkill {
    /// Unique skill identifier.
    pub id: String,
    /// Human-readable skill name.
    pub name: String,
    /// Short description of what the skill does.
    pub description: String,
    /// Optional JSON schema describing the skill's input parameters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<serde_json::Value>,
}

/// Authentication scheme required to call the agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    /// Authentication scheme name (e.g. `"bearer"`, `"apiKey"`).
    #[serde(rename = "type")]
    pub auth_type: String,
}

/// Lifecycle state of an A2A task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    /// Task has been received but not yet processed.
    Submitted,
    /// Task is actively being processed.
    Working,
    /// Task finished successfully.
    Completed,
    /// Task failed.
    Failed,
    /// Task was cancelled by the caller.
    Canceled,
}

/// An A2A task tracking a request from submission to completion.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    /// Unique task identifier.
    pub id: String,
    /// Current lifecycle state.
    pub status: TaskStatus,
    /// Conversation turns for this task.
    pub messages: Vec<A2aMessage>,
    /// Output artifacts produced by the task.
    pub artifacts: Vec<Artifact>,
    /// When the task was created.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
    /// When the task was last updated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
}

/// A single turn in an A2A conversation (user or agent).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2aMessage {
    /// Message author: `"user"` or `"agent"`.
    pub role: String,
    /// Content parts of the message.
    pub parts: Vec<MessagePart>,
}

/// Content fragment within an [`A2aMessage`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum MessagePart {
    /// Plain-text content.
    #[serde(rename = "text")]
    Text {
        /// The text content.
        text: String,
    },
}

/// A named output artifact produced by an A2A task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    /// Artifact name (e.g. `"response"`).
    pub name: String,
    /// Content parts of the artifact.
    pub parts: Vec<MessagePart>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_agent_card() -> AgentCard {
        AgentCard {
            name: "test-agent".into(),
            description: "A test agent".into(),
            url: "http://localhost:8080".into(),
            version: "1.0.0".into(),
            capabilities: AgentCapabilities {
                streaming: true,
                push_notifications: false,
                state_transition_history: true,
            },
            skills: vec![AgentSkill {
                id: "echo".into(),
                name: "Echo".into(),
                description: "Echoes input".into(),
                input_schema: Some(serde_json::json!({"type": "object"})),
            }],
            default_input_modes: vec!["text/plain".into()],
            default_output_modes: vec!["text/plain".into()],
            authentication: Some(AuthConfig {
                auth_type: "bearer".into(),
            }),
        }
    }

    fn sample_task() -> Task {
        Task {
            id: "task-001".into(),
            status: TaskStatus::Completed,
            messages: vec![
                A2aMessage {
                    role: "user".into(),
                    parts: vec![MessagePart::Text {
                        text: "Hello".into(),
                    }],
                },
                A2aMessage {
                    role: "agent".into(),
                    parts: vec![MessagePart::Text {
                        text: "Hi there!".into(),
                    }],
                },
            ],
            artifacts: vec![Artifact {
                name: "response".into(),
                parts: vec![MessagePart::Text {
                    text: "Hi there!".into(),
                }],
            }],
            created_at: None,
            updated_at: None,
        }
    }

    #[test]
    fn agent_card_json_snapshot() {
        let card = sample_agent_card();
        insta::assert_json_snapshot!("agent_card", card);
    }

    #[test]
    fn task_json_snapshot() {
        let task = sample_task();
        insta::assert_json_snapshot!("task", task);
    }

    #[test]
    fn agent_card_roundtrip() {
        let card = sample_agent_card();
        let json = serde_json::to_string(&card).unwrap();
        let parsed: AgentCard = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, card.name);
        assert_eq!(parsed.skills.len(), card.skills.len());
    }

    #[test]
    fn task_status_serialization() {
        assert_eq!(
            serde_json::to_string(&TaskStatus::Submitted).unwrap(),
            "\"submitted\""
        );
        assert_eq!(
            serde_json::to_string(&TaskStatus::Working).unwrap(),
            "\"working\""
        );
        assert_eq!(
            serde_json::to_string(&TaskStatus::Completed).unwrap(),
            "\"completed\""
        );
        assert_eq!(
            serde_json::to_string(&TaskStatus::Failed).unwrap(),
            "\"failed\""
        );
        assert_eq!(
            serde_json::to_string(&TaskStatus::Canceled).unwrap(),
            "\"canceled\""
        );
    }

    #[test]
    fn message_part_tagged_serialization() {
        let part = MessagePart::Text {
            text: "hello".into(),
        };
        let json = serde_json::to_value(&part).unwrap();
        assert_eq!(json["type"], "text");
        assert_eq!(json["text"], "hello");
    }
}
