//! Streaming infrastructure for real-time LLM response delivery.
//!
//! [`StreamRegistry`] routes [`StreamChunk`]s to WebSocket subscribers keyed by
//! session ID. The worker emits chunks as the LLM streams tokens; adapters
//! subscribe to forward them.

use std::sync::Arc;

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::{MessageId, SessionId};

/// A chunk of streaming data from an LLM response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct StreamChunk {
    /// The session this chunk belongs to.
    pub session_id: SessionId,
    /// The channel (adapter) originating the request.
    pub channel: String,
    /// The message this stream is replying to.
    pub reply_to: Option<MessageId>,
    /// The kind of chunk.
    pub kind: StreamChunkKind,
}

/// The kind of stream chunk.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(tag = "type", content = "data")]
pub enum StreamChunkKind {
    /// A text delta from the LLM response.
    Delta(String),
    /// The LLM requested a tool call.
    ToolStart {
        /// Tool name.
        name: String,
        /// Tool-use block ID.
        id: String,
    },
    /// The LLM tool-use block is complete.
    ToolEnd {
        /// Tool-use block ID.
        id: String,
        /// Whether the tool block was well-formed.
        success: bool,
    },
    /// Orka begins executing a skill.
    ToolExecStart {
        /// Skill name.
        name: String,
        /// Tool-use block ID.
        id: String,
        /// Human-readable argument summary (e.g. `"query: 'Iran war 2026'"`).
        #[serde(skip_serializing_if = "Option::is_none")]
        input_summary: Option<String>,
        /// Tool category tag (`"search"`, `"code"`, `"http"`, `"memory"`,
        /// `"schedule"`).
        #[serde(skip_serializing_if = "Option::is_none")]
        category: Option<String>,
    },
    /// Orka finished executing a skill.
    ToolExecEnd {
        /// Tool-use block ID.
        id: String,
        /// Whether the skill succeeded.
        success: bool,
        /// Wall-clock execution time in milliseconds.
        duration_ms: u64,
        /// Error message when `success` is false.
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        /// Brief output summary (e.g. `"Found 3 results"`).
        #[serde(skip_serializing_if = "Option::is_none")]
        result_summary: Option<String>,
    },
    /// An incremental thinking/reasoning chunk (extended thinking models).
    ThinkingDelta(String),
    /// Token usage and cost for a single LLM call.
    Usage {
        /// Input tokens consumed.
        input_tokens: u32,
        /// Output tokens generated.
        output_tokens: u32,
        /// Tokens read from cache, if any.
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_read_tokens: Option<u32>,
        /// Tokens written to cache, if any.
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_creation_tokens: Option<u32>,
        /// Reasoning/thinking tokens, if any.
        #[serde(skip_serializing_if = "Option::is_none")]
        reasoning_tokens: Option<u32>,
        /// Model identifier used for this call.
        model: String,
        /// Estimated cost in USD, if available.
        #[serde(skip_serializing_if = "Option::is_none")]
        cost_usd: Option<f64>,
    },
    /// An agent switch in a multi-agent graph execution.
    AgentSwitch {
        /// Internal agent identifier.
        agent_id: String,
        /// Human-readable agent name.
        display_name: String,
    },
    /// Context window pressure — emitted when history was truncated.
    ContextInfo {
        /// Estimated tokens currently in history.
        history_tokens: u32,
        /// Total context window size for the model.
        context_window: u32,
        /// Number of messages dropped to fit the context window.
        messages_truncated: u32,
        /// Whether a summary was generated to replace dropped turns.
        summary_generated: bool,
    },
    /// Number of learned principles injected into the system prompt.
    PrinciplesUsed {
        /// Count of principles applied.
        count: u32,
    },
    /// The LLM has started generating a response.
    ///
    /// Emitted once per turn, before the first [`Self::Delta`] or
    /// [`Self::ThinkingDelta`] arrives. Clients can use this to show a
    /// typing indicator without waiting for the first text token.
    GenerationStarted,
    /// The stream is complete.
    Done,
}

/// Why the agent stopped executing.
///
/// Propagated through [`crate::OutboundMessage`] metadata so that clients can
/// distinguish a natural completion from a limit-induced truncation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStopReason {
    /// Agent finished naturally (no pending tool calls).
    Complete,
    /// Agent loop exhausted `max_turns`.
    MaxTurns,
    /// Agent was warned of budget pressure and forced to conclude gracefully.
    SoftLimit,
    /// The LLM's final response was truncated by the output token limit.
    MaxTokens,
    /// Human-in-the-loop interrupt.
    Interrupted,
    /// Agent loop terminated due to an error.
    Error,
}

impl StreamChunk {
    /// Create a new stream chunk.
    pub fn new(
        session_id: SessionId,
        channel: impl Into<String>,
        reply_to: Option<MessageId>,
        kind: StreamChunkKind,
    ) -> Self {
        Self {
            session_id,
            channel: channel.into(),
            reply_to,
            kind,
        }
    }
}

/// Registry that maps session IDs to stream chunk senders.
/// Used to route streaming LLM output to WebSocket connections.
///
/// Uses `DashMap` for shard-level locking instead of a global `Mutex`,
/// reducing contention during high-frequency LLM streaming.
#[derive(Clone, Default)]
pub struct StreamRegistry {
    inner: Arc<DashMap<SessionId, Vec<mpsc::UnboundedSender<Arc<StreamChunk>>>>>,
}

impl StreamRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Subscribe to stream chunks for a session. Returns a receiver that yields
    /// chunks.
    pub fn subscribe(&self, session_id: SessionId) -> mpsc::UnboundedReceiver<Arc<StreamChunk>> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.inner.entry(session_id).or_default().push(tx);
        rx
    }

    /// Send a chunk to all subscribers of the chunk's session. Returns the
    /// number of subscribers that received the chunk. Disconnected senders
    /// are pruned automatically.
    ///
    /// The chunk is wrapped in `Arc` once before broadcasting, so cloning is
    /// O(1) regardless of subscriber count.
    pub fn send(&self, chunk: StreamChunk) -> usize {
        let Some(mut entry) = self.inner.get_mut(&chunk.session_id) else {
            return 0;
        };
        let senders = entry.value_mut();

        let shared = Arc::new(chunk);
        let mut delivered = 0;
        senders.retain(|tx| match tx.send(Arc::clone(&shared)) {
            Ok(()) => {
                delivered += 1;
                true
            }
            Err(_) => false,
        });

        let session_id = shared.session_id;
        if senders.is_empty() {
            drop(entry);
            self.inner.remove(&session_id);
        }

        delivered
    }

    /// Check whether any subscribers exist for a session.
    pub fn has_subscribers(&self, session_id: &SessionId) -> bool {
        self.inner.contains_key(session_id)
    }
}

/// Forward progress events from a `coding_delegate` skill run to SSE
/// subscribers.
///
/// Drains `rx`, wrapping each [`serde_json::Value`] as a JSON-string
/// [`StreamChunkKind::Delta`] and broadcasting it through `registry`. Sends a
/// final [`StreamChunkKind::Done`] chunk when the sender side closes.
pub async fn forward_delegate_progress(
    mut rx: mpsc::UnboundedReceiver<serde_json::Value>,
    registry: StreamRegistry,
    _event_sink: Arc<dyn crate::traits::EventSink>,
    session_id: SessionId,
    channel: String,
    reply_to: Option<MessageId>,
    _message_id: MessageId,
) {
    while let Some(value) = rx.recv().await {
        let text = serde_json::to_string(&value).unwrap_or_default();
        registry.send(StreamChunk::new(
            session_id,
            channel.clone(),
            reply_to,
            StreamChunkKind::Delta(text),
        ));
    }
    registry.send(StreamChunk::new(
        session_id,
        channel,
        reply_to,
        StreamChunkKind::Done,
    ));
}

/// Convert a [`StreamChunkKind`] into the canonical [`RealtimeEvent`].
///
/// This mapping is the authoritative translation between the internal worker
/// streaming format and the public contract exposed to all integration
/// surfaces.
impl From<StreamChunkKind> for crate::RealtimeEvent {
    fn from(kind: StreamChunkKind) -> Self {
        use crate::RealtimeEvent;
        match kind {
            StreamChunkKind::GenerationStarted => RealtimeEvent::GenerationStarted,
            StreamChunkKind::Delta(delta) => RealtimeEvent::MessageDelta { delta },
            StreamChunkKind::ThinkingDelta(delta) => RealtimeEvent::ThinkingDelta { delta },
            StreamChunkKind::ToolStart { id, name } => RealtimeEvent::ToolCallStart { id, name },
            StreamChunkKind::ToolEnd { id, success } => RealtimeEvent::ToolCallEnd { id, success },
            StreamChunkKind::ToolExecStart {
                id,
                name,
                input_summary,
                category,
                ..
            } => RealtimeEvent::ToolExecStart {
                id,
                name,
                input_summary,
                category,
            },
            StreamChunkKind::ToolExecEnd {
                id,
                success,
                duration_ms,
                error,
                result_summary,
                ..
            } => RealtimeEvent::ToolExecEnd {
                id,
                success,
                duration_ms,
                error,
                result_summary,
            },
            StreamChunkKind::AgentSwitch {
                agent_id,
                display_name,
                ..
            } => RealtimeEvent::AgentSwitch {
                agent_id,
                display_name,
            },
            StreamChunkKind::Usage {
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_creation_tokens,
                reasoning_tokens,
                model,
                cost_usd,
                ..
            } => RealtimeEvent::Usage {
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_creation_tokens,
                reasoning_tokens,
                model,
                cost_usd,
            },
            StreamChunkKind::ContextInfo {
                history_tokens,
                context_window,
                messages_truncated,
                summary_generated,
                ..
            } => RealtimeEvent::ContextInfo {
                history_tokens,
                context_window,
                messages_truncated,
                summary_generated,
            },
            StreamChunkKind::PrinciplesUsed { count, .. } => {
                RealtimeEvent::PrinciplesUsed { count }
            }
            StreamChunkKind::Done => RealtimeEvent::StreamDone,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok<T, E: std::fmt::Display>(result: std::result::Result<T, E>) -> T {
        result.unwrap_or_else(|err| panic!("unexpected error: {err}"))
    }

    fn json_value(chunk: &StreamChunkKind) -> serde_json::Value {
        ok(serde_json::from_str(&ok(serde_json::to_string(chunk))))
    }

    fn make_chunk(session_id: SessionId, kind: StreamChunkKind) -> StreamChunk {
        StreamChunk {
            session_id,
            channel: "custom".to_string(),
            reply_to: None,
            kind,
        }
    }

    #[tokio::test]
    async fn subscribe_and_receive_delta() {
        let reg = StreamRegistry::new();
        let sid = SessionId::new();

        let mut rx = reg.subscribe(sid);
        let chunk = make_chunk(sid, StreamChunkKind::Delta("hello".into()));
        let delivered = reg.send(chunk);

        assert_eq!(delivered, 1);
        let received = ok(rx.try_recv());
        assert!(matches!(received.kind, StreamChunkKind::Delta(ref s) if s == "hello"));
    }

    #[tokio::test]
    async fn no_subscribers_returns_zero() {
        let reg = StreamRegistry::new();
        let sid = SessionId::new();
        let chunk = make_chunk(sid, StreamChunkKind::Done);
        assert_eq!(reg.send(chunk), 0);
    }

    #[tokio::test]
    async fn disconnected_subscriber_is_pruned() {
        let reg = StreamRegistry::new();
        let sid = SessionId::new();

        let rx = reg.subscribe(sid);
        assert!(reg.has_subscribers(&sid));

        drop(rx);

        let chunk = make_chunk(sid, StreamChunkKind::Done);
        assert_eq!(reg.send(chunk), 0);
        assert!(!reg.has_subscribers(&sid));
    }

    #[tokio::test]
    async fn multiple_subscribers_all_receive() {
        let reg = StreamRegistry::new();
        let sid = SessionId::new();

        let mut rx1 = reg.subscribe(sid);
        let mut rx2 = reg.subscribe(sid);

        let chunk = make_chunk(sid, StreamChunkKind::Delta("x".into()));
        assert_eq!(reg.send(chunk), 2);

        assert!(rx1.try_recv().is_ok());
        assert!(rx2.try_recv().is_ok());
    }

    #[test]
    fn stream_chunk_kind_roundtrip_deserialization() {
        let variants: Vec<StreamChunkKind> = vec![
            StreamChunkKind::Delta("hello".into()),
            StreamChunkKind::ToolStart {
                name: "web_search".into(),
                id: "t1".into(),
            },
            StreamChunkKind::ToolEnd {
                id: "t1".into(),
                success: true,
            },
            StreamChunkKind::ToolExecStart {
                name: "echo".into(),
                id: "t2".into(),
                input_summary: None,
                category: None,
            },
            StreamChunkKind::ToolExecStart {
                name: "web_search".into(),
                id: "t4".into(),
                input_summary: Some("query: 'rust async'".into()),
                category: Some("search".into()),
            },
            StreamChunkKind::ToolExecEnd {
                id: "t2".into(),
                success: true,
                duration_ms: 1200,
                error: None,
                result_summary: None,
            },
            StreamChunkKind::ToolExecEnd {
                id: "t3".into(),
                success: false,
                duration_ms: 50,
                error: Some("fail".into()),
                result_summary: None,
            },
            StreamChunkKind::ToolExecEnd {
                id: "t5".into(),
                success: true,
                duration_ms: 3200,
                error: None,
                result_summary: Some("Found 3 results".into()),
            },
            StreamChunkKind::ThinkingDelta("I should think...".into()),
            StreamChunkKind::Usage {
                input_tokens: 100,
                output_tokens: 50,
                cache_read_tokens: Some(10),
                cache_creation_tokens: None,
                reasoning_tokens: Some(20),
                model: "claude-sonnet-4-6".into(),
                cost_usd: Some(0.001),
            },
            StreamChunkKind::AgentSwitch {
                agent_id: "agent-1".into(),
                display_name: "Research Agent".into(),
            },
            StreamChunkKind::ContextInfo {
                history_tokens: 5000,
                context_window: 200_000,
                messages_truncated: 3,
                summary_generated: false,
            },
            StreamChunkKind::PrinciplesUsed { count: 2 },
            StreamChunkKind::GenerationStarted,
            StreamChunkKind::Done,
        ];

        for variant in &variants {
            let json = ok(serde_json::to_string(variant));
            let back: StreamChunkKind = ok(serde_json::from_str(&json));
            let json2 = ok(serde_json::to_string(&back));
            assert_eq!(json, json2, "round-trip failed for {json}");
        }
    }

    #[test]
    fn stream_chunk_kind_json_serialization_delta() {
        let json = ok(serde_json::to_string(&StreamChunkKind::Delta("hi".into())));
        assert_eq!(json, r#"{"type":"Delta","data":"hi"}"#);
    }

    #[test]
    fn stream_chunk_kind_json_serialization_tool_events() {
        let v = json_value(&StreamChunkKind::ToolStart {
            name: "web_search".into(),
            id: "t1".into(),
        });
        assert_eq!(v["type"], "ToolStart");
        assert_eq!(v["data"]["name"], "web_search");

        let v = json_value(&StreamChunkKind::ToolEnd {
            id: "t1".into(),
            success: true,
        });
        assert_eq!(v["type"], "ToolEnd");
        assert_eq!(v["data"]["success"], true);
    }

    #[test]
    fn stream_chunk_kind_json_serialization_tool_exec_start() {
        let v = json_value(&StreamChunkKind::ToolExecStart {
            name: "echo".into(),
            id: "t2".into(),
            input_summary: None,
            category: None,
        });
        assert_eq!(v["type"], "ToolExecStart");
        assert!(v["data"]["input_summary"].is_null());
        assert!(v["data"]["category"].is_null());

        let v = json_value(&StreamChunkKind::ToolExecStart {
            name: "web_search".into(),
            id: "t10".into(),
            input_summary: Some("query: 'rust async'".into()),
            category: Some("search".into()),
        });
        assert_eq!(v["type"], "ToolExecStart");
        assert_eq!(v["data"]["input_summary"], "query: 'rust async'");
        assert_eq!(v["data"]["category"], "search");
    }

    #[test]
    fn stream_chunk_kind_json_serialization_tool_exec_end() {
        let v = json_value(&StreamChunkKind::ToolExecEnd {
            id: "t2".into(),
            success: true,
            duration_ms: 1200,
            error: None,
            result_summary: None,
        });
        assert_eq!(v["type"], "ToolExecEnd");
        assert_eq!(v["data"]["duration_ms"], 1200);
        assert!(v["data"]["error"].is_null());
        assert!(v["data"]["result_summary"].is_null());

        let v = json_value(&StreamChunkKind::ToolExecEnd {
            id: "t3".into(),
            success: false,
            duration_ms: 50,
            error: Some("Permission denied".into()),
            result_summary: None,
        });
        assert_eq!(v["type"], "ToolExecEnd");
        assert_eq!(v["data"]["success"], false);
        assert_eq!(v["data"]["error"], "Permission denied");

        let v = json_value(&StreamChunkKind::ToolExecEnd {
            id: "t11".into(),
            success: true,
            duration_ms: 800,
            error: None,
            result_summary: Some("Found 5 results".into()),
        });
        assert_eq!(v["data"]["result_summary"], "Found 5 results");
    }

    #[test]
    fn stream_chunk_kind_json_serialization_usage_and_agent_metadata() {
        let v = json_value(&StreamChunkKind::Usage {
            input_tokens: 100,
            output_tokens: 50,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            reasoning_tokens: None,
            model: "claude-sonnet-4-6".into(),
            cost_usd: None,
        });
        assert_eq!(v["type"], "Usage");
        assert_eq!(v["data"]["input_tokens"], 100);
        assert_eq!(v["data"]["output_tokens"], 50);
        assert_eq!(v["data"]["model"], "claude-sonnet-4-6");
        assert!(v["data"]["cache_read_tokens"].is_null());

        let v = json_value(&StreamChunkKind::AgentSwitch {
            agent_id: "a1".into(),
            display_name: "Research".into(),
        });
        assert_eq!(v["type"], "AgentSwitch");
        assert_eq!(v["data"]["agent_id"], "a1");
        assert_eq!(v["data"]["display_name"], "Research");

        let v = json_value(&StreamChunkKind::ContextInfo {
            history_tokens: 5000,
            context_window: 200_000,
            messages_truncated: 2,
            summary_generated: false,
        });
        assert_eq!(v["type"], "ContextInfo");
        assert_eq!(v["data"]["history_tokens"], 5000);
        assert_eq!(v["data"]["messages_truncated"], 2);
        assert_eq!(v["data"]["summary_generated"], false);

        let v = json_value(&StreamChunkKind::PrinciplesUsed { count: 3 });
        assert_eq!(v["type"], "PrinciplesUsed");
        assert_eq!(v["data"]["count"], 3);
    }

    #[test]
    fn stream_chunk_kind_json_serialization_done() {
        let json = ok(serde_json::to_string(&StreamChunkKind::Done));
        assert_eq!(json, r#"{"type":"Done"}"#);
    }

    #[test]
    fn stream_chunk_kind_json_serialization_generation_started() {
        let json = ok(serde_json::to_string(&StreamChunkKind::GenerationStarted));
        assert_eq!(json, r#"{"type":"GenerationStarted"}"#);
    }
}
