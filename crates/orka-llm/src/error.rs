//! Error types for the LLM subsystem.
//!
//! [`LlmError`] provides structured error categories for LLM provider failures.
//! All variants convert into [`orka_core::Error::Llm`] via `From`.

use thiserror::Error;

/// Structured error type for LLM provider communication failures.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum LlmError {
    /// Network-level failure (connection refused, DNS resolution, etc.).
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    /// Provider rejected the request due to an invalid or expired API key.
    #[error("authentication error: {0}")]
    Auth(String),

    /// Provider is rate-limiting or the account quota is exhausted.
    #[error("rate limit exceeded: {0}")]
    RateLimit(String),

    /// The input exceeds the model's context window.
    #[error("context window exceeded: {0}")]
    ContextWindow(String),

    /// The provider returned a non-2xx response with a status code.
    #[error("provider error {status}: {message}")]
    Provider {
        /// HTTP status code from the provider.
        status: u16,
        /// Human-readable error message from the provider response body.
        message: String,
    },

    /// Failed to parse or deserialize the provider's response.
    #[error("parse error: {0}")]
    Parse(String),

    /// The streaming response was interrupted unexpectedly.
    #[error("stream error: {0}")]
    Stream(String),

    /// Catch-all for unclassified LLM errors.
    #[error("{0}")]
    Other(String),
}

impl From<LlmError> for orka_core::Error {
    /// Convert an [`LlmError`] into the unified [`orka_core::Error::Llm`] variant.
    ///
    /// # Examples
    ///
    /// ```
    /// use orka_llm::error::LlmError;
    /// use orka_core::Error;
    ///
    /// let llm_err = LlmError::Auth("invalid API key".into());
    /// let core_err: Error = llm_err.into();
    /// assert!(core_err.to_string().contains("llm error"));
    /// ```
    fn from(e: LlmError) -> Self {
        let msg = e.to_string();
        orka_core::Error::llm_msg(msg)
    }
}
