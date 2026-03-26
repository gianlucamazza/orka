#![allow(missing_docs)]

//! Tests for the public health and version endpoints.
//!
//! Uses `tower::ServiceExt::oneshot` — no TCP listener required.

mod common;

use axum::body::Body;
use http::{Request, StatusCode};
use tower::ServiceExt;

#[tokio::test]
async fn health_returns_ok() -> common::TestResult {
    let app = common::test_router();
    let req = common::request(Request::builder().uri("/health"), Body::empty())?;
    let resp = app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let json = common::json_body(resp).await?;
    assert_eq!(json["status"], "ok");
    assert!(json["uptime_secs"].is_number());
    assert!(json["workers"].is_number());
    assert!(json["queue_depth"].is_number());
    Ok(())
}

#[tokio::test]
async fn health_live_returns_ok() -> common::TestResult {
    let app = common::test_router();
    let req = common::request(Request::builder().uri("/health/live"), Body::empty())?;
    let resp = app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let json = common::json_body(resp).await?;
    assert_eq!(json["status"], "ok");
    Ok(())
}

#[tokio::test]
async fn version_returns_build_info() -> common::TestResult {
    let app = common::test_router();
    let req = common::request(Request::builder().uri("/api/v1/version"), Body::empty())?;
    let resp = app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let json = common::json_body(resp).await?;
    assert!(json["version"].is_string(), "missing version field");
    assert!(json["git_sha"].is_string(), "missing git_sha field");
    assert!(json["build_date"].is_string(), "missing build_date field");
    Ok(())
}

#[tokio::test]
async fn openapi_spec_accessible() -> common::TestResult {
    let app = common::test_router();
    let req = common::request(
        Request::builder().uri("/api-doc/openapi.json"),
        Body::empty(),
    )?;
    let resp = app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let json = common::json_body(resp).await?;
    assert!(json.is_object(), "OpenAPI spec should be a JSON object");
    assert!(
        json["info"].is_object(),
        "OpenAPI spec should have an info field"
    );
    Ok(())
}

#[tokio::test]
async fn security_headers_present() -> common::TestResult {
    let app = common::test_router();
    let req = common::request(Request::builder().uri("/health/live"), Body::empty())?;
    let resp = app.oneshot(req).await?;

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
    Ok(())
}
