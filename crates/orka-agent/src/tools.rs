use orka_llm::client::ToolDefinition;

use crate::{agent::Agent, graph::AgentGraph};

/// Build the `transfer_to_agent` and `delegate_to_agent` tool definitions
/// based on the agent's `handoff_targets` list.
pub(crate) fn build_handoff_tools(agent: &Agent, graph: &AgentGraph) -> Vec<ToolDefinition> {
    if agent.handoff_targets.is_empty() {
        return Vec::new();
    }

    // Build descriptions of available target agents
    let targets_desc: Vec<String> = agent
        .handoff_targets
        .iter()
        .filter_map(|id| {
            graph
                .get_node(id)
                .map(|node| format!("- `{}`: {}", node.agent.id, node.agent.display_name))
        })
        .collect();

    let targets_list = targets_desc.join("\n");
    let agent_ids: Vec<String> = agent
        .handoff_targets
        .iter()
        .map(|id| id.0.to_string())
        .collect();

    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "agent_id": {
                "type": "string",
                "description": "ID of the target agent",
                "enum": agent_ids
            },
            "reason": {
                "type": "string",
                "description": "Why you are handing off to this agent"
            },
            "context": {
                "type": "object",
                "description": "Optional key-value context to pass to the target agent"
            }
        },
        "required": ["agent_id", "reason"]
    });

    let mut delegate_schema = schema.clone();
    if let Some(obj) = delegate_schema.as_object_mut() {
        if let Some(props) = obj.get_mut("properties").and_then(|p| p.as_object_mut()) {
            props.insert(
                "task".to_string(),
                serde_json::json!({
                    "type": "string",
                    "description": "The specific task to delegate"
                }),
            );
        }
        if let Some(req) = obj.get_mut("required").and_then(|r| r.as_array_mut()) {
            req.push(serde_json::json!("task"));
        }
    }

    vec![
        ToolDefinition::new(
            "transfer_to_agent",
            format!(
                "Permanently transfer control to another agent. The current agent will not resume.\n\nAvailable agents:\n{targets_list}"
            ),
            schema,
        ),
        ToolDefinition::new(
            "delegate_to_agent",
            format!(
                "Delegate a sub-task to another agent. You will resume after it completes.\n\nAvailable agents:\n{targets_list}"
            ),
            delegate_schema,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        agent::AgentId,
        graph::{GraphNode, NodeKind},
    };

    fn make_agent_with_targets(targets: Vec<&str>) -> Agent {
        let mut agent = Agent::new("orchestrator", "Orchestrator");
        agent.handoff_targets = targets.iter().map(|s| AgentId::new(*s)).collect();
        agent
    }

    fn make_graph_with_agents(agent_ids: &[&str]) -> AgentGraph {
        let entry = AgentId::new(agent_ids[0]);
        let mut graph = AgentGraph::new("test", entry);
        for id in agent_ids {
            graph.add_node(GraphNode {
                agent: Agent::new(*id, format!("{id} Agent")),
                kind: NodeKind::Agent,
            });
        }
        graph
    }

    #[test]
    fn empty_targets_returns_no_tools() {
        let agent = Agent::new("solo", "Solo");
        let graph = AgentGraph::new("g", AgentId::new("solo"));
        let tools = build_handoff_tools(&agent, &graph);
        assert!(tools.is_empty());
    }

    #[test]
    fn with_targets_returns_two_tools() {
        let agent = make_agent_with_targets(vec!["search", "coder"]);
        let graph = make_graph_with_agents(&["search", "coder"]);
        let tools = build_handoff_tools(&agent, &graph);
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name, "transfer_to_agent");
        assert_eq!(tools[1].name, "delegate_to_agent");
    }

    #[test]
    fn transfer_schema_contains_agent_id_enum() {
        let agent = make_agent_with_targets(vec!["search"]);
        let graph = make_graph_with_agents(&["search"]);
        let tools = build_handoff_tools(&agent, &graph);
        let schema = &tools[0].input_schema;
        let enum_vals = &schema["properties"]["agent_id"]["enum"];
        assert!(enum_vals.as_array().unwrap().iter().any(|v| v == "search"));
    }

    #[test]
    fn delegate_schema_requires_task_field() {
        let agent = make_agent_with_targets(vec!["coder"]);
        let graph = make_graph_with_agents(&["coder"]);
        let tools = build_handoff_tools(&agent, &graph);
        let delegate_tool = &tools[1];
        let required = delegate_tool.input_schema["required"].as_array().unwrap();
        let required_names: Vec<&str> = required
            .iter()
            .filter_map(|v: &serde_json::Value| v.as_str())
            .collect();
        assert!(required_names.contains(&"task"));
    }
}
