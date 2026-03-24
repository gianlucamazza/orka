use std::{collections::HashMap, sync::Arc};

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http::header::CONTENT_TYPE;
use orka_a2a::{
    A2aState, AgentCard, TaskStatus, a2a_router, build_agent_card,
    routes::extract_text_from_message,
};
use orka_core::testing::{EchoSkill, InMemorySecretManager};
use orka_skills::SkillRegistry;
use tokio::sync::Mutex;
use tower::ServiceExt;

fn test_state() -> A2aState {
    let mut skills = SkillRegistry::new();
    skills.register(Arc::new(EchoSkill));
    let skills = Arc::new(skills);

    let agent_card = build_agent_card(
        "test-agent",
        "A test agent",
        "http://localhost:8080",
        &skills,
    );
    let secrets: Arc<dyn orka_core::traits::SecretManager> = Arc::new(InMemorySecretManager::new());

    A2aState {
        agent_card,
        skills,
        secrets,
        tasks: Arc::new(Mutex::new(HashMap::new())),
    }
}

#[tokio::test]
async fn agent_card_serialization() {
    let state = test_state();
    let card = &state.agent_card;

    let json = serde_json::to_value(card).unwrap();
    assert_eq!(json["name"], "test-agent");
    assert_eq!(json["description"], "A test agent");
    assert_eq!(json["url"], "http://localhost:8080/a2a");
    assert!(json["capabilities"]["streaming"].as_bool().unwrap());
    assert!(!json["skills"].as_array().unwrap().is_empty());
    assert_eq!(json["defaultInputModes"][0], "text/plain");
    assert_eq!(json["defaultOutputModes"][0], "text/plain");
}

#[tokio::test]
async fn get_well_known_agent_json() {
    let state = test_state();
    let app = a2a_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/.well-known/agent.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let card: AgentCard = serde_json::from_slice(&body).unwrap();
    assert_eq!(card.name, "test-agent");
    assert_eq!(card.url, "http://localhost:8080/a2a");
}

#[tokio::test]
async fn tasks_send_creates_completed_task() {
    let state = test_state();
    let app = a2a_router(state);

    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tasks/send",
        "params": {
            "id": "task-1",
            "message": {
                "role": "user",
                "parts": [{"type": "text", "text": "hello world"}]
            }
        }
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/a2a")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let resp_body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let resp: serde_json::Value = serde_json::from_slice(&resp_body).unwrap();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["result"]["id"], "task-1");
    assert_eq!(resp["result"]["status"], "completed");
    assert!(resp["result"]["messages"].as_array().unwrap().len() >= 2);
}

#[tokio::test]
async fn tasks_get_retrieves_task() {
    let state = test_state();
    let app = a2a_router(state.clone());

    // First, send a task
    let send_body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tasks/send",
        "params": {
            "id": "task-get-1",
            "message": {
                "role": "user",
                "parts": [{"type": "text", "text": "test message"}]
            }
        }
    });

    let _ = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/a2a")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&send_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Now retrieve it
    let app = a2a_router(state);
    let get_body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tasks/get",
        "params": {
            "id": "task-get-1"
        }
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/a2a")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&get_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let resp_body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let resp: serde_json::Value = serde_json::from_slice(&resp_body).unwrap();
    assert_eq!(resp["result"]["id"], "task-get-1");
    assert_eq!(resp["result"]["status"], "completed");
}

#[tokio::test]
async fn tasks_cancel_changes_status() {
    let state = test_state();
    let app = a2a_router(state.clone());

    // Send a task first
    let send_body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tasks/send",
        "params": {
            "id": "task-cancel-1",
            "message": {
                "role": "user",
                "parts": [{"type": "text", "text": "to cancel"}]
            }
        }
    });

    let _ = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/a2a")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&send_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Cancel it
    let app = a2a_router(state);
    let cancel_body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tasks/cancel",
        "params": {
            "id": "task-cancel-1"
        }
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/a2a")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&cancel_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let resp_body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let resp: serde_json::Value = serde_json::from_slice(&resp_body).unwrap();
    assert_eq!(resp["result"]["status"], "canceled");
}

#[tokio::test]
async fn unknown_method_returns_error() {
    let state = test_state();
    let app = a2a_router(state);

    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tasks/unknown",
        "params": {}
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/a2a")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let resp_body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let resp: serde_json::Value = serde_json::from_slice(&resp_body).unwrap();
    assert_eq!(resp["error"]["code"], -32601);
}

#[test]
fn extract_text_from_parts_array() {
    let msg = serde_json::json!({
        "role": "user",
        "parts": [{"type": "text", "text": "hello from parts"}]
    });
    assert_eq!(extract_text_from_message(&msg), "hello from parts");
}

#[test]
fn extract_text_fallback_to_text_field() {
    let msg = serde_json::json!({"text": "fallback text"});
    assert_eq!(extract_text_from_message(&msg), "fallback text");
}

#[test]
fn extract_text_empty_when_no_text() {
    let msg = serde_json::json!({"role": "user"});
    assert_eq!(extract_text_from_message(&msg), "");
}

#[test]
fn task_status_serde_roundtrip() {
    let status = TaskStatus::Completed;
    let json = serde_json::to_string(&status).unwrap();
    assert_eq!(json, "\"completed\"");
    let back: TaskStatus = serde_json::from_str(&json).unwrap();
    assert_eq!(back, TaskStatus::Completed);
}
