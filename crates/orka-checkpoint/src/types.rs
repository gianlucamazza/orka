//! Core checkpoint types: snapshot structures and run status.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use orka_core::{Envelope, SessionId};
use orka_llm::client::ChatMessage;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique identifier for a single checkpoint within a run.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CheckpointId(Uuid);

impl CheckpointId {
    /// Generate a new time-ordered checkpoint identifier.
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    /// Return the underlying UUID.
    pub fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for CheckpointId {
    fn default() -> Self {
        Self::new()
    }
}

impl From<Uuid> for CheckpointId {
    fn from(id: Uuid) -> Self {
        Self(id)
    }
}

impl std::fmt::Display for CheckpointId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Reason why a graph run was interrupted and is waiting for external input.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum InterruptReason {
    /// An agent attempted to call a tool that requires human approval before
    /// it can proceed.
    HumanApproval {
        /// Name of the tool awaiting approval.
        tool_name: String,
        /// Serialized arguments the LLM wants to pass to the tool.
        tool_input: serde_json::Value,
        /// ID of the agent that issued the tool call.
        agent_id: String,
    },
    /// Execution reached a node configured as a breakpoint.
    Breakpoint {
        /// Node ID that triggered the breakpoint.
        node_id: String,
    },
}

/// Lifecycle status of a graph run.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum RunStatus {
    /// The run is actively executing.
    Running,
    /// The run is paused and awaiting external action before it can resume.
    Interrupted {
        /// Cause of the interruption.
        reason: InterruptReason,
    },
    /// The run completed successfully.
    Completed,
    /// The run terminated with an unrecoverable error.
    Failed {
        /// Human-readable error description.
        error: String,
    },
}

/// A serialized slot key for use as a checkpoint state-map key.
///
/// Stored as `"namespace::name"` in the checkpoint's state `HashMap` so that
/// JSON round-trips work without a custom serializer.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SerializableSlotKey {
    /// Agent namespace, or `"__shared"` for cross-agent values.
    pub namespace: String,
    /// Key name within the namespace.
    pub name: String,
}

impl SerializableSlotKey {
    /// Encode as the canonical `"namespace::name"` string used in the state
    /// `HashMap`.
    pub fn to_map_key(&self) -> String {
        format!("{}::{}", self.namespace, self.name)
    }

    /// Parse a `"namespace::name"` string back into a `SerializableSlotKey`.
    ///
    /// Returns `None` when the string does not contain `::`.
    pub fn from_map_key(s: &str) -> Option<Self> {
        let (ns, name) = s.split_once("::")?;
        Some(Self {
            namespace: ns.to_string(),
            name: name.to_string(),
        })
    }
}

/// A serializable state-change entry for the checkpoint changelog.
///
/// Mirrors the agent crate's `StateChange` but uses `DateTime<Utc>` instead of
/// `std::time::Instant` so it can cross process boundaries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableStateChange {
    /// UTC timestamp of the mutation.
    pub timestamp: DateTime<Utc>,
    /// Serialized slot key as `"namespace::name"`.
    pub slot: String,
    /// Agent that performed the write.
    pub agent_id: String,
    /// Previous value, `None` for inserts.
    pub old_value: Option<serde_json::Value>,
    /// New value.
    pub new_value: serde_json::Value,
}

/// A complete, serializable snapshot of execution state at a specific node
/// boundary.
///
/// Checkpoints are written after each node in the graph completes, before the
/// executor selects the next node. This means a process restart can reload the
/// latest checkpoint and continue from exactly where it left off.
///
/// State keys are serialized as `"namespace::name"` strings so they survive
/// JSON round-trips.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Unique identifier for this checkpoint.
    pub id: CheckpointId,
    /// Run this checkpoint belongs to.
    pub run_id: String,
    /// Session this run belongs to.
    pub session_id: SessionId,
    /// ID of the graph being executed.
    pub graph_id: String,
    /// The original inbound envelope that triggered this run.
    ///
    /// Stored so that a resumed run can reconstruct the `ExecutionContext`
    /// without access to any external store.
    pub trigger: Envelope,
    /// Node that was just completed (for observability and debugging).
    pub completed_node: String,
    /// The next node to execute when resuming from this checkpoint.
    ///
    /// `None` means the run has no further nodes — it has either completed
    /// or failed terminally. The executor sets this field after evaluating
    /// outgoing edges, so resume does not need to re-evaluate them.
    pub resume_node: Option<String>,
    /// Serialized execution-context state map (`"namespace::name"` → value).
    pub state: HashMap<String, serde_json::Value>,
    /// Conversation messages at the point of this checkpoint.
    pub messages: Vec<ChatMessage>,
    /// Cumulative token usage up to this point.
    pub total_tokens: u64,
    /// Cumulative LLM iterations up to this point.
    pub total_iterations: usize,
    /// Ordered list of agent IDs that have executed so far.
    pub agents_executed: Vec<String>,
    /// Ordered audit trail of state mutations up to this point.
    pub changelog: Vec<SerializableStateChange>,
    /// Run lifecycle status at the time of this checkpoint.
    pub status: RunStatus,
    /// Wall-clock time when this checkpoint was written.
    pub created_at: DateTime<Utc>,
}
