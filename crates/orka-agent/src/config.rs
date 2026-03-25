//! Translates `OrkaConfig` into `Agent` and `AgentGraph` objects.

use orka_core::config::{AgentDef, NodeKindDef, OrkaConfig};
use orka_llm::{ThinkingConfig, ThinkingEffort};
use orka_workspace::WorkspaceRegistry;

use crate::{
    agent::{Agent, AgentId, AgentLlmConfig, SystemPrompt, ToolScope},
    context::SlotKey,
    graph::{AgentGraph, Edge, EdgeCondition, GraphNode, NodeKind},
};

/// Build an `AgentGraph` from the `[[agents]]` + `[graph]` config sections.
///
/// Requires `OrkaConfig::validate()` to have run first, which ensures:
/// - `config.agents` is non-empty (default single-agent entry applied if
///   needed)
/// - `config.graph` is `Some` (auto-created for single-agent, required for
///   multi-agent)
/// - Legacy `[agent]` has been promoted by the v4→v5 TOML migration
pub async fn build_graph_from_config(
    config: &OrkaConfig,
    workspace_registry: &WorkspaceRegistry,
) -> orka_core::Result<AgentGraph> {
    let graph_def = config.graph.as_ref().ok_or_else(|| {
        orka_core::Error::Config(
            "[[agents]] is set but [graph] is missing — call OrkaConfig::validate() first".into(),
        )
    })?;

    // Entry point: explicit `graph.entry` overrides first agent in array
    let entry_id = graph_def
        .entry
        .as_deref()
        .map(AgentId::from)
        .or_else(|| config.agents.first().map(|a| AgentId::from(a.id.as_str())))
        .unwrap_or_else(|| AgentId::from("default"));

    // Build graph with max_hops from config
    let mut graph = AgentGraph::new("multi-agent", entry_id.clone());

    // Build nodes from agent definitions
    for agent_def in &config.agents {
        let agent = build_agent_from_def(agent_def, workspace_registry).await;
        let kind = match agent_def.kind {
            NodeKindDef::Agent => NodeKind::Agent,
            NodeKindDef::Router => NodeKind::Router,
            NodeKindDef::FanOut => NodeKind::FanOut,
            NodeKindDef::FanIn => NodeKind::FanIn,
        };
        graph.add_node(GraphNode { agent, kind });
    }

    // Build edges from graph definition
    for edge_def in &graph_def.edges {
        let from = AgentId::from(edge_def.from.as_str());
        let target = AgentId::from(edge_def.to.as_str());

        // Condition is now a simple String - parse it
        let condition = edge_def.condition.as_ref().map(|c| {
            if c == "always" {
                EdgeCondition::Always
            } else if c.starts_with("output_contains:") {
                let pattern = c.strip_prefix("output_contains:").unwrap_or(c).to_string();
                EdgeCondition::OutputContains(pattern)
            } else if c.starts_with("state_match:") {
                // Format: "state_match:key=value"
                let parts: Vec<&str> = c
                    .strip_prefix("state_match:")
                    .unwrap_or(c)
                    .split('=')
                    .collect();
                if parts.len() == 2 {
                    EdgeCondition::StateMatch {
                        key: SlotKey::shared(parts[0].to_string()),
                        pattern: serde_json::Value::String(parts[1].to_string()),
                    }
                } else {
                    EdgeCondition::Always
                }
            } else {
                EdgeCondition::Always
            }
        });

        graph.add_edge(
            from,
            Edge {
                target,
                condition,
                priority: edge_def.weight as u32, // weight maps to priority
            },
        );
    }

    // Derive handoff_targets for Agent-kind nodes from their outgoing edges.
    // Router / FanOut / FanIn nodes use structural routing and must not receive
    // handoff tools.
    let agent_ids: Vec<AgentId> = config
        .agents
        .iter()
        .filter(|a| a.kind == NodeKindDef::Agent)
        .map(|a| AgentId::from(a.id.as_str()))
        .collect();

    for agent_id in agent_ids {
        let targets: Vec<AgentId> = graph
            .outgoing_edges(&agent_id)
            .into_iter()
            .map(|e| e.target.clone())
            .collect();
        if !targets.is_empty()
            && let Some(node) = graph.get_node_mut(&agent_id)
        {
            node.agent.handoff_targets = targets;
        }
    }

    Ok(graph)
}

async fn build_agent_from_def(def: &AgentDef, workspace_registry: &WorkspaceRegistry) -> Agent {
    let agent_id = AgentId::from(def.id.as_str());
    let cfg = &def.config;

    let mut agent = Agent::new(agent_id.clone(), &cfg.name);

    // Load system prompt from the named workspace, falling back to the default
    // workspace (covers agents promoted from the legacy [agent] single-entry).
    let mut system_prompt = SystemPrompt::default();
    let ws_name = def.id.as_str();
    let state_lock = workspace_registry
        .state(ws_name)
        .unwrap_or_else(|| workspace_registry.default_state());
    {
        let state = state_lock.read().await;
        if let Some(soul) = &state.soul {
            system_prompt.persona = soul.body.clone();
        }
        if let Some(tools_body) = &state.tools_body {
            system_prompt.tool_instructions = tools_body.clone();
        }
    }

    agent.system_prompt = system_prompt;
    agent.max_iterations = cfg.max_iterations;

    let thinking = cfg.thinking.as_deref().map(|effort| {
        let level = match effort.to_lowercase().as_str() {
            "low" => ThinkingEffort::Low,
            "medium" => ThinkingEffort::Medium,
            "high" => ThinkingEffort::High,
            "max" => ThinkingEffort::Max,
            other => {
                tracing::warn!(
                    agent = %def.id,
                    value = %other,
                    "unknown thinking effort value, falling back to medium"
                );
                ThinkingEffort::Medium
            }
        };
        ThinkingConfig::Adaptive { effort: level }
    });

    // Use simplified LLM config
    agent.llm_config = AgentLlmConfig {
        model: Some(cfg.model.clone()),
        max_tokens: Some(cfg.max_tokens),
        temperature: Some(cfg.temperature),
        thinking,
        ..Default::default()
    };

    agent.history_filter = cfg.history_filter;
    agent.history_filter_n = cfg.history_filter_n;

    // Use allowed/denied tools from config
    agent.tools = if cfg.allowed_tools.is_empty() && cfg.denied_tools.is_empty() {
        ToolScope::All
    } else if !cfg.allowed_tools.is_empty() {
        ToolScope::Allow(cfg.allowed_tools.iter().cloned().collect())
    } else {
        ToolScope::Deny(cfg.denied_tools.iter().cloned().collect())
    };

    agent
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use orka_core::config::{AgentDef, EdgeDef, GraphDef, OrkaConfig, ServerConfig};
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
            soft_skills: Default::default(),
            session: Default::default(),
            queue: Default::default(),
            llm: Default::default(),
            tools: Default::default(),
            observe: Default::default(),
            audit: Default::default(),
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
            prompts: Default::default(),
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
    async fn single_agent_config_builds_one_node_graph() {
        let mut config = base_config();
        config.agents = vec![agent_def("orka")];
        config.graph = Some(GraphDef::default());
        let registry = make_registry();
        let graph = super::build_graph_from_config(&config, &registry)
            .await
            .expect("build_graph_from_config failed");

        let entry = &graph.entry;
        assert!(graph.get_node(entry).is_some());
        assert!(graph.outgoing_edges(entry).is_empty());
    }

    #[tokio::test]
    async fn multi_agent_config_builds_correct_topology() {
        let mut config = base_config();
        config.agents = vec![agent_def("router"), agent_def("worker")];
        let mut edge = EdgeDef::new("router", "worker");
        edge.condition = Some("always".to_string());
        let mut graph = GraphDef::default();
        graph.max_hops = 20;
        graph.edges = vec![edge];
        config.graph = Some(graph);

        let registry = make_registry();
        let graph = super::build_graph_from_config(&config, &registry)
            .await
            .expect("build_graph_from_config failed");

        // First agent is entry point
        let router = crate::agent::AgentId::new("router");
        let worker = crate::agent::AgentId::new("worker");
        assert!(graph.get_node(&router).is_some());
        assert!(graph.get_node(&worker).is_some());

        let edges = graph.outgoing_edges(&router);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].target, worker);
    }

    #[tokio::test]
    async fn node_kind_router_maps_correctly() {
        use orka_core::config::NodeKindDef;
        use crate::graph::NodeKind;

        let mut config = base_config();
        let mut router = agent_def("router");
        router.kind = NodeKindDef::Router;
        let worker = agent_def("worker");
        let mut graph_def = GraphDef::default();
        graph_def.edges = vec![EdgeDef::new("router", "worker")];
        config.agents = vec![router, worker];
        config.graph = Some(graph_def);

        let registry = make_registry();
        let graph = super::build_graph_from_config(&config, &registry)
            .await
            .expect("build failed");

        let router_id = crate::agent::AgentId::new("router");
        let node = graph.get_node(&router_id).expect("router node missing");
        assert!(matches!(node.kind, NodeKind::Router));
        // Router nodes must not get handoff targets
        assert!(node.agent.handoff_targets.is_empty());
    }

    #[tokio::test]
    async fn entry_from_config_overrides_first_agent() {
        let mut config = base_config();
        config.agents = vec![agent_def("a"), agent_def("b")];
        let mut graph_def = GraphDef::default();
        graph_def.entry = Some("b".to_string());
        graph_def.edges = vec![EdgeDef::new("b", "a")];
        config.graph = Some(graph_def);

        let registry = make_registry();
        let graph = super::build_graph_from_config(&config, &registry)
            .await
            .expect("build failed");

        assert_eq!(graph.entry, crate::agent::AgentId::new("b"));
    }

    #[tokio::test]
    async fn handoff_targets_derived_for_agent_nodes() {
        let mut config = base_config();
        config.agents = vec![agent_def("src"), agent_def("dst")];
        let mut graph_def = GraphDef::default();
        graph_def.edges = vec![EdgeDef::new("src", "dst")];
        config.graph = Some(graph_def);

        let registry = make_registry();
        let graph = super::build_graph_from_config(&config, &registry)
            .await
            .expect("build failed");

        let src_id = crate::agent::AgentId::new("src");
        let node = graph.get_node(&src_id).expect("src missing");
        assert_eq!(node.agent.handoff_targets, vec![crate::agent::AgentId::new("dst")]);
    }

    #[tokio::test]
    async fn node_kind_fan_out_maps_correctly() {
        use orka_core::config::NodeKindDef;
        use crate::graph::NodeKind;

        let mut config = base_config();
        let mut fanout = agent_def("fanout");
        fanout.kind = NodeKindDef::FanOut;
        let mut graph_def = GraphDef::default();
        graph_def.edges = vec![
            EdgeDef::new("fanout", "worker_a"),
            EdgeDef::new("fanout", "worker_b"),
        ];
        config.agents = vec![fanout, agent_def("worker_a"), agent_def("worker_b")];
        config.graph = Some(graph_def);

        let registry = make_registry();
        let graph = super::build_graph_from_config(&config, &registry)
            .await
            .expect("build failed");

        let fanout_id = crate::agent::AgentId::new("fanout");
        let node = graph.get_node(&fanout_id).expect("fanout missing");
        assert!(matches!(node.kind, NodeKind::FanOut));
        // FanOut nodes must not get handoff targets
        assert!(node.agent.handoff_targets.is_empty());
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

    #[tokio::test]
    async fn thinking_effort_high_builds_adaptive_config() {
        let mut def = agent_def("thinker");
        def.config.thinking = Some("high".to_string());
        let registry = make_registry();
        let agent = super::build_agent_from_def(&def, &registry).await;
        assert!(matches!(
            agent.llm_config.thinking,
            Some(orka_llm::ThinkingConfig::Adaptive {
                effort: orka_llm::ThinkingEffort::High
            })
        ));
    }

    #[tokio::test]
    async fn thinking_effort_max_builds_adaptive_config() {
        let mut def = agent_def("thinker");
        def.config.thinking = Some("max".to_string());
        let registry = make_registry();
        let agent = super::build_agent_from_def(&def, &registry).await;
        assert!(matches!(
            agent.llm_config.thinking,
            Some(orka_llm::ThinkingConfig::Adaptive {
                effort: orka_llm::ThinkingEffort::Max
            })
        ));
    }

    #[tokio::test]
    async fn thinking_absent_leaves_thinking_none() {
        let def = agent_def("plain");
        let registry = make_registry();
        let agent = super::build_agent_from_def(&def, &registry).await;
        assert!(agent.llm_config.thinking.is_none());
    }
}
