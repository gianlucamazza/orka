#![allow(missing_docs)]

//! Tests for the management API endpoints.
//!
//! Uses `tower::ServiceExt::oneshot` — no TCP listener required.

mod common;

use axum::body::Body;
use http::{Request, StatusCode};
use tower::ServiceExt;

#[tokio::test]
async fn list_skills_contains_echo() -> common::TestResult {
    let app = common::test_router();
    let req = common::request(Request::builder().uri("/api/v1/skills"), Body::empty())?;
    let resp = app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let json = common::json_body(resp).await?;
    let arr = common::json_array(&json)?;
    assert!(
        arr.iter().any(|s| s["name"] == "echo"),
        "expected 'echo' skill in list"
    );
    Ok(())
}

#[tokio::test]
async fn get_skill_by_name() -> common::TestResult {
    let app = common::test_router();
    let req = common::request(Request::builder().uri("/api/v1/skills/echo"), Body::empty())?;
    let resp = app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let json = common::json_body(resp).await?;
    assert_eq!(json["name"], "echo");
    assert!(json["description"].is_string());
    Ok(())
}

#[tokio::test]
async fn get_skill_not_found() -> common::TestResult {
    let app = common::test_router();
    let req = common::request(
        Request::builder().uri("/api/v1/skills/nonexistent"),
        Body::empty(),
    )?;
    let resp = app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
async fn list_sessions_empty() -> common::TestResult {
    let app = common::test_router();
    let req = common::request(Request::builder().uri("/api/v1/sessions"), Body::empty())?;
    let resp = app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let json = common::json_body(resp).await?;
    assert!(common::json_array(&json)?.is_empty());
    Ok(())
}

#[tokio::test]
async fn get_session_not_found() -> common::TestResult {
    let app = common::test_router();
    let id = uuid::Uuid::new_v4();
    let req = common::request(
        Request::builder().uri(format!("/api/v1/sessions/{id}")),
        Body::empty(),
    )?;
    let resp = app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
async fn dlq_list_empty() -> common::TestResult {
    let app = common::test_router();
    let req = common::request(Request::builder().uri("/api/v1/dlq"), Body::empty())?;
    let resp = app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let json = common::json_body(resp).await?;
    assert!(common::json_array(&json)?.is_empty());
    Ok(())
}

#[tokio::test]
async fn dlq_purge_empty() -> common::TestResult {
    let app = common::test_router();
    let req = common::request(
        Request::builder().method("DELETE").uri("/api/v1/dlq"),
        Body::empty(),
    )?;
    let resp = app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let json = common::json_body(resp).await?;
    assert_eq!(json["purged"], 0);
    Ok(())
}

#[tokio::test]
async fn dlq_replay_not_found() -> common::TestResult {
    let app = common::test_router();
    let id = uuid::Uuid::new_v4();
    let req = common::request(
        Request::builder()
            .method("POST")
            .uri(format!("/api/v1/dlq/{id}/replay")),
        Body::empty(),
    )?;
    let resp = app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
async fn list_workspaces() -> common::TestResult {
    let app = common::test_router();
    let req = common::request(Request::builder().uri("/api/v1/workspaces"), Body::empty())?;
    let resp = app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let json = common::json_body(resp).await?;
    assert!(json.is_array());
    Ok(())
}

#[tokio::test]
async fn get_graph() -> common::TestResult {
    let app = common::test_router();
    let req = common::request(Request::builder().uri("/api/v1/graph"), Body::empty())?;
    let resp = app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let json = common::json_body(resp).await?;
    assert!(json["id"].is_string());
    assert!(json["entry"].is_string());
    assert!(json["nodes"].is_array());
    assert!(json["edges"].is_array());
    Ok(())
}

#[tokio::test]
async fn experience_status_disabled() -> common::TestResult {
    let app = common::test_router();
    let req = common::request(
        Request::builder().uri("/api/v1/experience/status"),
        Body::empty(),
    )?;
    let resp = app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let json = common::json_body(resp).await?;
    assert_eq!(json["enabled"], false);
    Ok(())
}

#[tokio::test]
async fn schedules_not_enabled() -> common::TestResult {
    let app = common::test_router();
    let req = common::request(Request::builder().uri("/api/v1/schedules"), Body::empty())?;
    let resp = app.oneshot(req).await?;
    // scheduler_store is None → 503 Service Unavailable
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    Ok(())
}
