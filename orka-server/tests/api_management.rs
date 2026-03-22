//! Tests for the management API endpoints.
//!
//! Uses `tower::ServiceExt::oneshot` — no TCP listener required.

mod common;

use axum::body::Body;
use http::{Request, StatusCode};
use tower::ServiceExt;

#[tokio::test]
async fn list_skills_contains_echo() {
    let app = common::test_router();
    let req = Request::builder()
        .uri("/api/v1/skills")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let arr = json.as_array().expect("expected array");
    assert!(
        arr.iter().any(|s| s["name"] == "echo"),
        "expected 'echo' skill in list"
    );
}

#[tokio::test]
async fn get_skill_by_name() {
    let app = common::test_router();
    let req = Request::builder()
        .uri("/api/v1/skills/echo")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["name"], "echo");
    assert!(json["description"].is_string());
}

#[tokio::test]
async fn get_skill_not_found() {
    let app = common::test_router();
    let req = Request::builder()
        .uri("/api/v1/skills/nonexistent")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn list_sessions_empty() {
    let app = common::test_router();
    let req = Request::builder()
        .uri("/api/v1/sessions")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json.as_array().expect("expected array").is_empty());
}

#[tokio::test]
async fn get_session_not_found() {
    let app = common::test_router();
    let id = uuid::Uuid::new_v4();
    let req = Request::builder()
        .uri(format!("/api/v1/sessions/{id}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn dlq_list_empty() {
    let app = common::test_router();
    let req = Request::builder()
        .uri("/api/v1/dlq")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json.as_array().expect("expected array").is_empty());
}

#[tokio::test]
async fn dlq_purge_empty() {
    let app = common::test_router();
    let req = Request::builder()
        .method("DELETE")
        .uri("/api/v1/dlq")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["purged"], 0);
}

#[tokio::test]
async fn dlq_replay_not_found() {
    let app = common::test_router();
    let id = uuid::Uuid::new_v4();
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/dlq/{id}/replay"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn list_workspaces() {
    let app = common::test_router();
    let req = Request::builder()
        .uri("/api/v1/workspaces")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json.is_array());
}

#[tokio::test]
async fn get_graph() {
    let app = common::test_router();
    let req = Request::builder()
        .uri("/api/v1/graph")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json["id"].is_string());
    assert!(json["entry"].is_string());
    assert!(json["nodes"].is_array());
    assert!(json["edges"].is_array());
}

#[tokio::test]
async fn experience_status_disabled() {
    let app = common::test_router();
    let req = Request::builder()
        .uri("/api/v1/experience/status")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["enabled"], false);
}

#[tokio::test]
async fn schedules_not_enabled() {
    let app = common::test_router();
    let req = Request::builder()
        .uri("/api/v1/schedules")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    // scheduler_store is None → 503 Service Unavailable
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}
