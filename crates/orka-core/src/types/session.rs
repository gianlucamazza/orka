use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::ids::SessionId;

/// A stored session with associated state.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[non_exhaustive]
pub struct Session {
    /// Unique session identifier.
    pub id: SessionId,
    /// Channel this session is associated with.
    pub channel: String,
    /// Platform-specific user identifier.
    pub user_id: String,
    /// When the session was first opened.
    pub created_at: DateTime<Utc>,
    /// When the session was last modified.
    pub updated_at: DateTime<Utc>,
    /// Arbitrary key-value scratchpad for handler and skill state.
    pub state: HashMap<String, serde_json::Value>,
}

impl Session {
    /// Create a new session for the given channel and user.
    pub fn new(channel: impl Into<String>, user_id: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: SessionId::new(),
            channel: channel.into(),
            user_id: user_id.into(),
            created_at: now,
            updated_at: now,
            state: HashMap::new(),
        }
    }

    /// Read a value from the shared scratchpad.
    pub fn scratchpad_get(&self, key: &str) -> Option<&serde_json::Value> {
        self.state.get("scratchpad").and_then(|sp| sp.get(key))
    }

    /// Write a value to the shared scratchpad.
    pub fn scratchpad_set(&mut self, key: impl Into<String>, value: serde_json::Value) {
        let scratchpad = self
            .state
            .entry("scratchpad".to_string())
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        if let serde_json::Value::Object(map) = scratchpad {
            map.insert(key.into(), value);
        }
    }
}
