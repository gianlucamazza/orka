//! Shared test helpers for orka-server integration tests.
//!
//! Provides [`test_router`] and [`test_router_with_auth`] which build the
//! full production router backed by in-memory test doubles.

#![allow(dead_code)]

use std::sync::Arc;

use orka_agent::{Agent, AgentGraph, AgentId, GraphNode, NodeKind, TerminationPolicy};
use orka_core::testing::{InMemoryQueue, InMemorySessionStore};
use orka_server::router::{RouterParams, build_router};
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

    build_router(RouterParams {
        queue: Arc::new(InMemoryQueue::new()),
        skills: Arc::new(skills),
        soft_skills: None,
        sessions: Arc::new(InMemorySessionStore::new()),
        scheduler_store: None,
        workspace_registry: test_workspace_registry(),
        graph: test_graph(),
        experience_service: None,
        start_time: std::time::Instant::now(),
        concurrency: 1,
        redis_url: "redis://127.0.0.1:6379".to_string(),
        qdrant_url: None,
        auth_layer: None,
        a2a_state: None,
        metrics_handle: None,
    })
}

/// Build the server router with API-key authentication enabled.
///
/// Protected routes require `X-Api-Key: <key>`.
pub fn test_router_with_auth(key: &str) -> axum::Router {
    use orka_auth::{ApiKeyAuthenticator, AuthLayer};
    use orka_core::config::{ApiKeyEntry, AuthConfig};
    use sha2::{Digest, Sha256};

    let mut skills = SkillRegistry::new();
    skills.register(Arc::new(EchoSkill));

    let key_hash = format!("{:x}", Sha256::digest(key.as_bytes()));
    let entries = vec![ApiKeyEntry {
        name: "test-key".into(),
        key_hash,
        scopes: vec![],
    }];
    let auth_cfg = Arc::new(AuthConfig {
        enabled: true,
        api_key_header: "X-Api-Key".into(),
        api_keys: entries.clone(),
        jwt: None,
    });
    let authenticator = Arc::new(ApiKeyAuthenticator::new(&entries));
    let auth_layer = Some(AuthLayer::new(authenticator, auth_cfg));

    build_router(RouterParams {
        queue: Arc::new(InMemoryQueue::new()),
        skills: Arc::new(skills),
        soft_skills: None,
        sessions: Arc::new(InMemorySessionStore::new()),
        scheduler_store: None,
        workspace_registry: test_workspace_registry(),
        graph: test_graph(),
        experience_service: None,
        start_time: std::time::Instant::now(),
        concurrency: 1,
        redis_url: "redis://127.0.0.1:6379".to_string(),
        qdrant_url: None,
        auth_layer,
        a2a_state: None,
        metrics_handle: None,
    })
}
