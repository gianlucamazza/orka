use thiserror::Error;

/// Unified error type for the Orka platform.
///
/// Each variant corresponds to a subsystem; structured variants carry
/// a boxed source error for chaining and a human-readable context string.
#[derive(Debug, Error)]
#[non_exhaustive]
#[allow(missing_docs)]
pub enum Error {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("channel error: {channel}: {message}")]
    Channel { channel: String, message: String },

    #[error("session not found: {0}")]
    SessionNotFound(crate::SessionId),

    #[error("bus error: {context}")]
    Bus {
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
        context: String,
    },

    #[error("auth error: {0}")]
    Auth(String),

    #[error("queue error: {context}")]
    Queue {
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
        context: String,
    },

    #[error("worker error: {0}")]
    Worker(String),

    #[error("sandbox error: {0}")]
    Sandbox(String),

    #[error("observe error: {0}")]
    Observe(String),

    #[error("skill error: {0}")]
    Skill(String),

    #[error("memory error: {context}")]
    Memory {
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
        context: String,
    },

    #[error("secret error: {context}")]
    Secret {
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
        context: String,
    },

    #[error("workspace error: {0}")]
    Workspace(String),

    #[error("gateway error: {0}")]
    Gateway(String),

    #[error("knowledge error: {0}")]
    Knowledge(String),

    #[error("scheduler error: {0}")]
    Scheduler(String),

    #[error("http client error: {0}")]
    HttpClient(String),

    #[error("llm error: {0}")]
    Llm(String),

    #[error("adapter error: {context}")]
    Adapter {
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
        context: String,
    },

    #[error("config migration error: {0}")]
    Migration(#[from] crate::migrate::MigrationError),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

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

#[allow(missing_docs)]
impl Error {
    /// Create a bus error from a message string.
    pub fn bus(msg: impl Into<String>) -> Self {
        let s = msg.into();
        Self::Bus {
            source: Box::new(SimpleError(s.clone())),
            context: s,
        }
    }

    pub fn queue(msg: impl Into<String>) -> Self {
        let s = msg.into();
        Self::Queue {
            source: Box::new(SimpleError(s.clone())),
            context: s,
        }
    }

    pub fn memory(msg: impl Into<String>) -> Self {
        let s = msg.into();
        Self::Memory {
            source: Box::new(SimpleError(s.clone())),
            context: s,
        }
    }

    pub fn secret(msg: impl Into<String>) -> Self {
        let s = msg.into();
        Self::Secret {
            source: Box::new(SimpleError(s.clone())),
            context: s,
        }
    }

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
