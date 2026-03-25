//! Error types for the `orka-git` crate.

use thiserror::Error;

/// Git skill error.
#[derive(Debug, Error)]
pub enum GitError {
    /// A git CLI command failed or was rejected by the guard.
    #[error("git: {0}")]
    Command(String),

    /// The repository could not be discovered at the given path.
    #[error("no git repository found at '{path}': {source}")]
    Discover {
        /// Path that was searched.
        path: String,
        /// Underlying error.
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// A policy check blocked the operation.
    #[error("policy: {0}")]
    Policy(String),

    /// A required argument was missing or invalid.
    #[error("invalid argument: {0}")]
    InvalidArg(String),

    /// A git CLI command timed out.
    #[error("git command timed out after {secs}s")]
    Timeout {
        /// Timeout duration in seconds.
        secs: u64,
    },

    /// I/O error during worktree setup.
    #[error("worktree I/O: {0}")]
    Io(#[from] std::io::Error),

    /// gix internal error.
    #[error("gix: {0}")]
    Gix(String),
}

impl GitError {
    /// Map to an `orka_core::ErrorCategory`.
    pub fn category(&self) -> orka_core::ErrorCategory {
        match self {
            Self::Policy(_) | Self::InvalidArg(_) => orka_core::ErrorCategory::Input,
            Self::Timeout { .. } => orka_core::ErrorCategory::Timeout,
            Self::Discover { .. } | Self::Io(_) => orka_core::ErrorCategory::Environmental,
            Self::Command(_) | Self::Gix(_) => orka_core::ErrorCategory::Unknown,
        }
    }
}

impl From<GitError> for orka_core::Error {
    fn from(e: GitError) -> Self {
        Self::SkillCategorized {
            message: e.to_string(),
            category: e.category(),
        }
    }
}
