use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Functional class of a memory record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MemoryKind {
    /// Active thread/session state and rolling conversation context.
    Working,
    /// Summaries, handoffs, and decisions distilled from completed turns.
    Episodic,
    /// Durable facts retrievable by semantic relevance.
    Semantic,
    /// Learned heuristics and principles.
    Procedural,
}

impl std::fmt::Display for MemoryKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Working => "working",
            Self::Episodic => "episodic",
            Self::Semantic => "semantic",
            Self::Procedural => "procedural",
        };
        f.write_str(s)
    }
}

/// Visibility and retention scope for a memory record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MemoryScope {
    /// Only valid for the current session/thread.
    Session,
    /// Shared across a workspace.
    Workspace,
    /// Shared across sessions for a single user.
    User,
    /// Globally applicable.
    Global,
}

impl std::fmt::Display for MemoryScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Session => "session",
            Self::Workspace => "workspace",
            Self::User => "user",
            Self::Global => "global",
        };
        f.write_str(s)
    }
}

impl std::str::FromStr for MemoryScope {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "session" => Ok(Self::Session),
            "workspace" => Ok(Self::Workspace),
            "user" => Ok(Self::User),
            "global" => Ok(Self::Global),
            _ => Err("invalid memory scope"),
        }
    }
}

/// An entry in the memory store.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[non_exhaustive]
pub struct MemoryEntry {
    /// Lookup key for this entry within the memory store.
    pub key: String,
    /// Functional class of this memory record.
    pub kind: MemoryKind,
    /// Visibility/retention scope for this record.
    pub scope: MemoryScope,
    /// Origin of the memory record (e.g. `system`, `user`, `experience`).
    pub source: String,
    /// Stored value as a JSON document.
    pub value: serde_json::Value,
    /// Structured metadata for filtering and introspection.
    pub metadata: HashMap<String, String>,
    /// When this entry was first written.
    pub created_at: DateTime<Utc>,
    /// When this entry was last modified.
    pub updated_at: DateTime<Utc>,
    /// Optional labels for grouping and filtering entries.
    pub tags: Vec<String>,
}

impl MemoryEntry {
    /// Create a new memory entry.
    pub fn new(key: impl Into<String>, value: serde_json::Value) -> Self {
        let now = Utc::now();
        Self {
            key: key.into(),
            kind: MemoryKind::Working,
            scope: MemoryScope::Session,
            source: "system".into(),
            value,
            metadata: HashMap::new(),
            created_at: now,
            updated_at: now,
            tags: Vec::new(),
        }
    }

    /// Create a working-memory entry.
    pub fn working(key: impl Into<String>, value: serde_json::Value) -> Self {
        Self::new(key, value).with_kind(MemoryKind::Working)
    }

    /// Create an episodic-memory entry.
    pub fn episodic(key: impl Into<String>, value: serde_json::Value) -> Self {
        Self::new(key, value).with_kind(MemoryKind::Episodic)
    }

    /// Create a semantic-memory entry.
    pub fn semantic(key: impl Into<String>, value: serde_json::Value) -> Self {
        Self::new(key, value).with_kind(MemoryKind::Semantic)
    }

    /// Create a procedural-memory entry.
    pub fn procedural(key: impl Into<String>, value: serde_json::Value) -> Self {
        Self::new(key, value).with_kind(MemoryKind::Procedural)
    }

    /// Set the memory kind for this entry.
    #[must_use]
    pub fn with_kind(mut self, kind: MemoryKind) -> Self {
        self.kind = kind;
        self
    }

    /// Set the memory scope for this entry.
    #[must_use]
    pub fn with_scope(mut self, scope: MemoryScope) -> Self {
        self.scope = scope;
        self
    }

    /// Set the source marker for this entry.
    #[must_use]
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = source.into();
        self
    }

    /// Replace the metadata map for this entry.
    #[must_use]
    pub fn with_metadata(mut self, metadata: HashMap<String, String>) -> Self {
        self.metadata = metadata;
        self
    }

    /// Set tags on this entry.
    #[must_use]
    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }
}
