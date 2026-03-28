//! Shared test helpers for orka-server integration tests.
//!
//! Provides [`test_router`] and [`test_router_with_auth`] which build the
//! full production router backed by in-memory test doubles.

#![allow(dead_code, missing_docs)]

use std::{error::Error, sync::Arc};

use axum::{body::Body, response::Response};
use orka_a2a::AgentDirectory;
use orka_agent::{Agent, AgentGraph, AgentId, GraphNode, NodeKind, TerminationPolicy};
use orka_core::testing::{InMemoryQueue, InMemorySessionStore};
use orka_server::router::{RouterParams, ServerFeatures, build_router};
use orka_skills::{EchoSkill, SkillRegistry};
use orka_workspace::{WorkspaceLoader, WorkspaceRegistry};

pub(crate) type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

pub(crate) fn request(
    builder: http::request::Builder,
    body: Body,
) -> TestResult<http::Request<Body>> {
    builder.body(body).map_err(Into::into)
}

pub(crate) async fn json_body(response: Response) -> TestResult<serde_json::Value> {
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await?;
    Ok(serde_json::from_slice(&body)?)
}

pub(crate) fn json_array(value: &serde_json::Value) -> TestResult<&Vec<serde_json::Value>> {
    value.as_array().ok_or_else(|| "expected JSON array".into())
}

struct StaticJsonSkill {
    name: &'static str,
    description: &'static str,
    data: serde_json::Value,
}

#[async_trait::async_trait]
impl orka_core::traits::Skill for StaticJsonSkill {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        self.description
    }

    fn schema(&self) -> orka_core::SkillSchema {
        orka_core::SkillSchema::new(serde_json::json!({}))
    }

    async fn execute(
        &self,
        _input: orka_core::SkillInput,
    ) -> orka_core::Result<orka_core::SkillOutput> {
        Ok(orka_core::SkillOutput::new(self.data.clone()))
    }
}

fn register_static_json_skill(
    skills: &mut SkillRegistry,
    name: &'static str,
    description: &'static str,
    data: serde_json::Value,
) {
    skills.register(Arc::new(StaticJsonSkill {
        name,
        description,
        data,
    }));
}

fn register_research_test_skills(skills: &mut SkillRegistry) {
    register_static_json_skill(
        skills,
        "git_worktree_create",
        "create worktree",
        serde_json::json!({ "path": "/tmp/test-wt" }),
    );
    register_static_json_skill(
        skills,
        "coding_delegate",
        "coding delegate",
        serde_json::json!({ "backend": "codex", "result": "implemented" }),
    );
    register_static_json_skill(
        skills,
        "git_diff",
        "git diff",
        serde_json::json!({ "diff": "diff --git a/f b/f" }),
    );
    register_static_json_skill(
        skills,
        "shell_exec",
        "shell exec",
        serde_json::json!({
            "exit_code": 0,
            "stdout": "ok",
            "stderr": "",
            "duration_ms": 1,
        }),
    );
    register_static_json_skill(
        skills,
        "git_checkout",
        "git checkout",
        serde_json::json!({ "branch": "main" }),
    );
    register_static_json_skill(
        skills,
        "git_merge",
        "git merge",
        serde_json::json!({ "merged": true }),
    );
}

/// Build a minimal agent graph with a single "test" node for use in tests.
fn test_graph() -> Arc<AgentGraph> {
    let agent_id = AgentId::from("test");
    let agent = Agent::new(agent_id.clone(), "Test Agent");
    let policy = TerminationPolicy {
        terminal_agents: std::iter::once(agent_id.clone()).collect(),
        ..TerminationPolicy::default()
    };
    let mut graph = AgentGraph::new("test-graph", agent_id).with_termination(policy);
    graph.add_node(GraphNode {
        agent,
        kind: NodeKind::Agent,
    });
    Arc::new(graph)
}

/// Build a minimal `WorkspaceRegistry` with an empty default workspace.
fn test_workspace_registry() -> Arc<WorkspaceRegistry> {
    let loader = Arc::new(WorkspaceLoader::new("."));
    let mut reg = WorkspaceRegistry::new("default".to_string());
    reg.register("default".to_string(), loader);
    Arc::new(reg)
}

fn test_features() -> ServerFeatures {
    ServerFeatures {
        knowledge: false,
        scheduler: false,
        experience: false,
        guardrails: false,
        a2a: false,
        observe: false,
        research: false,
    }
}

/// Build the full server router backed by in-memory test doubles.
///
/// - No auth (all routes accessible without API key)
/// - No A2A
/// - No scheduler
/// - No experience service
/// - One registered skill: `EchoSkill`
pub(crate) fn test_router() -> axum::Router {
    let mut skills = SkillRegistry::new();
    skills.register(Arc::new(EchoSkill));

    let q = Arc::new(InMemoryQueue::new());
    build_router(RouterParams {
        queue: q.clone(),
        dlq: q,
        skills: Arc::new(skills),
        soft_skills: None,
        sessions: Arc::new(InMemorySessionStore::new()),
        scheduler_store: None,
        checkpoint_store: None,
        workspace_registry: test_workspace_registry(),
        graph: test_graph(),
        experience_service: None,
        start_time: std::time::Instant::now(),
        concurrency: 1,
        redis_url: "redis://127.0.0.1:6379".to_string(),
        qdrant_url: None,
        auth_layer: None,
        a2a_state: None,
        a2a_auth_enabled: false,
        agent_directory: Arc::new(AgentDirectory::new()),
        metrics_handle: None,
        agent_name: "Test Agent".to_string(),
        agent_model: "claude-sonnet-4-6".to_string(),
        mcp_server_count: 0,
        features: test_features(),
        thinking: None,
        agent_count: 1,
        auth_enabled: false,
        adapters: vec![],
        coding_backend: None,
        web_search: None,
        research_service: None,
        stream_registry: orka_core::StreamRegistry::new(),
    })
}

/// Build the server router with A2A enabled and optional auth protection on
/// `POST /a2a`.
///
/// When `auth_enabled` is `true`:
/// - `GET /.well-known/agent.json` remains public.
/// - `POST /a2a` requires `X-Api-Key: <key>`.
pub(crate) fn test_router_with_a2a(key: &str, a2a_auth_enabled: bool) -> axum::Router {
    use orka_a2a::{
        A2aState, InMemoryPushNotificationStore, InMemoryTaskStore, WebhookDeliverer,
        build_agent_card,
    };
    use orka_auth::{
        ApiKeyAuthenticator, ApiKeyEntry, AuthLayer, middleware::AuthMiddlewareConfig,
    };
    use orka_core::testing::InMemorySecretManager;
    use sha2::{Digest, Sha256};

    let mut skills = SkillRegistry::new();
    skills.register(Arc::new(EchoSkill));
    let skills = Arc::new(skills);

    let agent_card = build_agent_card("test-agent", "A test agent", "http://localhost", &skills);
    let push_store = Arc::new(InMemoryPushNotificationStore::default());
    let a2a_state = Some(A2aState {
        agent_card,
        skills: skills.clone(),
        secrets: Arc::new(InMemorySecretManager::new()),
        task_store: Arc::new(InMemoryTaskStore::default()),
        task_events: Arc::default(),
        push_store: push_store.clone(),
        webhook_deliverer: Arc::new(WebhookDeliverer::new(push_store)),
    });

    let key_hash = format!("{:x}", Sha256::digest(key.as_bytes()));
    let entries = vec![ApiKeyEntry::new("test-key", key_hash, vec![])];
    let auth_cfg = Arc::new(AuthMiddlewareConfig::default());
    let authenticator = Arc::new(ApiKeyAuthenticator::new(&entries));
    let auth_layer = Some(AuthLayer::new(authenticator, auth_cfg));

    let q = Arc::new(InMemoryQueue::new());
    build_router(RouterParams {
        queue: q.clone(),
        dlq: q,
        skills,
        soft_skills: None,
        sessions: Arc::new(InMemorySessionStore::new()),
        scheduler_store: None,
        checkpoint_store: None,
        workspace_registry: test_workspace_registry(),
        graph: test_graph(),
        experience_service: None,
        start_time: std::time::Instant::now(),
        concurrency: 1,
        redis_url: "redis://127.0.0.1:6379".to_string(),
        qdrant_url: None,
        auth_layer,
        a2a_state,
        a2a_auth_enabled,
        agent_directory: Arc::new(AgentDirectory::new()),
        metrics_handle: None,
        agent_name: "Test Agent".to_string(),
        agent_model: "claude-sonnet-4-6".to_string(),
        mcp_server_count: 0,
        features: test_features(),
        thinking: None,
        agent_count: 1,
        auth_enabled: true,
        adapters: vec![],
        coding_backend: None,
        web_search: None,
        research_service: None,
        stream_registry: orka_core::StreamRegistry::new(),
    })
}

/// Build the server router with API-key authentication enabled.
///
/// Protected routes require `X-Api-Key: <key>`.
pub(crate) fn test_router_with_auth(key: &str) -> axum::Router {
    use orka_auth::{
        ApiKeyAuthenticator, ApiKeyEntry, AuthLayer, middleware::AuthMiddlewareConfig,
    };
    use sha2::{Digest, Sha256};

    let mut skills = SkillRegistry::new();
    skills.register(Arc::new(EchoSkill));

    let key_hash = format!("{:x}", Sha256::digest(key.as_bytes()));
    let entries = vec![ApiKeyEntry::new("test-key", key_hash, vec![])];
    let auth_cfg = Arc::new(AuthMiddlewareConfig::default());
    let authenticator = Arc::new(ApiKeyAuthenticator::new(&entries));
    let auth_layer = Some(AuthLayer::new(authenticator, auth_cfg));

    let q = Arc::new(InMemoryQueue::new());
    build_router(RouterParams {
        queue: q.clone(),
        dlq: q,
        skills: Arc::new(skills),
        soft_skills: None,
        sessions: Arc::new(InMemorySessionStore::new()),
        scheduler_store: None,
        checkpoint_store: None,
        workspace_registry: test_workspace_registry(),
        graph: test_graph(),
        experience_service: None,
        start_time: std::time::Instant::now(),
        concurrency: 1,
        redis_url: "redis://127.0.0.1:6379".to_string(),
        qdrant_url: None,
        auth_layer,
        a2a_state: None,
        a2a_auth_enabled: false,
        agent_directory: Arc::new(AgentDirectory::new()),
        metrics_handle: None,
        agent_name: "Test Agent".to_string(),
        agent_model: "claude-sonnet-4-6".to_string(),
        mcp_server_count: 0,
        features: test_features(),
        thinking: None,
        agent_count: 1,
        auth_enabled: true,
        adapters: vec![],
        coding_backend: None,
        web_search: None,
        research_service: None,
        stream_registry: orka_core::StreamRegistry::new(),
    })
}

/// Build the server router with the research service enabled.
///
/// Uses in-memory stub skills so the full research pipeline can run
/// without external tools.
pub(crate) fn test_router_with_research() -> axum::Router {
    use orka_core::testing::InMemorySecretManager;
    use orka_research::{
        InMemoryResearchStore, ResearchConfig, create_research_service, create_research_skills,
    };

    let store = Arc::new(InMemoryResearchStore::new());
    let service = create_research_service(
        store,
        None,
        None,
        ResearchConfig::default(),
        Arc::new(InMemorySecretManager::new()),
        None,
    );

    let mut skills = SkillRegistry::new();
    skills.register(Arc::new(EchoSkill));
    register_research_test_skills(&mut skills);
    for skill in create_research_skills(service.clone()) {
        skills.register(skill);
    }
    let skills = Arc::new(skills);
    service.bind_registry(skills.clone());

    let stream_registry = orka_core::StreamRegistry::new();
    service.bind_stream_registry(stream_registry.clone());

    let q = Arc::new(InMemoryQueue::new());
    build_router(RouterParams {
        queue: q.clone(),
        dlq: q,
        skills,
        soft_skills: None,
        sessions: Arc::new(InMemorySessionStore::new()),
        scheduler_store: None,
        checkpoint_store: None,
        workspace_registry: test_workspace_registry(),
        graph: test_graph(),
        experience_service: None,
        start_time: std::time::Instant::now(),
        concurrency: 1,
        redis_url: "redis://127.0.0.1:6379".to_string(),
        qdrant_url: None,
        auth_layer: None,
        a2a_state: None,
        a2a_auth_enabled: false,
        agent_directory: Arc::new(AgentDirectory::new()),
        metrics_handle: None,
        agent_name: "Test Agent".to_string(),
        agent_model: "claude-sonnet-4-6".to_string(),
        mcp_server_count: 0,
        features: ServerFeatures {
            knowledge: false,
            scheduler: false,
            experience: false,
            guardrails: false,
            a2a: false,
            observe: false,
            research: true,
        },
        thinking: None,
        agent_count: 1,
        auth_enabled: false,
        adapters: vec![],
        coding_backend: None,
        web_search: None,
        research_service: Some(service),
        stream_registry,
    })
}
