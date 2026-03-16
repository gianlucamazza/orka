use axum::{
    extract::{
        ws::{Message, WebSocket},
        Query, State, WebSocketUpgrade,
    },
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use futures_util::{SinkExt, StreamExt};
use orka_core::{Envelope, MessageSink, SessionId};
use serde::Deserialize;
use tower_http::cors::{AllowMethods, AllowOrigin, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;
use tracing::info;
use uuid::Uuid;

use crate::types::{InboundRequest, InboundResponse};
use crate::ws::WsRegistry;

/// Maximum request body size: 1 MB.
const MAX_BODY_SIZE: usize = 1024 * 1024;

#[derive(Clone)]
pub struct AppState {
    pub sink: MessageSink,
    pub ws_registry: WsRegistry,
}

/// Middleware that adds security headers to all responses.
async fn security_headers(
    req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> impl IntoResponse {
    let mut response = next.run(req).await;
    let headers = response.headers_mut();
    headers.insert(
        http::header::X_CONTENT_TYPE_OPTIONS,
        http::HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        http::header::X_FRAME_OPTIONS,
        http::HeaderValue::from_static("DENY"),
    );
    headers.insert(
        http::header::STRICT_TRANSPORT_SECURITY,
        http::HeaderValue::from_static("max-age=63072000; includeSubDomains"),
    );
    headers.insert(
        http::HeaderName::from_static("x-content-security-policy"),
        http::HeaderValue::from_static("default-src 'none'"),
    );
    response
}

/// Build the application router with shared state.
pub fn app_router(
    sink: MessageSink,
    ws_registry: WsRegistry,
    auth_layer: Option<orka_auth::AuthLayer>,
) -> Router {
    let state = AppState { sink, ws_registry };

    let health = Router::new().route("/api/v1/health", get(handle_health));

    let protected = {
        let r = Router::new()
            .route("/api/v1/message", post(handle_message))
            .route("/api/v1/ws", get(handle_ws));
        match auth_layer {
            Some(layer) => r.layer(layer),
            None => r,
        }
    };

    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list(Vec::<http::HeaderValue>::new()))
        .allow_methods(AllowMethods::list([
            http::Method::GET,
            http::Method::POST,
        ]))
        .allow_headers([http::header::CONTENT_TYPE, http::header::AUTHORIZATION])
        .max_age(std::time::Duration::from_secs(3600));

    protected
        .merge(health)
        .layer(cors)
        .layer(axum::middleware::from_fn(security_headers))
        .layer(RequestBodyLimitLayer::new(MAX_BODY_SIZE))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn handle_message(
    State(state): State<AppState>,
    Json(req): Json<InboundRequest>,
) -> impl IntoResponse {
    let session_id = req
        .session_id
        .and_then(|s| s.parse::<Uuid>().ok())
        .map(SessionId)
        .unwrap_or_else(SessionId::new);

    let mut envelope = Envelope::text("custom", session_id.clone(), &req.text);

    if let Some(metadata) = req.metadata {
        envelope.metadata = metadata;
    }

    let message_id = envelope.id.to_string();
    let session_id_str = session_id.to_string();

    match state.sink.send(envelope).await {
        Ok(()) => (
            StatusCode::OK,
            Json(InboundResponse {
                message_id,
                session_id: session_id_str,
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn handle_health() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok" }))
}

#[derive(Deserialize)]
struct WsParams {
    session_id: String,
}

async fn handle_ws(
    State(state): State<AppState>,
    Query(params): Query<WsParams>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let session_id = match params.session_id.parse::<Uuid>() {
        Ok(uuid) => SessionId(uuid),
        Err(_) => {
            return (StatusCode::BAD_REQUEST, "invalid session_id").into_response();
        }
    };

    ws.on_upgrade(move |socket| handle_ws_connection(socket, state.ws_registry, session_id))
}

async fn handle_ws_connection(socket: WebSocket, registry: WsRegistry, session_id: SessionId) {
    info!(%session_id, "WebSocket connected");

    let (tx, mut rx) = registry.register(session_id.clone()).await;
    let (mut ws_sink, mut ws_stream) = socket.split();

    // Forward messages from registry channel to WS frames
    let send_task = tokio::spawn(async move {
        while let Some(text) = rx.recv().await {
            if ws_sink.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });

    // Read loop: consume messages (handle pings implicitly via axum)
    while let Some(Ok(msg)) = ws_stream.next().await {
        if let Message::Close(_) = msg {
            break;
        }
    }

    // Cleanup
    send_task.abort();
    registry.deregister(&session_id, &tx).await;
    info!(%session_id, "WebSocket disconnected");
}
