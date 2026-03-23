//! Orka agent definitions, graph topology, and execution engine.
#![warn(missing_docs)]

pub mod agent;
pub mod config;
mod context_adapters;
pub mod context;
pub mod executor;
pub mod graph;
pub mod handoff;
pub(crate) mod node_runner;
pub(crate) mod tools;

pub use agent::{Agent, AgentId, AgentLlmConfig, SystemPrompt, ToolScope};
pub use config::{build_graph_from_config, build_single_agent_graph};
pub use context::{ExecutionContext, RunId, SlotKey};
pub use executor::{ExecutionResult, ExecutorDeps, GraphExecutor};
pub use graph::{AgentGraph, Edge, EdgeCondition, GraphNode, NodeKind, TerminationPolicy};
pub use handoff::{Handoff, HandoffMode};
