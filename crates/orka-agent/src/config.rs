//! Translates validated agent and graph definitions into `AgentGraph` objects.

use orka_core::config::{AgentDef, GraphDef, NodeKindDef, primitives::GraphExecutionMode};
use orka_llm::ThinkingConfig;
use orka_workspace::WorkspaceRegistry;
use tracing::warn;

use crate::{
    agent::{Agent, AgentId, AgentLlmConfig, SystemPrompt, ToolScope},
    context::SlotKey,
    graph::{AgentGraph, Edge, EdgeCondition, GraphNode, NodeKind},
    reducer::ReducerStrategy,
};

/// Build an `AgentGraph` from validated `[[agents]]` + `[graph]` definitions.
#[allow(clippy::too_many_lines)]
pub async fn build_graph_from_config(
    agents: &[AgentDef],
    graph_def: Option<&GraphDef>,
    llm_config: &orka_llm::LlmConfig,
    workspace_registry: &WorkspaceRegistry,
) -> orka_core::Result<AgentGraph> {
    let graph_def = graph_def.ok_or_else(|| {
        orka_core::Error::Config(
            "[[agents]] is set but [graph] is missing — call OrkaConfig::validate() first".into(),
        )
    })?;

    // Entry point: explicit `graph.entry` overrides first agent in array
    let entry_id = graph_def
        .entry
        .as_deref()
        .map(AgentId::from)
        .or_else(|| agents.first().map(|a| AgentId::from(a.id.as_str())))
        .unwrap_or_else(|| AgentId::from("default"));

    let mut graph = AgentGraph::new("multi-agent", entry_id.clone());
    // Wire max_hops into the termination policy.
    graph.termination.max_total_iterations = graph_def.max_hops;

    // Build nodes from agent definitions
    for agent_def in agents {
        let agent = build_agent_from_def(agent_def, llm_config, workspace_registry).await;
        let kind = match agent_def.kind {
            NodeKindDef::Agent => NodeKind::Agent,
            NodeKindDef::Router => NodeKind::Router,
            NodeKindDef::FanOut => NodeKind::FanOut {
                max_concurrency: None,
            },
            NodeKindDef::FanIn => NodeKind::FanIn,
        };
        graph.add_node(GraphNode { agent, kind });
    }

    // Build edges from graph definition
    for edge_def in &graph_def.edges {
        let from = AgentId::from(edge_def.from.as_str());
        let target = AgentId::from(edge_def.to.as_str());

        // Condition is now a simple String - parse it
        let condition = edge_def.condition.as_ref().map(|c: &String| {
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
    if graph_def.edges.is_empty() && agents.len() > 1 {
        match graph_def.execution_mode {
            GraphExecutionMode::Sequential => {
                // Auto-generate a linear chain: A → B → C → ... in agent definition order.
                let ids: Vec<AgentId> = agents
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
                    node.kind = NodeKind::FanOut {
                        max_concurrency: None,
                    };
                }
                for agent_def in agents {
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
            .map(|(key, val): (&String, &String)| {
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
    let agent_ids: Vec<AgentId> = agents
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

async fn build_agent_from_def(
    def: &AgentDef,
    llm_config: &orka_llm::LlmConfig,
    workspace_registry: &WorkspaceRegistry,
) -> Agent {
    let agent_id = AgentId::from(def.id.as_str());
    let cfg = &def.config;

    let mut agent = Agent::new(agent_id.clone(), &cfg.name);

    // Load system prompt from the named workspace, falling back to the default
    // workspace (covers agents promoted from the legacy [agent] single-entry).
    let mut system_prompt = SystemPrompt::default();
    let ws_name = def.id.as_str();
    let state_lock = workspace_registry
        .state(ws_name)
        .or_else(|| workspace_registry.default_state());
    if let Some(state_lock) = state_lock {
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
    } else if !cfg.system_prompt.is_empty() {
        system_prompt.persona = cfg.system_prompt.clone();
    }

    agent.system_prompt = system_prompt;
    agent.max_turns = cfg.max_turns;
    agent.llm_call_timeout_secs = cfg.llm_call_timeout_secs;
    agent.max_run_secs = cfg.max_run_secs;
    agent.max_budget_extensions = cfg.max_budget_extensions;
    agent.budget_extension_size = cfg.budget_extension_size;
    agent.reflection_interval = cfg.reflection_interval;

    let thinking = cfg
        .thinking
        .map(|effort| ThinkingConfig::Adaptive { effort });

    // Resolve temperature: agent override → global default_temperature → 0.7
    let resolved_temperature = cfg.temperature.unwrap_or(llm_config.default_temperature);

    // Use simplified LLM config
    agent.llm_config = AgentLlmConfig {
        model: Some(cfg.model.clone()),
        max_tokens: Some(cfg.max_tokens),
        temperature: Some(resolved_temperature),
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
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::default_trait_access,
    clippy::field_reassign_with_default
)]
mod tests {
    use std::sync::Arc;

    use orka_core::config::{AgentDef, EdgeDef, GraphDef};
    use orka_workspace::{WorkspaceLoader, WorkspaceRegistry};

    fn make_registry() -> WorkspaceRegistry {
        let mut reg = WorkspaceRegistry::new("default".into());
        reg.register("default".into(), Arc::new(WorkspaceLoader::new(".")));
        reg
    }

    fn agent_def(id: &str) -> AgentDef {
        AgentDef::new(id)
    }

    async fn build_graph(
        agents: &[AgentDef],
        graph_def: Option<&GraphDef>,
        registry: &WorkspaceRegistry,
    ) -> orka_core::Result<crate::graph::AgentGraph> {
        super::build_graph_from_config(agents, graph_def, &orka_llm::LlmConfig::default(), registry)
            .await
    }

    #[tokio::test]
    async fn single_agent_config_builds_one_node_graph() {
        let agents = vec![agent_def("orka")];
        let graph_def = GraphDef::default();
        let registry = make_registry();
        let graph = build_graph(&agents, Some(&graph_def), &registry)
            .await
            .expect("build_graph_from_config failed");

        let entry = &graph.entry;
        assert!(graph.get_node(entry).is_some());
        assert!(graph.outgoing_edges(entry).is_empty());
    }

    #[tokio::test]
    async fn multi_agent_config_builds_correct_topology() {
        let agents = vec![agent_def("router"), agent_def("worker")];
        let mut edge = EdgeDef::new("router", "worker");
        edge.condition = Some("always".to_string());
        let mut graph_def = GraphDef::default();
        graph_def.max_hops = 20;
        graph_def.edges = vec![edge];

        let registry = make_registry();
        let graph = build_graph(&agents, Some(&graph_def), &registry)
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

        let mut router = agent_def("router");
        router.kind = NodeKindDef::Router;
        let worker = agent_def("worker");
        let mut graph_def = GraphDef::default();
        graph_def.edges = vec![EdgeDef::new("router", "worker")];
        let agents = vec![router, worker];

        let registry = make_registry();
        let graph = build_graph(&agents, Some(&graph_def), &registry)
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
        let agents = vec![agent_def("a"), agent_def("b")];
        let mut graph_def = GraphDef::default();
        graph_def.entry = Some("b".to_string());
        graph_def.edges = vec![EdgeDef::new("b", "a")];

        let registry = make_registry();
        let graph = build_graph(&agents, Some(&graph_def), &registry)
            .await
            .expect("build failed");

        assert_eq!(graph.entry, crate::agent::AgentId::new("b"));
    }

    #[tokio::test]
    async fn handoff_targets_derived_for_agent_nodes() {
        let agents = vec![agent_def("src"), agent_def("dst")];
        let mut graph_def = GraphDef::default();
        graph_def.edges = vec![EdgeDef::new("src", "dst")];

        let registry = make_registry();
        let graph = build_graph(&agents, Some(&graph_def), &registry)
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

        let mut fanout = agent_def("fanout");
        fanout.kind = NodeKindDef::FanOut;
        let mut graph_def = GraphDef::default();
        graph_def.edges = vec![
            EdgeDef::new("fanout", "worker_a"),
            EdgeDef::new("fanout", "worker_b"),
        ];
        let agents = vec![fanout, agent_def("worker_a"), agent_def("worker_b")];

        let registry = make_registry();
        let graph = build_graph(&agents, Some(&graph_def), &registry)
            .await
            .expect("build failed");

        let fanout_id = crate::agent::AgentId::new("fanout");
        let node = graph.get_node(&fanout_id).expect("fanout missing");
        assert!(matches!(node.kind, NodeKind::FanOut { .. }));
        // FanOut nodes must not get handoff targets
        assert!(node.agent.handoff_targets.is_empty());
    }

    #[tokio::test]
    async fn agents_without_graph_section_returns_error() {
        let agents = vec![agent_def("solo")];
        let registry = make_registry();
        let result = build_graph(&agents, None, &registry).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn thinking_effort_high_builds_adaptive_config() {
        let mut def = agent_def("thinker");
        def.config.thinking = Some(orka_core::config::primitives::ThinkingEffort::High);
        let registry = make_registry();
        let agent =
            super::build_agent_from_def(&def, &orka_llm::LlmConfig::default(), &registry).await;
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
        def.config.thinking = Some(orka_core::config::primitives::ThinkingEffort::Max);
        let registry = make_registry();
        let agent =
            super::build_agent_from_def(&def, &orka_llm::LlmConfig::default(), &registry).await;
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
        let agent =
            super::build_agent_from_def(&def, &orka_llm::LlmConfig::default(), &registry).await;
        assert!(agent.llm_config.thinking.is_none());
    }

    #[tokio::test]
    async fn max_hops_wired_into_termination_policy() {
        let agents = vec![agent_def("a")];
        let mut graph_def = GraphDef::default();
        graph_def.max_hops = 42;

        let registry = make_registry();
        let graph = build_graph(&agents, Some(&graph_def), &registry)
            .await
            .expect("build failed");

        assert_eq!(graph.termination.max_total_iterations, 42);
    }

    #[tokio::test]
    async fn tool_result_max_chars_from_config() {
        let mut def = agent_def("worker");
        def.config.tool_result_max_chars = 1234;
        let registry = make_registry();
        let agent =
            super::build_agent_from_def(&def, &orka_llm::LlmConfig::default(), &registry).await;
        assert_eq!(agent.tool_result_max_chars, 1234);
    }

    #[tokio::test]
    async fn system_prompt_fallback_to_config_field() {
        let mut def = agent_def("worker");
        def.config.system_prompt = "You are a test agent.".to_string();
        // Use a registry that has no SOUL.md for this agent.
        let registry = make_registry();
        let agent =
            super::build_agent_from_def(&def, &orka_llm::LlmConfig::default(), &registry).await;
        assert_eq!(agent.system_prompt.persona, "You are a test agent.");
    }

    #[tokio::test]
    async fn execution_mode_sequential_auto_generates_linear_edges() {
        use orka_core::config::primitives::GraphExecutionMode;

        let agents = vec![agent_def("a"), agent_def("b"), agent_def("c")];
        let mut graph_def = GraphDef::default();
        graph_def.execution_mode = GraphExecutionMode::Sequential;

        let registry = make_registry();
        let graph = build_graph(&agents, Some(&graph_def), &registry)
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

        let agents = vec![
            agent_def("entry"),
            agent_def("worker_a"),
            agent_def("worker_b"),
        ];
        let mut graph_def = GraphDef::default();
        graph_def.execution_mode = GraphExecutionMode::Parallel;

        let registry = make_registry();
        let graph = build_graph(&agents, Some(&graph_def), &registry)
            .await
            .expect("build failed");

        let entry_id = crate::agent::AgentId::new("entry");
        let node = graph.get_node(&entry_id).expect("entry missing");
        assert!(matches!(node.kind, NodeKind::FanOut { .. }));

        let edges = graph.outgoing_edges(&entry_id);
        assert_eq!(edges.len(), 2, "entry should fan out to both workers");
    }

    #[tokio::test]
    async fn temperature_none_falls_back_to_global_default() {
        let def = agent_def("worker");
        // Agent temperature is None (not set in config)
        assert!(def.config.temperature.is_none());

        let mut llm_config = orka_llm::LlmConfig::default();
        llm_config.default_temperature = 0.3;

        let registry = make_registry();
        let agent = super::build_agent_from_def(&def, &llm_config, &registry).await;
        assert_eq!(agent.llm_config.temperature, Some(0.3));
    }

    #[tokio::test]
    async fn temperature_explicit_overrides_global_default() {
        let mut def = agent_def("worker");
        def.config.temperature = Some(0.9);

        let mut llm_config = orka_llm::LlmConfig::default();
        llm_config.default_temperature = 0.3;

        let registry = make_registry();
        let agent = super::build_agent_from_def(&def, &llm_config, &registry).await;
        assert_eq!(agent.llm_config.temperature, Some(0.9));
    }
}
