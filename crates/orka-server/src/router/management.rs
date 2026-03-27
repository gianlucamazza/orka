//! Management API endpoints: skills, workspaces, sessions, graph, experience,
//! eval.

use std::{collections::HashMap, sync::Arc};

use axum::{
    Json,
    extract::{Path, Query},
    http::StatusCode,
    response::IntoResponse,
};
use orka_agent::AgentGraph;
use orka_core::traits::SessionStore;
use orka_experience::ExperienceService;
use orka_skills::{SkillRegistry, SoftSkillRegistry};
use orka_workspace::WorkspaceRegistry;

#[allow(clippy::too_many_lines)]
pub(super) fn routes(
    skills: Arc<SkillRegistry>,
    soft_skills: Option<Arc<SoftSkillRegistry>>,
    sessions: Arc<dyn SessionStore>,
    workspace_registry: Arc<WorkspaceRegistry>,
    graph: Arc<AgentGraph>,
    experience_service: Option<Arc<ExperienceService>>,
) -> axum::Router {
    let s1 = skills.clone();
    let s2 = skills.clone();
    let s3 = skills;
    let soft1 = soft_skills;
    let w1 = workspace_registry.clone();
    let w2 = workspace_registry;
    let g1 = graph;
    let e1 = experience_service.clone();
    let e2 = experience_service.clone();
    let e3 = experience_service;
    let ss1 = sessions.clone();
    let ss2 = sessions.clone();
    let ss3 = sessions;

    axum::Router::new()
        // Skills
        .route(
            "/api/v1/skills",
            axum::routing::get(move || {
                let skills = s1.clone();
                async move {
                    let list: Vec<serde_json::Value> = skills
                        .list_info()
                        .iter()
                        .map(|(name, skill, state)| {
                            let status = match state {
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
                        })
                        .collect();
                    axum::Json(list)
                }
            }),
        )
        .route(
            "/api/v1/soft-skills",
            axum::routing::get(move || {
                let reg = soft1.clone();
                async move {
                    let list: Vec<serde_json::Value> = reg
                        .as_deref()
                        .map(orka_skills::SoftSkillRegistry::summaries)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|s| {
                            serde_json::json!({
                                "name": s.name,
                                "description": s.description,
                                "tags": s.tags,
                            })
                        })
                        .collect();
                    axum::Json(list)
                }
            }),
        )
        .route(
            "/api/v1/skills/{name}",
            axum::routing::get(move |Path(name): Path<String>| {
                let skills = s2.clone();
                async move {
                    match skills.get(&name) {
                        Some(skill) => axum::Json(serde_json::json!({
                            "name": skill.name(),
                            "description": skill.description(),
                            "schema": skill.schema(),
                        }))
                        .into_response(),
                        None => (StatusCode::NOT_FOUND, format!("skill '{name}' not found"))
                            .into_response(),
                    }
                }
            }),
        )
        // Eval
        .route(
            "/api/v1/eval",
            axum::routing::post(move |Json(body): Json<serde_json::Value>| {
                let skills = s3.clone();
                async move {
                    let skill_filter = body["skill"].as_str().map(String::from);
                    let dir = body["dir"].as_str().unwrap_or("evals").to_string();
                    let runner = orka_eval::EvalRunner::new(skills);
                    match runner
                        .run_dir(std::path::Path::new(&dir), skill_filter.as_deref())
                        .await
                    {
                        Ok(report) => {
                            let json_str = report.to_json();
                            let val: serde_json::Value =
                                serde_json::from_str(&json_str).unwrap_or_default();
                            axum::Json(val).into_response()
                        }
                        Err(e) => (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("eval failed: {e}"),
                        )
                            .into_response(),
                    }
                }
            }),
        )
        // Workspaces
        .route(
            "/api/v1/workspaces",
            axum::routing::get(move || {
                let registry = w1.clone();
                async move {
                    let mut list = Vec::new();
                    for name in registry.list_names() {
                        if let Some(loader) = registry.get(name) {
                            let state = loader.state();
                            let state = state.read().await;
                            let (agent_name, description) = state
                                .soul
                                .as_ref()
                                .map_or((None, None), |d| {
                                    (
                                        d.frontmatter.name.clone(),
                                        d.frontmatter.description.clone(),
                                    )
                                });
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
            }),
        )
        .route(
            "/api/v1/workspaces/{name}",
            axum::routing::get(move |Path(ws_name): Path<String>| {
                let registry = w2.clone();
                async move {
                    match registry.get(&ws_name) {
                        None => (
                            StatusCode::NOT_FOUND,
                            format!("workspace '{ws_name}' not found"),
                        )
                            .into_response(),
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
                            }))
                            .into_response()
                        }
                    }
                }
            }),
        )
        // Agent graph
        .route(
            "/api/v1/graph",
            axum::routing::get(move || {
                let g = g1.clone();
                async move {
                    let nodes: Vec<serde_json::Value> = g
                        .nodes_iter()
                        .map(|(id, node)| {
                            serde_json::json!({
                                "id": id.to_string(),
                                "kind": format!("{:?}", node.kind),
                                "agent": {
                                    "id": node.agent.id.to_string(),
                                    "name": node.agent.display_name,
                                    "max_turns": node.agent.max_turns,
                                    "handoff_targets": node.agent.handoff_targets.iter()
                                        .map(std::string::ToString::to_string).collect::<Vec<_>>(),
                                }
                            })
                        })
                        .collect();
                    let edges: Vec<serde_json::Value> = g
                        .edges_iter()
                        .flat_map(|(from, edges)| {
                            let from = from.to_string();
                            edges.iter().map(move |e| {
                                let condition = match &e.condition {
                                    None => serde_json::json!("always"),
                                    Some(orka_agent::EdgeCondition::Always) => {
                                        serde_json::json!("always")
                                    }
                                    Some(orka_agent::EdgeCondition::OutputContains(s)) => {
                                        serde_json::json!({"output_contains": s})
                                    }
                                    Some(orka_agent::EdgeCondition::StateMatch {
                                        key,
                                        pattern,
                                    }) => {
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
                        })
                        .collect();
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
                                .map(std::string::ToString::to_string).collect::<Vec<_>>(),
                        }
                    }))
                }
            }),
        )
        // Experience
        .route(
            "/api/v1/experience/status",
            axum::routing::get(move || {
                let exp = e1.clone();
                async move {
                    axum::Json(serde_json::json!({
                        "enabled": exp.as_ref().is_some_and(|e| e.is_enabled()),
                    }))
                }
            }),
        )
        .route(
            "/api/v1/experience/principles",
            axum::routing::get(move |Query(params): Query<HashMap<String, String>>| {
                let exp = e2.clone();
                async move {
                    let Some(exp) = exp else {
                        return (StatusCode::SERVICE_UNAVAILABLE, "experience not enabled")
                            .into_response();
                    };
                    let workspace = params
                        .get("workspace")
                        .map_or("default", String::as_str);
                    let query = params.get("query").map_or("", String::as_str);
                    let limit: usize = params
                        .get("limit")
                        .and_then(|l| l.parse().ok())
                        .unwrap_or(10);
                    match exp.retrieve_principles(query, workspace).await {
                        Ok(mut principles) => {
                            principles.truncate(limit);
                            axum::Json(principles).into_response()
                        }
                        Err(e) => (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("retrieve failed: {e}"),
                        )
                            .into_response(),
                    }
                }
            }),
        )
        .route(
            "/api/v1/experience/distill",
            axum::routing::post(move |Json(body): Json<serde_json::Value>| {
                let exp = e3.clone();
                async move {
                    let Some(exp) = exp else {
                        return (StatusCode::SERVICE_UNAVAILABLE, "experience not enabled")
                            .into_response();
                    };
                    let workspace = body["workspace"].as_str().unwrap_or("default");
                    match exp.distill(workspace).await {
                        Ok(created) => {
                            axum::Json(serde_json::json!({ "created": created })).into_response()
                        }
                        Err(e) => (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("distill failed: {e}"),
                        )
                            .into_response(),
                    }
                }
            }),
        )
        // Sessions
        .route(
            "/api/v1/sessions",
            axum::routing::get(move |Query(params): Query<HashMap<String, String>>| {
                let sessions = ss1.clone();
                async move {
                    let limit: usize = params
                        .get("limit")
                        .and_then(|l| l.parse().ok())
                        .unwrap_or(20);
                    match sessions.list(limit).await {
                        Ok(list) => axum::Json(list).into_response(),
                        Err(e) => (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("list failed: {e}"),
                        )
                            .into_response(),
                    }
                }
            }),
        )
        .route(
            "/api/v1/sessions/{id}",
            axum::routing::get(move |Path(id): Path<String>| {
                let sessions = ss2.clone();
                async move {
                    match uuid::Uuid::parse_str(&id) {
                        Err(_) => (StatusCode::BAD_REQUEST, "invalid session ID").into_response(),
                        Ok(uuid) => {
                            let sid = orka_core::SessionId::from(uuid);
                            match sessions.get(&sid).await {
                                Ok(Some(s)) => axum::Json(s).into_response(),
                                Ok(None) => {
                                    (StatusCode::NOT_FOUND, "session not found").into_response()
                                }
                                Err(e) => (
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                    format!("get failed: {e}"),
                                )
                                    .into_response(),
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
                            let sid = orka_core::SessionId::from(uuid);
                            match sessions.delete(&sid).await {
                                Ok(()) => {
                                    axum::Json(serde_json::json!({ "deleted": true }))
                                        .into_response()
                                }
                                Err(e) => (
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                    format!("delete failed: {e}"),
                                )
                                    .into_response(),
                            }
                        }
                    }
                }
            }),
        )
}
