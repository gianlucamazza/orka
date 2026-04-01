use std::{collections::HashMap, convert::Infallible, sync::Arc};

use axum::{
    Extension, Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{
        IntoResponse,
        sse::{Event, Sse},
    },
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use orka_auth::AuthIdentity;
use orka_core::{
    Conversation, ConversationId, ConversationMessage, ConversationMessageRole, ConversationStatus,
    MessageId, SessionId, StreamChunkKind, StreamRegistry,
    traits::{ConversationStore, MessageBus},
};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, mpsc};
use tokio_stream::{StreamExt, wrappers::UnboundedReceiverStream};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::mobile_auth::{
    CompletePairingInput, MobileAuthError, MobileAuthService, MobileSession, PairingStatus,
    RefreshInput,
};

const DEFAULT_PAGE_SIZE: usize = 20;
const MAX_PAGE_SIZE: usize = 100;

/// Realtime event pushed to mobile clients after transcript persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum MobileStreamEvent {
    /// Assistant message has been finalized and persisted.
    MessageCompleted {
        /// Final persisted assistant message payload.
        message: ConversationMessage,
    },
    /// Message generation failed.
    MessageFailed {
        /// Conversation that failed.
        conversation_id: ConversationId,
        /// Human-readable error string.
        error: String,
    },
}

/// Per-conversation event hub used by the mobile product API.
#[derive(Clone, Default)]
pub struct MobileEventHub {
    inner: Arc<Mutex<HashMap<ConversationId, Vec<mpsc::UnboundedSender<MobileStreamEvent>>>>>,
}

impl MobileEventHub {
    /// Create an empty event hub.
    pub fn new() -> Self {
        Self::default()
    }

    /// Subscribe to events for a conversation.
    pub async fn subscribe(
        &self,
        conversation_id: ConversationId,
    ) -> mpsc::UnboundedReceiver<MobileStreamEvent> {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut inner = self.inner.lock().await;
        inner.entry(conversation_id).or_default().push(tx);
        rx
    }

    /// Publish an event to active subscribers.
    pub async fn publish(&self, conversation_id: ConversationId, event: MobileStreamEvent) {
        let mut inner = self.inner.lock().await;
        let Some(subscribers) = inner.get_mut(&conversation_id) else {
            return;
        };

        subscribers.retain(|tx| tx.send(event.clone()).is_ok());
        if subscribers.is_empty() {
            inner.remove(&conversation_id);
        }
    }
}

#[derive(Clone)]
pub(super) struct ProtectedMobileState {
    conversations: Arc<dyn ConversationStore>,
    bus: Arc<dyn MessageBus>,
    stream_registry: StreamRegistry,
    mobile_events: MobileEventHub,
    mobile_auth: Option<Arc<dyn MobileAuthService>>,
}

#[derive(Clone)]
pub(super) struct PublicMobileState {
    mobile_auth: Option<Arc<dyn MobileAuthService>>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub(super) struct CreateConversationRequest {
    title: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub(super) struct CreatePairingRequest {
    server_base_url: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub(super) struct SendMessageRequest {
    text: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub(super) struct CompletePairingRequest {
    pairing_id: String,
    pairing_secret: String,
    device_id: String,
    device_name: String,
    platform: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub(super) struct RefreshSessionRequest {
    refresh_token: String,
    device_id: String,
}

#[derive(Debug, Deserialize)]
struct PaginationQuery {
    limit: Option<usize>,
    offset: Option<usize>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(super) struct ApiError {
    error: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub(super) struct CurrentUserResponse {
    user_id: String,
    scopes: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(super) struct CreatePairingResponse {
    pairing_id: String,
    pairing_secret: String,
    expires_at: DateTime<Utc>,
    pairing_uri: String,
}

#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(super) enum PairingStatusKind {
    Pending,
    Completed,
    Expired,
}

#[derive(Debug, Serialize, ToSchema)]
pub(super) struct PairingStatusResponse {
    pairing_id: String,
    status: PairingStatusKind,
    expires_at: DateTime<Utc>,
    completed_at: Option<DateTime<Utc>>,
    device_label: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(super) struct MobileSessionResponse {
    access_token: String,
    access_token_expires_at: DateTime<Utc>,
    refresh_token: String,
    refresh_token_expires_at: DateTime<Utc>,
    user_id: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[allow(clippy::struct_field_names)]
pub(super) struct SendMessageResponse {
    conversation_id: ConversationId,
    session_id: SessionId,
    message_id: MessageId,
}

#[derive(Debug)]
struct SseFrame {
    event: &'static str,
    data: String,
}

fn stream_chunk_to_sse(
    kind: &StreamChunkKind,
    conversation_id: ConversationId,
    reply_to: Option<MessageId>,
) -> Option<SseFrame> {
    let (event, data) = match kind {
        StreamChunkKind::Delta(delta) => (
            "message_delta",
            serde_json::json!({
                "conversation_id": conversation_id,
                "reply_to": reply_to,
                "delta": delta,
            }),
        ),
        StreamChunkKind::Done => (
            "stream_done",
            serde_json::json!({ "conversation_id": conversation_id }),
        ),
        StreamChunkKind::ThinkingDelta(delta) => (
            "thinking_delta",
            serde_json::json!({
                "conversation_id": conversation_id,
                "delta": delta,
            }),
        ),
        StreamChunkKind::ToolExecStart {
            name,
            id,
            input_summary,
            category,
        } => (
            "tool_exec_start",
            serde_json::json!({
                "conversation_id": conversation_id,
                "id": id,
                "name": name,
                "input_summary": input_summary,
                "category": category,
            }),
        ),
        StreamChunkKind::ToolExecEnd {
            id,
            success,
            duration_ms,
            error,
            result_summary,
        } => (
            "tool_exec_end",
            serde_json::json!({
                "conversation_id": conversation_id,
                "id": id,
                "success": success,
                "duration_ms": duration_ms,
                "error": error,
                "result_summary": result_summary,
            }),
        ),
        StreamChunkKind::AgentSwitch { display_name, .. } => (
            "agent_switch",
            serde_json::json!({
                "conversation_id": conversation_id,
                "display_name": display_name,
            }),
        ),
        _ => return None,
    };
    Some(SseFrame {
        event,
        data: data.to_string(),
    })
}

/// Build the public mobile auth routes that remain accessible before login.
pub(super) fn public_routes(mobile_auth: Option<Arc<dyn MobileAuthService>>) -> Router {
    Router::new()
        .route(
            "/mobile/v1/pairings/complete",
            post(handle_complete_pairing),
        )
        .route("/mobile/v1/auth/refresh", post(handle_refresh_session))
        .with_state(PublicMobileState { mobile_auth })
}

/// Build the protected mobile product API routes.
pub(super) fn protected_routes(
    conversations: Arc<dyn ConversationStore>,
    bus: Arc<dyn MessageBus>,
    stream_registry: StreamRegistry,
    mobile_events: MobileEventHub,
    mobile_auth: Option<Arc<dyn MobileAuthService>>,
) -> Router {
    let state = ProtectedMobileState {
        conversations,
        bus,
        stream_registry,
        mobile_events,
        mobile_auth,
    };

    Router::new()
        .route("/mobile/v1/me", get(handle_me))
        .route("/mobile/v1/conversations", get(handle_list_conversations))
        .route("/mobile/v1/conversations", post(handle_create_conversation))
        .route("/mobile/v1/pairings", post(handle_create_pairing))
        .route("/mobile/v1/pairings/{id}", get(handle_get_pairing_status))
        .route(
            "/mobile/v1/conversations/{id}",
            get(handle_get_conversation),
        )
        .route(
            "/mobile/v1/conversations/{id}/messages",
            get(handle_list_messages).post(handle_send_message),
        )
        .route("/mobile/v1/conversations/{id}/stream", get(handle_stream))
        .with_state(state)
}

#[utoipa::path(
    get,
    path = "/mobile/v1/me",
    responses(
        (status = 200, description = "Current authenticated mobile user", body = CurrentUserResponse),
        (status = 401, description = "Authentication required", body = ApiError)
    ),
    tag = "mobile"
)]
/// GET `/mobile/v1/me` — return the current authenticated mobile identity.
async fn handle_me(Extension(identity): Extension<AuthIdentity>) -> impl IntoResponse {
    Json(CurrentUserResponse {
        user_id: identity.principal,
        scopes: identity.scopes,
    })
}

#[utoipa::path(
    post,
    path = "/mobile/v1/pairings",
    request_body = CreatePairingRequest,
    responses(
        (status = 201, description = "One-time pairing created", body = CreatePairingResponse),
        (status = 400, description = "Invalid request body", body = ApiError),
        (status = 401, description = "Authentication required", body = ApiError),
        (status = 503, description = "Mobile pairing unavailable on this server", body = ApiError)
    ),
    tag = "mobile"
)]
/// POST `/mobile/v1/pairings` — create a one-time pairing session for the
/// authenticated CLI/operator caller.
pub(super) async fn handle_create_pairing(
    State(state): State<ProtectedMobileState>,
    Extension(identity): Extension<AuthIdentity>,
    Json(body): Json<CreatePairingRequest>,
) -> impl IntoResponse {
    let Some(mobile_auth) = state.mobile_auth else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "mobile pairing is unavailable on this server",
        );
    };

    match mobile_auth
        .create_pairing(&identity.principal, &body.server_base_url)
        .await
    {
        Ok(created) => (
            StatusCode::CREATED,
            Json(CreatePairingResponse {
                pairing_id: created.pairing_id,
                pairing_secret: created.pairing_secret,
                expires_at: created.expires_at,
                pairing_uri: created.pairing_uri,
            }),
        )
            .into_response(),
        Err(error) => mobile_auth_error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/mobile/v1/pairings/{id}",
    params(
        ("id" = String, Path, description = "Pairing identifier")
    ),
    responses(
        (status = 200, description = "Pairing status for CLI polling", body = PairingStatusResponse),
        (status = 401, description = "Authentication required", body = ApiError),
        (status = 404, description = "Pairing session not found", body = ApiError),
        (status = 503, description = "Mobile pairing unavailable on this server", body = ApiError)
    ),
    tag = "mobile"
)]
/// GET `/mobile/v1/pairings/{id}` — poll the status of an existing pairing.
pub(super) async fn handle_get_pairing_status(
    State(state): State<ProtectedMobileState>,
    Extension(identity): Extension<AuthIdentity>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let Some(mobile_auth) = state.mobile_auth else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "mobile pairing is unavailable on this server",
        );
    };

    match mobile_auth
        .get_pairing_status(&identity.principal, &id)
        .await
    {
        Ok(Some(status)) => Json(PairingStatusResponse {
            pairing_id: status.pairing_id,
            status: map_pairing_status(status.status),
            expires_at: status.expires_at,
            completed_at: status.completed_at,
            device_label: status.device_label,
        })
        .into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "pairing session not found"),
        Err(error) => mobile_auth_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/mobile/v1/pairings/complete",
    request_body = CompletePairingRequest,
    responses(
        (status = 200, description = "Pairing completed and mobile session issued", body = MobileSessionResponse),
        (status = 400, description = "Invalid request body", body = ApiError),
        (status = 401, description = "Invalid pairing secret", body = ApiError),
        (status = 410, description = "Pairing expired or already used", body = ApiError),
        (status = 503, description = "Mobile pairing unavailable on this server", body = ApiError)
    ),
    tag = "mobile"
)]
/// POST `/mobile/v1/pairings/complete` — complete a one-time pairing from the
/// mobile app.
pub(super) async fn handle_complete_pairing(
    State(state): State<PublicMobileState>,
    Json(body): Json<CompletePairingRequest>,
) -> impl IntoResponse {
    let Some(mobile_auth) = state.mobile_auth else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "mobile pairing is unavailable on this server",
        );
    };

    match mobile_auth
        .complete_pairing(CompletePairingInput {
            pairing_id: body.pairing_id,
            pairing_secret: body.pairing_secret,
            device_id: body.device_id,
            device_name: body.device_name,
            platform: body.platform,
        })
        .await
    {
        Ok(session) => Json(mobile_session_response(session)).into_response(),
        Err(error) => mobile_auth_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/mobile/v1/auth/refresh",
    request_body = RefreshSessionRequest,
    responses(
        (status = 200, description = "Mobile session rotated", body = MobileSessionResponse),
        (status = 400, description = "Invalid request body", body = ApiError),
        (status = 401, description = "Invalid or expired refresh token", body = ApiError),
        (status = 503, description = "Mobile pairing unavailable on this server", body = ApiError)
    ),
    tag = "mobile"
)]
/// POST `/mobile/v1/auth/refresh` — rotate an existing mobile refresh token.
pub(super) async fn handle_refresh_session(
    State(state): State<PublicMobileState>,
    Json(body): Json<RefreshSessionRequest>,
) -> impl IntoResponse {
    let Some(mobile_auth) = state.mobile_auth else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "mobile pairing is unavailable on this server",
        );
    };

    match mobile_auth
        .refresh_session(RefreshInput {
            refresh_token: body.refresh_token,
            device_id: body.device_id,
        })
        .await
    {
        Ok(session) => Json(mobile_session_response(session)).into_response(),
        Err(error) => mobile_refresh_error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/mobile/v1/conversations",
    params(
        ("limit" = Option<usize>, Query, description = "Page size. Defaults to 20 and is capped at 100."),
        ("offset" = Option<usize>, Query, description = "Zero-based offset into the recency-sorted conversation list.")
    ),
    responses(
        (status = 200, description = "Recent conversations for the authenticated user", body = [Conversation]),
        (status = 400, description = "Invalid pagination parameters", body = ApiError),
        (status = 401, description = "Authentication required", body = ApiError),
        (status = 500, description = "Internal error", body = ApiError)
    ),
    tag = "mobile"
)]
/// GET `/mobile/v1/conversations` — list recent conversations for the
/// authenticated user.
async fn handle_list_conversations(
    State(state): State<ProtectedMobileState>,
    Extension(identity): Extension<AuthIdentity>,
    Query(params): Query<PaginationQuery>,
) -> impl IntoResponse {
    let (limit, offset) = match parse_pagination(&params) {
        Ok(values) => values,
        Err(response) => return response,
    };

    match state
        .conversations
        .list_conversations(&identity.principal, limit, offset)
        .await
    {
        Ok(conversations) => Json(conversations).into_response(),
        Err(error) => internal_error(error),
    }
}

#[utoipa::path(
    post,
    path = "/mobile/v1/conversations",
    request_body = CreateConversationRequest,
    responses(
        (status = 201, description = "Conversation created", body = Conversation),
        (status = 401, description = "Authentication required", body = ApiError),
        (status = 500, description = "Internal error", body = ApiError)
    ),
    tag = "mobile"
)]
/// POST `/mobile/v1/conversations` — create a new mobile conversation.
async fn handle_create_conversation(
    State(state): State<ProtectedMobileState>,
    Extension(identity): Extension<AuthIdentity>,
    Json(body): Json<CreateConversationRequest>,
) -> impl IntoResponse {
    let conversation_id = ConversationId::new();
    let session_id = SessionId::from(conversation_id);
    let title = body
        .title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("New conversation");

    let conversation = Conversation::new(conversation_id, session_id, &identity.principal, title);
    match state.conversations.put_conversation(&conversation).await {
        Ok(()) => (StatusCode::CREATED, Json(conversation)).into_response(),
        Err(error) => internal_error(error),
    }
}

#[utoipa::path(
    get,
    path = "/mobile/v1/conversations/{id}",
    params(
        ("id" = String, Path, description = "Conversation identifier")
    ),
    responses(
        (status = 200, description = "Conversation details", body = Conversation),
        (status = 400, description = "Invalid conversation id", body = ApiError),
        (status = 401, description = "Authentication required", body = ApiError),
        (status = 404, description = "Conversation not found", body = ApiError),
        (status = 500, description = "Internal error", body = ApiError)
    ),
    tag = "mobile"
)]
/// GET `/mobile/v1/conversations/{id}` — fetch mobile conversation metadata.
async fn handle_get_conversation(
    State(state): State<ProtectedMobileState>,
    Extension(identity): Extension<AuthIdentity>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let conversation_id = match parse_conversation_id(&id) {
        Ok(id) => id,
        Err(response) => return response,
    };

    match load_owned_conversation(&state, &identity, conversation_id).await {
        Ok(conversation) => Json(conversation).into_response(),
        Err(response) => response,
    }
}

#[utoipa::path(
    get,
    path = "/mobile/v1/conversations/{id}/messages",
    params(
        ("id" = String, Path, description = "Conversation identifier"),
        ("limit" = Option<usize>, Query, description = "Optional page size. Defaults to the full transcript after offset when omitted, capped at 100 when provided."),
        ("offset" = Option<usize>, Query, description = "Zero-based offset into the ascending transcript.")
    ),
    responses(
        (status = 200, description = "Conversation transcript page", body = [ConversationMessage]),
        (status = 400, description = "Invalid request parameters", body = ApiError),
        (status = 401, description = "Authentication required", body = ApiError),
        (status = 404, description = "Conversation not found", body = ApiError),
        (status = 500, description = "Internal error", body = ApiError)
    ),
    tag = "mobile"
)]
/// GET `/mobile/v1/conversations/{id}/messages` — list transcript messages in
/// ascending order.
async fn handle_list_messages(
    State(state): State<ProtectedMobileState>,
    Extension(identity): Extension<AuthIdentity>,
    Path(id): Path<String>,
    Query(params): Query<PaginationQuery>,
) -> impl IntoResponse {
    let conversation_id = match parse_conversation_id(&id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    if let Err(response) = load_owned_conversation(&state, &identity, conversation_id).await {
        return response;
    }

    let (limit, offset) = match parse_optional_pagination(&params) {
        Ok(values) => values,
        Err(response) => return response,
    };
    match state
        .conversations
        .list_messages(&conversation_id, limit, offset)
        .await
    {
        Ok(messages) => Json(messages).into_response(),
        Err(error) => internal_error(error),
    }
}

#[utoipa::path(
    post,
    path = "/mobile/v1/conversations/{id}/messages",
    params(
        ("id" = String, Path, description = "Conversation identifier")
    ),
    request_body = SendMessageRequest,
    responses(
        (status = 202, description = "Message accepted for processing", body = SendMessageResponse),
        (status = 400, description = "Invalid request body", body = ApiError),
        (status = 401, description = "Authentication required", body = ApiError),
        (status = 404, description = "Conversation not found", body = ApiError),
        (status = 500, description = "Internal error", body = ApiError)
    ),
    tag = "mobile"
)]
/// POST `/mobile/v1/conversations/{id}/messages` — append a user message and
/// enqueue agent processing.
async fn handle_send_message(
    State(state): State<ProtectedMobileState>,
    Extension(identity): Extension<AuthIdentity>,
    Path(id): Path<String>,
    Json(body): Json<SendMessageRequest>,
) -> impl IntoResponse {
    let conversation_id = match parse_conversation_id(&id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let mut conversation = match load_owned_conversation(&state, &identity, conversation_id).await {
        Ok(conversation) => conversation,
        Err(response) => return response,
    };

    let text = body.text.trim();
    if text.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "text must not be empty");
    }

    let user_message = ConversationMessage::new(
        MessageId::new(),
        conversation.id,
        conversation.session_id,
        ConversationMessageRole::User,
        text,
    );

    conversation.updated_at = user_message.created_at;
    conversation.status = ConversationStatus::Active;
    conversation.last_message_preview = Some(preview_text(text));
    if conversation.title == "New conversation" {
        conversation.title = derive_title(text);
    }

    if let Err(error) = state.conversations.append_message(&user_message).await {
        return internal_error(error);
    }
    if let Err(error) = state.conversations.put_conversation(&conversation).await {
        return internal_error(error);
    }

    let mut envelope = orka_core::Envelope::text("mobile", conversation.session_id, text);
    envelope.id = user_message.id;
    envelope
        .metadata
        .insert("user_id".into(), serde_json::json!(identity.principal));

    match state.bus.publish("inbound", &envelope).await {
        Ok(()) => (
            StatusCode::ACCEPTED,
            Json(SendMessageResponse {
                conversation_id: conversation.id,
                session_id: conversation.session_id,
                message_id: user_message.id,
            }),
        )
            .into_response(),
        Err(error) => internal_error(error),
    }
}

#[utoipa::path(
    get,
    path = "/mobile/v1/conversations/{id}/stream",
    params(
        ("id" = String, Path, description = "Conversation identifier")
    ),
    responses(
        (status = 200, description = "Server-sent event stream carrying message deltas and terminal events", content_type = "text/event-stream"),
        (status = 400, description = "Invalid conversation id", body = ApiError),
        (status = 401, description = "Authentication required", body = ApiError),
        (status = 404, description = "Conversation not found", body = ApiError),
        (status = 500, description = "Internal error", body = ApiError)
    ),
    tag = "mobile"
)]
/// GET `/mobile/v1/conversations/{id}/stream` — subscribe to mobile SSE
/// events for a conversation.
async fn handle_stream(
    State(state): State<ProtectedMobileState>,
    Extension(identity): Extension<AuthIdentity>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let conversation_id = match parse_conversation_id(&id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let conversation = match load_owned_conversation(&state, &identity, conversation_id).await {
        Ok(conversation) => conversation,
        Err(response) => return response,
    };

    let (tx, rx) = mpsc::unbounded_channel::<SseFrame>();
    let mut chunk_rx = state.stream_registry.subscribe(conversation.session_id);
    let mut mobile_rx = state.mobile_events.subscribe(conversation.id).await;

    tokio::spawn(async move {
        loop {
            tokio::select! {
                chunk = chunk_rx.recv() => {
                    let Some(chunk) = chunk else {
                        break;
                    };
                    if let Some(frame) = stream_chunk_to_sse(&chunk.kind, conversation.id, chunk.reply_to)
                        && tx.send(frame).is_err()
                    {
                        break;
                    }
                }
                event = mobile_rx.recv() => {
                    let Some(event) = event else {
                        break;
                    };
                    let (name, data) = match event {
                        MobileStreamEvent::MessageCompleted { message } => (
                            "message_completed",
                            serde_json::json!({ "message": message }).to_string(),
                        ),
                        MobileStreamEvent::MessageFailed { conversation_id, error } => (
                            "message_failed",
                            serde_json::json!({
                                "conversation_id": conversation_id,
                                "error": error,
                            }).to_string(),
                        ),
                    };
                    if tx.send(SseFrame { event: name, data }).is_err() {
                        break;
                    }
                }
            }
        }
    });

    let stream = UnboundedReceiverStream::new(rx)
        .map(|frame| Ok::<Event, Infallible>(Event::default().event(frame.event).data(frame.data)));

    Sse::new(stream).into_response()
}

async fn load_owned_conversation(
    state: &ProtectedMobileState,
    identity: &AuthIdentity,
    conversation_id: ConversationId,
) -> Result<Conversation, axum::response::Response> {
    let conversation = match state.conversations.get_conversation(&conversation_id).await {
        Ok(Some(conversation)) => conversation,
        Ok(None) => {
            return Err(error_response(
                StatusCode::NOT_FOUND,
                "conversation not found",
            ));
        }
        Err(error) => return Err(internal_error(error)),
    };

    if conversation.user_id != identity.principal {
        return Err(error_response(
            StatusCode::NOT_FOUND,
            "conversation not found",
        ));
    }

    Ok(conversation)
}

#[allow(clippy::result_large_err)]
fn parse_conversation_id(id: &str) -> Result<ConversationId, axum::response::Response> {
    Uuid::parse_str(id)
        .map(ConversationId::from)
        .map_err(|_| error_response(StatusCode::BAD_REQUEST, "invalid conversation id"))
}

#[allow(clippy::result_large_err)]
fn parse_pagination(params: &PaginationQuery) -> Result<(usize, usize), axum::response::Response> {
    let limit = params.limit.unwrap_or(DEFAULT_PAGE_SIZE);
    let offset = params.offset.unwrap_or(0);

    if limit == 0 {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "limit must be greater than zero",
        ));
    }
    if limit > MAX_PAGE_SIZE {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "limit must be less than or equal to 100",
        ));
    }

    Ok((limit, offset))
}

#[allow(clippy::result_large_err)]
fn parse_optional_pagination(
    params: &PaginationQuery,
) -> Result<(Option<usize>, usize), axum::response::Response> {
    let offset = params.offset.unwrap_or(0);
    let limit = match params.limit {
        Some(0) => {
            return Err(error_response(
                StatusCode::BAD_REQUEST,
                "limit must be greater than zero",
            ));
        }
        Some(limit) if limit > MAX_PAGE_SIZE => {
            return Err(error_response(
                StatusCode::BAD_REQUEST,
                "limit must be less than or equal to 100",
            ));
        }
        value => value,
    };

    Ok((limit, offset))
}

fn error_response(status: StatusCode, error: impl Into<String>) -> axum::response::Response {
    (
        status,
        Json(ApiError {
            error: error.into(),
        }),
    )
        .into_response()
}

fn mobile_auth_error_response(error: MobileAuthError) -> axum::response::Response {
    match error {
        MobileAuthError::Disabled => error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "mobile pairing is unavailable on this server",
        ),
        MobileAuthError::InvalidRequest(message) => {
            error_response(StatusCode::BAD_REQUEST, message)
        }
        MobileAuthError::NotFound | MobileAuthError::Forbidden => {
            error_response(StatusCode::NOT_FOUND, "pairing session not found")
        }
        MobileAuthError::Expired => error_response(StatusCode::GONE, "pairing session has expired"),
        MobileAuthError::AlreadyUsed => {
            error_response(StatusCode::GONE, "pairing session has already been used")
        }
        MobileAuthError::Unauthorized => {
            error_response(StatusCode::UNAUTHORIZED, "invalid pairing or refresh token")
        }
        MobileAuthError::Internal(message) => {
            error_response(StatusCode::INTERNAL_SERVER_ERROR, message)
        }
    }
}

fn mobile_refresh_error_response(error: MobileAuthError) -> axum::response::Response {
    match error {
        MobileAuthError::Disabled => error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "mobile pairing is unavailable on this server",
        ),
        MobileAuthError::InvalidRequest(message) => {
            error_response(StatusCode::BAD_REQUEST, message)
        }
        MobileAuthError::Unauthorized
        | MobileAuthError::Expired
        | MobileAuthError::AlreadyUsed
        | MobileAuthError::NotFound
        | MobileAuthError::Forbidden => {
            error_response(StatusCode::UNAUTHORIZED, "invalid pairing or refresh token")
        }
        MobileAuthError::Internal(message) => {
            error_response(StatusCode::INTERNAL_SERVER_ERROR, message)
        }
    }
}

fn internal_error(error: impl std::fmt::Display) -> axum::response::Response {
    error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
}

fn mobile_session_response(session: MobileSession) -> MobileSessionResponse {
    MobileSessionResponse {
        access_token: session.access_token,
        access_token_expires_at: session.access_token_expires_at,
        refresh_token: session.refresh_token,
        refresh_token_expires_at: session.refresh_token_expires_at,
        user_id: session.user_id,
    }
}

fn map_pairing_status(status: PairingStatus) -> PairingStatusKind {
    match status {
        PairingStatus::Pending => PairingStatusKind::Pending,
        PairingStatus::Completed => PairingStatusKind::Completed,
        PairingStatus::Expired => PairingStatusKind::Expired,
    }
}

fn derive_title(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return "New conversation".to_string();
    }
    preview_text(trimmed)
}

fn preview_text(text: &str) -> String {
    const MAX_CHARS: usize = 60;
    let truncated = text.trim().chars().take(MAX_CHARS).collect::<String>();
    if text.chars().count() > MAX_CHARS {
        format!("{truncated}...")
    } else {
        truncated
    }
}
