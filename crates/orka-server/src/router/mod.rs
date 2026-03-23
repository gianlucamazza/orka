//! HTTP router construction for the Orka server.
//!
//! The [`build_router`] function assembles the full axum `Router` from a
//! [`RouterParams`] struct.  Separating construction from the binary entry
//! point makes the router testable without starting the full server process.

mod dlq;
mod health;
mod management;
mod schedules;

use std::sync::Arc;

use orka_a2a::A2aState;
use orka_agent::AgentGraph;
use orka_auth::AuthLayer;
use orka_core::traits::{PriorityQueue, SessionStore};
use orka_experience::ExperienceService;
use orka_observe::metrics::PrometheusHandle;
use orka_scheduler::ScheduleStore;
use orka_skills::{SkillRegistry, SoftSkillRegistry};
use orka_workspace::WorkspaceRegistry;
use tower_http::limit::RequestBodyLimitLayer;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

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
    )),
    tags(
        (name = "messages", description = "Message endpoints"),
        (name = "health", description = "Health check endpoints")
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

/// All dependencies needed to build the server's HTTP router.
pub struct RouterParams {
    /// Priority queue (for health checks and DLQ endpoints).
    pub queue: Arc<dyn PriorityQueue>,
    /// Registered skills (for /api/v1/skills* and eval endpoints).
    pub skills: Arc<SkillRegistry>,
    /// Soft skill registry (for /api/v1/soft-skills).
    pub soft_skills: Option<Arc<SoftSkillRegistry>>,
    /// Session store (for /api/v1/sessions*).
    pub sessions: Arc<dyn SessionStore>,
    /// Schedule store (for /api/v1/schedules*; `None` = scheduler disabled).
    pub scheduler_store: Option<Arc<dyn ScheduleStore>>,
    /// Workspace registry (for /api/v1/workspaces*).
    pub workspace_registry: Arc<WorkspaceRegistry>,
    /// Agent graph (for /api/v1/graph).
    pub graph: Arc<AgentGraph>,
    /// Experience/self-learning service (for /api/v1/experience*; `None` = disabled).
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
    /// Optional A2A protocol state (enables `/.well-known/agent.json` + `POST /a2a`).
    pub a2a_state: Option<A2aState>,
    /// Optional Prometheus metrics handle (enables `GET /metrics`).
    pub metrics_handle: Option<PrometheusHandle>,
}

/// Build the complete server `Router` from the given parameters.
///
/// Returns the composed router (without binding a TCP listener).
/// The caller is responsible for binding and spawning `axum::serve`.
pub fn build_router(p: RouterParams) -> axum::Router {
    let RouterParams {
        queue,
        skills,
        soft_skills,
        sessions,
        scheduler_store,
        workspace_registry,
        graph,
        experience_service,
        start_time,
        concurrency,
        redis_url,
        qdrant_url,
        auth_layer,
        a2a_state,
        metrics_handle,
    } = p;

    // --- Public routes (no auth) ---

    let mut public_routes = health::routes(
        queue.clone(),
        start_time,
        concurrency,
        redis_url,
        qdrant_url,
    );

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

    // --- Protected API routes ---

    let api_routes = dlq::routes(queue)
        .merge(schedules::routes(scheduler_store))
        .merge(management::routes(
            skills,
            soft_skills,
            sessions,
            workspace_registry,
            graph,
            experience_service,
        ));

    // Apply optional auth middleware
    let api_routes = if let Some(layer) = auth_layer {
        axum::Router::new().merge(api_routes.layer(layer))
    } else {
        api_routes
    };

    // Optionally add A2A protocol routes
    let public_routes = if let Some(state) = a2a_state {
        public_routes.merge(orka_a2a::a2a_router(state))
    } else {
        public_routes
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
