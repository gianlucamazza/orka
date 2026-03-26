#![allow(missing_docs)]

use std::sync::Arc;

use axum::{body::Body, response::IntoResponse};
use http::{Request, StatusCode};
use orka_auth::{
    ApiKeyAuthenticator,
    middleware::{AuthLayer, AuthMiddlewareConfig},
    types::AuthIdentity,
};
use orka_core::config::ApiKeyEntry;
use sha2::{Digest, Sha256};
use tower::ServiceExt;

fn hash_key(raw: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raw.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn make_auth_layer(raw_key: &str, key_name: &str) -> AuthLayer {
    let entries = vec![ApiKeyEntry::new(
        key_name,
        hash_key(raw_key),
        vec!["read".into()],
    )];
    let authenticator = Arc::new(ApiKeyAuthenticator::new(&entries));
    let config = Arc::new(AuthMiddlewareConfig::default());
    AuthLayer::new(authenticator, config)
}

/// Build a simple axum Router wrapped in the `AuthLayer` for testing.
fn make_app(raw_key: &str, key_name: &str) -> axum::Router {
    use axum::routing::get;

    async fn echo_identity(
        axum::Extension(identity): axum::Extension<AuthIdentity>,
    ) -> axum::response::Response {
        let body = format!("principal:{}", identity.principal);
        (StatusCode::OK, body).into_response()
    }

    let layer = make_auth_layer(raw_key, key_name);
    axum::Router::new()
        .route("/", get(echo_identity))
        .layer(layer)
}

#[tokio::test]
async fn valid_api_key_returns_200_with_identity() {
    let raw_key = "test-secret-key-abc123";
    let app = make_app(raw_key, "test-client");

    let req = Request::builder()
        .uri("/")
        .header("X-Api-Key", raw_key)
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&body).contains("test-client"));
}

#[tokio::test]
async fn invalid_api_key_returns_401() {
    let app = make_app("correct-key", "test-client");

    let req = Request::builder()
        .uri("/")
        .header("X-Api-Key", "wrong-key")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn no_auth_header_returns_401_when_auth_enabled() {
    let app = make_app("some-key", "test-client");

    let req = Request::builder().uri("/").body(Body::empty()).unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
