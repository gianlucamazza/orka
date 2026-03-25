use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

// ── Datetime serialization
// ────────────────────────────────────────────────────

/// Serde helper: serialize/deserialize `DateTime<Utc>` as ISO 8601 UTC with
/// millisecond precision (e.g. `"2025-01-01T00:00:00.000Z"`).
pub(crate) mod datetime_ms {
    use chrono::{DateTime, Utc};
    use serde::{Deserialize, Deserializer, Serializer, de::Error as _};

    const FORMAT: &str = "%Y-%m-%dT%H:%M:%S%.3fZ";

    pub(crate) fn serialize<S: Serializer>(dt: &DateTime<Utc>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&dt.format(FORMAT).to_string())
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<DateTime<Utc>, D::Error> {
        let s = String::deserialize(d)?;
        DateTime::parse_from_rfc3339(&s)
            .map(|dt| dt.with_timezone(&Utc))
            .map_err(D::Error::custom)
    }
}

// ── Enums ─────────────────────────────────────────────────────────────────────

/// Task lifecycle state (A2A v1.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
pub enum TaskState {
    /// Default / unknown state.
    #[serde(rename = "TASK_STATE_UNSPECIFIED")]
    Unspecified,
    /// Received but not yet processed.
    #[serde(rename = "TASK_STATE_SUBMITTED")]
    Submitted,
    /// Actively being processed.
    #[serde(rename = "TASK_STATE_WORKING")]
    Working,
    /// Paused; waiting for additional user input.
    #[serde(rename = "TASK_STATE_INPUT_REQUIRED")]
    InputRequired,
    /// Finished successfully.
    #[serde(rename = "TASK_STATE_COMPLETED")]
    Completed,
    /// Cancelled by the caller.
    #[serde(rename = "TASK_STATE_CANCELED")]
    Canceled,
    /// Failed with an error.
    #[serde(rename = "TASK_STATE_FAILED")]
    Failed,
    /// Rejected by the agent (e.g. policy violation).
    #[serde(rename = "TASK_STATE_REJECTED")]
    Rejected,
    /// Paused; waiting for the caller to authenticate.
    #[serde(rename = "TASK_STATE_AUTH_REQUIRED")]
    AuthRequired,
}

impl TaskState {
    /// Returns `true` for terminal states (no further transitions possible).
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            TaskState::Completed | TaskState::Canceled | TaskState::Failed | TaskState::Rejected
        )
    }
}

/// Participant role in a conversation (A2A v1.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum Role {
    /// Default / unknown role.
    #[serde(rename = "ROLE_UNSPECIFIED")]
    Unspecified,
    /// Message sent by the end user.
    #[serde(rename = "ROLE_USER")]
    User,
    /// Message sent by the agent.
    #[serde(rename = "ROLE_AGENT")]
    Agent,
}

// ── Content parts
// ─────────────────────────────────────────────────────────────

/// Unified content part with `kind` discriminator (A2A v1.0).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Part {
    /// Plain-text content.
    Text {
        /// The text content.
        text: String,
        /// Optional arbitrary metadata.
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<HashMap<String, serde_json::Value>>,
    },
    /// File reference — either inline bytes (base64) or a URI.
    File {
        /// File content descriptor.
        file: FileContent,
        /// Optional arbitrary metadata.
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<HashMap<String, serde_json::Value>>,
    },
    /// Structured JSON data.
    Data {
        /// Arbitrary JSON payload.
        data: serde_json::Value,
        /// Optional arbitrary metadata.
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<HashMap<String, serde_json::Value>>,
    },
}

/// File reference within a [`Part::File`].
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FileContent {
    /// Optional file name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// MIME type of the file (e.g. `"application/pdf"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    /// Base64-encoded file bytes. Mutually exclusive with `uri`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<String>,
    /// Remote file URI. Mutually exclusive with `bytes`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
}

// ── Message
// ───────────────────────────────────────────────────────────────────

/// Constant discriminator for [`Message`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
pub enum MessageKind {
    /// The only valid value: `"message"`.
    #[serde(rename = "message")]
    #[default]
    Message,
}

/// A single conversational message (A2A v1.0).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    /// Always `"message"`.
    #[serde(default)]
    pub kind: MessageKind,
    /// Who sent the message.
    pub role: Role,
    /// Ordered list of content parts.
    pub parts: Vec<Part>,
    /// Unique identifier for this message.
    pub message_id: String,
    /// Conversation context this message belongs to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,
    /// Task this message is associated with.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Optional arbitrary metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

// ── Task ──────────────────────────────────────────────────────────────────────

/// Current status snapshot of a task (A2A v1.0).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TaskStatus {
    /// Current lifecycle state.
    pub state: TaskState,
    /// Latest message associated with this status transition.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<Message>,
    /// ISO 8601 UTC timestamp with millisecond precision.
    pub timestamp: String,
}

/// Output artifact produced by a task (A2A v1.0).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    /// Unique identifier for this artifact.
    pub artifact_id: String,
    /// Ordered content parts of the artifact.
    pub parts: Vec<Part>,
    /// Optional human-readable artifact name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Optional description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Optional arbitrary metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

/// Constant discriminator for [`Task`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
pub enum TaskKind {
    /// The only valid value: `"task"`.
    #[serde(rename = "task")]
    #[default]
    Task,
}

/// A tracked A2A task (A2A v1.0).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    /// Always `"task"`.
    #[serde(default)]
    pub kind: TaskKind,
    /// Unique task identifier.
    pub id: String,
    /// Groups related tasks and messages (conversation thread).
    pub context_id: String,
    /// Current lifecycle status.
    pub status: TaskStatus,
    /// Output artifacts produced by this task.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<Artifact>,
    /// Full status transition history (present when `stateTransitionHistory`
    /// capability is enabled).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<TaskStatus>,
    /// When the task was created.
    #[serde(with = "datetime_ms")]
    pub created_at: DateTime<Utc>,
    /// When the task was last modified.
    #[serde(with = "datetime_ms")]
    pub last_modified: DateTime<Utc>,
    /// Optional arbitrary metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

// ── Agent card
// ────────────────────────────────────────────────────────────────

/// Protocol capabilities for a specific interface endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct InterfaceCapabilities {
    /// Whether the agent supports streaming responses via SSE
    /// (`message/stream`).
    pub streaming: bool,
    /// Whether the agent supports outbound push notifications.
    pub push_notifications: bool,
    /// Whether status transition history is included in task responses.
    pub state_transition_history: bool,
}

/// OpenAPI-style security scheme (A2A v1.0).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SecurityScheme {
    /// API key passed in a header, query parameter, or cookie.
    #[serde(rename = "apiKey")]
    ApiKey {
        /// Header/query/cookie parameter name.
        name: String,
        /// Location: `"header"`, `"query"`, or `"cookie"`.
        #[serde(rename = "in")]
        location: String,
    },
    /// HTTP authentication (Bearer or Basic).
    Http {
        /// Scheme name: `"bearer"` or `"basic"`.
        scheme: String,
        /// Bearer token format hint (e.g. `"JWT"`).
        #[serde(skip_serializing_if = "Option::is_none")]
        bearer_format: Option<String>,
    },
    /// OAuth 2.0 with one or more flow definitions.
    Oauth2 {
        /// OAuth 2.0 flow definitions (AuthorizationCode, ClientCredentials,
        /// DeviceCode).
        flows: serde_json::Value,
    },
    /// OpenID Connect discovery.
    #[serde(rename = "openIdConnect")]
    OpenIdConnect {
        /// URL to the OpenID Connect discovery document.
        open_id_connect_url: String,
    },
}

/// A single interface endpoint advertised in the agent card (A2A v1.0).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SupportedInterface {
    /// Absolute URI of the A2A JSON-RPC endpoint.
    pub uri: String,
    /// A2A protocol version implemented at this URI (e.g. `"1.0"`).
    pub protocol_version: String,
    /// Capabilities declared for this interface.
    pub capabilities: InterfaceCapabilities,
    /// Named security schemes protecting this interface (OpenAPI 3.0 style).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub security_schemes: HashMap<String, SecurityScheme>,
}

/// Per-skill security requirement: scheme name → list of required OAuth scopes.
pub type SkillSecurity = HashMap<String, Vec<String>>;

/// A skill advertised in the agent card (A2A v1.0).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgentSkill {
    /// Unique skill identifier.
    pub id: String,
    /// Human-readable skill name.
    pub name: String,
    /// Short description of what the skill does.
    pub description: String,
    /// JSON Schema describing the skill's input parameters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<serde_json::Value>,
    /// JSON Schema describing the skill's output structure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<serde_json::Value>,
    /// Security requirements for invoking this skill.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub security: Vec<SkillSecurity>,
}

/// A2A agent discovery card served at `GET /.well-known/agent.json` (A2A v1.0).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgentCard {
    /// Human-readable agent name.
    pub name: String,
    /// Short description of what this agent does.
    pub description: String,
    /// Agent version string (e.g. the crate version).
    pub version: String,
    /// Protocol interface endpoints supported by this agent.
    pub supported_interfaces: Vec<SupportedInterface>,
    /// Skills this agent can invoke.
    pub skills: Vec<AgentSkill>,
    /// MIME types accepted as input (e.g. `["text/plain"]`).
    pub default_input_modes: Vec<String>,
    /// MIME types produced as output (e.g. `["text/plain"]`).
    pub default_output_modes: Vec<String>,
    /// Optional arbitrary metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

// ── Push notifications
// ────────────────────────────────────────────────────────

/// Authentication configuration for push notification delivery.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PushNotificationAuth {
    /// Authentication scheme, e.g. `"bearer"` or `"basic"`.
    pub scheme: String,
    /// Credentials value (e.g. a bearer token string).
    pub credentials: String,
}

/// Push notification registration for a task (A2A v1.0).
///
/// Clients register this config via `tasks/pushNotificationConfig/set`.
/// The server then POSTs [`TaskEvent`] payloads to [`url`](Self::url)
/// whenever the task's status or artifacts change.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PushNotificationConfig {
    /// Task ID this config is associated with.
    pub task_id: String,
    /// Webhook URL that will receive POST requests for each task event.
    pub url: String,
    /// Optional bearer token shorthand — added as `Authorization: Bearer
    /// <token>`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// Full authentication config (takes precedence over `token` when both
    /// present).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authentication: Option<PushNotificationAuth>,
}

// ── Streaming events
// ──────────────────────────────────────────────────────────

/// Discriminator for [`TaskEvent`] variants, matching A2A v1.0 event kinds.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TaskEvent {
    /// The task's status has changed (emitted on each transition).
    TaskStatusUpdate {
        /// Unique identifier of the task that changed.
        #[serde(rename = "taskId")]
        task_id: String,
        /// Context (conversation thread) this task belongs to.
        #[serde(rename = "contextId")]
        context_id: String,
        /// The new status snapshot.
        status: TaskStatus,
        /// Whether this is the last event in the stream for this task.
        #[serde(rename = "final")]
        is_final: bool,
    },
    /// A new artifact chunk has been produced.
    TaskArtifactUpdate {
        /// Unique identifier of the task.
        #[serde(rename = "taskId")]
        task_id: String,
        /// Context (conversation thread) this task belongs to.
        #[serde(rename = "contextId")]
        context_id: String,
        /// The artifact (or chunk thereof).
        artifact: Artifact,
        /// Whether this is the last chunk of the artifact.
        last_chunk: bool,
        /// Whether this is the last event in the stream for this task.
        #[serde(rename = "final")]
        is_final: bool,
    },
}

// ── Request / response params
// ─────────────────────────────────────────────────

/// Parameters for `tasks/list` (A2A v1.0).
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct ListTasksParams {
    /// Filter by task states. Empty = return all states.
    pub states: Vec<TaskState>,
    /// Maximum number of tasks to return per page (capped at 100).
    pub page_size: usize,
    /// Opaque pagination cursor from a previous response.
    pub page_token: Option<String>,
}

impl Default for ListTasksParams {
    fn default() -> Self {
        Self {
            states: Vec::new(),
            page_size: 20,
            page_token: None,
        }
    }
}

/// Response for `tasks/list` (A2A v1.0).
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListTasksResult {
    /// Page of tasks matching the request filters.
    pub tasks: Vec<Task>,
    /// Present when more pages are available; pass as `pageToken` in the next
    /// request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_page_token: Option<String>,
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_state_serialization() {
        let cases = [
            (TaskState::Unspecified, "TASK_STATE_UNSPECIFIED"),
            (TaskState::Submitted, "TASK_STATE_SUBMITTED"),
            (TaskState::Working, "TASK_STATE_WORKING"),
            (TaskState::InputRequired, "TASK_STATE_INPUT_REQUIRED"),
            (TaskState::Completed, "TASK_STATE_COMPLETED"),
            (TaskState::Canceled, "TASK_STATE_CANCELED"),
            (TaskState::Failed, "TASK_STATE_FAILED"),
            (TaskState::Rejected, "TASK_STATE_REJECTED"),
            (TaskState::AuthRequired, "TASK_STATE_AUTH_REQUIRED"),
        ];
        for (state, expected) in cases {
            let json = serde_json::to_string(&state).unwrap();
            assert_eq!(json, format!("\"{expected}\""), "state={state:?}");
            let back: TaskState = serde_json::from_str(&json).unwrap();
            assert_eq!(back, state);
        }
    }

    #[test]
    fn role_serialization() {
        assert_eq!(serde_json::to_string(&Role::User).unwrap(), "\"ROLE_USER\"");
        assert_eq!(
            serde_json::to_string(&Role::Agent).unwrap(),
            "\"ROLE_AGENT\""
        );
        let back: Role = serde_json::from_str("\"ROLE_USER\"").unwrap();
        assert_eq!(back, Role::User);
    }

    #[test]
    fn part_text_tagged_kind() {
        let part = Part::Text {
            text: "hello".into(),
            metadata: None,
        };
        let v = serde_json::to_value(&part).unwrap();
        assert_eq!(v["kind"], "text");
        assert_eq!(v["text"], "hello");
        assert!(v.get("metadata").is_none());
    }

    #[test]
    fn part_file_roundtrip() {
        let part = Part::File {
            file: FileContent {
                name: Some("report.pdf".into()),
                mime_type: Some("application/pdf".into()),
                bytes: None,
                uri: Some("https://example.com/report.pdf".into()),
            },
            metadata: None,
        };
        let json = serde_json::to_string(&part).unwrap();
        let back: Part = serde_json::from_str(&json).unwrap();
        if let Part::File { file, .. } = back {
            assert_eq!(file.name.unwrap(), "report.pdf");
            assert_eq!(file.uri.unwrap(), "https://example.com/report.pdf");
        } else {
            panic!("expected File part");
        }
    }

    #[test]
    fn task_state_terminal() {
        assert!(TaskState::Completed.is_terminal());
        assert!(TaskState::Canceled.is_terminal());
        assert!(TaskState::Failed.is_terminal());
        assert!(TaskState::Rejected.is_terminal());
        assert!(!TaskState::Working.is_terminal());
        assert!(!TaskState::InputRequired.is_terminal());
    }

    #[test]
    fn agent_card_roundtrip() {
        let card = AgentCard {
            name: "bot".into(),
            description: "test".into(),
            version: "0.1.0".into(),
            supported_interfaces: vec![SupportedInterface {
                uri: "http://localhost/a2a".into(),
                protocol_version: "1.0".into(),
                capabilities: InterfaceCapabilities {
                    streaming: false,
                    push_notifications: false,
                    state_transition_history: true,
                },
                security_schemes: HashMap::new(),
            }],
            skills: vec![],
            default_input_modes: vec!["text/plain".into()],
            default_output_modes: vec!["text/plain".into()],
            metadata: None,
        };
        let json = serde_json::to_string(&card).unwrap();
        let back: AgentCard = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, card.name);
        assert_eq!(back.supported_interfaces[0].uri, "http://localhost/a2a");
        assert_eq!(back.supported_interfaces[0].protocol_version, "1.0");
    }

    #[test]
    fn datetime_ms_format() {
        use chrono::TimeZone;
        let dt = Utc.with_ymd_and_hms(2025, 1, 1, 12, 0, 0).unwrap();
        let formatted = dt.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
        assert_eq!(formatted, "2025-01-01T12:00:00.000Z");
    }

    #[test]
    fn task_status_json_shape() {
        let ts = TaskStatus {
            state: TaskState::Completed,
            message: None,
            timestamp: "2025-01-01T00:00:00.000Z".into(),
        };
        let v = serde_json::to_value(&ts).unwrap();
        assert_eq!(v["state"], "TASK_STATE_COMPLETED");
        assert_eq!(v["timestamp"], "2025-01-01T00:00:00.000Z");
        assert!(v.get("message").is_none());
    }
}
