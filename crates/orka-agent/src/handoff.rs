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
    pub from: AgentId,
    pub to: AgentId,
    pub reason: String,
    pub context_transfer: HashMap<String, Value>,
    pub mode: HandoffMode,
}
