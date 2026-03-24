use std::collections::HashMap;

use serde::{Deserialize, Serialize};

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
