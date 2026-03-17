//! Streaming infrastructure for real-time LLM response delivery.
//!
//! [`StreamRegistry`] routes [`StreamChunk`]s to WebSocket subscribers keyed by session ID.
//! The worker emits chunks as the LLM streams tokens; adapters subscribe to forward them.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, mpsc};

use crate::{MessageId, SessionId};

/// A chunk of streaming data from an LLM response.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
        /// Tool category tag (`"search"`, `"code"`, `"http"`, `"memory"`, `"schedule"`).
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
    /// The stream is complete.
    Done,
}

/// Registry that maps session IDs to stream chunk senders.
/// Used to route streaming LLM output to WebSocket connections.
#[derive(Clone, Default)]
pub struct StreamRegistry {
    inner: Arc<Mutex<HashMap<SessionId, Vec<mpsc::UnboundedSender<StreamChunk>>>>>,
}

impl StreamRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Subscribe to stream chunks for a session. Returns a receiver that yields chunks.
    pub async fn subscribe(&self, session_id: SessionId) -> mpsc::UnboundedReceiver<StreamChunk> {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut map = self.inner.lock().await;
        map.entry(session_id).or_default().push(tx);
        rx
    }

    /// Send a chunk to all subscribers of the chunk's session. Returns the number of
    /// subscribers that received the chunk. Disconnected senders are pruned automatically.
    pub async fn send(&self, chunk: &StreamChunk) -> usize {
        let mut map = self.inner.lock().await;
        let senders = match map.get_mut(&chunk.session_id) {
            Some(s) => s,
            None => return 0,
        };

        let mut delivered = 0;
        senders.retain(|tx| match tx.send(chunk.clone()) {
            Ok(()) => {
                delivered += 1;
                true
            }
            Err(_) => false,
        });

        if senders.is_empty() {
            map.remove(&chunk.session_id);
        }

        delivered
    }

    /// Check whether any subscribers exist for a session.
    pub async fn has_subscribers(&self, session_id: &SessionId) -> bool {
        let map = self.inner.lock().await;
        map.contains_key(session_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

        let mut rx = reg.subscribe(sid.clone()).await;
        let chunk = make_chunk(sid.clone(), StreamChunkKind::Delta("hello".into()));
        let delivered = reg.send(&chunk).await;

        assert_eq!(delivered, 1);
        let received = rx.try_recv().unwrap();
        assert!(matches!(received.kind, StreamChunkKind::Delta(ref s) if s == "hello"));
    }

    #[tokio::test]
    async fn no_subscribers_returns_zero() {
        let reg = StreamRegistry::new();
        let sid = SessionId::new();
        let chunk = make_chunk(sid, StreamChunkKind::Done);
        assert_eq!(reg.send(&chunk).await, 0);
    }

    #[tokio::test]
    async fn disconnected_subscriber_is_pruned() {
        let reg = StreamRegistry::new();
        let sid = SessionId::new();

        let rx = reg.subscribe(sid.clone()).await;
        assert!(reg.has_subscribers(&sid).await);

        drop(rx);

        let chunk = make_chunk(sid.clone(), StreamChunkKind::Done);
        assert_eq!(reg.send(&chunk).await, 0);
        assert!(!reg.has_subscribers(&sid).await);
    }

    #[tokio::test]
    async fn multiple_subscribers_all_receive() {
        let reg = StreamRegistry::new();
        let sid = SessionId::new();

        let mut rx1 = reg.subscribe(sid.clone()).await;
        let mut rx2 = reg.subscribe(sid.clone()).await;

        let chunk = make_chunk(sid.clone(), StreamChunkKind::Delta("x".into()));
        assert_eq!(reg.send(&chunk).await, 2);

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
            StreamChunkKind::Done,
        ];

        for variant in &variants {
            let json = serde_json::to_string(variant).unwrap();
            let back: StreamChunkKind = serde_json::from_str(&json).unwrap();
            let json2 = serde_json::to_string(&back).unwrap();
            assert_eq!(json, json2, "round-trip failed for {json}");
        }
    }

    #[test]
    fn stream_chunk_kind_json_serialization() {
        // Delta
        let json = serde_json::to_string(&StreamChunkKind::Delta("hi".into())).unwrap();
        assert_eq!(json, r#"{"type":"Delta","data":"hi"}"#);

        // ToolStart
        let json = serde_json::to_string(&StreamChunkKind::ToolStart {
            name: "web_search".into(),
            id: "t1".into(),
        })
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "ToolStart");
        assert_eq!(v["data"]["name"], "web_search");

        // ToolEnd
        let json = serde_json::to_string(&StreamChunkKind::ToolEnd {
            id: "t1".into(),
            success: true,
        })
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "ToolEnd");
        assert_eq!(v["data"]["success"], true);

        // ToolExecStart (no optional fields)
        let json = serde_json::to_string(&StreamChunkKind::ToolExecStart {
            name: "echo".into(),
            id: "t2".into(),
            input_summary: None,
            category: None,
        })
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "ToolExecStart");
        assert!(v["data"]["input_summary"].is_null());
        assert!(v["data"]["category"].is_null());

        // ToolExecStart (with optional fields)
        let json = serde_json::to_string(&StreamChunkKind::ToolExecStart {
            name: "web_search".into(),
            id: "t10".into(),
            input_summary: Some("query: 'rust async'".into()),
            category: Some("search".into()),
        })
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "ToolExecStart");
        assert_eq!(v["data"]["input_summary"], "query: 'rust async'");
        assert_eq!(v["data"]["category"], "search");

        // ToolExecEnd (success — optional fields omitted from JSON)
        let json = serde_json::to_string(&StreamChunkKind::ToolExecEnd {
            id: "t2".into(),
            success: true,
            duration_ms: 1200,
            error: None,
            result_summary: None,
        })
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "ToolExecEnd");
        assert_eq!(v["data"]["duration_ms"], 1200);
        assert!(v["data"]["error"].is_null());
        assert!(v["data"]["result_summary"].is_null());

        // ToolExecEnd (failure — error included)
        let json = serde_json::to_string(&StreamChunkKind::ToolExecEnd {
            id: "t3".into(),
            success: false,
            duration_ms: 50,
            error: Some("Permission denied".into()),
            result_summary: None,
        })
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "ToolExecEnd");
        assert_eq!(v["data"]["success"], false);
        assert_eq!(v["data"]["error"], "Permission denied");

        // ToolExecEnd (with result_summary)
        let json = serde_json::to_string(&StreamChunkKind::ToolExecEnd {
            id: "t11".into(),
            success: true,
            duration_ms: 800,
            error: None,
            result_summary: Some("Found 5 results".into()),
        })
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["data"]["result_summary"], "Found 5 results");

        // Done
        let json = serde_json::to_string(&StreamChunkKind::Done).unwrap();
        assert_eq!(json, r#"{"type":"Done"}"#);
    }
}
