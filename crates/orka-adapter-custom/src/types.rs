use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Inbound message request from an HTTP client.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct InboundRequest {
    pub session_id: Option<String>,
    pub text: String,
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

/// Response returned after accepting an inbound message.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct InboundResponse {
    pub message_id: String,
    pub session_id: String,
}
