//! Orka agent definitions, graph topology, and execution engine.
#![warn(missing_docs)]

pub mod agent;
pub mod config;
pub mod context;
mod context_adapters;
pub mod executor;
pub mod graph;
pub mod handoff;
pub(crate) mod node_runner;
pub mod planner;
pub mod reducer;
pub(crate) mod tools;

pub use agent::{Agent, AgentId, AgentLlmConfig, HistoryStrategy, SystemPrompt, ToolScope};
pub use config::build_graph_from_config;
pub use context::{ExecutionContext, RunId, SlotKey};
pub use executor::{ExecutionResult, ExecutorDeps, GraphExecutor};
pub use graph::{AgentGraph, Edge, EdgeCondition, GraphNode, NodeKind, TerminationPolicy};
pub use handoff::{Handoff, HandoffMode};
pub use planner::{Plan, PlanStep, PlanningMode, StepStatus};
pub use reducer::ReducerStrategy;
