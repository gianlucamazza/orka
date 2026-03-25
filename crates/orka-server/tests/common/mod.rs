//! Shared test helpers for orka-server integration tests.
//!
//! Provides [`test_router`] and [`test_router_with_auth`] which build the
//! full production router backed by in-memory test doubles.

#![allow(dead_code, missing_docs)]

use std::sync::Arc;

use orka_a2a::AgentDirectory;
use orka_agent::{Agent, AgentGraph, AgentId, GraphNode, NodeKind, TerminationPolicy};
use orka_core::testing::{InMemoryQueue, InMemorySessionStore};
use orka_server::router::{RouterParams, ServerFeatures, build_router};
use orka_skills::{EchoSkill, SkillRegistry};
use orka_workspace::{WorkspaceLoader, WorkspaceRegistry};

/// Build a minimal agent graph with a single "test" node for use in tests.
fn test_graph() -> Arc<AgentGraph> {
    let agent_id = AgentId::from("test");
    let agent = Agent::new(agent_id.clone(), "Test Agent");
    let policy = TerminationPolicy {
        terminal_agents: std::iter::once(agent_id.clone()).collect(),
        ..TerminationPolicy::default()
    };
    let mut graph = AgentGraph::new("test-graph", agent_id.clone()).with_termination(policy);
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
    }
}

/// Build the full server router backed by in-memory test doubles.
///
/// - No auth (all routes accessible without API key)
/// - No A2A
/// - No scheduler
/// - No experience service
/// - One registered skill: `EchoSkill`
pub fn test_router() -> axum::Router {
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
    })
}

/// Build the server router with A2A enabled and optional auth protection on
/// `POST /a2a`.
///
/// When `auth_enabled` is `true`:
/// - `GET /.well-known/agent.json` remains public.
/// - `POST /a2a` requires `X-Api-Key: <key>`.
pub fn test_router_with_a2a(key: &str, a2a_auth_enabled: bool) -> axum::Router {
    use orka_a2a::{
        A2aState, InMemoryPushNotificationStore, InMemoryTaskStore, WebhookDeliverer,
        build_agent_card,
    };
    use orka_auth::{ApiKeyAuthenticator, AuthLayer, middleware::AuthMiddlewareConfig};
    use orka_core::{config::ApiKeyEntry, testing::InMemorySecretManager};
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
        task_events: Default::default(),
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
    })
}

/// Build the server router with API-key authentication enabled.
///
/// Protected routes require `X-Api-Key: <key>`.
pub fn test_router_with_auth(key: &str) -> axum::Router {
    use orka_auth::{ApiKeyAuthenticator, AuthLayer, middleware::AuthMiddlewareConfig};
    use orka_core::config::ApiKeyEntry;
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
    })
}
