use thiserror::Error;

use crate::types::ErrorCategory;

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
    #[error("worker error: {context}")]
    Worker {
        /// Root cause.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
        /// Human-readable context for the failure.
        context: String,
    },

    /// Sandbox execution failure.
    #[error("sandbox error: {context}")]
    Sandbox {
        /// Root cause.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
        /// Human-readable context for the failure.
        context: String,
    },

    /// Observability subsystem error.
    #[error("observe error: {context}")]
    Observe {
        /// Root cause.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
        /// Human-readable context for the failure.
        context: String,
    },

    /// Skill execution error.
    #[error("skill error: {0}")]
    Skill(String),

    /// Skill error with error categorization for circuit breaker decisions.
    #[error("skill error ({category:?}): {message}")]
    SkillCategorized {
        /// Human-readable error description.
        message: String,
        /// Error category for circuit breaker decisions.
        category: ErrorCategory,
    },

    /// Guardrail check failure.
    #[error("guardrail error: {0}")]
    Guardrail(String),

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
    #[error("http client error: {context}")]
    HttpClient {
        /// Root cause.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
        /// Human-readable context for the failure.
        context: String,
    },

    /// LLM provider communication error.
    #[error("llm error: {context}")]
    Llm {
        /// Root cause.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
        /// Human-readable context for the failure.
        context: String,
    },

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
    #[cfg(feature = "migrate")]
    #[error("config migration error: {0}")]
    Migration(#[from] crate::migrate::MigrationError),

    /// I/O error.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// Checkpoint store operation failure (run history, Redis persistence).
    #[error("checkpoint error: {context}")]
    Checkpoint {
        /// Root cause.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
        /// Human-readable context for the failure.
        context: String,
    },

    /// Experience service failure (principle retrieval, trajectory storage).
    #[error("experience error: {context}")]
    Experience {
        /// Root cause.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
        /// Human-readable context for the failure.
        context: String,
    },

    /// Research campaign or service error.
    #[error("research error: {0}")]
    Research(String),

    /// Research resource (campaign, run, candidate) not found.
    #[error("research not found: {0}")]
    ResearchNotFound(String),

    /// Research operation conflicts with current state.
    #[error("research conflict: {0}")]
    ResearchConflict(String),

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
    /// Extract the error category, if available.
    ///
    /// # Examples
    ///
    /// ```
    /// use orka_core::{Error, types::ErrorCategory};
    ///
    /// let e = Error::Auth("token expired".into());
    /// assert_eq!(e.category(), ErrorCategory::Environmental);
    /// ```
    pub fn category(&self) -> ErrorCategory {
        match self {
            Error::SkillCategorized { category, .. } => *category,
            Error::Auth(_) => ErrorCategory::Environmental,
            _ => ErrorCategory::Unknown,
        }
    }

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

    /// Create a worker error from a source error and context.
    pub fn worker(
        source: impl std::error::Error + Send + Sync + 'static,
        context: impl Into<String>,
    ) -> Self {
        Self::Worker {
            source: Box::new(source),
            context: context.into(),
        }
    }

    /// Create a worker error from a plain message string.
    pub fn worker_msg(msg: impl Into<String>) -> Self {
        let s = msg.into();
        Self::Worker {
            source: Box::new(SimpleError(s.clone())),
            context: s,
        }
    }

    /// Create a sandbox error from a source error and context.
    ///
    /// # Examples
    ///
    /// ```
    /// use orka_core::Error;
    ///
    /// let io_err = std::io::Error::other("process killed");
    /// let e = Error::sandbox(io_err, "wasm execution failed");
    /// assert!(e.to_string().contains("sandbox error"));
    /// ```
    pub fn sandbox(
        source: impl std::error::Error + Send + Sync + 'static,
        context: impl Into<String>,
    ) -> Self {
        Self::Sandbox {
            source: Box::new(source),
            context: context.into(),
        }
    }

    /// Create a sandbox error from a plain message string.
    ///
    /// # Examples
    ///
    /// ```
    /// use orka_core::Error;
    ///
    /// let e = Error::sandbox_msg("permission denied");
    /// assert!(e.to_string().contains("sandbox error"));
    /// ```
    pub fn sandbox_msg(msg: impl Into<String>) -> Self {
        let s = msg.into();
        Self::Sandbox {
            source: Box::new(SimpleError(s.clone())),
            context: s,
        }
    }

    /// Create an observe error from a source error and context.
    pub fn observe(
        source: impl std::error::Error + Send + Sync + 'static,
        context: impl Into<String>,
    ) -> Self {
        Self::Observe {
            source: Box::new(source),
            context: context.into(),
        }
    }

    /// Create an observe error from a plain message string.
    pub fn observe_msg(msg: impl Into<String>) -> Self {
        let s = msg.into();
        Self::Observe {
            source: Box::new(SimpleError(s.clone())),
            context: s,
        }
    }

    /// Create an HTTP client error from a source error and context.
    pub fn http_client(
        source: impl std::error::Error + Send + Sync + 'static,
        context: impl Into<String>,
    ) -> Self {
        Self::HttpClient {
            source: Box::new(source),
            context: context.into(),
        }
    }

    /// Create an HTTP client error from a plain message string.
    pub fn http_client_msg(msg: impl Into<String>) -> Self {
        let s = msg.into();
        Self::HttpClient {
            source: Box::new(SimpleError(s.clone())),
            context: s,
        }
    }

    /// Create a checkpoint error from a source error and context.
    pub fn checkpoint(
        source: impl std::error::Error + Send + Sync + 'static,
        context: impl Into<String>,
    ) -> Self {
        Self::Checkpoint {
            source: Box::new(source),
            context: context.into(),
        }
    }

    /// Create a checkpoint error from a plain message string.
    pub fn checkpoint_msg(msg: impl Into<String>) -> Self {
        let s = msg.into();
        Self::Checkpoint {
            source: Box::new(SimpleError(s.clone())),
            context: s,
        }
    }

    /// Create an experience error from a source error and context.
    pub fn experience(
        source: impl std::error::Error + Send + Sync + 'static,
        context: impl Into<String>,
    ) -> Self {
        Self::Experience {
            source: Box::new(source),
            context: context.into(),
        }
    }

    /// Create an experience error from a plain message string.
    pub fn experience_msg(msg: impl Into<String>) -> Self {
        let s = msg.into();
        Self::Experience {
            source: Box::new(SimpleError(s.clone())),
            context: s,
        }
    }

    /// Create an LLM error from a source error and context.
    pub fn llm(
        source: impl std::error::Error + Send + Sync + 'static,
        context: impl Into<String>,
    ) -> Self {
        Self::Llm {
            source: Box::new(source),
            context: context.into(),
        }
    }

    /// Create an LLM error from a plain message string.
    pub fn llm_msg(msg: impl Into<String>) -> Self {
        let s = msg.into();
        Self::Llm {
            source: Box::new(SimpleError(s.clone())),
            context: s,
        }
    }
}

/// Convenience alias used throughout the Orka crate ecosystem.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_category_skill_categorized() {
        let e = Error::SkillCategorized {
            message: "timeout".into(),
            category: ErrorCategory::Timeout,
        };
        assert_eq!(e.category(), ErrorCategory::Timeout);
    }

    #[test]
    fn error_category_auth() {
        let e = Error::Auth("token expired".into());
        assert_eq!(e.category(), ErrorCategory::Environmental);
    }

    #[test]
    fn error_category_other() {
        let e = Error::Other("unknown".into());
        assert_eq!(e.category(), ErrorCategory::Unknown);
    }

    #[test]
    fn error_bus_factory() {
        let e = Error::bus("connection refused");
        assert!(e.to_string().contains("bus error"));
        assert!(e.to_string().contains("connection refused"));
    }

    #[test]
    fn error_sandbox_msg_factory() {
        let e = Error::sandbox_msg("permission denied");
        assert!(e.to_string().contains("sandbox error"));
        assert!(e.to_string().contains("permission denied"));
    }

    #[test]
    fn error_worker_msg_factory() {
        let e = Error::worker_msg("handler crashed");
        assert!(e.to_string().contains("worker error"));
        assert!(e.to_string().contains("handler crashed"));
    }
}
