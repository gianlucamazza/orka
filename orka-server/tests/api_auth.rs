//! Tests for the authentication middleware on protected API routes.
//!
//! Uses `tower::ServiceExt::oneshot` — no TCP listener required.

mod common;

use axum::body::Body;
use http::{Request, StatusCode};
use tower::ServiceExt;

/// The protected route used across auth tests.
const PROTECTED: &str = "/api/v1/skills";
/// The public route (health) which should never require auth.
const PUBLIC: &str = "/health/live";
const TEST_KEY: &str = "test-secret-key-abc123";

#[tokio::test]
async fn protected_route_without_key_returns_401() {
    let app = common::test_router_with_auth(TEST_KEY);
    let req = Request::builder().uri(PROTECTED).body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn protected_route_with_valid_key_returns_200() {
    let app = common::test_router_with_auth(TEST_KEY);
    let req = Request::builder()
        .uri(PROTECTED)
        .header("X-Api-Key", TEST_KEY)
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn protected_route_with_wrong_key_returns_401() {
    let app = common::test_router_with_auth(TEST_KEY);
    let req = Request::builder()
        .uri(PROTECTED)
        .header("X-Api-Key", "wrong-key")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn public_route_always_ok() {
    let app = common::test_router_with_auth(TEST_KEY);
    let req = Request::builder().uri(PUBLIC).body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}
