//! Error types for the evaluation framework.

use thiserror::Error;

/// Error type for evaluation operations.
#[derive(Debug, Error)]
pub enum EvalError {
    /// Glob pattern is invalid.
    #[error("invalid glob pattern: {0}")]
    Pattern(#[from] glob::PatternError),

    /// Filesystem I/O error.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// TOML deserialization error.
    #[error("failed to parse eval file: {0}")]
    Parse(#[from] toml::de::Error),
}

/// Convenience alias.
pub type EvalResult<T> = Result<T, EvalError>;
