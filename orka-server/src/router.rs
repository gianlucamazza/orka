//! HTTP router construction for the Orka server.
//!
//! The [`build_router`] function assembles the full axum `Router` from a
//! [`RouterParams`] struct.  Separating construction from the binary entry
//! point makes the router testable without starting the full server process.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Json, Path, Query};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use orka_agent::AgentGraph;
use orka_auth::AuthLayer;
use orka_a2a::A2aState;
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

    let queue_for_health = queue.clone();
    let queue_for_dlq = queue.clone();

    // --- Public routes (no auth) ---

    let mut public_routes = axum::Router::new();

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

    let public_routes = public_routes
        .route(
            "/api/v1/version",
            axum::routing::get(|| async {
                axum::Json(serde_json::json!({
                    "version": VERSION,
                    "git_sha": GIT_SHA,
                    "build_date": BUILD_DATE,
                }))
            }),
        )
        .route(
            "/health",
            axum::routing::get(move || {
                let queue = queue_for_health.clone();
                async move {
                    let uptime_secs = start_time.elapsed().as_secs();
                    let queue_depth = queue.len().await.unwrap_or(0);

                    axum::Json(serde_json::json!({
                        "status": "ok",
                        "uptime_secs": uptime_secs,
                        "workers": concurrency,
                        "queue_depth": queue_depth,
                    }))
                }
            }),
        )
        .route(
            "/health/live",
            axum::routing::get(|| async { axum::Json(serde_json::json!({"status": "ok"})) }),
        )
        .route(
            "/health/ready",
            axum::routing::get({
                let queue = queue.clone();
                let redis_url = redis_url.clone();
                let qdrant_url = qdrant_url.clone();
                move || {
                    let queue = queue.clone();
                    let redis_url = redis_url.clone();
                    let qdrant_url = qdrant_url.clone();
                    async move {
                        let mut checks = serde_json::Map::new();
                        let mut all_ok = true;

                        match redis::Client::open(redis_url.as_str()) {
                            Ok(client) => match client.get_multiplexed_async_connection().await {
                                Ok(mut conn) => {
                                    match redis::cmd("PING").query_async::<String>(&mut conn).await
                                    {
                                        Ok(_) => {
                                            checks.insert("redis".into(), serde_json::json!("ok"));
                                        }
                                        Err(e) => {
                                            checks.insert(
                                                "redis".into(),
                                                serde_json::json!(format!("error: {e}")),
                                            );
                                            all_ok = false;
                                        }
                                    }
                                }
                                Err(e) => {
                                    checks.insert(
                                        "redis".into(),
                                        serde_json::json!(format!("error: {e}")),
                                    );
                                    all_ok = false;
                                }
                            },
                            Err(e) => {
                                checks.insert(
                                    "redis".into(),
                                    serde_json::json!(format!("error: {e}")),
                                );
                                all_ok = false;
                            }
                        }

                        match queue.len().await {
                            Ok(depth) => {
                                checks.insert(
                                    "queue".into(),
                                    serde_json::json!({"status": "ok", "depth": depth}),
                                );
                            }
                            Err(e) => {
                                checks.insert(
                                    "queue".into(),
                                    serde_json::json!(format!("error: {e}")),
                                );
                                all_ok = false;
                            }
                        }

                        if let Some(ref url) = qdrant_url {
                            match qdrant_client::Qdrant::from_url(url).build() {
                                Ok(client) => {
                                    match tokio::time::timeout(
                                        std::time::Duration::from_secs(2),
                                        client.health_check(),
                                    )
                                    .await
                                    {
                                        Ok(Ok(_)) => {
                                            checks.insert("qdrant".into(), serde_json::json!("ok"));
                                        }
                                        Ok(Err(e)) => {
                                            checks.insert(
                                                "qdrant".into(),
                                                serde_json::json!(format!("error: {e}")),
                                            );
                                            all_ok = false;
                                        }
                                        Err(_) => {
                                            checks.insert(
                                                "qdrant".into(),
                                                serde_json::json!("error: health check timed out"),
                                            );
                                            all_ok = false;
                                        }
                                    }
                                }
                                Err(e) => {
                                    checks.insert(
                                        "qdrant".into(),
                                        serde_json::json!(format!("error: {e}")),
                                    );
                                    all_ok = false;
                                }
                            }
                        }

                        let status = if all_ok { "ready" } else { "not_ready" };
                        let code = if all_ok {
                            axum::http::StatusCode::OK
                        } else {
                            axum::http::StatusCode::SERVICE_UNAVAILABLE
                        };

                        (
                            code,
                            axum::Json(serde_json::json!({
                                "status": status,
                                "checks": checks,
                            })),
                        )
                    }
                }
            }),
        );

    // --- Protected API routes (DLQ) ---

    let api_routes = axum::Router::new()
        .route(
            "/api/v1/dlq",
            axum::routing::get({
                let q = queue_for_dlq.clone();
                move || {
                    let q = q.clone();
                    async move {
                        match q.list_dlq().await {
                            Ok(items) => {
                                let json: Vec<serde_json::Value> = items
                                    .iter()
                                    .map(|e| {
                                        serde_json::json!({
                                            "id": e.id.to_string(),
                                            "channel": e.channel,
                                            "session_id": e.session_id.to_string(),
                                            "timestamp": e.timestamp.to_rfc3339(),
                                            "metadata": e.metadata,
                                        })
                                    })
                                    .collect();
                                axum::response::IntoResponse::into_response(axum::Json(json))
                            }
                            Err(e) => axum::response::IntoResponse::into_response((
                                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                                format!("DLQ list failed: {e}"),
                            )),
                        }
                    }
                }
            })
            .delete({
                let q = queue_for_dlq.clone();
                move || {
                    let q = q.clone();
                    async move {
                        match q.purge_dlq().await {
                            Ok(count) => axum::response::IntoResponse::into_response(axum::Json(
                                serde_json::json!({ "purged": count }),
                            )),
                            Err(e) => axum::response::IntoResponse::into_response((
                                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                                format!("DLQ purge failed: {e}"),
                            )),
                        }
                    }
                }
            }),
        )
        .route(
            "/api/v1/dlq/{id}/replay",
            axum::routing::post({
                let q = queue_for_dlq.clone();
                move |Path(id): Path<String>| {
                    let q = q.clone();
                    async move {
                        let msg_id = match uuid::Uuid::parse_str(&id) {
                            Ok(uuid) => orka_core::MessageId(uuid),
                            Err(_) => {
                                return axum::response::IntoResponse::into_response((
                                    axum::http::StatusCode::BAD_REQUEST,
                                    "invalid message ID",
                                ));
                            }
                        };
                        match q.replay_dlq(&msg_id).await {
                            Ok(true) => axum::response::IntoResponse::into_response(axum::Json(
                                serde_json::json!({ "replayed": true }),
                            )),
                            Ok(false) => axum::response::IntoResponse::into_response((
                                axum::http::StatusCode::NOT_FOUND,
                                "message not found in DLQ",
                            )),
                            Err(e) => axum::response::IntoResponse::into_response((
                                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                                format!("DLQ replay failed: {e}"),
                            )),
                        }
                    }
                }
            }),
        );

    // --- Management routes ---

    let s1 = skills.clone();
    let s2 = skills.clone();
    let s3 = skills.clone();
    let soft1 = soft_skills.clone();
    let sc1 = scheduler_store.clone();
    let sc2 = scheduler_store.clone();
    let sc3 = scheduler_store.clone();
    let w1 = workspace_registry.clone();
    let w2 = workspace_registry.clone();
    let g1 = graph.clone();
    let e1 = experience_service.clone();
    let e2 = experience_service.clone();
    let e3 = experience_service.clone();
    let ss1 = sessions.clone();
    let ss2 = sessions.clone();
    let ss3 = sessions.clone();

    let mgmt_routes = axum::Router::new()
        .route("/api/v1/skills", axum::routing::get(move || {
            let skills = s1.clone();
            async move {
                let list: Vec<serde_json::Value> = skills.list_info().iter().map(|(name, skill, state)| {
                    let status = match state {
                        orka_circuit_breaker::CircuitState::Closed => "ok",
                        orka_circuit_breaker::CircuitState::HalfOpen => "degraded",
                        orka_circuit_breaker::CircuitState::Open => "disabled",
                        _ => "ok",
                    };
                    serde_json::json!({
                        "name": name,
                        "category": skill.category(),
                        "description": skill.description(),
                        "status": status,
                        "schema": skill.schema(),
                    })
                }).collect();
                axum::Json(list)
            }
        }))
        .route("/api/v1/soft-skills", axum::routing::get(move || {
            let reg = soft1.clone();
            async move {
                let list: Vec<serde_json::Value> = reg
                    .as_deref()
                    .map(|r| r.summaries())
                    .unwrap_or_default()
                    .into_iter()
                    .map(|s| serde_json::json!({
                        "name": s.name,
                        "description": s.description,
                        "tags": s.tags,
                    }))
                    .collect();
                axum::Json(list)
            }
        }))
        .route("/api/v1/skills/{name}", axum::routing::get(move |Path(name): Path<String>| {
            let skills = s2.clone();
            async move {
                match skills.get(&name) {
                    Some(skill) => axum::Json(serde_json::json!({
                        "name": skill.name(),
                        "description": skill.description(),
                        "schema": skill.schema(),
                    })).into_response(),
                    None => (StatusCode::NOT_FOUND, format!("skill '{name}' not found")).into_response(),
                }
            }
        }))
        .route("/api/v1/eval", axum::routing::post(move |Json(body): Json<serde_json::Value>| {
            let skills = s3.clone();
            async move {
                let skill_filter = body["skill"].as_str().map(String::from);
                let dir = body["dir"].as_str().unwrap_or("evals").to_string();
                let runner = orka_eval::EvalRunner::new(skills);
                match runner.run_dir(std::path::Path::new(&dir), skill_filter.as_deref()).await {
                    Ok(report) => {
                        let json_str = report.to_json();
                        let val: serde_json::Value = serde_json::from_str(&json_str).unwrap_or_default();
                        axum::Json(val).into_response()
                    }
                    Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("eval failed: {e}")).into_response(),
                }
            }
        }))
        .route("/api/v1/schedules",
            axum::routing::get(move || {
                let store = sc1.clone();
                async move {
                    let Some(store) = store else {
                        return (StatusCode::SERVICE_UNAVAILABLE, "scheduler not enabled").into_response();
                    };
                    match store.list(false).await {
                        Ok(s) => axum::Json(s).into_response(),
                        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("list failed: {e}")).into_response(),
                    }
                }
            })
            .post(move |Json(body): Json<serde_json::Value>| {
                let store = sc2.clone();
                async move {
                    let Some(store) = store else {
                        return (StatusCode::SERVICE_UNAVAILABLE, "scheduler not enabled").into_response();
                    };
                    let name = match body["name"].as_str() {
                        Some(n) => n.to_string(),
                        None => return (StatusCode::BAD_REQUEST, "'name' is required").into_response(),
                    };
                    let cron_expr = body["cron"].as_str().map(String::from);
                    let run_at_str = body["run_at"].as_str().map(String::from);
                    if cron_expr.is_none() && run_at_str.is_none() {
                        return (StatusCode::BAD_REQUEST, "either 'cron' or 'run_at' is required").into_response();
                    }
                    let next_run = if let Some(ref cron_str) = cron_expr {
                        use std::str::FromStr as _;
                        match cron::Schedule::from_str(cron_str) {
                            Ok(sched) => match sched.upcoming(chrono::Utc).next().map(|t| t.timestamp()) {
                                Some(ts) => ts,
                                None => return (StatusCode::BAD_REQUEST, "no upcoming run for cron").into_response(),
                            },
                            Err(e) => return (StatusCode::BAD_REQUEST, format!("invalid cron: {e}")).into_response(),
                        }
                    } else if let Some(ref run_at) = run_at_str {
                        match chrono::DateTime::parse_from_rfc3339(run_at) {
                            Ok(dt) => dt.timestamp(),
                            Err(e) => return (StatusCode::BAD_REQUEST, format!("invalid run_at: {e}")).into_response(),
                        }
                    } else { 0 };
                    let args: Option<HashMap<String, serde_json::Value>> = body["args"]
                        .as_object()
                        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
                    let schedule = orka_scheduler::types::Schedule {
                        id: uuid::Uuid::now_v7().to_string(),
                        name,
                        cron: cron_expr,
                        run_at: run_at_str,
                        timezone: body["timezone"].as_str().map(String::from),
                        skill: body["skill"].as_str().map(String::from),
                        args,
                        message: body["message"].as_str().map(String::from),
                        next_run,
                        created_at: chrono::Utc::now().to_rfc3339(),
                        completed: false,
                    };
                    match store.add(&schedule).await {
                        Ok(()) => (StatusCode::CREATED, axum::Json(schedule)).into_response(),
                        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("create failed: {e}")).into_response(),
                    }
                }
            })
        )
        .route("/api/v1/schedules/{id}", axum::routing::delete(move |Path(id): Path<String>| {
            let store = sc3.clone();
            async move {
                let Some(store) = store else {
                    return (StatusCode::SERVICE_UNAVAILABLE, "scheduler not enabled").into_response();
                };
                match store.remove(&id).await {
                    Ok(found) => axum::Json(serde_json::json!({ "deleted": found })).into_response(),
                    Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("delete failed: {e}")).into_response(),
                }
            }
        }))
        .route("/api/v1/workspaces", axum::routing::get(move || {
            let registry = w1.clone();
            async move {
                let mut list = Vec::new();
                for name in registry.list_names() {
                    if let Some(loader) = registry.get(name) {
                        let state = loader.state();
                        let state = state.read().await;
                        let (agent_name, description) = state.soul.as_ref()
                            .map(|d| (d.frontmatter.name.clone(), d.frontmatter.description.clone()))
                            .unwrap_or((None, None));
                        list.push(serde_json::json!({
                            "name": name,
                            "agent_name": agent_name,
                            "description": description,
                            "has_tools": state.tools_body.is_some(),
                        }));
                    }
                }
                axum::Json(list)
            }
        }))
        .route("/api/v1/workspaces/{name}", axum::routing::get(move |Path(ws_name): Path<String>| {
            let registry = w2.clone();
            async move {
                match registry.get(&ws_name) {
                    None => (StatusCode::NOT_FOUND, format!("workspace '{ws_name}' not found")).into_response(),
                    Some(loader) => {
                        let state = loader.state();
                        let state = state.read().await;
                        let fm = state.soul.as_ref().map(|d| &d.frontmatter);
                        axum::Json(serde_json::json!({
                            "name": ws_name,
                            "agent_name": fm.and_then(|f| f.name.as_deref()),
                            "description": fm.and_then(|f| f.description.as_deref()),
                            "version": fm.and_then(|f| f.version.as_deref()),
                            "soul_body": state.soul.as_ref().map(|d| d.body.as_str()),
                            "tools_body": state.tools_body.as_deref(),
                        })).into_response()
                    }
                }
            }
        }))
        .route("/api/v1/graph", axum::routing::get(move || {
            let g = g1.clone();
            async move {
                let nodes: Vec<serde_json::Value> = g.nodes_iter().map(|(id, node)| {
                    serde_json::json!({
                        "id": id.to_string(),
                        "kind": format!("{:?}", node.kind),
                        "agent": {
                            "id": node.agent.id.to_string(),
                            "name": node.agent.display_name,
                            "max_iterations": node.agent.max_iterations,
                            "handoff_targets": node.agent.handoff_targets.iter()
                                .map(|t| t.to_string()).collect::<Vec<_>>(),
                        }
                    })
                }).collect();
                let edges: Vec<serde_json::Value> = g.edges_iter().flat_map(|(from, edges)| {
                    let from = from.to_string();
                    edges.iter().map(move |e| {
                        let condition = match &e.condition {
                            None => serde_json::json!("always"),
                            Some(orka_agent::EdgeCondition::Always) => serde_json::json!("always"),
                            Some(orka_agent::EdgeCondition::OutputContains(s)) => {
                                serde_json::json!({"output_contains": s})
                            }
                            Some(orka_agent::EdgeCondition::StateMatch { key, pattern }) => {
                                serde_json::json!({
                                    "state_match": {
                                        "key": format!("{}.{}", key.namespace, key.name),
                                        "pattern": pattern,
                                    }
                                })
                            }
                        };
                        serde_json::json!({
                            "from": from,
                            "to": e.target.to_string(),
                            "priority": e.priority,
                            "condition": condition,
                        })
                    }).collect::<Vec<_>>()
                }).collect();
                axum::Json(serde_json::json!({
                    "id": g.id,
                    "entry": g.entry.to_string(),
                    "nodes": nodes,
                    "edges": edges,
                    "termination": {
                        "max_total_iterations": g.termination.max_total_iterations,
                        "max_total_tokens": g.termination.max_total_tokens,
                        "max_duration_secs": g.termination.max_duration.as_secs(),
                        "terminal_agents": g.termination.terminal_agents.iter()
                            .map(|a| a.to_string()).collect::<Vec<_>>(),
                    }
                }))
            }
        }))
        .route("/api/v1/experience/status", axum::routing::get(move || {
            let exp = e1.clone();
            async move {
                axum::Json(serde_json::json!({
                    "enabled": exp.as_ref().map(|e| e.is_enabled()).unwrap_or(false),
                }))
            }
        }))
        .route("/api/v1/experience/principles", axum::routing::get(
            move |Query(params): Query<HashMap<String, String>>| {
                let exp = e2.clone();
                async move {
                    let Some(exp) = exp else {
                        return (StatusCode::SERVICE_UNAVAILABLE, "experience not enabled").into_response();
                    };
                    let workspace = params.get("workspace").map(String::as_str).unwrap_or("default");
                    let query = params.get("query").map(String::as_str).unwrap_or("");
                    let limit: usize = params.get("limit").and_then(|l| l.parse().ok()).unwrap_or(10);
                    match exp.retrieve_principles(query, workspace).await {
                        Ok(mut principles) => {
                            principles.truncate(limit);
                            axum::Json(principles).into_response()
                        }
                        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("retrieve failed: {e}")).into_response(),
                    }
                }
            }
        ))
        .route("/api/v1/experience/distill", axum::routing::post(
            move |Json(body): Json<serde_json::Value>| {
                let exp = e3.clone();
                async move {
                    let Some(exp) = exp else {
                        return (StatusCode::SERVICE_UNAVAILABLE, "experience not enabled").into_response();
                    };
                    let workspace = body["workspace"].as_str().unwrap_or("default");
                    match exp.distill(workspace).await {
                        Ok(created) => axum::Json(serde_json::json!({ "created": created })).into_response(),
                        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("distill failed: {e}")).into_response(),
                    }
                }
            }
        ))
        .route("/api/v1/sessions",
            axum::routing::get(move |Query(params): Query<HashMap<String, String>>| {
                let sessions = ss1.clone();
                async move {
                    let limit: usize = params.get("limit").and_then(|l| l.parse().ok()).unwrap_or(20);
                    match sessions.list(limit).await {
                        Ok(list) => axum::Json(list).into_response(),
                        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("list failed: {e}")).into_response(),
                    }
                }
            })
        )
        .route("/api/v1/sessions/{id}",
            axum::routing::get(move |Path(id): Path<String>| {
                let sessions = ss2.clone();
                async move {
                    match uuid::Uuid::parse_str(&id) {
                        Err(_) => (StatusCode::BAD_REQUEST, "invalid session ID").into_response(),
                        Ok(uuid) => {
                            let sid = orka_core::SessionId(uuid);
                            match sessions.get(&sid).await {
                                Ok(Some(s)) => axum::Json(s).into_response(),
                                Ok(None) => (StatusCode::NOT_FOUND, "session not found").into_response(),
                                Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("get failed: {e}")).into_response(),
                            }
                        }
                    }
                }
            })
            .delete(move |Path(id): Path<String>| {
                let sessions = ss3.clone();
                async move {
                    match uuid::Uuid::parse_str(&id) {
                        Err(_) => (StatusCode::BAD_REQUEST, "invalid session ID").into_response(),
                        Ok(uuid) => {
                            let sid = orka_core::SessionId(uuid);
                            match sessions.delete(&sid).await {
                                Ok(()) => axum::Json(serde_json::json!({ "deleted": true })).into_response(),
                                Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("delete failed: {e}")).into_response(),
                            }
                        }
                    }
                }
            })
        );

    let api_routes = api_routes.merge(mgmt_routes);

    // Apply optional auth middleware to all protected API routes
    let api_routes = if let Some(layer) = auth_layer {
        axum::Router::new().merge(api_routes.layer(layer))
    } else {
        api_routes
    };

    // Optionally add A2A protocol routes to the public routes
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
