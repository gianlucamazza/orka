#![allow(missing_docs)]

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
    let req = Request::builder()
        .uri(PROTECTED)
        .body(Body::empty())
        .unwrap();
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

// ── A2A auth scenario tests
// ───────────────────────────────────────────────────

const A2A_AGENT_CARD: &str = "/.well-known/agent.json";
const A2A_RPC: &str = "/a2a";

/// A minimal valid JSON-RPC body for `tasks/list` (reads no state, always
/// safe).
fn a2a_body() -> Body {
    Body::from(r#"{"jsonrpc":"2.0","id":1,"method":"tasks/list","params":{"filter":{}}}"#)
}

#[tokio::test]
async fn a2a_agent_card_is_always_public_when_auth_enabled() {
    let app = common::test_router_with_a2a(TEST_KEY, true);
    let req = Request::builder()
        .uri(A2A_AGENT_CARD)
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn a2a_rpc_without_key_returns_401_when_auth_enabled() {
    let app = common::test_router_with_a2a(TEST_KEY, true);
    let req = Request::builder()
        .method("POST")
        .uri(A2A_RPC)
        .header("content-type", "application/json")
        .body(a2a_body())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a2a_rpc_with_valid_key_returns_200_when_auth_enabled() {
    let app = common::test_router_with_a2a(TEST_KEY, true);
    let req = Request::builder()
        .method("POST")
        .uri(A2A_RPC)
        .header("content-type", "application/json")
        .header("X-Api-Key", TEST_KEY)
        .body(a2a_body())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn a2a_rpc_without_key_returns_200_when_auth_disabled() {
    let app = common::test_router_with_a2a(TEST_KEY, false);
    let req = Request::builder()
        .method("POST")
        .uri(A2A_RPC)
        .header("content-type", "application/json")
        .body(a2a_body())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// ── Discovery endpoint test
// ───────────────────────────────────────────────────

#[tokio::test]
async fn a2a_agents_discovery_returns_empty_list() {
    let app = common::test_router_with_a2a(TEST_KEY, false);
    let req = Request::builder()
        .uri("/api/v1/a2a/agents")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json.is_array(), "expected JSON array, got: {json}");
}
