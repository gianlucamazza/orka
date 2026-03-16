use std::collections::HashMap;
use std::sync::Arc;

use orka_core::{MessageId, SessionId};
use serde::Serialize;
use tokio::sync::{mpsc, Mutex};

/// A chunk of streaming data from an LLM response.
#[derive(Debug, Clone, Serialize)]
pub struct StreamChunk {
    pub session_id: SessionId,
    pub channel: String,
    pub reply_to: Option<MessageId>,
    pub kind: StreamChunkKind,
}

/// The kind of stream chunk.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "data")]
pub enum StreamChunkKind {
    Delta(String),
    ToolStart { name: String, id: String },
    ToolEnd { id: String, success: bool },
    Done,
}

/// Registry that maps session IDs to stream chunk senders.
/// Used to route streaming LLM output to WebSocket connections.
#[derive(Clone, Default)]
pub struct StreamRegistry {
    inner: Arc<Mutex<HashMap<SessionId, Vec<mpsc::UnboundedSender<StreamChunk>>>>>,
}

impl StreamRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn subscribe(&self, session_id: SessionId) -> mpsc::UnboundedReceiver<StreamChunk> {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut map = self.inner.lock().await;
        map.entry(session_id).or_default().push(tx);
        rx
    }

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

    pub async fn has_subscribers(&self, session_id: &SessionId) -> bool {
        let map = self.inner.lock().await;
        map.contains_key(session_id)
    }
}
