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
    /// Convert an [`LlmError`] into the unified [`orka_core::Error::Llm`]
    /// variant.
    ///
    /// # Examples
    ///
    /// ```
    /// use orka_core::Error;
    /// use orka_llm::error::LlmError;
    ///
    /// let llm_err = LlmError::Auth("invalid API key".into());
    /// let core_err: Error = llm_err.into();
    /// assert!(core_err.to_string().contains("llm error"));
    /// ```
    fn from(e: LlmError) -> Self {
        let context = e.to_string();
        orka_core::Error::llm(e, context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_error_display() {
        let e = LlmError::Auth("bad key".into());
        assert_eq!(e.to_string(), "authentication error: bad key");
    }

    #[test]
    fn rate_limit_error_display() {
        let e = LlmError::RateLimit("quota".into());
        assert_eq!(e.to_string(), "rate limit exceeded: quota");
    }

    #[test]
    fn context_window_error_display() {
        let e = LlmError::ContextWindow("too long".into());
        assert_eq!(e.to_string(), "context window exceeded: too long");
    }

    #[test]
    fn provider_error_display() {
        let e = LlmError::Provider {
            status: 500,
            message: "internal".into(),
        };
        assert_eq!(e.to_string(), "provider error 500: internal");
    }

    #[test]
    fn parse_and_stream_error_display() {
        assert!(
            LlmError::Parse("bad json".into())
                .to_string()
                .contains("parse error")
        );
        assert!(
            LlmError::Stream("cut off".into())
                .to_string()
                .contains("stream error")
        );
    }

    #[test]
    fn llm_error_converts_to_core_error() {
        let e: orka_core::Error = LlmError::Auth("x".into()).into();
        assert!(e.to_string().contains("llm error"));
    }
}
