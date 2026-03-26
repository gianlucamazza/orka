//! Translates `OrkaConfig` into `Agent` and `AgentGraph` objects.

use orka_core::config::{AgentDef, NodeKindDef, OrkaConfig, primitives::GraphExecutionMode};
use orka_llm::{ThinkingConfig, ThinkingEffort};
use orka_workspace::WorkspaceRegistry;
use tracing::warn;

use crate::{
    agent::{Agent, AgentId, AgentLlmConfig, SystemPrompt, ToolScope},
    context::SlotKey,
    graph::{AgentGraph, Edge, EdgeCondition, GraphNode, NodeKind},
    reducer::ReducerStrategy,
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

    let mut graph = AgentGraph::new("multi-agent", entry_id.clone());
    // Wire max_hops into the termination policy.
    graph.termination.max_total_iterations = graph_def.max_hops;

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
                    warn!(condition = %c, "malformed state_match condition (expected 'state_match:key=value'), falling back to Always");
                    EdgeCondition::Always
                }
            } else {
                warn!(condition = %c, "unrecognized edge condition, falling back to Always");
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

    // Apply execution_mode as a convenience shortcut when no explicit edges are
    // defined. When edges are present, the topology takes precedence and
    // execution_mode is ignored.
    if graph_def.edges.is_empty() && config.agents.len() > 1 {
        match graph_def.execution_mode {
            GraphExecutionMode::Sequential => {
                // Auto-generate a linear chain: A → B → C → ... in agent definition order.
                let ids: Vec<AgentId> = config
                    .agents
                    .iter()
                    .map(|a| AgentId::from(a.id.as_str()))
                    .collect();
                for pair in ids.windows(2) {
                    graph.add_edge(
                        pair[0].clone(),
                        Edge {
                            target: pair[1].clone(),
                            condition: Some(EdgeCondition::Always),
                            priority: 0,
                        },
                    );
                }
            }
            GraphExecutionMode::Parallel => {
                // Auto-generate FanOut: entry dispatches all other agents in parallel.
                // Mark the entry node as FanOut kind.
                if let Some(node) = graph.get_node_mut(&entry_id) {
                    node.kind = NodeKind::FanOut;
                }
                for agent_def in &config.agents {
                    let target_id = AgentId::from(agent_def.id.as_str());
                    if target_id == entry_id {
                        continue;
                    }
                    graph.add_edge(
                        entry_id.clone(),
                        Edge {
                            target: target_id,
                            condition: Some(EdgeCondition::Always),
                            priority: 0,
                        },
                    );
                }
            }
            GraphExecutionMode::Autonomous => {
                // No auto-edges — agents route via handoff tools / LLM
                // decisions.
            }
        }
    } else if !graph_def.edges.is_empty()
        && graph_def.execution_mode != GraphExecutionMode::Sequential
    {
        tracing::debug!(
            mode = ?graph_def.execution_mode,
            "explicit edges defined; execution_mode ignored (topology takes precedence)"
        );
    }

    // Parse reducer strategies from config
    if !graph_def.reducers.is_empty() {
        let reducer_map: std::collections::HashMap<String, ReducerStrategy> = graph_def
            .reducers
            .iter()
            .map(|(key, val)| {
                let strategy = match val.to_lowercase().as_str() {
                    "append" => ReducerStrategy::Append,
                    "merge_object" => ReducerStrategy::MergeObject,
                    "sum" => ReducerStrategy::Sum,
                    "max" => ReducerStrategy::Max,
                    "min" => ReducerStrategy::Min,
                    _ => ReducerStrategy::LastWriteWins,
                };
                (key.clone(), strategy)
            })
            .collect();
        graph.reducers = reducer_map;
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
        } else if !cfg.system_prompt.is_empty() {
            // Fallback: use the inline system_prompt from config when no workspace file
            // exists.
            system_prompt.persona = cfg.system_prompt.clone();
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

    // Map planning_mode string to PlanningMode enum
    if let Some(ref mode) = cfg.planning_mode {
        agent.planning_mode = match mode.to_lowercase().as_str() {
            "adaptive" => crate::planner::PlanningMode::Adaptive,
            "always" => crate::planner::PlanningMode::Always,
            _ => crate::planner::PlanningMode::None,
        };
    }

    // Wire interrupt_before_tools
    if !cfg.interrupt_before_tools.is_empty() {
        agent.interrupt_before_tools = cfg.interrupt_before_tools.iter().cloned().collect();
    }

    // Map history_strategy string to HistoryStrategy enum
    if let Some(ref strategy) = cfg.history_strategy {
        agent.history_strategy = if strategy == "summarize" {
            crate::agent::HistoryStrategy::Summarize
        } else if let Some(n_str) = strategy.strip_prefix("rolling_window:") {
            let n = n_str.parse::<usize>().unwrap_or(10);
            crate::agent::HistoryStrategy::RollingWindow { recent_turns: n }
        } else {
            crate::agent::HistoryStrategy::Truncate
        };
    }

    // Use allowed/denied tools from config
    agent.tools = if cfg.allowed_tools.is_empty() && cfg.denied_tools.is_empty() {
        ToolScope::All
    } else if !cfg.allowed_tools.is_empty() {
        ToolScope::Allow(cfg.allowed_tools.iter().cloned().collect())
    } else {
        ToolScope::Deny(cfg.denied_tools.iter().cloned().collect())
    };

    agent.tool_result_max_chars = cfg.tool_result_max_chars;
    agent.skill_timeout_secs = cfg.skill_timeout_secs;

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
            git: Default::default(),
            prompts: Default::default(),
            agents: vec![],
            graph: None,
            research: Default::default(),
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
        assert_eq!(
            node.agent.handoff_targets,
            vec![crate::agent::AgentId::new("dst")]
        );
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

    #[tokio::test]
    async fn max_hops_wired_into_termination_policy() {
        let mut config = base_config();
        config.agents = vec![agent_def("a")];
        let mut graph_def = GraphDef::default();
        graph_def.max_hops = 42;
        config.graph = Some(graph_def);

        let registry = make_registry();
        let graph = super::build_graph_from_config(&config, &registry)
            .await
            .expect("build failed");

        assert_eq!(graph.termination.max_total_iterations, 42);
    }

    #[tokio::test]
    async fn tool_result_max_chars_from_config() {
        let mut def = agent_def("worker");
        def.config.tool_result_max_chars = 1234;
        let registry = make_registry();
        let agent = super::build_agent_from_def(&def, &registry).await;
        assert_eq!(agent.tool_result_max_chars, 1234);
    }

    #[tokio::test]
    async fn system_prompt_fallback_to_config_field() {
        let mut def = agent_def("worker");
        def.config.system_prompt = "You are a test agent.".to_string();
        // Use a registry that has no SOUL.md for this agent.
        let registry = make_registry();
        let agent = super::build_agent_from_def(&def, &registry).await;
        assert_eq!(agent.system_prompt.persona, "You are a test agent.");
    }

    #[tokio::test]
    async fn execution_mode_sequential_auto_generates_linear_edges() {
        use orka_core::config::primitives::GraphExecutionMode;

        let mut config = base_config();
        config.agents = vec![agent_def("a"), agent_def("b"), agent_def("c")];
        let mut graph_def = GraphDef::default();
        graph_def.execution_mode = GraphExecutionMode::Sequential;
        // No explicit edges — sequential mode should auto-generate a→b→c chain.
        config.graph = Some(graph_def);

        let registry = make_registry();
        let graph = super::build_graph_from_config(&config, &registry)
            .await
            .expect("build failed");

        let a = crate::agent::AgentId::new("a");
        let b = crate::agent::AgentId::new("b");
        let c = crate::agent::AgentId::new("c");

        let a_edges = graph.outgoing_edges(&a);
        assert_eq!(a_edges.len(), 1, "a should have one edge to b");
        assert_eq!(a_edges[0].target, b);

        let b_edges = graph.outgoing_edges(&b);
        assert_eq!(b_edges.len(), 1, "b should have one edge to c");
        assert_eq!(b_edges[0].target, c);

        assert!(graph.outgoing_edges(&c).is_empty(), "c is terminal");
    }

    #[tokio::test]
    async fn execution_mode_parallel_marks_entry_as_fan_out() {
        use orka_core::config::primitives::GraphExecutionMode;

        use crate::graph::NodeKind;

        let mut config = base_config();
        config.agents = vec![
            agent_def("entry"),
            agent_def("worker_a"),
            agent_def("worker_b"),
        ];
        let mut graph_def = GraphDef::default();
        graph_def.execution_mode = GraphExecutionMode::Parallel;
        config.graph = Some(graph_def);

        let registry = make_registry();
        let graph = super::build_graph_from_config(&config, &registry)
            .await
            .expect("build failed");

        let entry_id = crate::agent::AgentId::new("entry");
        let node = graph.get_node(&entry_id).expect("entry missing");
        assert!(matches!(node.kind, NodeKind::FanOut));

        let edges = graph.outgoing_edges(&entry_id);
        assert_eq!(edges.len(), 2, "entry should fan out to both workers");
    }
}
