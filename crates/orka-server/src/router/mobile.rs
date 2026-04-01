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
use orka_auth::AuthIdentity;
use orka_core::{
    Conversation, ConversationId, ConversationMessage, ConversationMessageRole, MessageId,
    SessionId, StreamChunkKind, StreamRegistry,
    traits::{ConversationStore, MessageBus},
};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, mpsc};
use tokio_stream::{StreamExt, wrappers::UnboundedReceiverStream};
use utoipa::ToSchema;
use uuid::Uuid;

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
struct MobileState {
    conversations: Arc<dyn ConversationStore>,
    bus: Arc<dyn MessageBus>,
    stream_registry: StreamRegistry,
    mobile_events: MobileEventHub,
}

#[derive(Debug, Deserialize, ToSchema)]
pub(super) struct CreateConversationRequest {
    title: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub(super) struct SendMessageRequest {
    text: String,
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
pub(super) struct SendMessageResponse {
    conversation_id: ConversationId,
    session_id: SessionId,
    message_id: MessageId,
}

#[derive(Debug, Serialize, ToSchema)]
pub(super) struct StreamDonePayload {
    conversation_id: ConversationId,
}

#[derive(Debug)]
struct SseFrame {
    event: &'static str,
    data: String,
}

/// Build the mobile product API routes.
pub(super) fn routes(
    conversations: Arc<dyn ConversationStore>,
    bus: Arc<dyn MessageBus>,
    stream_registry: StreamRegistry,
    mobile_events: MobileEventHub,
) -> Router {
    let state = MobileState {
        conversations,
        bus,
        stream_registry,
        mobile_events,
    };

    Router::new()
        .route("/mobile/v1/me", get(handle_me))
        .route("/mobile/v1/conversations", get(handle_list_conversations))
        .route("/mobile/v1/conversations", post(handle_create_conversation))
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
    State(state): State<MobileState>,
    Extension(identity): Extension<AuthIdentity>,
    Query(params): Query<PaginationQuery>,
) -> impl IntoResponse {
    let (limit, offset) = match parse_pagination(params) {
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
    State(state): State<MobileState>,
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
    State(state): State<MobileState>,
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
    State(state): State<MobileState>,
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

    let (limit, offset) = match parse_optional_pagination(params) {
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
    State(state): State<MobileState>,
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
    State(state): State<MobileState>,
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
                    match &chunk.kind {
                        StreamChunkKind::Delta(delta) => {
                            if tx.send(SseFrame {
                                event: "message_delta",
                                data: serde_json::json!({
                                    "conversation_id": conversation.id,
                                    "reply_to": chunk.reply_to,
                                    "delta": delta,
                                }).to_string(),
                            }).is_err() {
                                break;
                            }
                        }
                        StreamChunkKind::Done => {
                            if tx.send(SseFrame {
                                event: "stream_done",
                                data: serde_json::to_string(&StreamDonePayload {
                                    conversation_id: conversation.id,
                                }).unwrap_or_else(|_| "{}".to_string()),
                            }).is_err() {
                                break;
                            }
                        }
                        _ => {}
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
    state: &MobileState,
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

fn parse_conversation_id(id: &str) -> Result<ConversationId, axum::response::Response> {
    Uuid::parse_str(id)
        .map(ConversationId::from)
        .map_err(|_| error_response(StatusCode::BAD_REQUEST, "invalid conversation id"))
}

fn parse_pagination(params: PaginationQuery) -> Result<(usize, usize), axum::response::Response> {
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

fn parse_optional_pagination(
    params: PaginationQuery,
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
    (status, Json(ApiError { error: error.into() })).into_response()
}

fn internal_error(error: impl std::fmt::Display) -> axum::response::Response {
    error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
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
