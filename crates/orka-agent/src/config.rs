//! Translates `OrkaConfig` into `Agent` and `AgentGraph` objects.

use orka_core::config::{AgentDef, OrkaConfig};
use orka_workspace::WorkspaceRegistry;

use crate::agent::{Agent, AgentId, AgentLlmConfig, SystemPrompt, ToolScope};
use crate::context::SlotKey;
use crate::graph::{AgentGraph, Edge, EdgeCondition, GraphNode, NodeKind, TerminationPolicy};

/// Build an `AgentGraph` from the `[[agents]]` + `[graph]` config sections.
pub async fn build_graph_from_config(
    config: &OrkaConfig,
    workspace_registry: &WorkspaceRegistry,
) -> orka_core::Result<AgentGraph> {
    if config.agents.is_empty() {
        return build_single_agent_graph(config, workspace_registry).await;
    }

    let graph_def = config.graph.as_ref().ok_or_else(|| {
        orka_core::Error::Config(
            "[[agents]] is set but [graph] is missing — add [graph] section to config".into(),
        )
    })?;

    // Use first agent as entry point (new simplified model)
    let entry_id = config.agents.first()
        .map(|a| AgentId::from(a.id.as_str()))
        .unwrap_or_else(|| AgentId::from("default"));

    // Build graph with max_hops from config
    let mut graph = AgentGraph::new("multi-agent", entry_id.clone());

    // Build nodes from agent definitions
    for agent_def in &config.agents {
        let agent = build_agent_from_def(agent_def, workspace_registry).await;
        graph.add_node(GraphNode {
            agent,
            kind: NodeKind::Agent,
        });
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
                let parts: Vec<&str> = c.strip_prefix("state_match:").unwrap_or(c).split('=').collect();
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

    Ok(graph)
}

/// Build a single-node graph from the `[agent]` config section.
pub async fn build_single_agent_graph(
    config: &OrkaConfig,
    workspace_registry: &WorkspaceRegistry,
) -> orka_core::Result<AgentGraph> {
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

    let mut agent = Agent::new(agent_id.clone(), &agent_cfg.name);
    agent.system_prompt = system_prompt;
    agent.max_iterations = agent_cfg.max_iterations;
    
    // Use default LLM config with simplified fields
    agent.llm_config = AgentLlmConfig {
        model: Some(agent_cfg.model.clone()),
        max_tokens: Some(agent_cfg.max_tokens),
        temperature: Some(agent_cfg.temperature),
        ..Default::default()
    };

    let policy = TerminationPolicy::default();

    let mut graph = AgentGraph::new("single", agent_id).with_termination(policy);
    graph.add_node(GraphNode {
        agent,
        kind: NodeKind::Agent,
    });

    Ok(graph)
}

async fn build_agent_from_def(def: &AgentDef, workspace_registry: &WorkspaceRegistry) -> Agent {
    let agent_id = AgentId::from(def.id.as_str());
    let cfg = &def.config;
    
    let mut agent = Agent::new(agent_id.clone(), &cfg.name);

    // Load system prompt from workspace registry
    let mut system_prompt = SystemPrompt::default();
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

    agent.system_prompt = system_prompt;
    agent.max_iterations = cfg.max_iterations;
    
    // Use simplified LLM config
    agent.llm_config = AgentLlmConfig {
        model: Some(cfg.model.clone()),
        max_tokens: Some(cfg.max_tokens),
        temperature: Some(cfg.temperature),
        ..Default::default()
    };

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
            soft_skills: Default::default(),
            session: Default::default(),
            queue: Default::default(),
            llm: Default::default(),
            agent: AgentConfig::default(),
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
        let mut edge = EdgeDef {
            from: "router".to_string(),
            to: "worker".to_string(),
            condition: Some("always".to_string()),
            weight: 1.0,
        };
        let graph = GraphDef {
            execution_mode: orka_core::config::primitives::GraphExecutionMode::default(),
            max_hops: 20,
            edges: vec![edge],
        };
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
    async fn agents_without_graph_section_returns_error() {
        let mut config = base_config();
        config.agents = vec![agent_def("solo")];
        // graph is None — must error
        let registry = make_registry();
        let result = super::build_graph_from_config(&config, &registry).await;
        assert!(result.is_err());
    }
}
