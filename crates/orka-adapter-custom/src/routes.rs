use axum::{
    Json, Router,
    extract::{
        Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use futures_util::{SinkExt, StreamExt};
use orka_contracts::{InboundInteraction, InteractionContent, PlatformContext, SenderInfo, TraceContext};
use orka_core::{InteractionSink, SessionId, StreamRegistry};
use serde::Deserialize;
use tower_http::{
    cors::{AllowMethods, AllowOrigin, CorsLayer},
    limit::RequestBodyLimitLayer,
    trace::TraceLayer,
};
use tracing::info;
use uuid::Uuid;

use crate::{
    types::{InboundRequest, InboundResponse},
    ws::WsRegistry,
};

/// Maximum request body size: 1 MB.
const MAX_BODY_SIZE: usize = 1024 * 1024;

/// Shared state injected into axum route handlers.
#[derive(Clone)]
pub struct AppState {
    /// Sender that routes inbound interactions into the Orka bus bridge.
    pub sink: InteractionSink,
    /// Registry tracking active WebSocket connections per session.
    pub ws_registry: WsRegistry,
    /// Registry for streaming SSE/WS responses back to clients.
    pub stream_registry: StreamRegistry,
    /// Trust level declared by this adapter, stamped on every inbound interaction.
    pub trust_level: orka_contracts::TrustLevel,
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
    sink: InteractionSink,
    ws_registry: WsRegistry,
    stream_registry: StreamRegistry,
    auth_layer: Option<orka_auth::AuthLayer>,
    trust_level: orka_contracts::TrustLevel,
) -> Router {
    let state = AppState {
        sink,
        ws_registry,
        stream_registry,
        trust_level,
    };

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
        .allow_methods(AllowMethods::list([http::Method::GET, http::Method::POST]))
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

#[utoipa::path(
    post,
    path = "/api/v1/message",
    request_body = InboundRequest,
    responses(
        (status = 200, description = "Message accepted", body = InboundResponse),
        (status = 500, description = "Internal error")
    ),
    tag = "messages"
)]
/// POST `/api/v1/message` — accept an inbound message and route it into the
/// bus.
pub async fn handle_message(
    State(state): State<AppState>,
    Json(req): Json<InboundRequest>,
) -> impl IntoResponse {
    let session_uuid = req
        .session_id
        .and_then(|s| s.parse::<Uuid>().ok())
        .unwrap_or_else(Uuid::now_v7);

    let interaction_id = Uuid::now_v7();
    let message_id = interaction_id.to_string();
    let session_id_str = session_uuid.to_string();

    let interaction = InboundInteraction {
        id: interaction_id,
        source_channel: "custom".into(),
        session_id: session_uuid,
        timestamp: chrono::Utc::now(),
        content: InteractionContent::Text(req.text),
        context: PlatformContext {
            sender: SenderInfo {
                user_id: req.user_id,
                ..Default::default()
            },
            extensions: req.metadata.unwrap_or_default(),
            trust_level: Some(state.trust_level),
            ..Default::default()
        },
        trace: TraceContext::default(),
    };

    match state.sink.send(interaction).await {
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

#[utoipa::path(
    get,
    path = "/api/v1/health",
    responses(
        (status = 200, description = "Health check", body = serde_json::Value)
    ),
    tag = "health"
)]
/// GET `/api/v1/health` — liveness check that always returns `{"status":
/// "ok"}`.
pub async fn handle_health() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok" }))
}

#[derive(Deserialize)]
struct WsParams {
    session_id: String,
    channels: Option<String>, // comma-separated, e.g. "telegram,discord"
}

async fn handle_ws(
    State(state): State<AppState>,
    Query(params): Query<WsParams>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let session_id = match params.session_id.parse::<Uuid>() {
        Ok(uuid) => SessionId::from(uuid),
        Err(_) => {
            return (StatusCode::BAD_REQUEST, "invalid session_id").into_response();
        }
    };

    let channel_filter = params.channels.as_deref().map(|s| {
        s.split(',')
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty())
            .collect::<Vec<_>>()
    });

    ws.on_upgrade(move |socket| {
        handle_ws_connection(
            socket,
            state.ws_registry,
            state.stream_registry,
            session_id,
            channel_filter,
        )
    })
}

async fn handle_ws_connection(
    socket: WebSocket,
    registry: WsRegistry,
    stream_registry: StreamRegistry,
    session_id: SessionId,
    channel_filter: Option<Vec<String>>,
) {
    info!(%session_id, "WebSocket connected");

    let (tx, mut rx) = registry.register(session_id).await;
    let mut stream_rx = stream_registry.subscribe(session_id);
    let (mut ws_sink, mut ws_stream) = socket.split();

    // Forward stream chunks, final outbound messages, and periodic pings to the
    // WebSocket. The ping keeps the connection alive through proxies/NAT gateways
    // that close idle connections (most default to 30–60 s).
    let send_task = tokio::spawn(async move {
        let mut ping_interval = tokio::time::interval(std::time::Duration::from_secs(25));
        ping_interval.tick().await; // skip immediate tick

        loop {
            tokio::select! {
                // Stream chunks (real-time deltas, tool status) — always forwarded
                chunk = stream_rx.recv() => {
                    if let Some(chunk) = chunk
                        && let Ok(json) = serde_json::to_string(&orka_contracts::RealtimeEvent::from(chunk.kind.clone()))
                            && ws_sink.send(Message::Text(json.into())).await.is_err() {
                                break;
                            }
                }
                // Final outbound messages (backward compat)
                msg = rx.recv() => {
                    match msg {
                        Some(text) => {
                            // Apply channel filter: skip if channel not in the allowed list
                            if let Some(ref filter) = channel_filter
                                && let Ok(val) = serde_json::from_str::<serde_json::Value>(&text)
                                && let Some(ch) = val.get("channel").and_then(|v| v.as_str())
                                && !filter.iter().any(|f| f == ch)
                            {
                                continue;
                            }
                            if ws_sink.send(Message::Text(text.into())).await.is_err() {
                                break;
                            }
                        }
                        None => break,
                    }
                }
                // Keepalive ping — sent every 25 s. Axum responds to client
                // Pong frames automatically; the Ping itself resets NAT/proxy
                // idle timers and detects dead connections (send failure → break).
                _ = ping_interval.tick() => {
                    if ws_sink.send(Message::Ping(vec![].into())).await.is_err() {
                        break;
                    }
                }
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
