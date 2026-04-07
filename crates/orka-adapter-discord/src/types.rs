//! Internal Discord protocol types.

use serde::Deserialize;

/// JSON response from `GET /gateway/bot`.
#[derive(Debug, Deserialize)]
pub(crate) struct GatewayResponse {
    pub(crate) url: String,
}

/// A single WebSocket frame from the Discord gateway.
#[derive(Debug, Deserialize)]
pub(crate) struct GatewayEvent {
    pub(crate) op: u8,
    pub(crate) t: Option<String>,
    pub(crate) s: Option<u64>,
    pub(crate) d: Option<serde_json::Value>,
}

/// Session resumption state tracked across WebSocket reconnects.
pub(crate) struct ResumeState {
    pub(crate) session_id: Option<String>,
    pub(crate) resume_gateway_url: Option<String>,
    pub(crate) sequence: Option<u64>,
}
