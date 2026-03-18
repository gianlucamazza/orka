//! Translates `OrkaConfig` into `Agent` and `AgentGraph` objects.

use std::collections::HashSet;

use orka_core::config::{AgentDef, OrkaConfig, ToolScopeDef};
use orka_llm::client::{ReasoningEffort, ThinkingConfig};
use orka_workspace::WorkspaceRegistry;

use crate::agent::{Agent, AgentId, AgentLlmConfig, SystemPrompt, ToolScope};
use crate::context::SlotKey;
use crate::graph::{AgentGraph, Edge, EdgeCondition, GraphNode, NodeKind, TerminationPolicy};

/// Build an `AgentGraph` from the `[[agents]]` + `[graph]` config sections.
pub async fn build_graph_from_config(
    config: &OrkaConfig,
    workspace_registry: &WorkspaceRegistry,
) -> anyhow::Result<AgentGraph> {
    if config.agents.is_empty() {
        return build_single_agent_graph(config, workspace_registry).await;
    }

    let graph_def = config.graph.as_ref().ok_or_else(|| {
        anyhow::anyhow!("[[agents]] is set but [graph] is missing — add [graph] section to config")
    })?;

    let graph_id = graph_def.id.clone().unwrap_or_else(|| "default".into());
    let entry_id = AgentId::from(graph_def.entry.as_str());

    let terminal_agents: HashSet<AgentId> = graph_def
        .terminal
        .iter()
        .map(|s| AgentId::from(s.as_str()))
        .collect();

    let mut policy = TerminationPolicy {
        terminal_agents,
        ..TerminationPolicy::default()
    };
    if let Some(max_iter) = graph_def.max_total_iterations {
        policy.max_total_iterations = max_iter;
    }
    if let Some(max_tokens) = graph_def.max_total_tokens {
        policy.max_total_tokens = Some(max_tokens);
    }
    if let Some(max_secs) = graph_def.max_duration_secs {
        policy.max_duration = std::time::Duration::from_secs(max_secs);
    }

    let mut graph = AgentGraph::new(graph_id, entry_id).with_termination(policy);

    // Build nodes
    for agent_def in &config.agents {
        let agent = build_agent_from_def(agent_def, workspace_registry).await;
        graph.add_node(GraphNode {
            agent,
            kind: NodeKind::Agent,
        });
    }

    // Build edges
    for edge_def in &graph_def.edges {
        let from = AgentId::from(edge_def.from.as_str());
        let target = AgentId::from(edge_def.to.as_str());

        let condition = edge_def.condition.as_ref().map(|c| match c {
            orka_core::config::EdgeConditionDef::Always => EdgeCondition::Always,
            orka_core::config::EdgeConditionDef::OutputContains { pattern } => {
                EdgeCondition::OutputContains(pattern.clone())
            }
            orka_core::config::EdgeConditionDef::StateMatch { key, value } => {
                EdgeCondition::StateMatch {
                    key: SlotKey::shared(key.clone()),
                    pattern: value.clone(),
                }
            }
        });

        graph.add_edge(
            from,
            Edge {
                target,
                condition,
                priority: edge_def.priority.unwrap_or(0),
            },
        );
    }

    Ok(graph)
}

/// Build a single-node graph from the legacy `[agent]` config section.
/// This ensures backward compatibility — single-agent deployments still work.
pub async fn build_single_agent_graph(
    config: &OrkaConfig,
    workspace_registry: &WorkspaceRegistry,
) -> anyhow::Result<AgentGraph> {
    let agent_cfg = &config.agent;
    let agent_id = AgentId::from(agent_cfg.id.as_str());

    let mut system_prompt = SystemPrompt::default();

    // Load from workspace registry
    let state_lock = workspace_registry.default_state();
    let state = state_lock.read().await;
    if let Some(soul) = &state.soul {
        system_prompt.persona = soul.body.clone();
    }
    if let Some(tools_body) = &state.tools_body {
        system_prompt.tool_instructions = tools_body.clone();
    }
    drop(state);

    let mut agent = Agent::new(agent_id.clone(), &agent_cfg.display_name);
    agent.system_prompt = system_prompt;
    agent.max_iterations = agent_cfg.max_iterations;
    agent.skill_timeout_secs = agent_cfg.skill_timeout_secs;
    agent.llm_config = AgentLlmConfig {
        model: agent_cfg.model.clone(),
        max_tokens: agent_cfg.max_tokens,
        context_window: agent_cfg.context_window_tokens,
        temperature: agent_cfg.temperature,
        thinking: validated_thinking_config(
            build_thinking_config(
                agent_cfg.thinking_budget_tokens,
                agent_cfg.reasoning_effort.as_deref(),
            ),
            agent_cfg.max_tokens,
        ),
    };

    let policy = TerminationPolicy {
        terminal_agents: std::iter::once(agent_id.clone()).collect(),
        ..TerminationPolicy::default()
    };

    let mut graph = AgentGraph::new("single", agent_id).with_termination(policy);
    graph.add_node(GraphNode {
        agent,
        kind: NodeKind::Agent,
    });

    Ok(graph)
}

async fn build_agent_from_def(def: &AgentDef, workspace_registry: &WorkspaceRegistry) -> Agent {
    let agent_id = AgentId::from(def.id.as_str());
    let mut agent = Agent::new(agent_id.clone(), &def.display_name);

    // Load system prompt from soul_file or inline soul
    let mut system_prompt = SystemPrompt::default();

    if let Some(soul_content) = &def.soul {
        system_prompt.persona = soul_content.clone();
    } else if let Some(soul_file) = &def.soul_file {
        match tokio::fs::read_to_string(soul_file).await {
            Ok(content) => {
                system_prompt.persona = orka_workspace::strip_frontmatter(&content);
            }
            Err(e) => {
                tracing::warn!(file = %soul_file, %e, "failed to load soul file");
            }
        }
    } else {
        // Fall back to workspace registry
        let ws_name = def.id.as_str();
        if let Some(state_lock) = workspace_registry.state(ws_name) {
            let state = state_lock.read().await;
            if let Some(soul) = &state.soul {
                system_prompt.persona = soul.body.clone();
            }
            if let Some(tools_body) = &state.tools_body {
                system_prompt.tool_instructions = tools_body.clone();
            }
        }
    }

    if let Some(tools_file) = &def.tools_file {
        match tokio::fs::read_to_string(tools_file).await {
            Ok(content) => {
                system_prompt.tool_instructions = orka_workspace::strip_frontmatter(&content);
            }
            Err(e) => {
                tracing::warn!(file = %tools_file, %e, "failed to load tools file");
            }
        }
    }

    agent.system_prompt = system_prompt;
    agent.max_iterations = def.max_iterations.unwrap_or(15);
    agent.llm_config = AgentLlmConfig {
        model: def.model.clone(),
        max_tokens: def.max_tokens,
        context_window: def.context_window,
        temperature: def.temperature,
        thinking: validated_thinking_config(
            build_thinking_config(def.thinking_budget_tokens, def.reasoning_effort.as_deref()),
            def.max_tokens,
        ),
    };

    agent.handoff_targets = def
        .handoff_targets
        .iter()
        .map(|s| AgentId::from(s.as_str()))
        .collect();

    agent.tools = match &def.tools {
        None => ToolScope::All,
        Some(ToolScopeDef::Allow { allow }) => ToolScope::Allow(allow.iter().cloned().collect()),
        Some(ToolScopeDef::Deny { deny }) => ToolScope::Deny(deny.iter().cloned().collect()),
    };

    agent
}

/// Validate that `budget_tokens < max_tokens` when both are set.
///
/// If the constraint is violated, emits a warning and returns `None` to
/// disable thinking rather than sending an invalid request to the API.
fn validated_thinking_config(
    thinking: Option<ThinkingConfig>,
    max_tokens: Option<u32>,
) -> Option<ThinkingConfig> {
    if let (Some(ThinkingConfig::Enabled { budget_tokens }), Some(max)) = (&thinking, max_tokens) {
        if *budget_tokens >= max {
            tracing::warn!(
                budget_tokens,
                max_tokens = max,
                "thinking_budget_tokens must be less than max_tokens; disabling thinking"
            );
            return None;
        }
    }
    thinking
}

/// Convert the config thinking fields into a `ThinkingConfig`.
///
/// `thinking_budget_tokens` takes precedence over `reasoning_effort` if both are set.
fn build_thinking_config(
    budget_tokens: Option<u32>,
    reasoning_effort: Option<&str>,
) -> Option<ThinkingConfig> {
    if let Some(budget) = budget_tokens {
        if reasoning_effort.is_some() {
            tracing::warn!(
                "both thinking_budget_tokens and reasoning_effort are set; using thinking_budget_tokens"
            );
        }
        return Some(ThinkingConfig::Enabled {
            budget_tokens: budget,
        });
    }
    match reasoning_effort {
        Some("low") => Some(ThinkingConfig::ReasoningEffort(ReasoningEffort::Low)),
        Some("medium") => Some(ThinkingConfig::ReasoningEffort(ReasoningEffort::Medium)),
        Some("high") => Some(ThinkingConfig::ReasoningEffort(ReasoningEffort::High)),
        Some(other) => {
            tracing::warn!(
                reasoning_effort = other,
                "unknown reasoning_effort value; expected \"low\", \"medium\", or \"high\""
            );
            None
        }
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use orka_core::config::{AgentConfig, AgentDef, EdgeDef, GraphDef, OrkaConfig, ServerConfig};
    use orka_workspace::{WorkspaceLoader, WorkspaceRegistry};

    fn base_config() -> OrkaConfig {
        OrkaConfig {
            config_version: 1,
            server: ServerConfig::default(),
            bus: Default::default(),
            redis: Default::default(),
            logging: Default::default(),
            workspace_dir: ".".into(),
            workspaces: vec![],
            default_workspace: None,
            adapters: Default::default(),
            worker: Default::default(),
            memory: Default::default(),
            secrets: Default::default(),
            auth: Default::default(),
            sandbox: Default::default(),
            plugins: Default::default(),
            session: Default::default(),
            queue: Default::default(),
            llm: Default::default(),
            agent: AgentConfig::default(),
            tools: Default::default(),
            observe: Default::default(),
            gateway: Default::default(),
            mcp: Default::default(),
            guardrails: Default::default(),
            web: Default::default(),
            os: Default::default(),
            a2a: Default::default(),
            knowledge: Default::default(),
            scheduler: Default::default(),
            http: Default::default(),
            experience: Default::default(),
            agents: vec![],
            graph: None,
        }
    }

    fn make_registry() -> WorkspaceRegistry {
        let mut reg = WorkspaceRegistry::new("default".into());
        reg.register("default".into(), Arc::new(WorkspaceLoader::new(".")));
        reg
    }

    fn agent_def(id: &str) -> AgentDef {
        AgentDef::new(id)
    }

    #[tokio::test]
    async fn single_agent_fallback_builds_one_node_graph() {
        let config = base_config();
        let registry = make_registry();
        let graph = super::build_graph_from_config(&config, &registry)
            .await
            .expect("build_graph_from_config failed");

        assert_eq!(graph.id, "single");
        let entry = &graph.entry;
        assert!(graph.get_node(entry).is_some());
        assert!(graph.outgoing_edges(entry).is_empty());
    }

    #[tokio::test]
    async fn multi_agent_config_builds_correct_topology() {
        let mut config = base_config();
        config.agents = vec![agent_def("router"), agent_def("worker")];
        let mut edge = EdgeDef::new("router", "worker");
        edge.priority = Some(1);
        let mut graph = GraphDef::new("router");
        graph.id = Some("main".into());
        graph.terminal = vec!["worker".into()];
        graph.max_total_iterations = Some(20);
        graph.edges = vec![edge];
        config.graph = Some(graph);

        let registry = make_registry();
        let graph = super::build_graph_from_config(&config, &registry)
            .await
            .expect("build_graph_from_config failed");

        assert_eq!(graph.id, "main");
        assert_eq!(graph.entry.0.as_ref(), "router");

        let router = crate::agent::AgentId::new("router");
        let worker = crate::agent::AgentId::new("worker");
        assert!(graph.get_node(&router).is_some());
        assert!(graph.get_node(&worker).is_some());

        let edges = graph.outgoing_edges(&router);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].target, worker);

        assert_eq!(graph.termination.max_total_iterations, 20);
        assert!(graph.termination.terminal_agents.contains(&worker));
    }

    #[tokio::test]
    async fn agents_without_graph_section_returns_error() {
        let mut config = base_config();
        config.agents = vec![agent_def("solo")];
        // graph is None — must error
        let registry = make_registry();
        let result = super::build_graph_from_config(&config, &registry).await;
        assert!(result.is_err());
    }
}
