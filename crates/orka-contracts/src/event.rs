//! Unified realtime event schema for all integration surfaces.
//!
//! [`RealtimeEvent`] is the single contract for streaming output, whether
//! transported via SSE (mobile), WebSocket (custom adapter, CLI), or future
//! transports. All surfaces emit the same events with the same semantics;
//! surfaces that cannot express a given event type degrade silently.
//!
//! This unifies the previous split between `StreamChunkKind` (internal worker
//! output) and `MobileStreamEvent` (mobile-only persistence events, now
//! deleted). `StreamChunkKind` remains as the internal worker format;
//! `RealtimeEvent` is the public wire contract produced from it.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

impl RealtimeEvent {
    /// Return the SSE `event:` field name for this event variant.
    ///
    /// This allows generic SSE serialization loops to label each frame
    /// without duplicating the dispatch table.
    #[must_use]
    pub fn sse_event_name(&self) -> &'static str {
        match self {
            Self::GenerationStarted => "generation_started",
            Self::MessageDelta { .. } => "message_delta",
            Self::ThinkingDelta { .. } => "thinking_delta",
            Self::ToolCallStart { .. } => "tool_call_start",
            Self::ToolCallEnd { .. } => "tool_call_end",
            Self::ToolExecStart { .. } => "tool_exec_start",
            Self::ToolExecEnd { .. } => "tool_exec_end",
            Self::AgentSwitch { .. } => "agent_switch",
            Self::Usage { .. } => "usage",
            Self::ContextInfo { .. } => "context_info",
            Self::PrinciplesUsed { .. } => "principles_used",
            Self::StreamDone => "stream_done",
            Self::MessageCompleted { .. } => "message_completed",
            Self::MessageFailed { .. } => "message_failed",
            Self::ArtifactReady { .. } => "artifact_ready",
        }
    }
}

/// A realtime event emitted by Orka during or after message processing.
///
/// The JSON representation uses `{"type": "...", "data": {...}}` tagged
/// encoding for straightforward client-side dispatch.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum RealtimeEvent {
    // ── Worker streaming events ───────────────────────────────────────────
    /// The LLM has started generating a response.
    ///
    /// Emitted once per turn, before the first text or thinking delta.
    /// Clients can use this to show a typing indicator immediately.
    GenerationStarted,

    /// An incremental text delta from the LLM response.
    MessageDelta {
        /// The text fragment to append.
        delta: String,
    },

    /// An incremental reasoning/thinking delta (extended thinking models).
    ThinkingDelta {
        /// The thinking fragment to append.
        delta: String,
    },

    /// The LLM has requested a tool call (model-level, before execution).
    ToolCallStart {
        /// Tool-use block ID.
        id: String,
        /// Tool name.
        name: String,
    },

    /// The LLM tool-use block is structurally complete.
    ToolCallEnd {
        /// Tool-use block ID.
        id: String,
        /// Whether the block was well-formed.
        success: bool,
    },

    /// Orka has begun executing a skill.
    ToolExecStart {
        /// Tool-use block ID.
        id: String,
        /// Skill name.
        name: String,
        /// Human-readable argument summary.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input_summary: Option<String>,
        /// Tool category (`"search"`, `"code"`, `"http"`, `"memory"`, etc.).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        category: Option<String>,
    },

    /// Orka has finished executing a skill.
    ToolExecEnd {
        /// Tool-use block ID.
        id: String,
        /// Whether the skill succeeded.
        success: bool,
        /// Wall-clock execution time in milliseconds.
        duration_ms: u64,
        /// Error message when `success` is false.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        /// Brief output summary.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result_summary: Option<String>,
    },

    /// An agent switch in a multi-agent graph execution.
    AgentSwitch {
        /// Internal agent identifier.
        agent_id: String,
        /// Human-readable agent display name.
        display_name: String,
    },

    /// Token usage and cost for a single LLM call.
    Usage {
        /// Input tokens consumed.
        input_tokens: u32,
        /// Output tokens generated.
        output_tokens: u32,
        /// Tokens read from cache, if any.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache_read_tokens: Option<u32>,
        /// Tokens written to cache, if any.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache_creation_tokens: Option<u32>,
        /// Reasoning/thinking tokens, if any.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning_tokens: Option<u32>,
        /// Model identifier.
        model: String,
        /// Estimated cost in USD.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cost_usd: Option<f64>,
    },

    /// Context window pressure — history was truncated before this turn.
    ContextInfo {
        /// Estimated tokens in history.
        history_tokens: u32,
        /// Total context window for the model.
        context_window: u32,
        /// Number of messages dropped.
        messages_truncated: u32,
        /// Whether a summary was generated to replace dropped turns.
        summary_generated: bool,
    },

    /// Number of learned principles injected into the system prompt.
    PrinciplesUsed {
        /// Count of principles applied.
        count: u32,
    },

    /// The stream for this session is complete.
    StreamDone,

    // ── Post-processing / persistence events ─────────────────────────────
    /// An assistant message has been finalized and persisted to the transcript.
    ///
    /// The `message` field is the full persisted message, serialized as a JSON
    /// object. Using `Value` keeps this crate independent of `orka-core` types.
    MessageCompleted {
        /// Conversation that owns the message.
        conversation_id: Uuid,
        /// Full persisted assistant message (schema matches
        /// `orka_core::ConversationMessage`).
        message: Value,
    },

    /// Message generation failed.
    MessageFailed {
        /// Conversation that failed.
        conversation_id: Uuid,
        /// Human-readable error description.
        error: String,
    },

    /// A new artifact is available for the conversation.
    ///
    /// The `artifact` field is serialized as a JSON object (schema matches
    /// `orka_core::ConversationArtifact`).
    ArtifactReady {
        /// Conversation that owns the artifact.
        conversation_id: Uuid,
        /// Persisted artifact metadata.
        artifact: Value,
    },
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn json(event: &RealtimeEvent) -> serde_json::Value {
        serde_json::to_value(event).expect("serialization must not fail")
    }

    #[test]
    fn snapshot_message_delta() {
        let event = RealtimeEvent::MessageDelta {
            delta: "Hello, world!".into(),
        };
        insta::assert_json_snapshot!(json(&event), @r#"
        {
          "data": {
            "delta": "Hello, world!"
          },
          "type": "message_delta"
        }
        "#);
    }

    #[test]
    fn snapshot_stream_done() {
        insta::assert_json_snapshot!(json(&RealtimeEvent::StreamDone), @r#"
        {
          "type": "stream_done"
        }
        "#);
    }

    #[test]
    fn snapshot_tool_exec_start() {
        let event = RealtimeEvent::ToolExecStart {
            id: "t1".into(),
            name: "web_search".into(),
            input_summary: Some("query: orka rust".into()),
            category: Some("search".into()),
        };
        insta::assert_json_snapshot!(json(&event), @r#"
        {
          "data": {
            "category": "search",
            "id": "t1",
            "input_summary": "query: orka rust",
            "name": "web_search"
          },
          "type": "tool_exec_start"
        }
        "#);
    }

    #[test]
    fn snapshot_usage() {
        let event = RealtimeEvent::Usage {
            input_tokens: 100,
            output_tokens: 50,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            reasoning_tokens: None,
            model: "claude-sonnet-4-6".into(),
            cost_usd: Some(0.001_5),
        };
        insta::assert_json_snapshot!(json(&event), @r#"
        {
          "data": {
            "cost_usd": 0.0015,
            "input_tokens": 100,
            "model": "claude-sonnet-4-6",
            "output_tokens": 50
          },
          "type": "usage"
        }
        "#);
    }

    #[test]
    fn sse_event_names_are_snake_case() {
        assert_eq!(
            RealtimeEvent::GenerationStarted.sse_event_name(),
            "generation_started"
        );
        assert_eq!(RealtimeEvent::StreamDone.sse_event_name(), "stream_done");
        assert_eq!(
            RealtimeEvent::MessageDelta {
                delta: String::new()
            }
            .sse_event_name(),
            "message_delta"
        );
    }

    #[test]
    fn roundtrip_message_delta() {
        let original = RealtimeEvent::MessageDelta { delta: "hi".into() };
        let json = serde_json::to_string(&original).unwrap();
        let parsed: RealtimeEvent = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, RealtimeEvent::MessageDelta { .. }));
    }
}
