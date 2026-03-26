use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{A2aError, ERR_INVALID_REQUEST};

/// Incoming JSON-RPC 2.0 request envelope.
#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    /// Must be `"2.0"`.
    #[allow(dead_code)]
    pub jsonrpc: String,
    /// Request identifier — echoed back in the response.
    pub id: Option<Value>,
    /// Method name (e.g. `"message/send"`).
    pub method: String,
    /// Method parameters.
    #[serde(default)]
    pub params: Value,
    /// Optional multi-tenancy scope (A2A v1.0).
    #[serde(default)]
    pub tenant: Option<String>,
}

/// Outgoing JSON-RPC 2.0 response envelope.
#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    jsonrpc: &'static str,
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcErrorBody>,
}

impl JsonRpcResponse {
    /// Successful response.
    pub fn ok(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    /// Error response from an [`A2aError`].
    pub fn from_error(id: Option<Value>, err: &A2aError) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(JsonRpcErrorBody {
                code: err.code(),
                message: err.to_string(),
            }),
        }
    }

    /// Error response from a raw code and message (for parse / invalid-request
    /// errors).
    pub fn raw_error(id: Option<Value>, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(JsonRpcErrorBody {
                code,
                message: message.into(),
            }),
        }
    }
}

#[derive(Debug, Serialize)]
struct JsonRpcErrorBody {
    code: i32,
    message: String,
}

/// Try to parse a raw JSON value into a [`JsonRpcRequest`], returning a
/// well-formed JSON-RPC error response on failure.
///
/// Also validates that `jsonrpc == "2.0"` per the JSON-RPC 2.0 specification.
pub fn parse_request(raw: Value) -> Result<JsonRpcRequest, JsonRpcResponse> {
    let id = raw.get("id").cloned();
    let req: JsonRpcRequest = serde_json::from_value(raw).map_err(|e| {
        JsonRpcResponse::raw_error(
            id.clone(),
            ERR_INVALID_REQUEST,
            format!("invalid request: {e}"),
        )
    })?;
    if req.jsonrpc != "2.0" {
        return Err(JsonRpcResponse::raw_error(
            id,
            ERR_INVALID_REQUEST,
            format!(
                "invalid jsonrpc version: expected \"2.0\", got \"{}\"",
                req.jsonrpc
            ),
        ));
    }
    Ok(req)
}
