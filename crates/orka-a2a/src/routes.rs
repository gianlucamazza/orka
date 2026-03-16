use std::collections::HashMap;
use std::sync::Arc;

use axum::{extract::State, response::IntoResponse, Json};
use chrono::Utc;
use serde_json::json;
use tokio::sync::Mutex;
use tracing::debug;

use orka_core::traits::SecretManager;
use orka_core::{SkillContext, SkillInput};
use orka_skills::SkillRegistry;

use crate::types::*;

#[derive(Clone)]
pub struct A2aState {
    pub agent_card: AgentCard,
    pub skills: Arc<SkillRegistry>,
    pub secrets: Arc<dyn SecretManager>,
    pub tasks: Arc<Mutex<HashMap<String, Task>>>,
}

/// GET /.well-known/agent.json
pub async fn handle_agent_card(State(state): State<A2aState>) -> impl IntoResponse {
    Json(state.agent_card.clone())
}

/// POST /a2a -- JSON-RPC 2.0 dispatcher
pub async fn handle_a2a(
    State(state): State<A2aState>,
    Json(request): Json<serde_json::Value>,
) -> impl IntoResponse {
    let id = request.get("id").cloned();
    let method = request["method"].as_str().unwrap_or("");

    let result = match method {
        "tasks/send" => handle_task_send(&state, &request["params"]).await,
        "tasks/get" => handle_task_get(&state, &request["params"]).await,
        "tasks/cancel" => handle_task_cancel(&state, &request["params"]).await,
        _ => Err(json!({
            "code": -32601,
            "message": format!("Method not found: {method}")
        })),
    };

    match result {
        Ok(data) => Json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": data
        })),
        Err(error) => Json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": error
        })),
    }
}

async fn handle_task_send(
    state: &A2aState,
    params: &serde_json::Value,
) -> Result<serde_json::Value, serde_json::Value> {
    let task_id = params["id"]
        .as_str()
        .map(|s| s.to_string())
        .unwrap_or_else(|| uuid::Uuid::now_v7().to_string());

    // Extract the user message
    let message = params
        .get("message")
        .ok_or_else(|| json!({"code": -32602, "message": "missing 'message' parameter"}))?;
    let text = extract_text_from_message(message);

    // Create task
    let now = Utc::now();
    let mut task = Task {
        id: task_id.clone(),
        status: TaskStatus::Working,
        messages: vec![A2aMessage {
            role: "user".to_string(),
            parts: vec![MessagePart::Text { text: text.clone() }],
        }],
        artifacts: Vec::new(),
        created_at: Some(now),
        updated_at: Some(now),
    };

    // Try to find a matching skill and invoke it, or return the text as-is
    let response_text = if let Some(skill_name) = find_matching_skill(&state.skills, &text) {
        let args: HashMap<String, serde_json::Value> =
            HashMap::from([("input".to_string(), serde_json::Value::String(text.clone()))]);
        let input = SkillInput {
            args,
            context: Some(SkillContext {
                secrets: state.secrets.clone(),
            }),
        };
        match state.skills.invoke(&skill_name, input).await {
            Ok(output) => output.data.to_string(),
            Err(_e) => {
                task.status = TaskStatus::Failed;
                task.updated_at = Some(Utc::now());
                state
                    .tasks
                    .lock()
                    .await
                    .insert(task_id.clone(), task.clone());
                return Ok(serde_json::to_value(&task).unwrap_or_default());
            }
        }
    } else {
        format!("Received: {text}")
    };

    task.messages.push(A2aMessage {
        role: "agent".to_string(),
        parts: vec![MessagePart::Text {
            text: response_text.clone(),
        }],
    });
    task.artifacts.push(Artifact {
        name: "response".to_string(),
        parts: vec![MessagePart::Text {
            text: response_text,
        }],
    });
    task.status = TaskStatus::Completed;
    task.updated_at = Some(Utc::now());

    state
        .tasks
        .lock()
        .await
        .insert(task_id.clone(), task.clone());
    debug!(task_id, "A2A task completed");
    Ok(serde_json::to_value(&task).unwrap_or_default())
}

async fn handle_task_get(
    state: &A2aState,
    params: &serde_json::Value,
) -> Result<serde_json::Value, serde_json::Value> {
    let task_id = params["id"]
        .as_str()
        .ok_or_else(|| json!({"code": -32602, "message": "missing 'id' parameter"}))?;

    let tasks = state.tasks.lock().await;
    match tasks.get(task_id) {
        Some(task) => Ok(serde_json::to_value(task).unwrap_or_default()),
        None => Err(json!({"code": -32602, "message": "task not found"})),
    }
}

async fn handle_task_cancel(
    state: &A2aState,
    params: &serde_json::Value,
) -> Result<serde_json::Value, serde_json::Value> {
    let task_id = params["id"]
        .as_str()
        .ok_or_else(|| json!({"code": -32602, "message": "missing 'id' parameter"}))?;

    let mut tasks = state.tasks.lock().await;
    match tasks.get_mut(task_id) {
        Some(task) => {
            task.status = TaskStatus::Canceled;
            task.updated_at = Some(Utc::now());
            Ok(serde_json::to_value(&*task).unwrap_or_default())
        }
        None => Err(json!({"code": -32602, "message": "task not found"})),
    }
}

pub fn extract_text_from_message(message: &serde_json::Value) -> String {
    // Try to extract text from parts array
    if let Some(parts) = message["parts"].as_array() {
        for part in parts {
            if part["type"].as_str() == Some("text") {
                if let Some(text) = part["text"].as_str() {
                    return text.to_string();
                }
            }
        }
    }
    // Fallback: try direct text field
    message["text"].as_str().unwrap_or("").to_string()
}

fn find_matching_skill(skills: &SkillRegistry, text: &str) -> Option<String> {
    let skill_names = skills.list();
    for name in skill_names {
        if text.starts_with(name) {
            return Some(name.to_string());
        }
    }
    None
}

/// Build an axum Router with A2A routes.
pub fn a2a_router(state: A2aState) -> axum::Router {
    axum::Router::new()
        .route(
            "/.well-known/agent.json",
            axum::routing::get(handle_agent_card),
        )
        .route("/a2a", axum::routing::post(handle_a2a))
        .with_state(state)
}
