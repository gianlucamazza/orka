use thiserror::Error;

/// Unified error type for the Orka platform.
#[derive(Debug, Error)]
pub enum Error {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("channel error: {channel}: {message}")]
    Channel { channel: String, message: String },

    #[error("session not found: {0}")]
    SessionNotFound(crate::SessionId),

    #[error("bus error: {0}")]
    Bus(String),

    #[error("auth error: {0}")]
    Auth(String),

    #[error("queue error: {0}")]
    Queue(String),

    #[error("worker error: {0}")]
    Worker(String),

    #[error("sandbox error: {0}")]
    Sandbox(String),

    #[error("memory error: {0}")]
    Memory(String),

    #[error("secret error: {0}")]
    Secret(String),

    #[error("workspace error: {0}")]
    Workspace(String),

    #[error("gateway error: {0}")]
    Gateway(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;
