use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Context data for a session.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionContext {
    /// Session identifier.
    pub session_id: String,

    /// User's current workspace.
    pub workspace: String,

    /// User's message or trigger.
    pub user_message: String,

    /// Current working directory.
    pub cwd: Option<String>,

    /// Recent shell commands.
    pub recent_commands: Vec<String>,

    /// Additional metadata.
    #[serde(flatten)]
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Context data for building a prompt.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BuildContext {
    /// Agent display name.
    pub agent_name: String,

    /// Agent persona content.
    pub persona: String,

    /// Tool instructions.
    pub tool_instructions: String,

    /// Workspace context.
    pub workspace: WorkspaceContext,

    /// Principles to inject.
    pub principles: Vec<PrincipleContext>,

    /// Conversation summary.
    pub conversation_summary: Option<String>,

    /// Current datetime (ISO 8601).
    pub datetime: String,

    /// Timezone.
    pub timezone: String,

    /// Dynamic sections.
    pub dynamic_sections: HashMap<String, String>,
}

/// Workspace context data.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkspaceContext {
    /// Current workspace name.
    pub name: String,

    /// Available workspaces.
    pub available: Vec<String>,

    /// Current working directory.
    pub cwd: Option<String>,
}

/// Principle context data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrincipleContext {
    /// Principle text.
    pub text: String,

    /// Principle kind ("do" or "avoid").
    pub kind: String,

    /// 1-based index for display.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<usize>,
}
