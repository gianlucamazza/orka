use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Inbound message request from an HTTP client.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct InboundRequest {
    /// Optional session ID; a new session is created if omitted.
    pub session_id: Option<String>,
    /// Text content of the message.
    pub text: String,
    /// Optional key-value metadata attached to the message.
    pub metadata: Option<HashMap<String, serde_json::Value>>,
    /// Optional user identifier (shown in dashboard as "User").
    pub user_id: Option<String>,
}

/// Response returned after accepting an inbound message.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct InboundResponse {
    /// Unique identifier assigned to the accepted message.
    pub message_id: String,
    /// Session ID for this conversation (new or existing).
    pub session_id: String,
}
