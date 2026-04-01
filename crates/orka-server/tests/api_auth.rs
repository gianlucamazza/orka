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
const JWT_SECRET: &str = "test-secret-key-at-least-32-bytes-long!";
const JWT_ISSUER: &str = "orka-tests";

#[tokio::test]
async fn protected_route_without_key_returns_401() -> common::TestResult {
    let app = common::test_router_with_auth(TEST_KEY);
    let req = common::request(Request::builder().uri(PROTECTED), Body::empty())?;
    let resp = app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

#[tokio::test]
async fn protected_route_with_valid_key_returns_200() -> common::TestResult {
    let app = common::test_router_with_auth(TEST_KEY);
    let req = common::request(
        Request::builder()
            .uri(PROTECTED)
            .header("X-Api-Key", TEST_KEY),
        Body::empty(),
    )?;
    let resp = app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    Ok(())
}

#[tokio::test]
async fn protected_route_with_wrong_key_returns_401() -> common::TestResult {
    let app = common::test_router_with_auth(TEST_KEY);
    let req = common::request(
        Request::builder()
            .uri(PROTECTED)
            .header("X-Api-Key", "wrong-key"),
        Body::empty(),
    )?;
    let resp = app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

#[tokio::test]
async fn public_route_always_ok() -> common::TestResult {
    let app = common::test_router_with_auth(TEST_KEY);
    let req = common::request(Request::builder().uri(PUBLIC), Body::empty())?;
    let resp = app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    Ok(())
}

#[tokio::test]
async fn composite_auth_accepts_api_key_and_jwt() -> common::TestResult {
    let app = common::test_router_with_composite_auth(TEST_KEY, JWT_SECRET, JWT_ISSUER);
    let jwt = common::make_jwt(JWT_SECRET, JWT_ISSUER, "user-123", &["chat:read"]);

    let operator_req = common::request(
        Request::builder()
            .uri(PROTECTED)
            .header("X-Api-Key", TEST_KEY),
        Body::empty(),
    )?;
    let operator_resp = app.clone().oneshot(operator_req).await?;
    assert_eq!(operator_resp.status(), StatusCode::OK);

    let mobile_req = common::request(
        Request::builder()
            .uri("/mobile/v1/me")
            .header("Authorization", format!("Bearer {jwt}")),
        Body::empty(),
    )?;
    let mobile_resp = app.oneshot(mobile_req).await?;
    assert_eq!(mobile_resp.status(), StatusCode::OK);
    Ok(())
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
async fn a2a_agent_card_is_always_public_when_auth_enabled() -> common::TestResult {
    let app = common::test_router_with_a2a(TEST_KEY, true);
    let req = common::request(Request::builder().uri(A2A_AGENT_CARD), Body::empty())?;
    let resp = app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    Ok(())
}

#[tokio::test]
async fn a2a_rpc_without_key_returns_401_when_auth_enabled() -> common::TestResult {
    let app = common::test_router_with_a2a(TEST_KEY, true);
    let req = common::request(
        Request::builder()
            .method("POST")
            .uri(A2A_RPC)
            .header("content-type", "application/json"),
        a2a_body(),
    )?;
    let resp = app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

#[tokio::test]
async fn a2a_rpc_with_valid_key_returns_200_when_auth_enabled() -> common::TestResult {
    let app = common::test_router_with_a2a(TEST_KEY, true);
    let req = common::request(
        Request::builder()
            .method("POST")
            .uri(A2A_RPC)
            .header("content-type", "application/json")
            .header("X-Api-Key", TEST_KEY),
        a2a_body(),
    )?;
    let resp = app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    Ok(())
}

#[tokio::test]
async fn a2a_rpc_without_key_returns_200_when_auth_disabled() -> common::TestResult {
    let app = common::test_router_with_a2a(TEST_KEY, false);
    let req = common::request(
        Request::builder()
            .method("POST")
            .uri(A2A_RPC)
            .header("content-type", "application/json"),
        a2a_body(),
    )?;
    let resp = app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    Ok(())
}

// ── Discovery endpoint test
// ───────────────────────────────────────────────────

#[tokio::test]
async fn a2a_agents_discovery_returns_empty_list() -> common::TestResult {
    let app = common::test_router_with_a2a(TEST_KEY, false);
    let req = common::request(Request::builder().uri("/api/v1/a2a/agents"), Body::empty())?;
    let resp = app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let json = common::json_body(resp).await?;
    assert!(json.is_array(), "expected JSON array, got: {json}");
    Ok(())
}
