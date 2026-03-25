#![allow(missing_docs)]

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http::header::CONTENT_TYPE;
use orka_a2a::{
    A2aState, AgentCard, InMemoryPushNotificationStore, InMemoryTaskStore, TaskState,
    WebhookDeliverer, a2a_router, build_agent_card, routes::extract_text_from_message,
};
use orka_core::testing::{EchoSkill, InMemorySecretManager};
use orka_skills::SkillRegistry;
use tower::ServiceExt;

// ── Helpers
// ───────────────────────────────────────────────────────────────────

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

    let push_store = Arc::new(InMemoryPushNotificationStore::default());
    A2aState {
        agent_card,
        skills,
        secrets,
        task_store: Arc::new(InMemoryTaskStore::default()),
        task_events: Default::default(),
        push_store: push_store.clone(),
        webhook_deliverer: Arc::new(WebhookDeliverer::new(push_store)),
    }
}

async fn post_rpc(app: axum::Router, body: serde_json::Value) -> serde_json::Value {
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
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// Post a streaming request and return (status, raw body string).
async fn post_sse(app: axum::Router, body: serde_json::Value) -> (StatusCode, String) {
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
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

fn send_params(task_id: &str, text: &str) -> serde_json::Value {
    serde_json::json!({
        "id": task_id,
        "message": {
            "kind": "message",
            "role": "ROLE_USER",
            "parts": [{"kind": "text", "text": text}],
            "messageId": "msg-1"
        }
    })
}

// ── Agent card
// ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn agent_card_has_v1_structure() {
    let state = test_state();
    let json = serde_json::to_value(&state.agent_card).unwrap();

    assert_eq!(json["name"], "test-agent");
    assert_eq!(json["description"], "A test agent");
    // v1.0: supported_interfaces[] instead of url
    assert!(json["supportedInterfaces"].is_array());
    let iface = &json["supportedInterfaces"][0];
    assert_eq!(iface["uri"], "http://localhost:8080/a2a");
    assert_eq!(iface["protocolVersion"], "1.0");
    // streaming=true after Phase 3
    assert_eq!(iface["capabilities"]["streaming"], true);
    assert_eq!(iface["capabilities"]["stateTransitionHistory"], true);
    assert!(!json["skills"].as_array().unwrap().is_empty());
    assert_eq!(json["defaultInputModes"][0], "text/plain");
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
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let card: AgentCard = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(card.name, "test-agent");
    assert_eq!(
        card.supported_interfaces[0].uri,
        "http://localhost:8080/a2a"
    );
    assert_eq!(card.supported_interfaces[0].protocol_version, "1.0");
}

// ── message/send
// ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn message_send_creates_completed_task() {
    let app = a2a_router(test_state());
    let resp = post_rpc(
        app,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "message/send",
            "params": send_params("task-1", "hello world")
        }),
    )
    .await;

    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["result"]["id"], "task-1");
    assert_eq!(resp["result"]["kind"], "task");
    // v1.0: status is an object with `state` field
    assert_eq!(resp["result"]["status"]["state"], "TASK_STATE_COMPLETED");
    // history contains at least the Submitted and Working statuses
    assert!(
        resp["result"]["history"]
            .as_array()
            .is_some_and(|h| h.len() >= 2)
    );
    // timestamps present
    assert!(resp["result"]["createdAt"].is_string());
    assert!(resp["result"]["lastModified"].is_string());
}

#[tokio::test]
async fn message_send_task_has_artifact() {
    let app = a2a_router(test_state());
    let resp = post_rpc(
        app,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "message/send",
            "params": send_params("task-artifact", "echo hello")
        }),
    )
    .await;

    let artifacts = resp["result"]["artifacts"].as_array().unwrap();
    assert!(!artifacts.is_empty());
    assert!(artifacts[0]["artifactId"].is_string());
    assert!(artifacts[0]["parts"][0]["text"].is_string());
    assert_eq!(artifacts[0]["parts"][0]["kind"], "text");
}

#[tokio::test]
async fn message_send_agent_response_in_status() {
    let app = a2a_router(test_state());
    let resp = post_rpc(
        app,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "message/send",
            "params": send_params("task-resp", "hello")
        }),
    )
    .await;

    let status_msg = &resp["result"]["status"]["message"];
    assert_eq!(status_msg["role"], "ROLE_AGENT");
    assert_eq!(status_msg["kind"], "message");
    assert_eq!(status_msg["parts"][0]["kind"], "text");
}

// ── message/stream
// ────────────────────────────────────────────────────────────

#[tokio::test]
async fn message_stream_returns_sse_events() {
    let app = a2a_router(test_state());
    let (status, body) = post_sse(
        app,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "message/stream",
            "params": send_params("stream-1", "echo hello")
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    // Body must contain SSE data lines
    assert!(
        body.contains("data:"),
        "body should contain SSE data lines: {body}"
    );
    // Final event should announce completed state
    assert!(
        body.contains("TASK_STATE_COMPLETED"),
        "stream should end with completed state: {body}"
    );
}

#[tokio::test]
async fn message_stream_sends_artifact_event() {
    let app = a2a_router(test_state());
    let (status, body) = post_sse(
        app,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "message/stream",
            "params": send_params("stream-artifact", "echo test")
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    // Should contain a TaskArtifactUpdate event
    assert!(
        body.contains("taskArtifactUpdate"),
        "stream should include artifact event: {body}"
    );
}

#[tokio::test]
async fn message_stream_missing_message_returns_error() {
    let app = a2a_router(test_state());
    let (status, body) = post_sse(
        app,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "message/stream",
            "params": {"id": "no-msg"}
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    // Should get a JSON-RPC error (invalid params)
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["error"]["code"], -32602);
}

// ── tasks/get ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn tasks_get_retrieves_task() {
    let state = test_state();

    // Send
    let send_app = a2a_router(state.clone());
    post_rpc(
        send_app,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "message/send",
            "params": send_params("task-get-1", "test message")
        }),
    )
    .await;

    // Get
    let get_app = a2a_router(state);
    let resp = post_rpc(
        get_app,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tasks/get",
            "params": {"id": "task-get-1"}
        }),
    )
    .await;

    assert_eq!(resp["result"]["id"], "task-get-1");
    assert_eq!(resp["result"]["status"]["state"], "TASK_STATE_COMPLETED");
}

#[tokio::test]
async fn tasks_get_missing_returns_task_not_found() {
    let resp = post_rpc(
        a2a_router(test_state()),
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tasks/get",
            "params": {"id": "nonexistent"}
        }),
    )
    .await;

    // -32001 = TaskNotFound
    assert_eq!(resp["error"]["code"], -32001);
}

// ── tasks/list
// ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn tasks_list_returns_tasks() {
    let state = test_state();

    // Create two tasks
    for i in 0..2 {
        let app = a2a_router(state.clone());
        post_rpc(
            app,
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": i,
                "method": "message/send",
                "params": send_params(&format!("list-task-{i}"), "hello")
            }),
        )
        .await;
    }

    let resp = post_rpc(
        a2a_router(state),
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 99,
            "method": "tasks/list",
            "params": {}
        }),
    )
    .await;

    let tasks = resp["result"]["tasks"].as_array().unwrap();
    assert!(tasks.len() >= 2);
}

#[tokio::test]
async fn tasks_list_filter_by_state() {
    let state = test_state();

    // Create one completed task
    let app = a2a_router(state.clone());
    post_rpc(
        app,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "message/send",
            "params": send_params("list-filter-1", "hello")
        }),
    )
    .await;

    // Filter: should find no WORKING tasks (all are already COMPLETED)
    let resp = post_rpc(
        a2a_router(state),
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tasks/list",
            "params": {"states": ["TASK_STATE_WORKING"]}
        }),
    )
    .await;

    let tasks = resp["result"]["tasks"].as_array().unwrap();
    assert!(tasks.is_empty());
}

// ── tasks/cancel ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn tasks_cancel_is_not_cancelable_when_completed() {
    let state = test_state();

    let app = a2a_router(state.clone());
    post_rpc(
        app,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "message/send",
            "params": send_params("task-cancel-1", "to cancel")
        }),
    )
    .await;

    // Try to cancel an already-completed task
    let resp = post_rpc(
        a2a_router(state),
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tasks/cancel",
            "params": {"id": "task-cancel-1"}
        }),
    )
    .await;

    // -32002 = TaskNotCancelable (task is already in a terminal state)
    assert_eq!(resp["error"]["code"], -32002);
}

#[tokio::test]
async fn tasks_cancel_missing_returns_task_not_found() {
    let resp = post_rpc(
        a2a_router(test_state()),
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tasks/cancel",
            "params": {"id": "no-such-task"}
        }),
    )
    .await;

    assert_eq!(resp["error"]["code"], -32001);
}

// ── tasks/subscribe
// ───────────────────────────────────────────────────────────

#[tokio::test]
async fn tasks_subscribe_completed_task_returns_sse() {
    let state = test_state();

    // Create a completed task
    post_rpc(
        a2a_router(state.clone()),
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "message/send",
            "params": send_params("subscribe-done", "hello")
        }),
    )
    .await;

    // Subscribe — should get a single terminal event
    let (status, body) = post_sse(
        a2a_router(state),
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tasks/subscribe",
            "params": {"id": "subscribe-done"}
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("TASK_STATE_COMPLETED"),
        "subscribe should emit completed state: {body}"
    );
    assert!(
        body.contains("\"final\":true"),
        "event should be marked final: {body}"
    );
}

#[tokio::test]
async fn tasks_subscribe_missing_returns_task_not_found() {
    let (status, body) = post_sse(
        a2a_router(test_state()),
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tasks/subscribe",
            "params": {"id": "ghost-task"}
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["error"]["code"], -32001);
}

// ── agentCard/getExtended ────────────────────────────────────────────────────

#[tokio::test]
async fn agent_card_get_extended_returns_card() {
    let resp = post_rpc(
        a2a_router(test_state()),
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "agentCard/getExtended",
            "params": {}
        }),
    )
    .await;

    assert_eq!(resp["result"]["name"], "test-agent");
    assert!(resp["result"]["supportedInterfaces"].is_array());
}

// ── Unsupported / unimplemented methods ──────────────────────────────────────

#[tokio::test]
async fn unknown_method_returns_method_not_found() {
    let resp = post_rpc(
        a2a_router(test_state()),
        serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "tasks/unknown", "params": {}}),
    )
    .await;
    // -32601 = MethodNotFound
    assert_eq!(resp["error"]["code"], -32601);
}

// ── tasks/pushNotificationConfig ─────────────────────────────────────────────

#[tokio::test]
async fn push_notification_config_set_and_get() {
    let state = test_state();

    let resp = post_rpc(
        a2a_router(state.clone()),
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tasks/pushNotificationConfig/set",
            "params": {
                "taskId": "push-task-1",
                "url": "https://example.com/webhook",
                "token": "secret"
            }
        }),
    )
    .await;
    assert!(resp["error"].is_null(), "set should succeed: {resp}");
    assert_eq!(resp["result"]["taskId"], "push-task-1");
    assert_eq!(resp["result"]["url"], "https://example.com/webhook");

    let resp = post_rpc(
        a2a_router(state),
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tasks/pushNotificationConfig/get",
            "params": {"taskId": "push-task-1"}
        }),
    )
    .await;
    assert_eq!(resp["result"]["taskId"], "push-task-1");
    assert_eq!(resp["result"]["url"], "https://example.com/webhook");
}

#[tokio::test]
async fn push_notification_config_list() {
    let state = test_state();

    for i in 0..2u32 {
        post_rpc(
            a2a_router(state.clone()),
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": i,
                "method": "tasks/pushNotificationConfig/set",
                "params": {
                    "taskId": format!("push-list-{i}"),
                    "url": "https://example.com/hook"
                }
            }),
        )
        .await;
    }

    let resp = post_rpc(
        a2a_router(state),
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 99,
            "method": "tasks/pushNotificationConfig/list",
            "params": {}
        }),
    )
    .await;
    let configs = resp["result"].as_array().unwrap();
    assert!(configs.len() >= 2);
}

#[tokio::test]
async fn push_notification_config_delete() {
    let state = test_state();

    post_rpc(
        a2a_router(state.clone()),
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tasks/pushNotificationConfig/set",
            "params": {"taskId": "push-del-1", "url": "https://example.com/hook"}
        }),
    )
    .await;

    let resp = post_rpc(
        a2a_router(state.clone()),
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tasks/pushNotificationConfig/delete",
            "params": {"taskId": "push-del-1"}
        }),
    )
    .await;
    assert!(resp["error"].is_null(), "delete should succeed");
    assert_eq!(resp["result"]["deleted"], true);

    // Get returns null after deletion
    let resp = post_rpc(
        a2a_router(state),
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tasks/pushNotificationConfig/get",
            "params": {"taskId": "push-del-1"}
        }),
    )
    .await;
    assert!(resp["result"].is_null());
}

#[tokio::test]
async fn push_notification_config_set_missing_url_returns_error() {
    let resp = post_rpc(
        a2a_router(test_state()),
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tasks/pushNotificationConfig/set",
            "params": {"taskId": "push-bad"}
        }),
    )
    .await;
    // -32602 = InvalidParams (missing url)
    assert_eq!(resp["error"]["code"], -32602);
}

// ── skillId routing
// ───────────────────────────────────────────────────────────

#[tokio::test]
async fn message_send_with_explicit_skill_id() {
    let app = a2a_router(test_state());
    let resp = post_rpc(
        app,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "message/send",
            "params": {
                "id": "skill-id-task",
                "skillId": "echo",
                "message": {
                    "kind": "message",
                    "role": "ROLE_USER",
                    "parts": [{"kind": "text", "text": "hello via skillId"}],
                    "messageId": "msg-skill"
                }
            }
        }),
    )
    .await;
    assert_eq!(resp["result"]["status"]["state"], "TASK_STATE_COMPLETED");
}

// ── extract_text_from_message ────────────────────────────────────────────────

#[test]
fn extract_text_from_parts_array() {
    let msg = serde_json::json!({
        "role": "ROLE_USER",
        "parts": [{"kind": "text", "text": "hello from parts"}]
    });
    assert_eq!(extract_text_from_message(&msg), "hello from parts");
}

#[test]
fn extract_text_skips_non_text_parts() {
    let msg = serde_json::json!({
        "parts": [
            {"kind": "file", "file": {"uri": "http://example.com/x"}},
            {"kind": "text", "text": "found it"}
        ]
    });
    assert_eq!(extract_text_from_message(&msg), "found it");
}

#[test]
fn extract_text_empty_when_no_text_part() {
    let msg = serde_json::json!({"role": "ROLE_USER", "parts": []});
    assert_eq!(extract_text_from_message(&msg), "");
}

// ── TaskState serialization
// ───────────────────────────────────────────────────

#[test]
fn task_state_serde_roundtrip() {
    let state = TaskState::Completed;
    let json = serde_json::to_string(&state).unwrap();
    assert_eq!(json, "\"TASK_STATE_COMPLETED\"");
    let back: TaskState = serde_json::from_str(&json).unwrap();
    assert_eq!(back, state);
}

#[test]
fn task_state_all_variants_serialize() {
    use TaskState::*;
    let cases = [
        (Unspecified, "TASK_STATE_UNSPECIFIED"),
        (Submitted, "TASK_STATE_SUBMITTED"),
        (Working, "TASK_STATE_WORKING"),
        (InputRequired, "TASK_STATE_INPUT_REQUIRED"),
        (Completed, "TASK_STATE_COMPLETED"),
        (Canceled, "TASK_STATE_CANCELED"),
        (Failed, "TASK_STATE_FAILED"),
        (Rejected, "TASK_STATE_REJECTED"),
        (AuthRequired, "TASK_STATE_AUTH_REQUIRED"),
    ];
    for (variant, expected) in cases {
        assert_eq!(
            serde_json::to_string(&variant).unwrap(),
            format!("\"{expected}\""),
            "{variant:?}"
        );
    }
}
