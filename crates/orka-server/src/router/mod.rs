//! HTTP router construction for the Orka server.
//!
//! The [`build_router`] function assembles the full axum `Router` from a
//! [`RouterParams`] struct.  Separating construction from the binary entry
//! point makes the router testable without starting the full server process.

// The `OpenApi` derive macro generates code that triggers this lint.
#![allow(clippy::needless_for_each)]

mod checkpoints;
mod dlq;
mod health;
mod management;
mod mobile;
mod research;
mod schedules;

use std::sync::Arc;

use orka_a2a::{A2aState, AgentDirectory};
use orka_agent::AgentGraph;
use orka_auth::AuthLayer;
use orka_checkpoint::CheckpointStore;
use orka_core::traits::{
    ConversationStore, DeadLetterQueue, MessageBus, PriorityQueue, SessionStore,
};
use orka_experience::ExperienceService;
use orka_observe::metrics::PrometheusHandle;
use orka_research::ResearchService;
use orka_scheduler::ScheduleStore;
use orka_skills::{SkillRegistry, SoftSkillRegistry};
use orka_workspace::WorkspaceRegistry;
use tower_http::limit::RequestBodyLimitLayer;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

pub use mobile::{MobileEventHub, MobileStreamEvent};

/// Server version from `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Short git SHA injected by `build.rs`.
pub const GIT_SHA: &str = env!("ORKA_GIT_SHA");
/// Build date injected by `build.rs`.
pub const BUILD_DATE: &str = env!("ORKA_BUILD_DATE");

/// Maximum request body size for server API endpoints: 1 MB.
const MAX_BODY_SIZE: usize = 1024 * 1024;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Orka API",
        description = "Orka AI Agent Orchestration Platform",
    ),
    paths(
        orka_adapter_custom::routes::handle_message,
        orka_adapter_custom::routes::handle_health,
        orka_a2a::routes::handle_agent_card,
        orka_a2a::routes::handle_a2a,
        mobile::handle_me,
        mobile::handle_list_conversations,
        mobile::handle_create_conversation,
        mobile::handle_get_conversation,
        mobile::handle_list_messages,
        mobile::handle_send_message,
        mobile::handle_stream,
    ),
    components(schemas(
        orka_core::Envelope,
        orka_core::Payload,
        orka_core::Priority,
        orka_core::OutboundMessage,
        orka_core::Session,
        orka_core::SessionId,
        orka_core::MessageId,
        orka_core::TraceContext,
        orka_core::MediaPayload,
        orka_core::CommandPayload,
        orka_core::EventPayload,
        orka_core::Conversation,
        orka_core::ConversationId,
        orka_core::ConversationMessage,
        orka_core::ConversationMessageRole,
        orka_core::ConversationMessageStatus,
        orka_core::ConversationStatus,
        orka_a2a::AgentCard,
        orka_a2a::AgentSkill,
        orka_a2a::SupportedInterface,
        orka_a2a::InterfaceCapabilities,
        orka_a2a::SecurityScheme,
        orka_a2a::Task,
        orka_a2a::TaskKind,
        orka_a2a::TaskState,
        orka_a2a::TaskStatus,
        orka_a2a::Message,
        orka_a2a::MessageKind,
        orka_a2a::Role,
        orka_a2a::Part,
        orka_a2a::FileContent,
        orka_a2a::Artifact,
        orka_a2a::TaskEvent,
        orka_a2a::PushNotificationConfig,
        orka_a2a::PushNotificationAuth,
        orka_a2a::ListTasksParams,
        orka_a2a::ListTasksResult,
        mobile::ApiError,
        mobile::CreateConversationRequest,
        mobile::CurrentUserResponse,
        mobile::SendMessageRequest,
        mobile::SendMessageResponse,
        mobile::StreamDonePayload,
    )),
    tags(
        (name = "messages", description = "Message endpoints"),
        (name = "health", description = "Health check endpoints"),
        (name = "a2a", description = "Agent-to-Agent (A2A) protocol endpoints"),
        (name = "mobile", description = "Public mobile product API")
    )
)]
struct ApiDoc;

/// Middleware that adds security headers to all responses.
pub async fn security_headers(
    req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> axum::response::Response {
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

/// Feature flags reported by the server in `/api/v1/info`.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, serde::Serialize)]
pub struct ServerFeatures {
    /// Whether knowledge and retrieval endpoints are enabled.
    pub knowledge: bool,
    /// Whether scheduler endpoints are enabled.
    pub scheduler: bool,
    /// Whether experience/self-learning endpoints are enabled.
    pub experience: bool,
    /// Whether guardrail endpoints are enabled.
    pub guardrails: bool,
    /// Whether A2A endpoints are enabled.
    pub a2a: bool,
    /// Whether observability endpoints are enabled.
    pub observe: bool,
    /// Whether research campaign endpoints are enabled.
    pub research: bool,
}

/// Lightweight server info returned by `GET /api/v1/info` for CLI banners and
/// tooling. Built once at router construction time with no runtime I/O.
#[derive(Clone, serde::Serialize)]
pub struct ServerInfo {
    /// Semantic version of the running build.
    pub version: &'static str,
    /// Git commit SHA embedded at build time.
    pub git_sha: &'static str,
    /// Build timestamp embedded at build time.
    pub build_date: &'static str,
    /// Display name of the primary agent.
    pub agent_name: String,
    /// Model configured for the primary agent.
    pub agent_model: String,
    /// Total registered skill count.
    pub skill_count: usize,
    /// Number of configured MCP servers.
    pub mcp_server_count: usize,
    /// Configured worker concurrency.
    pub workers: usize,
    /// Feature flags exposed by this server.
    pub features: ServerFeatures,
    /// Extended thinking mode for the primary agent, if configured.
    pub thinking: Option<String>,
    /// Number of agents in the graph.
    pub agent_count: usize,
    /// Whether API-key or JWT auth is enabled.
    pub auth_enabled: bool,
    /// Channel IDs of active adapters (e.g. "telegram", "discord").
    pub adapters: Vec<String>,
    /// Selected coding backend, if OS/coding integration is active.
    pub coding_backend: Option<String>,
    /// Configured web search provider, if active.
    pub web_search: Option<String>,
}

/// All dependencies needed to build the server's HTTP router.
pub struct RouterParams {
    /// Bus used by product-facing routes to publish inbound messages.
    pub bus: Arc<dyn MessageBus>,
    /// Priority queue (for health checks).
    pub queue: Arc<dyn PriorityQueue>,
    /// Dead-letter queue (for DLQ API endpoints).
    pub dlq: Arc<dyn DeadLetterQueue>,
    /// Registered skills (for /api/v1/skills* and eval endpoints).
    pub skills: Arc<SkillRegistry>,
    /// Soft skill registry (for /api/v1/soft-skills).
    pub soft_skills: Option<Arc<SoftSkillRegistry>>,
    /// Session store (for /api/v1/sessions*).
    pub sessions: Arc<dyn SessionStore>,
    /// Product-facing conversation store.
    pub conversations: Arc<dyn ConversationStore>,
    /// Schedule store (for /api/v1/schedules*; `None` = scheduler disabled).
    pub scheduler_store: Option<Arc<dyn ScheduleStore>>,
    /// Checkpoint store (for /api/v1/runs*; `None` = checkpointing disabled).
    pub checkpoint_store: Option<Arc<dyn CheckpointStore>>,
    /// Workspace registry (for /api/v1/workspaces*).
    pub workspace_registry: Arc<WorkspaceRegistry>,
    /// Agent graph (for /api/v1/graph).
    pub graph: Arc<AgentGraph>,
    /// Experience/self-learning service (for /api/v1/experience*; `None` =
    /// disabled).
    pub experience_service: Option<Arc<ExperienceService>>,
    /// Server start time used in the health endpoint's `uptime_secs` field.
    pub start_time: std::time::Instant,
    /// Worker concurrency reported in /health.
    pub concurrency: usize,
    /// Redis URL used by /health/ready to verify connectivity.
    pub redis_url: String,
    /// Qdrant URL checked in /health/ready (only when knowledge is enabled).
    pub qdrant_url: Option<String>,
    /// Optional auth middleware applied to all protected API routes.
    pub auth_layer: Option<AuthLayer>,
    /// Optional A2A protocol state (enables `/.well-known/agent.json` + `POST
    /// /a2a`).
    pub a2a_state: Option<A2aState>,
    /// When `true`, mount `POST /a2a` inside the auth-protected route group.
    /// `GET /.well-known/agent.json` remains public regardless of this flag.
    pub a2a_auth_enabled: bool,
    /// Discovered remote agent directory (for `GET /api/v1/a2a/agents`).
    pub agent_directory: Arc<AgentDirectory>,
    /// Optional Prometheus metrics handle (enables `GET /metrics`).
    pub metrics_handle: Option<PrometheusHandle>,
    /// Primary agent display name for `/api/v1/info`.
    pub agent_name: String,
    /// Primary agent model identifier for `/api/v1/info`.
    pub agent_model: String,
    /// Number of successfully connected MCP servers for `/api/v1/info`.
    pub mcp_server_count: usize,
    /// Feature flags for `/api/v1/info`.
    pub features: ServerFeatures,
    /// Thinking mode of the primary agent for `/api/v1/info`.
    pub thinking: Option<String>,
    /// Number of agents in the graph for `/api/v1/info`.
    pub agent_count: usize,
    /// Whether API-key or JWT auth is enabled for `/api/v1/info`.
    pub auth_enabled: bool,
    /// Active adapter channel IDs for `/api/v1/info`.
    pub adapters: Vec<String>,
    /// Selected coding backend for `/api/v1/info`.
    pub coding_backend: Option<String>,
    /// Configured web search provider for `/api/v1/info`.
    pub web_search: Option<String>,
    /// Research service (for /api/v1/research*; `None` = research disabled).
    pub research_service: Option<Arc<ResearchService>>,
    /// Stream registry for SSE endpoints.
    pub stream_registry: orka_core::StreamRegistry,
    /// Product-facing mobile event hub.
    pub mobile_events: MobileEventHub,
    /// Whether the mobile product API should be exposed.
    pub mobile_enabled: bool,
}

/// Build the complete server `Router` from the given parameters.
///
/// Returns the composed router (without binding a TCP listener).
/// The caller is responsible for binding and spawning `axum::serve`.
#[allow(clippy::too_many_lines)]
pub fn build_router(p: RouterParams) -> axum::Router {
    let RouterParams {
        queue,
        bus,
        dlq,
        skills,
        soft_skills,
        sessions,
        conversations,
        scheduler_store,
        checkpoint_store,
        workspace_registry,
        graph,
        experience_service,
        start_time,
        concurrency,
        redis_url,
        qdrant_url,
        auth_layer,
        a2a_state,
        a2a_auth_enabled,
        agent_directory,
        metrics_handle,
        agent_name,
        agent_model,
        mcp_server_count,
        features,
        thinking,
        agent_count,
        auth_enabled,
        adapters,
        coding_backend,
        web_search,
        research_service,
        stream_registry,
        mobile_events,
        mobile_enabled,
    } = p;

    // --- Public routes (no auth) ---

    let mut public_routes = health::routes(&queue, start_time, concurrency, redis_url, qdrant_url);

    if let Some(handle) = metrics_handle {
        let handle = Arc::new(handle);
        public_routes = public_routes.route(
            "/metrics",
            axum::routing::get(move || {
                let h = handle.clone();
                async move { h.render() }
            }),
        );
    }

    public_routes = public_routes.route(
        "/api/v1/version",
        axum::routing::get(|| async {
            axum::Json(serde_json::json!({
                "version": VERSION,
                "git_sha": GIT_SHA,
                "build_date": BUILD_DATE,
            }))
        }),
    );

    {
        let info = std::sync::Arc::new(ServerInfo {
            version: VERSION,
            git_sha: GIT_SHA,
            build_date: BUILD_DATE,
            agent_name,
            agent_model,
            skill_count: skills.list().len(),
            mcp_server_count,
            workers: concurrency,
            features,
            thinking,
            agent_count,
            auth_enabled,
            adapters,
            coding_backend,
            web_search,
        });
        public_routes = public_routes.route(
            "/api/v1/info",
            axum::routing::get(move || {
                let info = info.clone();
                async move { axum::Json((*info).clone()) }
            }),
        );
    }

    // --- A2A discovery directory endpoint ---
    {
        let dir = agent_directory;
        public_routes = public_routes.route(
            "/api/v1/a2a/agents",
            axum::routing::get(move || {
                let d = dir.clone();
                async move { axum::Json(d.all().await) }
            }),
        );
    }

    // --- Protected API routes ---

    let api_routes = dlq::routes(dlq)
        .merge(schedules::routes(scheduler_store))
        .merge(checkpoints::routes(checkpoint_store, queue.clone()))
        .merge(management::routes(
            skills,
            soft_skills,
            sessions,
            workspace_registry,
            graph,
            experience_service,
        ))
        .merge(research::routes(research_service, stream_registry.clone()));

    let api_routes = if mobile_enabled {
        api_routes.merge(mobile::routes(
            conversations,
            bus,
            stream_registry,
            mobile_events,
        ))
    } else {
        api_routes
    };

    // Optionally add A2A protocol routes.
    // When auth is enabled, the agent-card stays public while POST /a2a is
    // merged into protected_routes BEFORE the auth layer is applied.
    let (public_routes, api_routes) = if let Some(state) = a2a_state {
        if a2a_auth_enabled {
            let (a2a_public, a2a_protected) = orka_a2a::a2a_routes_split(state);
            (
                public_routes.merge(a2a_public),
                api_routes.merge(a2a_protected),
            )
        } else {
            (public_routes.merge(orka_a2a::a2a_router(state)), api_routes)
        }
    } else {
        (public_routes, api_routes)
    };

    // Apply optional auth middleware (after A2A protected routes are merged in)
    let api_routes = if let Some(layer) = auth_layer {
        axum::Router::new().merge(api_routes.layer(layer))
    } else {
        api_routes
    };

    public_routes
        .merge(api_routes)
        .merge(SwaggerUi::new("/docs").url("/api-doc/openapi.json", {
            let mut doc = ApiDoc::openapi();
            doc.info.version = VERSION.to_string();
            doc
        }))
        .layer(axum::middleware::from_fn(security_headers))
        .layer(RequestBodyLimitLayer::new(MAX_BODY_SIZE))
}
