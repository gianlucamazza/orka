//! Agent handoff types for control transfer between agents in a graph.

use std::collections::HashMap;

use serde_json::Value;

use crate::agent::AgentId;

/// How the control transfers to another agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandoffMode {
    /// Permanent transfer — the source agent does not resume.
    Transfer,
    /// Delegation — the source agent resumes after the target completes.
    Delegate,
}

/// A handoff request from one agent to another.
#[derive(Debug, Clone)]
pub struct Handoff {
    /// The agent initiating the handoff.
    pub from: AgentId,
    /// The target agent to hand off to.
    pub to: AgentId,
    /// Human-readable reason logged for observability.
    pub reason: String,
    /// Optional key-value data passed to the target agent's context.
    pub context_transfer: HashMap<String, Value>,
    /// Whether to transfer or delegate control.
    pub mode: HandoffMode,
}
