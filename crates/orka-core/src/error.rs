use thiserror::Error;

/// Unified error type for the Orka platform.
///
/// Each variant corresponds to a subsystem; structured variants carry
/// a boxed source error for chaining and a human-readable context string.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// Configuration parsing or validation failure.
    #[error("configuration error: {0}")]
    Config(String),

    /// JSON serialization/deserialization error.
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// Error from a specific channel adapter.
    #[error("channel error: {channel}: {message}")]
    Channel {
        /// Channel identifier where the error occurred.
        channel: String,
        /// Human-readable error description.
        message: String,
    },

    /// No session exists for the given ID.
    #[error("session not found: {0}")]
    SessionNotFound(crate::SessionId),

    /// Message bus operation failure.
    #[error("bus error: {context}")]
    Bus {
        /// Root cause.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
        /// Human-readable context for the failure.
        context: String,
    },

    /// Authentication or authorization failure.
    #[error("auth error: {0}")]
    Auth(String),

    /// Priority queue operation failure.
    #[error("queue error: {context}")]
    Queue {
        /// Root cause.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
        /// Human-readable context for the failure.
        context: String,
    },

    /// Worker processing failure.
    #[error("worker error: {0}")]
    Worker(String),

    /// Sandbox execution failure.
    #[error("sandbox error: {0}")]
    Sandbox(String),

    /// Observability subsystem error.
    #[error("observe error: {0}")]
    Observe(String),

    /// Skill execution error.
    #[error("skill error: {0}")]
    Skill(String),

    /// Memory store operation failure.
    #[error("memory error: {context}")]
    Memory {
        /// Root cause.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
        /// Human-readable context for the failure.
        context: String,
    },

    /// Secret retrieval or storage failure.
    #[error("secret error: {context}")]
    Secret {
        /// Root cause.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
        /// Human-readable context for the failure.
        context: String,
    },

    /// Workspace management error.
    #[error("workspace error: {0}")]
    Workspace(String),

    /// Gateway routing or filtering error.
    #[error("gateway error: {0}")]
    Gateway(String),

    /// Knowledge/RAG subsystem error.
    #[error("knowledge error: {0}")]
    Knowledge(String),

    /// Task scheduler error.
    #[error("scheduler error: {0}")]
    Scheduler(String),

    /// HTTP client request failure.
    #[error("http client error: {0}")]
    HttpClient(String),

    /// LLM provider communication error.
    #[error("llm error: {0}")]
    Llm(String),

    /// Channel adapter error.
    #[error("adapter error: {context}")]
    Adapter {
        /// Root cause.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
        /// Human-readable context for the failure.
        context: String,
    },

    /// Config migration error.
    #[error("config migration error: {0}")]
    Migration(#[from] crate::migrate::MigrationError),

    /// I/O error.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// Catch-all for unclassified errors.
    #[error("{0}")]
    Other(String),
}

/// Simple string-based error for use as a boxed source.
#[derive(Debug)]
struct SimpleError(String);

impl std::fmt::Display for SimpleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SimpleError {}

impl Error {
    /// Create a bus error from a message string.
    pub fn bus(msg: impl Into<String>) -> Self {
        let s = msg.into();
        Self::Bus {
            source: Box::new(SimpleError(s.clone())),
            context: s,
        }
    }

    /// Create a queue error from a message string.
    pub fn queue(msg: impl Into<String>) -> Self {
        let s = msg.into();
        Self::Queue {
            source: Box::new(SimpleError(s.clone())),
            context: s,
        }
    }

    /// Create a memory error from a message string.
    pub fn memory(msg: impl Into<String>) -> Self {
        let s = msg.into();
        Self::Memory {
            source: Box::new(SimpleError(s.clone())),
            context: s,
        }
    }

    /// Create a secret error from a message string.
    pub fn secret(msg: impl Into<String>) -> Self {
        let s = msg.into();
        Self::Secret {
            source: Box::new(SimpleError(s.clone())),
            context: s,
        }
    }

    /// Create an adapter error from a message string.
    pub fn adapter(msg: impl Into<String>) -> Self {
        let s = msg.into();
        Self::Adapter {
            source: Box::new(SimpleError(s.clone())),
            context: s,
        }
    }
}

/// Convenience alias used throughout the Orka crate ecosystem.
pub type Result<T> = std::result::Result<T, Error>;
