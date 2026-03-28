use std::{
    sync::Arc,
    task::{Context, Poll},
};

use axum::{body::Body, response::IntoResponse};
use http::{Request, Response};
use tower_layer::Layer;
use tower_service::Service;

use crate::{
    AuthConfig,
    authenticator::Authenticator,
    types::{AuthIdentity, Credentials},
};

/// Authentication middleware configuration.
#[derive(Clone)]
pub struct AuthMiddlewareConfig {
    /// Whether authentication is enabled.
    pub enabled: bool,
    /// Header name for API key authentication.
    pub api_key_header: http::HeaderName,
}

impl Default for AuthMiddlewareConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            api_key_header: http::HeaderName::from_static("x-api-key"),
        }
    }
}

/// Tower layer that injects authentication middleware.
#[derive(Clone)]
pub struct AuthLayer {
    authenticator: Arc<dyn Authenticator>,
    config: Arc<AuthMiddlewareConfig>,
}

impl AuthLayer {
    /// Create the layer with the given authenticator and middleware config.
    pub fn new(authenticator: Arc<dyn Authenticator>, config: Arc<AuthMiddlewareConfig>) -> Self {
        Self {
            authenticator,
            config,
        }
    }

    /// Create the layer with the given authenticator and default config (auth
    /// enabled).
    pub fn new_with_auth_config(
        authenticator: Arc<dyn Authenticator>,
        _config: &AuthConfig,
    ) -> Self {
        Self {
            authenticator,
            config: Arc::new(AuthMiddlewareConfig::default()),
        }
    }
}

impl<S> Layer<S> for AuthLayer {
    type Service = AuthService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        AuthService {
            inner,
            authenticator: self.authenticator.clone(),
            config: self.config.clone(),
        }
    }
}

/// Tower service that performs authentication before forwarding requests.
#[derive(Clone)]
pub struct AuthService<S> {
    inner: S,
    authenticator: Arc<dyn Authenticator>,
    config: Arc<AuthMiddlewareConfig>,
}

impl<S> Service<Request<Body>> for AuthService<S>
where
    S: Service<Request<Body>, Response = Response<Body>> + Clone + Send + 'static,
    S::Future: Send,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<Body>) -> Self::Future {
        let authenticator = self.authenticator.clone();
        let config = self.config.clone();
        let mut inner = self.inner.clone();
        // Swap so `self.inner` keeps a ready clone for the next call.
        std::mem::swap(&mut self.inner, &mut inner);

        Box::pin(async move {
            if !config.enabled {
                req.extensions_mut().insert(AuthIdentity::anonymous());
                return inner.call(req).await;
            }

            // Check for Bearer token first, then fall back to API key header.
            let creds = if let Some(auth_header) = req
                .headers()
                .get(http::header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
            {
                if let Some(token) = auth_header.strip_prefix("Bearer ") {
                    Credentials::Bearer(token.to_owned())
                } else {
                    Credentials::None
                }
            } else if let Some(api_key) = req
                .headers()
                .get(&config.api_key_header)
                .and_then(|v| v.to_str().ok())
            {
                Credentials::ApiKey(api_key.to_owned())
            } else {
                Credentials::None
            };

            if let Ok(identity) = authenticator.authenticate(&creds).await {
                req.extensions_mut().insert(identity);
                inner.call(req).await
            } else {
                let resp = (
                    http::StatusCode::UNAUTHORIZED,
                    axum::Json(serde_json::json!({"error": "unauthorized"})),
                )
                    .into_response();
                Ok(resp)
            }
        })
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::field_reassign_with_default,
    clippy::default_trait_access,
    clippy::needless_pass_by_value,
    clippy::stable_sort_primitive,
    clippy::unused_async,
    clippy::map_unwrap_or
)]
mod tests {
    use http::StatusCode;
    use tower::ServiceExt;

    use super::*;
    use crate::testing::InMemoryAuthenticator;

    fn auth_config(enabled: bool) -> Arc<AuthMiddlewareConfig> {
        Arc::new(AuthMiddlewareConfig {
            enabled,
            api_key_header: http::HeaderName::from_static("x-api-key"),
        })
    }

    /// Simple echo service that returns 200 with the principal in the body.
    async fn echo_handler(req: Request<Body>) -> Result<Response<Body>, std::convert::Infallible> {
        let principal = req
            .extensions()
            .get::<AuthIdentity>()
            .map(|id| id.principal.clone())
            .unwrap_or_else(|| "none".into());
        Ok(Response::builder()
            .status(200)
            .body(Body::from(principal))
            .unwrap())
    }

    macro_rules! make_service {
        ($auth:expr, $config:expr) => {{
            let layer = AuthLayer::new($auth, $config);
            let svc = tower::service_fn(|req: Request<Body>| echo_handler(req));
            layer.layer(svc)
        }};
    }

    #[tokio::test]
    async fn auth_disabled_passes_anonymous() {
        let auth = Arc::new(InMemoryAuthenticator::new());
        let mut svc = make_service!(auth, auth_config(false));

        let req = Request::builder().uri("/").body(Body::empty()).unwrap();
        let resp = svc.ready().await.unwrap().call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        assert_eq!(&body[..], b"anonymous");
    }

    #[tokio::test]
    async fn valid_api_key_passes() {
        let auth = Arc::new(InMemoryAuthenticator::new().with_key(
            "secret-key",
            "admin",
            vec!["read".into()],
        ));
        let mut svc = make_service!(auth, auth_config(true));

        let req = Request::builder()
            .uri("/")
            .header("X-Api-Key", "secret-key")
            .body(Body::empty())
            .unwrap();
        let resp = svc.ready().await.unwrap().call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        assert_eq!(&body[..], b"admin");
    }

    #[tokio::test]
    async fn missing_credentials_returns_401() {
        let auth = Arc::new(InMemoryAuthenticator::new().with_key("secret-key", "admin", vec![]));
        let mut svc = make_service!(auth, auth_config(true));

        let req = Request::builder().uri("/").body(Body::empty()).unwrap();
        let resp = svc.ready().await.unwrap().call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn invalid_api_key_returns_401() {
        let auth = Arc::new(InMemoryAuthenticator::new().with_key("real-key", "admin", vec![]));
        let mut svc = make_service!(auth, auth_config(true));

        let req = Request::builder()
            .uri("/")
            .header("X-Api-Key", "wrong-key")
            .body(Body::empty())
            .unwrap();
        let resp = svc.ready().await.unwrap().call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn bearer_token_extracted() {
        // InMemoryAuthenticator doesn't support Bearer, so this should 401
        // but we verify the extraction path runs
        let auth = Arc::new(InMemoryAuthenticator::new());
        let mut svc = make_service!(auth, auth_config(true));

        let req = Request::builder()
            .uri("/")
            .header("Authorization", "Bearer some-token")
            .body(Body::empty())
            .unwrap();
        let resp = svc.ready().await.unwrap().call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
