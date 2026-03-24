//! Agent graph topology: nodes, edges, conditions, and termination policy.

use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use serde_json::Value;

use crate::{
    agent::{Agent, AgentId},
    context::SlotKey,
};

/// How an agent node behaves in the graph.
#[derive(Debug, Clone)]
pub enum NodeKind {
    /// Standard: executes LLM tool loop, can hand off to other agents.
    Agent,
    /// Evaluates edge conditions without calling the LLM, routing to a
    /// successor.
    Router,
    /// Dispatches to all successors in parallel (fan-out).
    FanOut,
    /// Waits for all predecessors to complete, then synthesizes results.
    FanIn,
}

/// A node in the agent graph.
#[derive(Debug, Clone)]
pub struct GraphNode {
    /// The agent definition associated with this node.
    pub agent: Agent,
    /// Execution behaviour for this node.
    pub kind: NodeKind,
}

/// An edge from one agent to another in the graph.
#[derive(Debug, Clone)]
pub struct Edge {
    /// Target agent to transition to.
    pub target: AgentId,
    /// Optional condition that must be satisfied for this edge to be taken.
    pub condition: Option<EdgeCondition>,
    /// Priority: lower value = checked first. Used when multiple edges exist.
    pub priority: u32,
}

/// Condition that guards an edge transition.
#[derive(Debug, Clone)]
pub enum EdgeCondition {
    /// A state slot must match a specific JSON value.
    StateMatch {
        /// The slot key to inspect.
        key: SlotKey,
        /// The JSON value it must equal.
        pattern: Value,
    },
    /// The agent's final output text must contain this string.
    OutputContains(String),
    /// Always take this edge (fallback).
    Always,
}

/// Policy controlling when the graph execution terminates.
#[derive(Debug, Clone)]
pub struct TerminationPolicy {
    /// Maximum total LLM iterations across all nodes.
    pub max_total_iterations: usize,
    /// Optional maximum total token budget.
    pub max_total_tokens: Option<u64>,
    /// Maximum wall-clock time for the entire execution.
    pub max_duration: Duration,
    /// Agents whose completion without handoff ends the run.
    pub terminal_agents: HashSet<AgentId>,
}

impl Default for TerminationPolicy {
    fn default() -> Self {
        Self {
            max_total_iterations: 50,
            max_total_tokens: None,
            max_duration: Duration::from_secs(300),
            terminal_agents: HashSet::new(),
        }
    }
}

/// A directed graph of agents with entry point and termination policy.
#[derive(Debug, Clone)]
pub struct AgentGraph {
    /// Unique identifier for this graph (used in events and logs).
    pub id: String,
    /// Entry-point agent where execution begins.
    pub entry: AgentId,
    nodes: HashMap<AgentId, GraphNode>,
    edges: HashMap<AgentId, Vec<Edge>>,
    /// Policy controlling when the overall graph run terminates.
    pub termination: TerminationPolicy,
}

impl AgentGraph {
    /// Create a new graph with the given id and entry agent.
    pub fn new(id: impl Into<String>, entry: AgentId) -> Self {
        Self {
            id: id.into(),
            entry,
            nodes: HashMap::new(),
            edges: HashMap::new(),
            termination: TerminationPolicy::default(),
        }
    }

    /// Add a node to the graph, replacing any existing node with the same agent
    /// id.
    pub fn add_node(&mut self, node: GraphNode) {
        self.nodes.insert(node.agent.id.clone(), node);
    }

    /// Add a directed edge from `from` to the edge's target.
    pub fn add_edge(&mut self, from: AgentId, edge: Edge) {
        self.edges.entry(from).or_default().push(edge);
    }

    /// Look up a node by agent id.
    pub fn get_node(&self, id: &AgentId) -> Option<&GraphNode> {
        self.nodes.get(id)
    }

    /// Get outgoing edges from a node, sorted by priority.
    pub fn outgoing_edges(&self, from: &AgentId) -> Vec<&Edge> {
        let mut edges: Vec<&Edge> = self
            .edges
            .get(from)
            .map(|v| v.iter().collect())
            .unwrap_or_default();
        edges.sort_by_key(|e| e.priority);
        edges
    }

    /// Set a custom termination policy (builder style).
    pub fn with_termination(mut self, policy: TerminationPolicy) -> Self {
        self.termination = policy;
        self
    }

    /// Iterate over all (AgentId, GraphNode) pairs in the graph.
    pub fn nodes_iter(&self) -> impl Iterator<Item = (&AgentId, &GraphNode)> {
        self.nodes.iter()
    }

    /// Iterate over all (AgentId, Vec<Edge>) pairs in the graph.
    pub fn edges_iter(&self) -> impl Iterator<Item = (&AgentId, &Vec<Edge>)> {
        self.edges.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::Agent;

    fn make_graph() -> AgentGraph {
        let entry = AgentId::new("entry");
        AgentGraph::new("test-graph", entry)
    }

    #[test]
    fn add_node_and_get() {
        let mut graph = make_graph();
        let agent = Agent::new("entry", "Entry Agent");
        graph.add_node(GraphNode {
            agent,
            kind: NodeKind::Agent,
        });
        assert!(graph.get_node(&AgentId::new("entry")).is_some());
        assert!(graph.get_node(&AgentId::new("missing")).is_none());
    }

    #[test]
    fn outgoing_edges_sorted_by_priority() {
        let mut graph = make_graph();
        let from = AgentId::new("entry");
        graph.add_edge(
            from.clone(),
            Edge {
                target: AgentId::new("c"),
                condition: None,
                priority: 10,
            },
        );
        graph.add_edge(
            from.clone(),
            Edge {
                target: AgentId::new("a"),
                condition: None,
                priority: 1,
            },
        );
        graph.add_edge(
            from.clone(),
            Edge {
                target: AgentId::new("b"),
                condition: None,
                priority: 5,
            },
        );

        let edges = graph.outgoing_edges(&from);
        assert_eq!(edges.len(), 3);
        assert_eq!(edges[0].target, AgentId::new("a"));
        assert_eq!(edges[1].target, AgentId::new("b"));
        assert_eq!(edges[2].target, AgentId::new("c"));
    }

    #[test]
    fn outgoing_edges_empty_for_unknown_node() {
        let graph = make_graph();
        let edges = graph.outgoing_edges(&AgentId::new("no-such-node"));
        assert!(edges.is_empty());
    }

    #[test]
    fn with_termination_overrides_defaults() {
        let graph = make_graph().with_termination(TerminationPolicy {
            max_total_iterations: 10,
            max_total_tokens: Some(5000),
            ..Default::default()
        });
        assert_eq!(graph.termination.max_total_iterations, 10);
        assert_eq!(graph.termination.max_total_tokens, Some(5000));
    }

    #[test]
    fn nodes_iter_returns_all() {
        let mut graph = make_graph();
        for name in ["entry", "mid", "end"] {
            graph.add_node(GraphNode {
                agent: Agent::new(name, name),
                kind: NodeKind::Agent,
            });
        }
        assert_eq!(graph.nodes_iter().count(), 3);
    }

    #[test]
    fn termination_policy_defaults() {
        let graph = make_graph();
        assert_eq!(graph.termination.max_total_iterations, 50);
        assert!(graph.termination.max_total_tokens.is_none());
    }
}
