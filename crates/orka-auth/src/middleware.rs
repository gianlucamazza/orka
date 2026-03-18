use std::sync::Arc;
use std::task::{Context, Poll};

use axum::body::Body;
use axum::response::IntoResponse;
use http::{Request, Response};
use tower_layer::Layer;
use tower_service::Service;

use orka_core::config::AuthConfig;

use crate::authenticator::Authenticator;
use crate::types::{AuthIdentity, Credentials};

/// Tower layer that injects authentication middleware.
#[derive(Clone)]
pub struct AuthLayer {
    authenticator: Arc<dyn Authenticator>,
    config: Arc<AuthConfig>,
}

impl AuthLayer {
    /// Create the layer with the given authenticator and auth config.
    pub fn new(authenticator: Arc<dyn Authenticator>, config: Arc<AuthConfig>) -> Self {
        Self {
            authenticator,
            config,
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
    config: Arc<AuthConfig>,
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

            match authenticator.authenticate(&creds).await {
                Ok(identity) => {
                    req.extensions_mut().insert(identity);
                    inner.call(req).await
                }
                Err(_) => {
                    let resp = (
                        http::StatusCode::UNAUTHORIZED,
                        axum::Json(serde_json::json!({"error": "unauthorized"})),
                    )
                        .into_response();
                    Ok(resp)
                }
            }
        })
    }
}
