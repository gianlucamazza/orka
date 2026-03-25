/// Standard JSON-RPC 2.0 error code: parse error.
pub const ERR_PARSE_ERROR: i32 = -32700;
/// Standard JSON-RPC 2.0 error code: invalid request.
pub const ERR_INVALID_REQUEST: i32 = -32600;
/// Standard JSON-RPC 2.0 error code: method not found.
pub const ERR_METHOD_NOT_FOUND: i32 = -32601;
/// Standard JSON-RPC 2.0 error code: invalid params.
pub const ERR_INVALID_PARAMS: i32 = -32602;
/// Standard JSON-RPC 2.0 error code: internal error.
pub const ERR_INTERNAL_ERROR: i32 = -32603;

/// A2A v1.0 error code: task not found.
pub const ERR_TASK_NOT_FOUND: i32 = -32001;
/// A2A v1.0 error code: task cannot be canceled in its current state.
pub const ERR_TASK_NOT_CANCELABLE: i32 = -32002;
/// A2A v1.0 error code: push notifications not supported.
pub const ERR_PUSH_NOTIFICATION_NOT_SUPPORTED: i32 = -32003;
/// A2A v1.0 error code: operation not supported by this agent.
pub const ERR_UNSUPPORTED_OPERATION: i32 = -32004;
/// A2A v1.0 error code: content type not supported.
pub const ERR_CONTENT_TYPE_NOT_SUPPORTED: i32 = -32005;
/// A2A v1.0 error code: invalid agent response.
pub const ERR_INVALID_AGENT_RESPONSE: i32 = -32006;

/// Typed A2A error, carrying both a numeric code and a human-readable message.
#[derive(Debug, thiserror::Error)]
pub enum A2aError {
    // ── A2A-specific ─────────────────────────────────────────────────────────
    /// The requested task does not exist.
    #[error("task not found")]
    TaskNotFound,

    /// The task is in a terminal state and cannot be canceled.
    #[error("task cannot be canceled in its current state")]
    TaskNotCancelable,

    /// This agent does not support push notifications.
    #[error("push notifications are not supported by this agent")]
    PushNotificationNotSupported,

    /// The requested operation is not supported by this agent.
    #[error("operation not supported")]
    UnsupportedOperation,

    /// The content type in the request is not supported.
    #[error("content type not supported")]
    ContentTypeNotSupported,

    /// The agent returned a response that could not be interpreted.
    #[error("invalid agent response")]
    InvalidAgentResponse,

    // ── JSON-RPC standard ────────────────────────────────────────────────────
    /// The requested JSON-RPC method does not exist.
    #[error("method not found: {0}")]
    MethodNotFound(String),

    /// The request parameters are invalid or missing a required field.
    #[error("invalid params: {0}")]
    InvalidParams(String),

    /// An unexpected internal error occurred.
    #[error("internal error: {0}")]
    Internal(String),
}

impl A2aError {
    /// Returns the JSON-RPC error code for this error variant.
    pub fn code(&self) -> i32 {
        match self {
            A2aError::TaskNotFound => ERR_TASK_NOT_FOUND,
            A2aError::TaskNotCancelable => ERR_TASK_NOT_CANCELABLE,
            A2aError::PushNotificationNotSupported => ERR_PUSH_NOTIFICATION_NOT_SUPPORTED,
            A2aError::UnsupportedOperation => ERR_UNSUPPORTED_OPERATION,
            A2aError::ContentTypeNotSupported => ERR_CONTENT_TYPE_NOT_SUPPORTED,
            A2aError::InvalidAgentResponse => ERR_INVALID_AGENT_RESPONSE,
            A2aError::MethodNotFound(_) => ERR_METHOD_NOT_FOUND,
            A2aError::InvalidParams(_) => ERR_INVALID_PARAMS,
            A2aError::Internal(_) => ERR_INTERNAL_ERROR,
        }
    }
}
