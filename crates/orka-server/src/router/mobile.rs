use std::{collections::HashMap, convert::Infallible, num::NonZeroU32, sync::Arc};

use axum::{
    Extension, Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Multipart, Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::Next,
    response::{
        IntoResponse, Response,
        sse::{Event, Sse},
    },
    routing::{delete, get, post},
};
use chrono::{DateTime, Utc};
use governor::{Quota, RateLimiter, state::keyed::DefaultKeyedStateStore};
use orka_auth::AuthIdentity;
use orka_contracts::RealtimeEvent;
use orka_core::{
    ArtifactId, Conversation, ConversationArtifact, ConversationArtifactOrigin, ConversationId,
    ConversationMessage, ConversationMessageRole, ConversationStatus, DomainEvent, MediaPayload,
    MemoryEntry, MessageCursor, MessageId, Payload, RichInputPayload, SessionId, StreamRegistry,
    traits::{ArtifactStore, ConversationStore, EventSink, MemoryStore, MessageBus},
    types::DomainEventKind,
};
use orka_workspace::WorkspaceRegistry;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, mpsc};
use tokio_stream::{StreamExt, wrappers::UnboundedReceiverStream};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    export::{ExportFormat, export_json, export_markdown, export_pdf},
    mobile_auth::{
        CompletePairingInput, DeviceInfo, MobileAuthError, MobileAuthService, MobileSession,
        PairingStatus, RefreshInput,
    },
};

const DEFAULT_PAGE_SIZE: usize = 20;
const MAX_PAGE_SIZE: usize = 100;
const MAX_UPLOAD_BYTES: usize = 25 * 1024 * 1024;
const MAX_ARTIFACTS_PER_MESSAGE: usize = 10;

/// Per-user rate limiter backed by `governor`'s `DashMap` store.
pub(super) type MobileRateLimiter =
    RateLimiter<String, DefaultKeyedStateStore<String>, governor::clock::DefaultClock>;

/// A pair of per-user rate limiters: (`read_limiter`, `write_limiter`).
///
/// Read-only methods (GET, HEAD) use the first limiter; mutating methods
/// (POST, PATCH, PUT, DELETE) use the second.
pub(super) type MobileRateLimiters = (Arc<MobileRateLimiter>, Arc<MobileRateLimiter>);

/// Create per-user mobile rate limiters with custom quotas.
pub(super) fn new_mobile_rate_limiters_per_minute(
    read_rpm: u32,
    write_rpm: u32,
) -> MobileRateLimiters {
    (
        new_mobile_rate_limiter_per_minute(read_rpm),
        new_mobile_rate_limiter_per_minute(write_rpm),
    )
}

/// Create a rate limiter with a custom requests-per-minute limit.
pub(super) fn new_mobile_rate_limiter_per_minute(reqs_per_minute: u32) -> Arc<MobileRateLimiter> {
    #[allow(clippy::expect_used)]
    let quota = Quota::per_minute(
        NonZeroU32::new(reqs_per_minute).expect("reqs_per_minute must be non-zero"),
    );
    Arc::new(RateLimiter::dashmap(quota))
}

/// Axum middleware: enforce per-user rate limiting on protected mobile routes.
///
/// The rate limit key is the authenticated user's `principal` (JWT `sub`
/// claim). Per-user keying is intentional: keying by `device_id` is not
/// possible because the device ID is not carried in JWT access token claims.
///
/// Read-only requests (GET, HEAD) consume from the read limiter (300/min by
/// default); mutating requests (POST, PATCH, PUT, DELETE) consume from the
/// write limiter (60/min).
///
/// Returns HTTP 429 when the limit is exceeded.
pub(super) async fn rate_limit_middleware(
    State((read_limiter, write_limiter)): State<MobileRateLimiters>,
    Extension(identity): Extension<AuthIdentity>,
    request: axum::http::Request<Body>,
    next: Next,
) -> Response {
    let limiter = if request.method() == axum::http::Method::GET
        || request.method() == axum::http::Method::HEAD
    {
        &read_limiter
    } else {
        &write_limiter
    };
    match limiter.check_key(&identity.principal) {
        Ok(()) => next.run(request).await,
        Err(_) => (
            StatusCode::TOO_MANY_REQUESTS,
            Json(ApiError {
                error: "Rate limit exceeded. Please slow down.".into(),
            }),
        )
            .into_response(),
    }
}

/// Per-conversation event hub used by the mobile product API.
///
/// Subscribers receive [`RealtimeEvent`] values directly — the former
/// `MobileStreamEvent` wrapper was removed in Phase 7 of the architectural
/// modernization.
#[derive(Clone, Default)]
pub struct MobileEventHub {
    inner: Arc<Mutex<HashMap<ConversationId, Vec<mpsc::UnboundedSender<RealtimeEvent>>>>>,
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
    ) -> mpsc::UnboundedReceiver<RealtimeEvent> {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut inner = self.inner.lock().await;
        inner.entry(conversation_id).or_default().push(tx);
        rx
    }

    /// Publish an event to active subscribers.
    pub async fn publish(&self, conversation_id: ConversationId, event: RealtimeEvent) {
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
    pub(super) conversations: Arc<dyn ConversationStore>,
    pub(super) artifacts: Arc<dyn ArtifactStore>,
    pub(super) bus: Arc<dyn MessageBus>,
    pub(super) stream_registry: StreamRegistry,
    pub(super) mobile_events: MobileEventHub,
    pub(super) controller: Arc<orka_core::conversation_controller::ConversationController>,
    pub(super) mobile_auth: Option<Arc<dyn MobileAuthService>>,
    pub(super) memory: Arc<dyn MemoryStore>,
    pub(super) workspace_registry: Arc<WorkspaceRegistry>,
    pub(super) event_sink: Arc<dyn EventSink>,
}

#[derive(Clone)]
pub(super) struct PublicMobileState {
    mobile_auth: Option<Arc<dyn MobileAuthService>>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub(super) struct CreateConversationRequest {
    title: Option<String>,
    /// Name of the workspace to bind this conversation to.  Must match a
    /// registered workspace on the server.  When omitted the server's default
    /// workspace is used.
    workspace: Option<String>,
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
struct ConversationListQuery {
    limit: Option<usize>,
    offset: Option<usize>,
    archived: Option<bool>,
    /// Filter conversations by workspace name. When omitted all conversations
    /// are returned regardless of their workspace binding.
    workspace: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MessageListQuery {
    limit: Option<usize>,
    after: Option<String>,
    before: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub(super) struct MarkReadRequest {
    /// The ID of the last message the client has read.
    last_read_message_id: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub(super) struct ConversationListItem {
    #[serde(flatten)]
    conversation: Conversation,
    unread_count: u64,
}

#[derive(Debug, Deserialize, ToSchema)]
pub(super) struct UpdateConversationRequest {
    /// Archive (`true`) or unarchive (`false`) the conversation.
    #[serde(default)]
    archived: Option<bool>,
    /// Rename the conversation. Must be non-blank when provided.
    #[serde(default)]
    title: Option<String>,
    /// Pin (`true`) or unpin (`false`) the conversation.
    #[serde(default)]
    pinned: Option<bool>,
    /// Replace the conversation's tag list. Pass an empty array to clear all
    /// tags.
    #[serde(default)]
    tags: Option<Vec<String>>,
    /// Rebind the conversation to a different workspace.  Must match a
    /// registered workspace on the server.
    #[serde(default)]
    workspace: Option<String>,
}

// ── Workspace management ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize, ToSchema)]
pub(super) struct UpdateWorkspaceRequest {
    /// Agent display name (maps to SOUL.md frontmatter `name`).
    #[serde(default)]
    agent_name: Option<String>,
    /// One-line workspace description (maps to SOUL.md frontmatter
    /// `description`).
    #[serde(default)]
    description: Option<String>,
    /// Semantic version string (maps to SOUL.md frontmatter `version`).
    #[serde(default)]
    version: Option<String>,
    /// Markdown body of SOUL.md (content after the frontmatter block).
    #[serde(default)]
    soul_body: Option<String>,
    /// Full content of TOOLS.md.
    #[serde(default)]
    tools_body: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub(super) struct CreateWorkspaceRequest {
    /// Workspace identifier (lowercase letters, digits, hyphens; 1–64 chars).
    name: String,
    /// Agent display name.
    #[serde(default)]
    agent_name: Option<String>,
    /// One-line description.
    #[serde(default)]
    description: Option<String>,
    /// Semantic version string.
    #[serde(default)]
    version: Option<String>,
    /// Initial SOUL.md body (markdown content after the frontmatter block).
    #[serde(default)]
    soul_body: Option<String>,
    /// Initial TOOLS.md content.
    #[serde(default)]
    tools_body: Option<String>,
}

// ── Device management ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, ToSchema)]
pub(super) struct RenameDeviceRequest {
    device_name: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub(super) struct DeviceResponse {
    id: String,
    device_name: String,
    platform: String,
    last_seen_at: Option<DateTime<Utc>>,
    push_token_registered: bool,
    is_current: bool,
}

impl From<DeviceInfo> for DeviceResponse {
    fn from(d: DeviceInfo) -> Self {
        Self {
            id: d.id,
            device_name: d.device_name,
            platform: d.platform,
            last_seen_at: d.last_seen_at,
            push_token_registered: d.push_token_registered,
            is_current: d.is_current,
        }
    }
}

// ── Push tokens ──────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, ToSchema)]
pub(super) struct RegisterPushTokenRequest {
    push_token: String,
    platform: String,
    app_version: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(super) struct RegisterPushTokenResponse {
    device_id: String,
    push_token_registered: bool,
}

// ── Search ───────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(super) struct SearchQuery {
    q: String,
    #[serde(default = "default_page_size")]
    limit: usize,
    #[serde(default)]
    offset: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub(super) struct SearchResultItem {
    message_id: MessageId,
    conversation_id: ConversationId,
    conversation_title: String,
    role: ConversationMessageRole,
    text_snippet: String,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(super) struct SearchResponse {
    results: Vec<SearchResultItem>,
    total: usize,
}

// ── Export ───────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(super) struct ExportQuery {
    #[serde(default = "default_export_format")]
    format: String,
}

fn default_export_format() -> String {
    "md".to_string()
}

fn default_page_size() -> usize {
    20
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
pub(super) fn protected_routes(state: ProtectedMobileState) -> Router {
    Router::new()
        .route("/mobile/v1/me", get(handle_me))
        .route("/mobile/v1/conversations", get(handle_list_conversations))
        .route("/mobile/v1/conversations", post(handle_create_conversation))
        .route("/mobile/v1/pairings", post(handle_create_pairing))
        .route("/mobile/v1/pairings/{id}", get(handle_get_pairing_status))
        .route(
            "/mobile/v1/uploads",
            post(handle_upload_artifact).layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES)),
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
            get(handle_get_conversation)
                .patch(handle_update_conversation)
                .delete(handle_delete_conversation),
        )
        .route(
            "/mobile/v1/conversations/{id}/messages",
            get(handle_list_messages).post(handle_send_message),
        )
        .route("/mobile/v1/conversations/{id}/read", post(handle_mark_read))
        .route(
            "/mobile/v1/conversations/{id}/messages/{message_id}",
            delete(handle_delete_message),
        )
        .route(
            "/mobile/v1/conversations/{id}/cancel",
            post(handle_cancel_generation),
        )
        .route(
            "/mobile/v1/conversations/{id}/retry",
            post(handle_retry_generation),
        )
        .route("/mobile/v1/conversations/{id}/stream", get(handle_stream))
        // Device management
        .route("/mobile/v1/devices", get(handle_list_devices))
        .route(
            "/mobile/v1/devices/{id}",
            delete(handle_remove_device).patch(handle_rename_device),
        )
        .route(
            "/mobile/v1/devices/{id}/push-token",
            post(handle_register_push_token).delete(handle_unregister_push_token),
        )
        // Search
        .route("/mobile/v1/search", get(handle_search_messages))
        // Transcription
        .route(
            "/mobile/v1/transcribe",
            post(handle_transcribe).layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES)),
        )
        // Export
        .route(
            "/mobile/v1/conversations/{id}/export",
            get(handle_export_conversation),
        )
        // Workspaces
        .route(
            "/mobile/v1/workspaces",
            get(handle_list_workspaces).post(handle_create_workspace),
        )
        .route(
            "/mobile/v1/workspaces/{name}",
            get(handle_get_workspace)
                .patch(handle_update_workspace)
                .delete(handle_delete_workspace),
        )
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
        ("offset" = Option<usize>, Query, description = "Zero-based offset into the recency-sorted conversation list."),
        ("archived" = Option<bool>, Query, description = "When true, return only archived conversations. Defaults to false.")
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
    Query(params): Query<ConversationListQuery>,
) -> impl IntoResponse {
    let (limit, offset) = match parse_conversation_list_pagination(&params) {
        Ok(values) => values,
        Err(response) => return response,
    };
    let include_archived = params.archived.unwrap_or(false);
    let workspace_filter = params.workspace.as_deref();

    let conversations = match state
        .conversations
        .list_conversations(
            &identity.principal,
            limit,
            offset,
            include_archived,
            workspace_filter,
        )
        .await
    {
        Ok(conversations) => conversations,
        Err(error) => return internal_error(error),
    };

    let mut items = Vec::with_capacity(conversations.len());
    for conversation in conversations {
        let watermark = match state
            .conversations
            .get_read_watermark(&identity.principal, &conversation.id)
            .await
        {
            Ok(w) => w,
            Err(error) => return internal_error(error),
        };
        let unread_count = match state
            .conversations
            .list_messages(&conversation.id, watermark.as_ref(), None, usize::MAX)
            .await
        {
            Ok(msgs) => msgs.len() as u64,
            Err(error) => return internal_error(error),
        };
        items.push(ConversationListItem {
            conversation,
            unread_count,
        });
    }

    Json(items).into_response()
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
    // Validate workspace name when provided.
    if let Some(ws) = &body.workspace
        && state.workspace_registry.get(ws).await.is_none()
    {
        return error_response(
            StatusCode::BAD_REQUEST,
            format!("workspace '{ws}' not found"),
        );
    }

    let conversation_id = ConversationId::new();
    let session_id = SessionId::from(conversation_id);
    let title = body
        .title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("New conversation");

    let mut conversation =
        Conversation::new(conversation_id, session_id, &identity.principal, title);
    if let Some(ws) = body.workspace {
        conversation = conversation.with_workspace(&ws);
        // Persist the workspace binding into the memory store so the worker's
        // 3-tier resolution picks it up automatically on the first message.
        let override_key = format!("workspace_override:{session_id}");
        let entry = MemoryEntry::new(&override_key, serde_json::json!({ "workspace_name": ws }))
            .with_source("mobile_api");
        if let Err(error) = state.memory.store(&override_key, entry, None).await {
            return internal_error(error);
        }
    }

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
    patch,
    path = "/mobile/v1/conversations/{id}",
    params(
        ("id" = String, Path, description = "Conversation identifier")
    ),
    request_body = UpdateConversationRequest,
    responses(
        (status = 200, description = "Conversation updated", body = Conversation),
        (status = 400, description = "Invalid conversation id", body = ApiError),
        (status = 401, description = "Authentication required", body = ApiError),
        (status = 404, description = "Conversation not found", body = ApiError),
        (status = 500, description = "Internal error", body = ApiError)
    ),
    tag = "mobile"
)]
/// PATCH `/mobile/v1/conversations/{id}` — update conversation metadata.
async fn handle_update_conversation(
    State(state): State<ProtectedMobileState>,
    Extension(identity): Extension<AuthIdentity>,
    Path(id): Path<String>,
    Json(body): Json<UpdateConversationRequest>,
) -> impl IntoResponse {
    if body.archived.is_none()
        && body.title.is_none()
        && body.pinned.is_none()
        && body.tags.is_none()
        && body.workspace.is_none()
    {
        return error_response(StatusCode::BAD_REQUEST, "no fields to update");
    }

    // Validate workspace name when provided.
    if let Some(ws) = &body.workspace
        && state.workspace_registry.get(ws).await.is_none()
    {
        return error_response(
            StatusCode::BAD_REQUEST,
            format!("workspace '{ws}' not found"),
        );
    }

    let conversation_id = match parse_conversation_id(&id) {
        Ok(id) => id,
        Err(response) => return response,
    };

    let mut conversation = match load_owned_conversation(&state, &identity, conversation_id).await {
        Ok(conversation) => conversation,
        Err(response) => return response,
    };

    if let Some(archived) = body.archived {
        conversation.archived_at = if archived { Some(Utc::now()) } else { None };
    }
    if let Some(title) = body.title {
        let trimmed = title.trim().to_string();
        if trimmed.is_empty() {
            return error_response(StatusCode::BAD_REQUEST, "title must not be blank");
        }
        conversation.title = trimmed;
    }
    if let Some(pinned) = body.pinned {
        conversation.pinned = pinned;
    }
    if let Some(tags) = body.tags {
        conversation.tags = tags;
    }
    if let Some(ws) = body.workspace {
        let session_id = SessionId::from(conversation_id);
        let override_key = format!("workspace_override:{session_id}");
        let entry = MemoryEntry::new(&override_key, serde_json::json!({ "workspace_name": ws }))
            .with_source("mobile_api");
        if let Err(error) = state.memory.store(&override_key, entry, None).await {
            return internal_error(error);
        }
        conversation.workspace = Some(ws);
    }
    conversation.updated_at = Utc::now();

    match state.conversations.put_conversation(&conversation).await {
        Ok(()) => Json(conversation).into_response(),
        Err(error) => internal_error(error),
    }
}

#[utoipa::path(
    delete,
    path = "/mobile/v1/conversations/{id}",
    params(
        ("id" = String, Path, description = "Conversation identifier")
    ),
    responses(
        (status = 204, description = "Conversation deleted"),
        (status = 400, description = "Invalid conversation id", body = ApiError),
        (status = 401, description = "Authentication required", body = ApiError),
        (status = 404, description = "Conversation not found", body = ApiError),
        (status = 500, description = "Internal error", body = ApiError)
    ),
    tag = "mobile"
)]
/// DELETE `/mobile/v1/conversations/{id}` — permanently delete a conversation
/// and its transcript.
async fn handle_delete_conversation(
    State(state): State<ProtectedMobileState>,
    Extension(identity): Extension<AuthIdentity>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let conversation_id = match parse_conversation_id(&id) {
        Ok(id) => id,
        Err(response) => return response,
    };

    if let Err(response) = load_owned_conversation(&state, &identity, conversation_id).await {
        return response;
    }

    match state
        .conversations
        .delete_conversation(&conversation_id)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => internal_error(error),
    }
}

#[utoipa::path(
    get,
    path = "/mobile/v1/conversations/{id}/messages",
    params(
        ("id" = String, Path, description = "Conversation identifier"),
        ("limit" = Option<usize>, Query, description = "Maximum messages to return. Defaults to 20 when omitted, capped at 100."),
        ("after" = Option<String>, Query, description = "Opaque cursor. Return messages strictly after this position. Mutually exclusive with `before`. Read from the `x-next-cursor` response header."),
        ("before" = Option<String>, Query, description = "Opaque cursor. Return messages strictly before this position. Mutually exclusive with `after`. Read from the `x-prev-cursor` response header.")
    ),
    responses(
        (status = 200, description = "Conversation transcript page. Response headers `x-next-cursor` and `x-prev-cursor` carry opaque pagination cursors derived from the last and first messages in the page respectively.", body = [ConversationMessage]),
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
    Query(params): Query<MessageListQuery>,
) -> impl IntoResponse {
    let conversation_id = match parse_conversation_id(&id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    if let Err(response) = load_owned_conversation(&state, &identity, conversation_id).await {
        return response;
    }

    if params.after.is_some() && params.before.is_some() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "after and before are mutually exclusive",
        );
    }
    let limit = match parse_message_list_limit(params.limit) {
        Ok(v) => v,
        Err(response) => return response,
    };

    let after = params.after.as_deref().and_then(MessageCursor::decode);
    let before = params.before.as_deref().and_then(MessageCursor::decode);

    if params.after.is_some() && after.is_none() {
        return error_response(StatusCode::BAD_REQUEST, "invalid after cursor");
    }
    if params.before.is_some() && before.is_none() {
        return error_response(StatusCode::BAD_REQUEST, "invalid before cursor");
    }

    let messages = match state
        .conversations
        .list_messages(&conversation_id, after.as_ref(), before.as_ref(), limit)
        .await
    {
        Ok(messages) => messages,
        Err(error) => return internal_error(error),
    };

    let mut headers = HeaderMap::new();
    if let Some(last) = messages.last() {
        let next_cursor = MessageCursor::from_message(last).encode();
        if let Ok(value) = HeaderValue::from_str(&next_cursor) {
            headers.insert("x-next-cursor", value);
        }
    }
    if let Some(first) = messages.first() {
        let prev_cursor = MessageCursor::from_message(first).encode();
        if let Ok(value) = HeaderValue::from_str(&prev_cursor) {
            headers.insert("x-prev-cursor", value);
        }
    }

    (headers, Json(messages)).into_response()
}

#[utoipa::path(
    post,
    path = "/mobile/v1/conversations/{id}/read",
    params(
        ("id" = String, Path, description = "Conversation identifier")
    ),
    request_body = MarkReadRequest,
    responses(
        (status = 204, description = "Read watermark advanced"),
        (status = 400, description = "Invalid conversation or message id", body = ApiError),
        (status = 401, description = "Authentication required", body = ApiError),
        (status = 404, description = "Conversation or message not found", body = ApiError),
        (status = 500, description = "Internal error", body = ApiError)
    ),
    tag = "mobile"
)]
/// POST `/mobile/v1/conversations/{id}/read` — advance the read watermark.
async fn handle_mark_read(
    State(state): State<ProtectedMobileState>,
    Extension(identity): Extension<AuthIdentity>,
    Path(id): Path<String>,
    Json(body): Json<MarkReadRequest>,
) -> impl IntoResponse {
    let conversation_id = match parse_conversation_id(&id) {
        Ok(id) => id,
        Err(response) => return response,
    };

    let message_id = match Uuid::parse_str(&body.last_read_message_id) {
        Ok(uuid) => MessageId::from(uuid),
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid message id"),
    };

    match state
        .controller
        .mark_read(&identity.principal, &conversation_id, &message_id)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => map_control_error(e),
    }
}

#[utoipa::path(
    delete,
    path = "/mobile/v1/conversations/{id}/messages/{message_id}",
    params(
        ("id" = String, Path, description = "Conversation identifier"),
        ("message_id" = String, Path, description = "Message identifier")
    ),
    responses(
        (status = 204, description = "Message deleted"),
        (status = 400, description = "Invalid id", body = ApiError),
        (status = 401, description = "Authentication required", body = ApiError),
        (status = 404, description = "Conversation or message not found", body = ApiError),
        (status = 500, description = "Internal error", body = ApiError)
    ),
    tag = "mobile"
)]
/// DELETE `/mobile/v1/conversations/{id}/messages/{message_id}` — remove a
/// single message from the conversation transcript.
async fn handle_delete_message(
    State(state): State<ProtectedMobileState>,
    Extension(identity): Extension<AuthIdentity>,
    Path((id, message_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let conversation_id = match parse_conversation_id(&id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    if let Err(e) = state
        .controller
        .load_owned(&identity.principal, conversation_id)
        .await
    {
        return map_control_error(e);
    }

    let message_id = match Uuid::parse_str(&message_id) {
        Ok(uuid) => MessageId::from(uuid),
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid message id"),
    };

    match state
        .controller
        .delete_message(&conversation_id, &message_id)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => map_control_error(e),
    }
}

fn map_control_error(
    e: orka_core::conversation_controller::ControlError,
) -> axum::response::Response {
    use orka_core::conversation_controller::ControlError;
    match e {
        ControlError::NotFound | ControlError::NotOwned => {
            error_response(StatusCode::NOT_FOUND, "not found")
        }
        ControlError::InvalidState(msg) => error_response(StatusCode::CONFLICT, msg),
        ControlError::NoActiveGeneration => {
            error_response(StatusCode::CONFLICT, "no active generation to cancel")
        }
        ControlError::Store(err) => internal_error(err),
    }
}

#[utoipa::path(
    post,
    path = "/mobile/v1/conversations/{id}/cancel",
    params(
        ("id" = String, Path, description = "Conversation identifier")
    ),
    responses(
        (status = 202, description = "Cancellation accepted"),
        (status = 400, description = "Invalid conversation id", body = ApiError),
        (status = 401, description = "Authentication required", body = ApiError),
        (status = 404, description = "Conversation not found", body = ApiError),
        (status = 409, description = "No active generation to cancel", body = ApiError),
        (status = 500, description = "Internal error", body = ApiError)
    ),
    tag = "mobile"
)]
/// POST `/mobile/v1/conversations/{id}/cancel` — cancel an in-progress
/// generation.
async fn handle_cancel_generation(
    State(state): State<ProtectedMobileState>,
    Extension(identity): Extension<AuthIdentity>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let conversation_id = match parse_conversation_id(&id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let mut conversation = match state
        .controller
        .load_owned(&identity.principal, conversation_id)
        .await
    {
        Ok(c) => c,
        Err(e) => return map_control_error(e),
    };

    match state.controller.cancel_generation(&mut conversation).await {
        Ok(()) => StatusCode::ACCEPTED.into_response(),
        Err(e) => map_control_error(e),
    }
}

#[utoipa::path(
    post,
    path = "/mobile/v1/conversations/{id}/retry",
    params(
        ("id" = String, Path, description = "Conversation identifier")
    ),
    responses(
        (status = 202, description = "Retry accepted", body = SendMessageResponse),
        (status = 400, description = "Invalid conversation id or no user message found", body = ApiError),
        (status = 401, description = "Authentication required", body = ApiError),
        (status = 404, description = "Conversation not found", body = ApiError),
        (status = 409, description = "Conversation is not in failed state", body = ApiError),
        (status = 500, description = "Internal error", body = ApiError)
    ),
    tag = "mobile"
)]
/// POST `/mobile/v1/conversations/{id}/retry` — retry the last failed
/// generation.
async fn handle_retry_generation(
    State(state): State<ProtectedMobileState>,
    Extension(identity): Extension<AuthIdentity>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let conversation_id = match parse_conversation_id(&id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let mut conversation = match state
        .controller
        .load_owned(&identity.principal, conversation_id)
        .await
    {
        Ok(c) => c,
        Err(e) => return map_control_error(e),
    };

    match state.controller.retry_generation(&mut conversation).await {
        Ok(result) => (
            StatusCode::ACCEPTED,
            Json(SendMessageResponse {
                conversation_id: result.conversation_id,
                session_id: result.session_id,
                message_id: result.message_id,
            }),
        )
            .into_response(),
        Err(e) => map_control_error(e),
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
                    return error_response(
                        StatusCode::PAYLOAD_TOO_LARGE,
                        "upload exceeds size limit",
                    );
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
        return error_response(
            StatusCode::BAD_REQUEST,
            "multipart body is missing a file field",
        );
    };

    let filename = filename.unwrap_or_else(|| "upload.bin".to_string());
    let mime_type = detect_mime_type(&bytes, mime_type.as_deref(), &filename);
    if !is_allowed_upload_mime(&mime_type) {
        return error_response(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported artifact media type",
        );
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
            return Err(error_response(
                StatusCode::NOT_FOUND,
                "artifact content is missing",
            ));
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

    // Channel carries (sse_event_name, data_json) pairs.
    let (tx, rx) = mpsc::unbounded_channel::<(&'static str, String)>();
    let mut chunk_rx = state.stream_registry.subscribe(conversation.session_id);
    let mut mobile_rx = state.mobile_events.subscribe(conversation.id).await;
    let conv_id = conversation.id;

    tokio::spawn(async move {
        loop {
            tokio::select! {
                chunk = chunk_rx.recv() => {
                    let Some(chunk) = chunk else { break; };
                    let event = RealtimeEvent::from(chunk.kind.clone());
                    let name = event.sse_event_name();
                    let data = serde_json::json!({
                        "conversation_id": conv_id,
                        "reply_to": chunk.reply_to,
                        "event": event,
                    }).to_string();
                    if tx.send((name, data)).is_err() {
                        break;
                    }
                }
                mobile_event = mobile_rx.recv() => {
                    let Some(event) = mobile_event else { break; };
                    let name = event.sse_event_name();
                    let data = serde_json::json!({
                        "conversation_id": conv_id,
                        "reply_to": serde_json::Value::Null,
                        "event": event,
                    }).to_string();
                    if tx.send((name, data)).is_err() {
                        break;
                    }
                }
            }
        }
    });

    let stream = UnboundedReceiverStream::new(rx)
        .map(|(name, data)| Ok::<Event, Infallible>(Event::default().event(name).data(data)));

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
fn parse_conversation_list_pagination(
    params: &ConversationListQuery,
) -> Result<(usize, usize), axum::response::Response> {
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
fn parse_message_list_limit(limit: Option<usize>) -> Result<usize, axum::response::Response> {
    match limit {
        None => Ok(DEFAULT_PAGE_SIZE),
        Some(0) => Err(error_response(
            StatusCode::BAD_REQUEST,
            "limit must be greater than zero",
        )),
        Some(l) if l > MAX_PAGE_SIZE => Err(error_response(
            StatusCode::BAD_REQUEST,
            "limit must be less than or equal to 100",
        )),
        Some(l) => Ok(l),
    }
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
    artifacts.first().map_or_else(
        || "New message".to_string(),
        |a| format!("[{}] {}", a.mime_type, a.filename),
    )
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
        .or_else(|| {
            mime_guess::from_path(filename)
                .first_raw()
                .map(ToString::to_string)
        })
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
    response_headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
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

// ── Device management handlers ───────────────────────────────────────────────

/// GET `/mobile/v1/devices` — list all active devices for the current user.
async fn handle_list_devices(
    State(state): State<ProtectedMobileState>,
    Extension(identity): Extension<AuthIdentity>,
) -> impl IntoResponse {
    let Some(mobile_auth) = state.mobile_auth else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "mobile auth is unavailable on this server",
        );
    };
    let current_device_id = identity.device_id.as_deref().unwrap_or("");
    match mobile_auth
        .list_devices(&identity.principal, current_device_id)
        .await
    {
        Ok(devices) => {
            let response: Vec<DeviceResponse> = devices.into_iter().map(Into::into).collect();
            (StatusCode::OK, Json(response)).into_response()
        }
        Err(error) => internal_error(error),
    }
}

/// DELETE `/mobile/v1/devices/{id}` — revoke a device session.
async fn handle_remove_device(
    State(state): State<ProtectedMobileState>,
    Extension(identity): Extension<AuthIdentity>,
    Path(device_id): Path<String>,
) -> impl IntoResponse {
    let Some(mobile_auth) = state.mobile_auth else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "mobile auth is unavailable on this server",
        );
    };
    match mobile_auth
        .revoke_device(&identity.principal, &device_id)
        .await
    {
        Ok(()) => (StatusCode::NO_CONTENT, Body::empty()).into_response(),
        Err(MobileAuthError::NotFound) => error_response(StatusCode::NOT_FOUND, "device not found"),
        Err(error) => internal_error(error),
    }
}

/// PATCH `/mobile/v1/devices/{id}` — rename a device.
async fn handle_rename_device(
    State(state): State<ProtectedMobileState>,
    Extension(identity): Extension<AuthIdentity>,
    Path(device_id): Path<String>,
    Json(body): Json<RenameDeviceRequest>,
) -> impl IntoResponse {
    let Some(mobile_auth) = state.mobile_auth else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "mobile auth is unavailable on this server",
        );
    };
    if body.device_name.trim().is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "device_name must not be empty");
    }
    let current_device_id = identity.device_id.as_deref().unwrap_or("");
    match mobile_auth
        .rename_device(
            &identity.principal,
            &device_id,
            body.device_name.trim(),
            current_device_id,
        )
        .await
    {
        Ok(device) => (StatusCode::OK, Json(DeviceResponse::from(device))).into_response(),
        Err(MobileAuthError::NotFound) => error_response(StatusCode::NOT_FOUND, "device not found"),
        Err(error) => internal_error(error),
    }
}

// ── Push token handlers ──────────────────────────────────────────────────────

/// POST `/mobile/v1/devices/{id}/push-token` — register a push notification
/// token.
async fn handle_register_push_token(
    State(state): State<ProtectedMobileState>,
    Extension(identity): Extension<AuthIdentity>,
    Path(device_id): Path<String>,
    Json(body): Json<RegisterPushTokenRequest>,
) -> impl IntoResponse {
    let Some(mobile_auth) = state.mobile_auth else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "mobile auth is unavailable on this server",
        );
    };
    if body.push_token.trim().is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "push_token must not be empty");
    }
    match mobile_auth
        .register_push_token(
            &identity.principal,
            &device_id,
            body.push_token.trim(),
            body.platform.trim(),
            body.app_version.as_deref(),
        )
        .await
    {
        Ok(()) => (
            StatusCode::OK,
            Json(RegisterPushTokenResponse {
                device_id,
                push_token_registered: true,
            }),
        )
            .into_response(),
        Err(error) => internal_error(error),
    }
}

/// DELETE `/mobile/v1/devices/{id}/push-token` — unregister a push token.
async fn handle_unregister_push_token(
    State(state): State<ProtectedMobileState>,
    Extension(identity): Extension<AuthIdentity>,
    Path(device_id): Path<String>,
) -> impl IntoResponse {
    let Some(mobile_auth) = state.mobile_auth else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "mobile auth is unavailable on this server",
        );
    };
    match mobile_auth
        .unregister_push_token(&identity.principal, &device_id)
        .await
    {
        Ok(()) => (StatusCode::NO_CONTENT, Body::empty()).into_response(),
        Err(MobileAuthError::NotFound) => {
            error_response(StatusCode::NOT_FOUND, "push token not registered")
        }
        Err(error) => internal_error(error),
    }
}

// ── Search handler ───────────────────────────────────────────────────────────

/// GET `/mobile/v1/search?q=...` — search messages across all conversations.
async fn handle_search_messages(
    State(state): State<ProtectedMobileState>,
    Extension(identity): Extension<AuthIdentity>,
    Query(params): Query<SearchQuery>,
) -> impl IntoResponse {
    let q = params.q.trim();
    if q.is_empty() {
        return (
            StatusCode::OK,
            Json(SearchResponse {
                results: vec![],
                total: 0,
            }),
        )
            .into_response();
    }
    let limit = params.limit.min(MAX_PAGE_SIZE);
    let offset = params.offset;
    match state
        .conversations
        .search_messages(&identity.principal, q, limit, offset)
        .await
    {
        Ok((hits, total)) => {
            let results = hits
                .into_iter()
                .map(|h| SearchResultItem {
                    message_id: h.message_id,
                    conversation_id: h.conversation_id,
                    conversation_title: h.conversation_title,
                    role: h.role,
                    text_snippet: h.text_snippet,
                    created_at: h.created_at,
                })
                .collect();
            (StatusCode::OK, Json(SearchResponse { results, total })).into_response()
        }
        Err(error) => internal_error(error),
    }
}

// ── Transcription handler ────────────────────────────────────────────────────

/// POST `/mobile/v1/transcribe` — transcribe an audio file.
///
/// Returns 501 Not Implemented until a Whisper-compatible endpoint is
/// configured on the server.
async fn handle_transcribe(
    State(_state): State<ProtectedMobileState>,
    Extension(_identity): Extension<AuthIdentity>,
    _multipart: Multipart,
) -> impl IntoResponse {
    error_response(
        StatusCode::NOT_IMPLEMENTED,
        "audio transcription is not configured on this server",
    )
}

// ── Export handler ───────────────────────────────────────────────────────────

/// GET `/mobile/v1/conversations/{id}/export?format=md` — export a
/// conversation.
async fn handle_export_conversation(
    State(state): State<ProtectedMobileState>,
    Extension(identity): Extension<AuthIdentity>,
    Path(id): Path<String>,
    Query(params): Query<ExportQuery>,
) -> impl IntoResponse {
    let conversation_id = match parse_conversation_id(&id) {
        Ok(id) => id,
        Err(response) => return response,
    };

    let Some(format) = ExportFormat::parse(&params.format) else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "format must be one of: json, md, pdf",
        );
    };

    let conversation = match state.conversations.get_conversation(&conversation_id).await {
        Ok(Some(c)) => c,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "conversation not found"),
        Err(error) => return internal_error(error),
    };

    if conversation.user_id != identity.principal {
        return error_response(StatusCode::NOT_FOUND, "conversation not found");
    }

    let messages = match state
        .conversations
        .list_messages(&conversation_id, None, None, usize::MAX)
        .await
    {
        Ok(msgs) => msgs,
        Err(error) => return internal_error(error),
    };

    let filename = format!(
        "conversation-{}.{}",
        &id[..id.len().min(8)],
        format.extension()
    );
    let content_type = format.content_type();
    let disposition = format!("attachment; filename=\"{filename}\"");

    let bytes = match format {
        ExportFormat::Json => match export_json(&conversation, &messages) {
            Ok(b) => b,
            Err(e) => return internal_error(e),
        },
        ExportFormat::Markdown => export_markdown(&conversation, &messages),
        ExportFormat::Pdf => match export_pdf(&conversation, &messages) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!("PDF export failed: {e}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "failed to generate PDF");
            }
        },
    };

    let mut response = Response::new(Body::from(bytes));
    *response.status_mut() = StatusCode::OK;
    let hdrs = response.headers_mut();
    if let Ok(v) = HeaderValue::from_str(content_type) {
        hdrs.insert(header::CONTENT_TYPE, v);
    }
    if let Ok(v) = HeaderValue::from_str(&disposition) {
        hdrs.insert(header::CONTENT_DISPOSITION, v);
    }
    response
}

// ── Workspace endpoints
// ───────────────────────────────────────────────────────

/// GET `/mobile/v1/workspaces` — list all registered workspaces.
async fn handle_list_workspaces(State(state): State<ProtectedMobileState>) -> impl IntoResponse {
    let mut list = Vec::new();
    for name in state.workspace_registry.list_names().await {
        if let Some(loader) = state.workspace_registry.get(&name).await {
            let ws_state = loader.state();
            let ws_state = ws_state.read().await;
            let (agent_name, description) = ws_state.soul.as_ref().map_or((None, None), |d| {
                (
                    d.frontmatter.name.clone(),
                    d.frontmatter.description.clone(),
                )
            });
            list.push(serde_json::json!({
                "name": name,
                "agent_name": agent_name,
                "description": description,
                "has_tools": ws_state.tools_body.is_some(),
            }));
        }
    }
    Json(list).into_response()
}

/// GET `/mobile/v1/workspaces/{name}` — get workspace details.
async fn handle_get_workspace(
    State(state): State<ProtectedMobileState>,
    Path(ws_name): Path<String>,
) -> impl IntoResponse {
    match state.workspace_registry.get(&ws_name).await {
        None => error_response(
            StatusCode::NOT_FOUND,
            format!("workspace '{ws_name}' not found"),
        ),
        Some(loader) => {
            let ws_state = loader.state();
            let ws_state = ws_state.read().await;
            workspace_detail_response(&ws_name, &ws_state)
        }
    }
}

/// PATCH `/mobile/v1/workspaces/{name}` — update workspace SOUL.md / TOOLS.md.
async fn handle_update_workspace(
    State(state): State<ProtectedMobileState>,
    Path(ws_name): Path<String>,
    Json(body): Json<UpdateWorkspaceRequest>,
) -> impl IntoResponse {
    // Require at least one field.
    if body.agent_name.is_none()
        && body.description.is_none()
        && body.version.is_none()
        && body.soul_body.is_none()
        && body.tools_body.is_none()
    {
        return error_response(StatusCode::BAD_REQUEST, "no fields to update");
    }

    let Some(loader) = state.workspace_registry.get(&ws_name).await else {
        return error_response(
            StatusCode::NOT_FOUND,
            format!("workspace '{ws_name}' not found"),
        );
    };

    // Update SOUL.md if any soul-related field was provided.
    let soul_changed = body.agent_name.is_some()
        || body.description.is_some()
        || body.version.is_some()
        || body.soul_body.is_some();

    if soul_changed {
        // Merge partial update on top of the current in-memory state.
        let (mut fm, mut body_text) = {
            let ws_state = loader.state();
            let ws_state = ws_state.read().await;
            let (fm, b) = ws_state.soul.as_ref().map_or_else(
                || (orka_workspace::SoulFrontmatter::default(), String::new()),
                |d| (d.frontmatter.clone(), d.body.clone()),
            );
            (fm, b)
        };

        if let Some(name) = body.agent_name {
            fm.name = if name.is_empty() { None } else { Some(name) };
        }
        if let Some(desc) = body.description {
            fm.description = if desc.is_empty() { None } else { Some(desc) };
        }
        if let Some(ver) = body.version {
            fm.version = if ver.is_empty() { None } else { Some(ver) };
        }
        if let Some(soul) = body.soul_body {
            body_text = soul;
        }

        let doc = orka_workspace::Document {
            frontmatter: fm,
            body: body_text,
        };
        if let Err(e) = loader.save_soul(&doc).await {
            return internal_error(e);
        }
    }

    // Update TOOLS.md if provided.
    let tools_changed = if let Some(tools) = body.tools_body {
        if let Err(e) = loader.save_tools(&tools).await {
            return internal_error(e);
        }
        true
    } else {
        false
    };

    // Build changed_fields list for the domain event.
    let mut changed_fields: Vec<String> = Vec::new();
    if soul_changed {
        changed_fields.extend(["soul_body".into()]);
    }
    if tools_changed {
        changed_fields.push("tools_body".into());
    }

    state
        .event_sink
        .emit(DomainEvent::new(DomainEventKind::WorkspaceUpdated {
            name: ws_name.clone(),
            changed_fields,
        }))
        .await;

    // Return the updated workspace detail.
    let ws_state = loader.state();
    let ws_state = ws_state.read().await;
    workspace_detail_response(&ws_name, &ws_state)
}

/// POST `/mobile/v1/workspaces` — create a new workspace.
async fn handle_create_workspace(
    State(state): State<ProtectedMobileState>,
    Json(body): Json<CreateWorkspaceRequest>,
) -> impl IntoResponse {
    use orka_workspace::{Document, SoulFrontmatter};

    let soul = Document {
        frontmatter: SoulFrontmatter {
            name: body.agent_name,
            description: body.description,
            version: body.version,
        },
        body: body.soul_body.unwrap_or_default(),
    };

    let loader = match state
        .workspace_registry
        .create_workspace(&body.name, Some(soul), body.tools_body.as_deref())
        .await
    {
        Ok(l) => l,
        Err(e) => {
            let msg = e.to_string();
            let status = if msg.contains("already exists") {
                StatusCode::CONFLICT
            } else if msg.contains("single-workspace")
                || msg.contains("invalid")
                || msg.contains("name must")
            {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            return error_response(status, msg);
        }
    };

    let ws_state = loader.state();
    let ws_state = ws_state.read().await;
    let agent_name = ws_state
        .soul
        .as_ref()
        .and_then(|d| d.frontmatter.name.clone());
    state
        .event_sink
        .emit(DomainEvent::new(DomainEventKind::WorkspaceCreated {
            name: body.name.clone(),
            agent_name,
        }))
        .await;
    (
        StatusCode::CREATED,
        workspace_detail_response(&body.name, &ws_state),
    )
        .into_response()
}

/// DELETE `/mobile/v1/workspaces/{name}` — delete (archive) a workspace.
async fn handle_delete_workspace(
    State(state): State<ProtectedMobileState>,
    Path(ws_name): Path<String>,
) -> impl IntoResponse {
    match state.workspace_registry.remove_workspace(&ws_name).await {
        Ok(()) => {
            state
                .event_sink
                .emit(DomainEvent::new(DomainEventKind::WorkspaceRemoved {
                    name: ws_name,
                }))
                .await;
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => {
            let msg = e.to_string();
            let status = if msg.contains("not found") {
                StatusCode::NOT_FOUND
            } else if msg.contains("cannot delete the default") || msg.contains("single-workspace")
            {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            error_response(status, msg)
        }
    }
}

/// Build the JSON response for a workspace detail (shared by GET and PATCH).
fn workspace_detail_response(
    name: &str,
    ws_state: &orka_workspace::WorkspaceState,
) -> axum::response::Response {
    let fm = ws_state.soul.as_ref().map(|d| &d.frontmatter);
    Json(serde_json::json!({
        "name": name,
        "agent_name": fm.and_then(|f| f.name.as_deref()),
        "description": fm.and_then(|f| f.description.as_deref()),
        "version": fm.and_then(|f| f.version.as_deref()),
        "soul_body": ws_state.soul.as_ref().map(|d| d.body.as_str()),
        "tools_body": ws_state.tools_body.as_deref(),
    }))
    .into_response()
}
