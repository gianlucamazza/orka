use std::sync::Arc;

use axum::{Json, extract::State, response::IntoResponse};

use crate::server::McpServer;

/// Axum shared state for the MCP HTTP endpoint.
#[derive(Clone)]
pub struct McpServerState {
    /// The underlying MCP server that processes JSON-RPC requests.
    pub server: Arc<McpServer>,
}

/// POST /mcp -- handle a JSON-RPC 2.0 request
pub async fn handle_mcp_post(
    State(state): State<McpServerState>,
    Json(request): Json<serde_json::Value>,
) -> impl IntoResponse {
    match state.server.handle_request(request).await {
        Some(response) => Json(response).into_response(),
        None => axum::http::StatusCode::NO_CONTENT.into_response(),
    }
}
