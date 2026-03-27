//! Progress bridge: forwards `coding_delegate` progress events to the
//! originating chat platform via the outbound message bus.
//!
//! When a coding task is invoked from Telegram, Discord, Slack, or WhatsApp,
//! the user would otherwise receive no feedback until the entire task
//! completes.  This bridge consumes [`DelegateEvent`]s emitted by the coding
//! backend, throttles them to avoid flooding the chat, and publishes plain-text
//! status updates as [`OutboundMessage`]s directly on the `"outbound"` bus
//! topic so the adapter can deliver them to the user in real time.

use std::sync::Arc;

use serde_json::Value;
use tokio::sync::mpsc;
use tracing::debug;

use crate::{Envelope, MessageId, SessionId, traits::MessageBus};

/// Configuration for the progress bridge.
#[derive(Debug, Clone)]
pub struct ProgressBridgeConfig {
    /// Minimum seconds between chat updates (throttle window).
    ///
    /// Tool-start events within this window are batched into a single message.
    /// `Result` and `Error` events always bypass the throttle.
    pub throttle_secs: u64,
}

impl Default for ProgressBridgeConfig {
    fn default() -> Self {
        Self { throttle_secs: 15 }
    }
}

/// Deserialise the `type` tag from a raw `DelegateEvent` JSON value without
/// pulling in a hard dependency on `orka-os`.
fn event_type(val: &Value) -> Option<&str> {
    val.get("type").and_then(Value::as_str)
}

/// Format a single significant `DelegateEvent` into a human-readable string.
///
/// Returns `None` for events that should be suppressed (e.g. `text_delta`,
/// `thinking_delta`, successful `tool_end`).
fn format_event(val: &Value) -> Option<String> {
    match event_type(val)? {
        "tool_start" => {
            let name = val.get("name").and_then(Value::as_str).unwrap_or("tool");
            let label = tool_label(name);
            Some(format!("{label}..."))
        }
        "tool_end" => {
            // Only surface failures; successes are too noisy.
            let success = val.get("success").and_then(Value::as_bool).unwrap_or(true);
            if success {
                None
            } else {
                Some("Tool execution failed.".to_string())
            }
        }
        "result" => Some("Task complete.".to_string()),
        "error" => {
            let msg = val
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown error");
            Some(format!("Error: {msg}"))
        }
        // text_delta, thinking_delta, usage — suppressed
        _ => None,
    }
}

/// Map a tool name to a short human-readable label.
fn tool_label(name: &str) -> &str {
    match name {
        "bash" | "shell" | "shell_exec" => "Running command",
        "read_file" | "Read" | "read" => "Reading file",
        "write_file" | "Write" | "write" => "Writing file",
        "edit_file" | "Edit" | "edit" => "Editing file",
        "list_directory" | "list" | "ls" => "Listing directory",
        "search" | "grep" | "Grep" => "Searching code",
        "glob" | "Glob" => "Finding files",
        _ => "Running tool",
    }
}

/// Build the outbound `Envelope` carrying a progress update back to the
/// originating chat channel.
fn make_progress_envelope(
    text: String,
    channel: &str,
    session_id: SessionId,
    metadata: &std::collections::HashMap<String, serde_json::Value>,
    reply_to: MessageId,
) -> Envelope {
    let mut env = Envelope::text(channel.to_string(), session_id, text);
    env.metadata.clone_from(metadata);
    // Keep reply threading so the platform can group the updates with the
    // original request.
    env.metadata.insert(
        "reply_to".to_string(),
        serde_json::Value::String(reply_to.to_string()),
    );
    env
}

/// Consume progress events from `rx` and forward significant ones as outbound
/// chat messages on the bus.
///
/// Designed to be spawned as a background task alongside a `coding_delegate`
/// skill invocation.  The task exits naturally when the sender side of `rx`
/// is dropped (i.e. when the skill completes or is cancelled).
pub async fn forward_progress_to_chat(
    mut rx: mpsc::UnboundedReceiver<Value>,
    bus: Arc<dyn MessageBus>,
    channel: String,
    session_id: SessionId,
    metadata: std::collections::HashMap<String, serde_json::Value>,
    reply_to: MessageId,
    config: ProgressBridgeConfig,
) {
    let mut last_sent = std::time::Instant::now()
        .checked_sub(std::time::Duration::from_secs(config.throttle_secs + 1))
        .unwrap_or_else(std::time::Instant::now);
    let throttle = std::time::Duration::from_secs(config.throttle_secs);

    // Buffer tool_start labels received within the throttle window.
    let mut pending: Vec<String> = Vec::new();

    while let Some(val) = rx.recv().await {
        let Some(text) = format_event(&val) else {
            continue;
        };

        let is_terminal = matches!(event_type(&val), Some("result") | Some("error"));

        if is_terminal {
            // Flush any buffered pending messages plus this terminal one.
            let combined = if pending.is_empty() {
                text
            } else {
                let mut parts = std::mem::take(&mut pending);
                parts.push(text);
                parts.join("\n")
            };
            let env = make_progress_envelope(combined, &channel, session_id, &metadata, reply_to);
            if let Err(e) = bus.publish("outbound", &env).await {
                debug!(%e, "progress_bridge: failed to publish terminal event");
            }
            return;
        }

        // Accumulate into pending buffer.
        pending.push(text);

        // Flush if throttle window has elapsed.
        let now = std::time::Instant::now();
        if !pending.is_empty() && now.duration_since(last_sent) >= throttle {
            let combined = pending.join("\n");
            pending.clear();
            last_sent = now;
            let env = make_progress_envelope(combined, &channel, session_id, &metadata, reply_to);
            if let Err(e) = bus.publish("outbound", &env).await {
                debug!(%e, "progress_bridge: failed to publish progress event");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_tool_start_bash() {
        let val = serde_json::json!({"type": "tool_start", "name": "bash", "id": "x"});
        assert_eq!(format_event(&val), Some("Running command...".to_string()));
    }

    #[test]
    fn format_tool_end_success_suppressed() {
        let val = serde_json::json!({"type": "tool_end", "id": "x", "success": true});
        assert!(format_event(&val).is_none());
    }

    #[test]
    fn format_tool_end_failure() {
        let val = serde_json::json!({"type": "tool_end", "id": "x", "success": false});
        assert_eq!(
            format_event(&val),
            Some("Tool execution failed.".to_string())
        );
    }

    #[test]
    fn format_result() {
        let val = serde_json::json!({"type": "result", "text": "done"});
        assert_eq!(format_event(&val), Some("Task complete.".to_string()));
    }

    #[test]
    fn format_error() {
        let val = serde_json::json!({"type": "error", "message": "timeout"});
        assert_eq!(format_event(&val), Some("Error: timeout".to_string()));
    }

    #[test]
    fn format_text_delta_suppressed() {
        let val = serde_json::json!({"type": "text_delta", "text": "hello"});
        assert!(format_event(&val).is_none());
    }

    #[test]
    fn format_unknown_tool_label() {
        let val = serde_json::json!({"type": "tool_start", "name": "qdrant_search", "id": "x"});
        assert_eq!(format_event(&val), Some("Running tool...".to_string()));
    }
}
