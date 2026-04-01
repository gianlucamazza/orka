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
use uuid::Uuid;

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

#[derive(Debug, Deserialize)]
struct CreateConversationRequest {
    title: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SendMessageRequest {
    text: String,
}

#[derive(Debug, Serialize)]
struct CurrentUserResponse {
    user_id: String,
    scopes: Vec<String>,
}

#[derive(Debug, Serialize)]
struct SendMessageResponse {
    conversation_id: ConversationId,
    session_id: SessionId,
    message_id: MessageId,
}

#[derive(Debug, Serialize)]
struct StreamDonePayload {
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
        .route("/mobile/v1/conversations/{id}", get(handle_get_conversation))
        .route(
            "/mobile/v1/conversations/{id}/messages",
            get(handle_list_messages).post(handle_send_message),
        )
        .route("/mobile/v1/conversations/{id}/stream", get(handle_stream))
        .with_state(state)
}

async fn handle_me(Extension(identity): Extension<AuthIdentity>) -> impl IntoResponse {
    Json(CurrentUserResponse {
        user_id: identity.principal,
        scopes: identity.scopes,
    })
}

async fn handle_list_conversations(
    State(state): State<MobileState>,
    Extension(identity): Extension<AuthIdentity>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let limit = params
        .get("limit")
        .and_then(|value| value.parse().ok())
        .unwrap_or(20);

    match state
        .conversations
        .list_conversations(&identity.principal, limit)
        .await
    {
        Ok(conversations) => Json(conversations).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

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
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

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

async fn handle_list_messages(
    State(state): State<MobileState>,
    Extension(identity): Extension<AuthIdentity>,
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let conversation_id = match parse_conversation_id(&id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    if let Err(response) = load_owned_conversation(&state, &identity, conversation_id).await {
        return response;
    }

    let limit = params.get("limit").and_then(|value| value.parse().ok());
    match state
        .conversations
        .list_messages(&conversation_id, limit)
        .await
    {
        Ok(messages) => Json(messages).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

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
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "text must not be empty" })),
        )
            .into_response();
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
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error.to_string() })),
        )
            .into_response();
    }
    if let Err(error) = state.conversations.put_conversation(&conversation).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error.to_string() })),
        )
            .into_response();
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
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

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

    let stream = UnboundedReceiverStream::new(rx).map(|frame| {
        Ok::<Event, Infallible>(
            Event::default()
                .event(frame.event)
                .data(frame.data),
        )
    });

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
            return Err((
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "conversation not found" })),
            )
                .into_response())
        }
        Err(error) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": error.to_string() })),
            )
                .into_response())
        }
    };

    if conversation.user_id != identity.principal {
        return Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "conversation not found" })),
        )
            .into_response());
    }

    Ok(conversation)
}

fn parse_conversation_id(id: &str) -> Result<ConversationId, axum::response::Response> {
    Uuid::parse_str(id)
        .map(ConversationId::from)
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "invalid conversation id" })),
            )
                .into_response()
        })
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
