//! Multi-Agent Workflow Example
//!
//! Demonstrates how to build a multi-agent system where specialized agents
//! collaborate to complete complex tasks.
//!
//! ## Architecture
//!
//! ```text
//! User Query
//!     │
//!     ▼
//! ┌─────────────┐
//! │  Router     │ ──► Decides which agent handles the query
//! │   Agent     │
//! └──────┬──────┘
//!        │
//!        ├─────────────┬─────────────┐
//!        ▼             ▼             ▼
//!   ┌─────────┐  ┌──────────┐  ┌──────────┐
//!   │ Research│  │  Code    │  │  Data    │
//!   │  Agent  │  │  Agent   │  │  Agent   │
//!   └────┬────┘  └────┬─────┘  └────┬─────┘
//!        │            │             │
//!        └────────────┴─────────────┘
//!                     │
//!                     ▼
//!              ┌─────────────┐
//!              │  Summary    │ ──► Final response
//!              │   Agent     │
//!              └─────────────┘
//! ```
//!
//! ## Running
//!
//! ```bash
//! cargo run --bin multi_agent
//! ```

use anyhow::Result;
use orka_agent::{Agent, AgentGraph, AgentId, ExecutionContext, GraphExecutor, GraphNode, NodeKind, TerminationPolicy};
use orka_core::types::SessionId;
use std::sync::Arc;
use tracing::{info, instrument};

/// A specialized agent for research tasks
struct ResearchAgent;

impl ResearchAgent {
    fn new() -> Agent {
        Agent::new(
            AgentId::from("researcher"),
            "Research Specialist - Finds and analyzes information",
        )
    }
}

/// A specialized agent for coding tasks
struct CodeAgent;

impl CodeAgent {
    fn new() -> Agent {
        Agent::new(
            AgentId::from("coder"),
            "Code Specialist - Writes and reviews code",
        )
    }
}

/// A specialized agent for data analysis
struct DataAgent;

impl DataAgent {
    fn new() -> Agent {
        Agent::new(
            AgentId::from("analyst"),
            "Data Analyst - Processes and visualizes data",
        )
    }
}

/// A router agent that decides which specialist to use
struct RouterAgent;

impl RouterAgent {
    fn new() -> Agent {
        Agent::new(
            AgentId::from("router"),
            "Router - Analyzes queries and routes to appropriate specialist",
        )
    }
}

/// A summary agent that combines outputs from specialists
struct SummaryAgent;

impl SummaryAgent {
    fn new() -> Agent {
        Agent::new(
            AgentId::from("summarizer"),
            "Summarizer - Combines outputs into coherent response",
        )
    }
}

/// Build a multi-agent graph for collaborative task execution
fn build_multi_agent_graph() -> Arc<AgentGraph> {
    // Create agents
    let router = RouterAgent::new();
    let researcher = ResearchAgent::new();
    let coder = CodeAgent::new();
    let analyst = DataAgent::new();
    let summarizer = SummaryAgent::new();

    // Define termination policy
    let policy = TerminationPolicy {
        terminal_agents: [AgentId::from("summarizer")].iter().cloned().collect(),
        max_total_iterations: 10,
        max_total_tokens: 10000,
        max_duration_secs: 300,
    };

    // Build the graph
    let mut graph = AgentGraph::new("multi_agent_workflow", AgentId::from("router"))
        .with_termination(policy);

    // Add router node (entry point)
    graph.add_node(GraphNode {
        agent: router,
        kind: NodeKind::Agent,
    });

    // Add specialist nodes
    graph.add_node(GraphNode {
        agent: researcher,
        kind: NodeKind::Agent,
    });

    graph.add_node(GraphNode {
        agent: coder,
        kind: NodeKind::Agent,
    });

    graph.add_node(GraphNode {
        agent: analyst,
        kind: NodeKind::Agent,
    });

    // Add summarizer (terminal node)
    graph.add_node(GraphNode {
        agent: summarizer,
        kind: NodeKind::Agent,
    });

    // Define edges with conditions
    // Router -> Researcher (for research queries)
    graph.add_edge(
        AgentId::from("router"),
        orka_agent::Edge {
            target: AgentId::from("researcher"),
            condition: Some(orka_agent::EdgeCondition::OutputContains("RESEARCH".to_string())),
            priority: 0,
        },
    );

    // Router -> Coder (for coding queries)
    graph.add_edge(
        AgentId::from("router"),
        orka_agent::Edge {
            target: AgentId::from("coder"),
            condition: Some(orka_agent::EdgeCondition::OutputContains("CODE".to_string())),
            priority: 1,
        },
    );

    // Router -> Analyst (for data queries)
    graph.add_edge(
        AgentId::from("router"),
        orka_agent::Edge {
            target: AgentId::from("analyst"),
            condition: Some(orka_agent::EdgeCondition::OutputContains("DATA".to_string())),
            priority: 2,
        },
    );

    // All specialists -> Summarizer
    graph.add_edge(
        AgentId::from("researcher"),
        orka_agent::Edge {
            target: AgentId::from("summarizer"),
            condition: Some(orka_agent::EdgeCondition::Always),
            priority: 0,
        },
    );

    graph.add_edge(
        AgentId::from("coder"),
        orka_agent::Edge {
            target: AgentId::from("summarizer"),
            condition: Some(orka_agent::EdgeCondition::Always),
            priority: 0,
        },
    );

    graph.add_edge(
        AgentId::from("analyst"),
        orka_agent::Edge {
            target: AgentId::from("summarizer"),
            condition: Some(orka_agent::EdgeCondition::Always),
            priority: 0,
        },
    );

    Arc::new(graph)
}

/// Simulated agent execution for demonstration
async fn simulate_agent_execution(
    executor: &GraphExecutor,
    graph: &AgentGraph,
    query: &str,
) -> Result<String> {
    let ctx = ExecutionContext::new(orka_core::types::Envelope::text(
        "demo",
        SessionId::new(),
        query,
    ));

    info!(query, "Starting multi-agent workflow");

    // In a real implementation, this would call the LLM
    // For demo purposes, we simulate the routing
    let response = if query.contains("research") || query.contains("find") {
        format!(
            "[Router] Detected research query\n\
             [Researcher] Searching for information about: {}\n\
             [Summarizer] Research complete. Found relevant information about {}.",
            query, query
        )
    } else if query.contains("code") || query.contains("program") {
        format!(
            "[Router] Detected coding query\n\
             [Coder] Writing code for: {}\n\
             [Summarizer] Code generated successfully for {}.",
            query, query
        )
    } else if query.contains("data") || query.contains("analyze") {
        format!(
            "[Router] Detected data analysis query\n\
             [Analyst] Analyzing data for: {}\n\
             [Summarizer] Data analysis complete for {}.",
            query, query
        )
    } else {
        format!(
            "[Router] General query detected\n\
             [Researcher] Gathering information about: {}\n\
             [Summarizer] Here's what I found about {}.",
            query, query
        )
    };

    Ok(response)
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("multi_agent=info")
        .init();

    println!("=== Multi-Agent Workflow Example ===\n");

    // Build the agent graph
    let graph = build_multi_agent_graph();
    info!("Multi-agent graph built with {} nodes", graph.node_count());

    // Create executor (in real use, would have LLM client)
    // For demo, we simulate execution

    // Example queries
    let queries = vec![
        "Research the history of Rust programming",
        "Write a function to calculate fibonacci numbers",
        "Analyze sales data trends for Q4",
        "Explain quantum computing",
    ];

    for (i, query) in queries.iter().enumerate() {
        println!("\n--- Query {} ---", i + 1);
        println!("User: {}", query);
        println!("\nWorkflow:");

        // Simulate execution
        let response = simulate_execution(query).await?;
        println!("{}", response);
    }

    println!("\n=== Example Complete ===");
    println!("\nIn a real implementation:");
    println!("- Each agent would be backed by an LLM");
    println!("- The router would use LLM to decide routing");
    println!("- Agents would communicate via shared context");
    println!("- The graph executor would manage the flow");

    Ok(())
}

async fn simulate_execution(query: &str) -> Result<String> {
    // Simulate the multi-agent workflow
    let agent_id = match query {
        q if q.contains("research") || q.contains("history") => "researcher",
        q if q.contains("function") || q.contains("code") => "coder",
        q if q.contains("data") || q.contains("analyze") => "analyst",
        _ => "researcher", // default
    };

    let agent_name = match agent_id {
        "researcher" => "Research Agent",
        "coder" => "Code Agent",
        "analyst" => "Data Agent",
        _ => "Unknown",
    };

    let output = format!(
        "[Router Agent] → Routing to {}\n\
         [{}] → Processing query\n\
         [{}] → Task complete\n\
         [Summarizer] → Compiling final response\n\n\
         Result: Task '{}' completed by {}.",
        agent_name, agent_name, agent_name, query, agent_name
    );

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_graph_construction() {
        let graph = build_multi_agent_graph();
        assert!(graph.node_count() >= 5); // router + 3 specialists + summarizer
    }

    #[tokio::test]
    async fn test_simulate_execution() {
        let result = simulate_execution("Research Rust").await.unwrap();
        assert!(result.contains("Research Agent"));

        let result = simulate_execution("Write code").await.unwrap();
        assert!(result.contains("Code Agent"));

        let result = simulate_execution("Analyze data").await.unwrap();
        assert!(result.contains("Data Agent"));
    }
}
