use std::sync::Arc;

use axum::{extract::State, response::IntoResponse, Json};

use crate::server::McpServer;

#[derive(Clone)]
pub struct McpServerState {
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
