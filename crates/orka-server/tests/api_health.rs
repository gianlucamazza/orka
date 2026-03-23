//! Tests for the public health and version endpoints.
//!
//! Uses `tower::ServiceExt::oneshot` — no TCP listener required.

mod common;

use axum::body::Body;
use http::{Request, StatusCode};
use tower::ServiceExt;

#[tokio::test]
async fn health_returns_ok() {
    let app = common::test_router();
    let req = Request::builder()
        .uri("/health")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "ok");
    assert!(json["uptime_secs"].is_number());
    assert!(json["workers"].is_number());
    assert!(json["queue_depth"].is_number());
}

#[tokio::test]
async fn health_live_returns_ok() {
    let app = common::test_router();
    let req = Request::builder()
        .uri("/health/live")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "ok");
}

#[tokio::test]
async fn version_returns_build_info() {
    let app = common::test_router();
    let req = Request::builder()
        .uri("/api/v1/version")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json["version"].is_string(), "missing version field");
    assert!(json["git_sha"].is_string(), "missing git_sha field");
    assert!(json["build_date"].is_string(), "missing build_date field");
}

#[tokio::test]
async fn openapi_spec_accessible() {
    let app = common::test_router();
    let req = Request::builder()
        .uri("/api-doc/openapi.json")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json.is_object(), "OpenAPI spec should be a JSON object");
    assert!(
        json["info"].is_object(),
        "OpenAPI spec should have an info field"
    );
}

#[tokio::test]
async fn security_headers_present() {
    let app = common::test_router();
    let req = Request::builder()
        .uri("/health/live")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();

    let headers = resp.headers();
    assert_eq!(
        headers
            .get("x-content-type-options")
            .and_then(|v| v.to_str().ok()),
        Some("nosniff")
    );
    assert_eq!(
        headers.get("x-frame-options").and_then(|v| v.to_str().ok()),
        Some("DENY")
    );
}
