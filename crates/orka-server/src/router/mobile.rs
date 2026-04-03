use std::{collections::HashMap, convert::Infallible, sync::Arc};

use axum::{
    Extension, Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Multipart, Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{
        IntoResponse, Response,
        sse::{Event, Sse},
    },
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use orka_auth::AuthIdentity;
use orka_core::{
    ArtifactId, Conversation, ConversationArtifact, ConversationArtifactOrigin, ConversationId,
    ConversationMessage, ConversationMessageRole, ConversationStatus, MediaPayload, MessageId,
    Payload, RichInputPayload, SessionId, StreamChunkKind, StreamRegistry,
    traits::{ArtifactStore, ConversationStore, MessageBus},
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
const MAX_UPLOAD_BYTES: usize = 25 * 1024 * 1024;
const MAX_ARTIFACTS_PER_MESSAGE: usize = 10;

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
    /// A new artifact became available for the conversation.
    ArtifactReady {
        /// Conversation that owns the artifact.
        conversation_id: ConversationId,
        /// Persisted artifact metadata.
        artifact: ConversationArtifact,
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
    artifacts: Arc<dyn ArtifactStore>,
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
    #[serde(default)]
    text: String,
    #[serde(default)]
    artifact_ids: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(super) struct UploadArtifactResponse {
    artifact: ConversationArtifact,
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
    artifacts: Arc<dyn ArtifactStore>,
    bus: Arc<dyn MessageBus>,
    stream_registry: StreamRegistry,
    mobile_events: MobileEventHub,
    mobile_auth: Option<Arc<dyn MobileAuthService>>,
) -> Router {
    let state = ProtectedMobileState {
        conversations,
        artifacts,
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
            "/mobile/v1/uploads",
            post(handle_upload_artifact)
                .layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES)),
        )
        .route(
            "/mobile/v1/artifacts/{id}",
            get(handle_get_artifact).delete(handle_delete_artifact),
        )
        .route(
            "/mobile/v1/artifacts/{id}/content",
            get(handle_get_artifact_content),
        )
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
    path = "/mobile/v1/uploads",
    responses(
        (status = 201, description = "Artifact uploaded", body = UploadArtifactResponse),
        (status = 400, description = "Invalid multipart upload", body = ApiError),
        (status = 401, description = "Authentication required", body = ApiError),
        (status = 413, description = "Upload too large", body = ApiError),
        (status = 415, description = "Unsupported media type", body = ApiError)
    ),
    tag = "mobile"
)]
async fn handle_upload_artifact(
    State(state): State<ProtectedMobileState>,
    Extension(identity): Extension<AuthIdentity>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let mut file_bytes: Option<Vec<u8>> = None;
    let mut filename: Option<String> = None;
    let mut mime_type: Option<String> = None;
    let mut caption: Option<String> = None;

    loop {
        let next_field = match multipart.next_field().await {
            Ok(value) => value,
            Err(error) => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    format!("invalid multipart body: {error}"),
                );
            }
        };
        let Some(field) = next_field else {
            break;
        };

        match field.name() {
            Some("file") => {
                filename = field.file_name().map(sanitize_filename);
                mime_type = field.content_type().map(ToString::to_string);
                let bytes = match field.bytes().await {
                    Ok(bytes) => bytes.to_vec(),
                    Err(error) => {
                        return error_response(
                            StatusCode::BAD_REQUEST,
                            format!("failed to read file body: {error}"),
                        );
                    }
                };
                if bytes.len() > MAX_UPLOAD_BYTES {
                    return error_response(StatusCode::PAYLOAD_TOO_LARGE, "upload exceeds size limit");
                }
                file_bytes = Some(bytes);
            }
            Some("caption") => {
                let text = match field.text().await {
                    Ok(text) => text,
                    Err(error) => {
                        return error_response(
                            StatusCode::BAD_REQUEST,
                            format!("failed to read caption: {error}"),
                        );
                    }
                };
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    caption = Some(trimmed.to_string());
                }
            }
            _ => {}
        }
    }

    let Some(bytes) = file_bytes else {
        return error_response(StatusCode::BAD_REQUEST, "multipart body is missing a file field");
    };

    let filename = filename.unwrap_or_else(|| "upload.bin".to_string());
    let mime_type = detect_mime_type(&bytes, mime_type.as_deref(), &filename);
    if !is_allowed_upload_mime(&mime_type) {
        return error_response(StatusCode::UNSUPPORTED_MEDIA_TYPE, "unsupported artifact media type");
    }

    let mut artifact = ConversationArtifact::new(
        identity.principal.clone(),
        ConversationArtifactOrigin::UserUpload,
        mime_type,
        filename,
    );
    artifact.caption = caption;
    artifact.size_bytes = Some(bytes.len() as u64);

    match state.artifacts.put_artifact(&artifact, &bytes).await {
        Ok(()) => (
            StatusCode::CREATED,
            Json(UploadArtifactResponse { artifact }),
        )
            .into_response(),
        Err(error) => internal_error(error),
    }
}

#[utoipa::path(
    get,
    path = "/mobile/v1/artifacts/{id}",
    responses(
        (status = 200, description = "Artifact metadata", body = ConversationArtifact),
        (status = 401, description = "Authentication required", body = ApiError),
        (status = 404, description = "Artifact not found", body = ApiError)
    ),
    tag = "mobile"
)]
async fn handle_get_artifact(
    State(state): State<ProtectedMobileState>,
    Extension(identity): Extension<AuthIdentity>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let artifact_id = match parse_artifact_id(&id) {
        Ok(value) => value,
        Err(response) => return response,
    };

    match load_owned_artifact(&state, &identity, artifact_id).await {
        Ok(artifact) => Json(artifact).into_response(),
        Err(response) => response,
    }
}

#[utoipa::path(
    delete,
    path = "/mobile/v1/artifacts/{id}",
    responses(
        (status = 204, description = "Artifact deleted"),
        (status = 401, description = "Authentication required", body = ApiError),
        (status = 404, description = "Artifact not found", body = ApiError),
        (status = 409, description = "Artifact is already attached to a message", body = ApiError)
    ),
    tag = "mobile"
)]
async fn handle_delete_artifact(
    State(state): State<ProtectedMobileState>,
    Extension(identity): Extension<AuthIdentity>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let artifact_id = match parse_artifact_id(&id) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let artifact = match load_owned_artifact(&state, &identity, artifact_id).await {
        Ok(artifact) => artifact,
        Err(response) => return response,
    };
    if artifact.message_id.is_some() {
        return error_response(
            StatusCode::CONFLICT,
            "artifact is already attached to a message",
        );
    }

    match state.artifacts.delete_artifact(&artifact.id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => internal_error(error),
    }
}

#[utoipa::path(
    get,
    path = "/mobile/v1/artifacts/{id}/content",
    responses(
        (status = 200, description = "Artifact content"),
        (status = 206, description = "Partial artifact content"),
        (status = 401, description = "Authentication required", body = ApiError),
        (status = 404, description = "Artifact not found", body = ApiError)
    ),
    tag = "mobile"
)]
async fn handle_get_artifact_content(
    State(state): State<ProtectedMobileState>,
    Extension(identity): Extension<AuthIdentity>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let artifact_id = match parse_artifact_id(&id) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let artifact = match load_owned_artifact(&state, &identity, artifact_id).await {
        Ok(artifact) => artifact,
        Err(response) => return response,
    };
    let Some(bytes) = (match state.artifacts.get_artifact_bytes(&artifact.id).await {
        Ok(value) => value,
        Err(error) => return internal_error(error),
    }) else {
        return error_response(StatusCode::NOT_FOUND, "artifact not found");
    };

    build_artifact_content_response(&artifact, bytes, &headers)
}

/// Resolve a list of artifact IDs for an outgoing message.
///
/// Validates ownership, attachment status, and loads bytes. Returns the
/// updated [`ConversationArtifact`] list and the corresponding
/// [`MediaPayload`] list ready for inclusion in the envelope.
#[allow(clippy::result_large_err)]
async fn resolve_message_artifacts(
    state: &ProtectedMobileState,
    owner: &str,
    conversation_id: ConversationId,
    message_id: MessageId,
    raw_ids: &[String],
) -> Result<(Vec<ConversationArtifact>, Vec<MediaPayload>), axum::response::Response> {
    let mut artifacts = Vec::with_capacity(raw_ids.len());
    let mut rich_attachments = Vec::with_capacity(raw_ids.len());

    for raw_id in raw_ids {
        let artifact_id = parse_artifact_id(raw_id)?;

        let Some(mut artifact) = state
            .artifacts
            .get_artifact(&artifact_id)
            .await
            .map_err(internal_error)?
        else {
            return Err(error_response(StatusCode::NOT_FOUND, "artifact not found"));
        };
        if artifact.owner_user_id != owner {
            return Err(error_response(StatusCode::NOT_FOUND, "artifact not found"));
        }
        if artifact.message_id.is_some() {
            return Err(error_response(
                StatusCode::CONFLICT,
                "artifact is already attached to a message",
            ));
        }
        let Some(bytes) = state
            .artifacts
            .get_artifact_bytes(&artifact_id)
            .await
            .map_err(internal_error)?
        else {
            return Err(error_response(StatusCode::NOT_FOUND, "artifact content is missing"));
        };

        artifact.conversation_id = Some(conversation_id);
        artifact.message_id = Some(message_id);
        state
            .artifacts
            .update_artifact(&artifact)
            .await
            .map_err(internal_error)?;

        let mut media =
            MediaPayload::inline(artifact.mime_type.clone(), bytes, artifact.caption.clone())
                .with_filename(artifact.filename.clone());
        media.size_bytes = artifact.size_bytes;
        artifacts.push(artifact);
        rich_attachments.push(media);
    }

    Ok((artifacts, rich_attachments))
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
    if text.is_empty() && body.artifact_ids.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "message must include text or at least one artifact",
        );
    }
    if body.artifact_ids.len() > MAX_ARTIFACTS_PER_MESSAGE {
        return error_response(
            StatusCode::BAD_REQUEST,
            format!("message may not include more than {MAX_ARTIFACTS_PER_MESSAGE} artifacts"),
        );
    }

    let user_message_id = MessageId::new();
    let (artifacts, rich_attachments) = match resolve_message_artifacts(
        &state,
        &identity.principal,
        conversation.id,
        user_message_id,
        &body.artifact_ids,
    )
    .await
    {
        Ok(value) => value,
        Err(response) => return response,
    };

    let mut user_message = ConversationMessage::new(
        MessageId::new(),
        conversation.id,
        conversation.session_id,
        ConversationMessageRole::User,
        text,
    );
    user_message.id = user_message_id;
    user_message.artifacts = artifacts.clone();

    conversation.updated_at = user_message.created_at;
    conversation.status = ConversationStatus::Active;
    conversation.last_message_preview = Some(preview_text_for_message(text, &artifacts));
    if conversation.title == "New conversation" {
        conversation.title = if text.is_empty() {
            preview_text_for_message(text, &artifacts)
        } else {
            derive_title(text)
        };
    }

    if let Err(error) = state.conversations.append_message(&user_message).await {
        return internal_error(error);
    }
    if let Err(error) = state.conversations.put_conversation(&conversation).await {
        return internal_error(error);
    }

    let payload = if rich_attachments.is_empty() {
        Payload::Text(text.to_string())
    } else {
        Payload::RichInput(RichInputPayload {
            text: if text.is_empty() {
                None
            } else {
                Some(text.to_string())
            },
            attachments: rich_attachments,
        })
    };
    let mut envelope = orka_core::Envelope::with_payload(
        "mobile",
        conversation.session_id,
        payload,
        &orka_core::Envelope::text("mobile", conversation.session_id, ""),
    );
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
                        MobileStreamEvent::ArtifactReady { conversation_id, artifact } => (
                            "artifact_ready",
                            serde_json::json!({
                                "conversation_id": conversation_id,
                                "artifact": artifact,
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

async fn load_owned_artifact(
    state: &ProtectedMobileState,
    identity: &AuthIdentity,
    artifact_id: ArtifactId,
) -> Result<ConversationArtifact, axum::response::Response> {
    let artifact = match state.artifacts.get_artifact(&artifact_id).await {
        Ok(Some(artifact)) => artifact,
        Ok(None) => return Err(error_response(StatusCode::NOT_FOUND, "artifact not found")),
        Err(error) => return Err(internal_error(error)),
    };

    if artifact.owner_user_id != identity.principal {
        return Err(error_response(StatusCode::NOT_FOUND, "artifact not found"));
    }

    Ok(artifact)
}

#[allow(clippy::result_large_err)]
fn parse_conversation_id(id: &str) -> Result<ConversationId, axum::response::Response> {
    Uuid::parse_str(id)
        .map(ConversationId::from)
        .map_err(|_| error_response(StatusCode::BAD_REQUEST, "invalid conversation id"))
}

#[allow(clippy::result_large_err)]
fn parse_artifact_id(id: &str) -> Result<ArtifactId, axum::response::Response> {
    Uuid::parse_str(id)
        .map(ArtifactId::from)
        .map_err(|_| error_response(StatusCode::BAD_REQUEST, "invalid artifact id"))
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

fn preview_text_for_message(text: &str, artifacts: &[ConversationArtifact]) -> String {
    let trimmed = text.trim();
    if !trimmed.is_empty() {
        return preview_text(trimmed);
    }
    artifacts
        .first()
        .map_or_else(|| "New message".to_string(), |a| format!("[{}] {}", a.mime_type, a.filename))
}

fn sanitize_filename(input: &str) -> String {
    let trimmed = input.trim();
    let sanitized: String = trimmed
        .chars()
        .map(|ch| match ch {
            '/' | '\\' | ':' | '\0' => '_',
            _ if ch.is_control() => '_',
            _ => ch,
        })
        .collect();
    if sanitized.is_empty() {
        "upload.bin".to_string()
    } else {
        sanitized
    }
}

fn detect_mime_type(bytes: &[u8], provided: Option<&str>, filename: &str) -> String {
    infer::get(bytes)
        .map(|kind| kind.mime_type().to_string())
        .or_else(|| provided.map(ToString::to_string))
        .or_else(|| mime_guess::from_path(filename).first_raw().map(ToString::to_string))
        .unwrap_or_else(|| "application/octet-stream".to_string())
}

fn is_allowed_upload_mime(mime: &str) -> bool {
    mime.starts_with("image/")
        || mime.starts_with("audio/")
        || mime.starts_with("video/")
        || mime.starts_with("text/")
        || matches!(
            mime,
            "application/pdf"
                | "application/json"
                | "application/zip"
                | "application/gzip"
                | "application/x-tar"
                | "application/msword"
                | "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                | "application/vnd.ms-excel"
                | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
                | "application/vnd.ms-powerpoint"
                | "application/vnd.openxmlformats-officedocument.presentationml.presentation"
                | "application/octet-stream"
        )
}

fn build_artifact_content_response(
    artifact: &ConversationArtifact,
    bytes: Vec<u8>,
    headers: &HeaderMap,
) -> Response {
    let total_len = bytes.len();
    let (status, content_range, body_bytes) = match headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_range_header)
    {
        Some((start, end)) if start < total_len && start <= end => {
            let bounded_end = end.min(total_len.saturating_sub(1));
            (
                StatusCode::PARTIAL_CONTENT,
                Some(format!("bytes {start}-{bounded_end}/{total_len}")),
                bytes[start..=bounded_end].to_vec(),
            )
        }
        _ => (StatusCode::OK, None, bytes),
    };

    let content_length = body_bytes.len();
    let mut response = Response::new(Body::from(body_bytes));
    *response.status_mut() = status;
    let response_headers = response.headers_mut();
    response_headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&artifact.mime_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    response_headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("inline; filename=\"{}\"", artifact.filename))
            .unwrap_or_else(|_| HeaderValue::from_static("inline")),
    );
    response_headers.insert(
        header::ACCEPT_RANGES,
        HeaderValue::from_static("bytes"),
    );
    if let Ok(value) = HeaderValue::from_str(&content_length.to_string()) {
        response_headers.insert(header::CONTENT_LENGTH, value);
    }
    if let Some(range) = content_range
        && let Ok(value) = HeaderValue::from_str(&range)
    {
        response_headers.insert(header::CONTENT_RANGE, value);
    }
    response
}

fn parse_range_header(value: &str) -> Option<(usize, usize)> {
    let range = value.strip_prefix("bytes=")?;
    let (start, end) = range.split_once('-')?;
    let start = start.parse::<usize>().ok()?;
    let end = if end.is_empty() {
        usize::MAX
    } else {
        end.parse::<usize>().ok()?
    };
    Some((start, end))
}
