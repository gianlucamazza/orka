//! Normalized event types and line parsers for coding-delegate backends.
//!
//! Both Claude Code (`--output-format stream-json`) and Codex (`--json`)
//! produce NDJSON on stdout. This module maps each backend's line format to a
//! common [`DelegateEvent`] vocabulary that the rest of the skill can handle
//! uniformly.

use serde::{Deserialize, Serialize};

/// A normalized progress event emitted by a coding-delegate backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum DelegateEvent {
    /// A text token from the assistant response.
    TextDelta { text: String },
    /// An extended-thinking token (Claude Code only).
    ThinkingDelta { text: String },
    /// The backend began calling a tool.
    ToolStart { name: String, id: String },
    /// The backend finished calling a tool.
    ToolEnd {
        id: String,
        success: bool,
        duration_ms: Option<u64>,
    },
    /// Token-usage snapshot.
    Usage {
        input_tokens: u32,
        output_tokens: u32,
    },
    /// Final result text — the delegated task is complete.
    Result { text: String },
    /// A non-fatal error reported by the backend.
    Error { message: String },
}

// ── Claude Code stream-json ──────────────────────────────────────────────────

/// Parse one line from `claude --output-format stream-json` stdout.
///
/// Returns `None` for unknown or irrelevant event types so the caller can
/// simply skip the line.
pub(crate) fn parse_claude_stream_line(line: &str) -> Option<DelegateEvent> {
    let val: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    match val.get("type")?.as_str()? {
        "assistant" => parse_claude_assistant(&val),
        "user" => parse_claude_tool_result(&val),
        "result" => {
            let text = val.get("result")?.as_str()?;
            Some(DelegateEvent::Result {
                text: text.to_string(),
            })
        }
        "system" if val.get("subtype").and_then(|s| s.as_str()) == Some("usage") => {
            let usage = val.get("usage")?;
            Some(DelegateEvent::Usage {
                input_tokens: usage.get("input_tokens")?.as_u64()? as u32,
                output_tokens: usage
                    .get("output_tokens")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0) as u32,
            })
        }
        _ => None,
    }
}

fn parse_claude_assistant(val: &serde_json::Value) -> Option<DelegateEvent> {
    let content = val
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_array())?;

    // Return the first recognisable block. Text deltas are most common; tool
    // starts and thinking deltas are returned on subsequent calls for the same
    // assistant message (caller iterates lines independently).
    for block in content {
        let block_type = block.get("type").and_then(serde_json::Value::as_str)?;
        match block_type {
            "text" => {
                let text = block.get("text").and_then(serde_json::Value::as_str)?;
                if !text.is_empty() {
                    return Some(DelegateEvent::TextDelta {
                        text: text.to_string(),
                    });
                }
            }
            "thinking" => {
                let text = block.get("thinking").and_then(serde_json::Value::as_str)?;
                if !text.is_empty() {
                    return Some(DelegateEvent::ThinkingDelta {
                        text: text.to_string(),
                    });
                }
            }
            "tool_use" => {
                let name = block.get("name").and_then(|n| n.as_str())?;
                let id = block.get("id").and_then(|i| i.as_str())?;
                return Some(DelegateEvent::ToolStart {
                    name: name.to_string(),
                    id: id.to_string(),
                });
            }
            _ => {}
        }
    }
    None
}

fn parse_claude_tool_result(val: &serde_json::Value) -> Option<DelegateEvent> {
    let content = val
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_array())?;

    for block in content {
        if block.get("type").and_then(serde_json::Value::as_str) == Some("tool_result") {
            let id = block.get("tool_use_id").and_then(|i| i.as_str())?;
            let is_error = block
                .get("is_error")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            return Some(DelegateEvent::ToolEnd {
                id: id.to_string(),
                success: !is_error,
                duration_ms: None,
            });
        }
    }
    None
}

// ── Codex --json NDJSON ──────────────────────────────────────────────────────

/// Parse one line from `codex exec --json` stdout.
///
/// Returns `None` for unknown or irrelevant event types.
pub(crate) fn parse_codex_stream_line(line: &str) -> Option<DelegateEvent> {
    let val: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    match val.get("type")?.as_str()? {
        "item.completed" => parse_codex_item(&val),
        "turn.completed" => {
            // Codex also sends a last_response_id here; we use this event as
            // the final result trigger by extracting the last assistant message
            // from the completed turn's items array.
            let items = val.get("items").and_then(|v| v.as_array())?;
            for item in items.iter().rev() {
                if item.get("role").and_then(serde_json::Value::as_str) == Some("assistant")
                    && let Some(text) = extract_codex_text(item.get("content")?)
                {
                    return Some(DelegateEvent::Result { text });
                }
            }
            None
        }
        "turn.usage" => Some(DelegateEvent::Usage {
            input_tokens: val
                .get("input_tokens")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0) as u32,
            output_tokens: val
                .get("output_tokens")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0) as u32,
        }),
        _ => None,
    }
}

fn parse_codex_item(val: &serde_json::Value) -> Option<DelegateEvent> {
    let item = val.get("item")?;
    match item.get("type")?.as_str()? {
        "message" if item.get("role").and_then(serde_json::Value::as_str) == Some("assistant") => {
            let content = item.get("content")?;
            let text = extract_codex_text(content)?;
            Some(DelegateEvent::TextDelta { text })
        }
        "tool_call" => {
            let name = item
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("unknown");
            let id = item.get("id").and_then(|i| i.as_str()).unwrap_or("");
            Some(DelegateEvent::ToolStart {
                name: name.to_string(),
                id: id.to_string(),
            })
        }
        "tool_call_output" => {
            let id = item.get("call_id").and_then(|i| i.as_str()).unwrap_or("");
            Some(DelegateEvent::ToolEnd {
                id: id.to_string(),
                success: true,
                duration_ms: None,
            })
        }
        _ => None,
    }
}

fn extract_codex_text(content: &serde_json::Value) -> Option<String> {
    match content {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Array(items) => {
            for item in items {
                if (item.get("type").and_then(serde_json::Value::as_str) == Some("output_text")
                    || item.get("type").and_then(serde_json::Value::as_str) == Some("text"))
                    && let Some(text) = item.get("text").and_then(serde_json::Value::as_str)
                {
                    return Some(text.to_string());
                }
            }
            None
        }
        serde_json::Value::Object(map) => map
            .get("text")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        _ => None,
    }
}

// ── OpenCode --format json NDJSON ────────────────────────────────────────────
//
// OpenCode emits one JSON object per line. The top-level `"type"` field
// identifies the event kind; the `"part"` object contains the payload.
//
// Real output captured from `opencode run --format json` (v1.3.0):
//   {"type":"step_start","part":{"type":"step-start",...}}
//   {"type":"text","part":{"type":"text","text":"...","time":{...}}}
//   {"type":"tool_use","part":{"type":"tool","callID":"...","tool":"bash",
//       "state":{"status":"completed","input":{...},"output":"..."},...}}
//   {"type":"step_finish","part":{"type":"step-finish","reason":"stop"|"
// tool-calls",       "cost":0.042,"tokens":{"total":N,"input":N,"output":N,...
// }}}
//
// Note: `tool_use` events are emitted with state already `"completed"` (no
// separate start/end pair). We emit a `ToolStart` for progress tracking only.
// The final result text comes from accumulated `TextDelta` events — no
// dedicated `Result` line is emitted, so the fallback parser
// (`parse_opencode_output`) extracts the result from the raw accumulated lines.

/// Parse one line from `opencode run --format json` stdout.
///
/// Returns `None` for unknown or irrelevant event types so the caller can
/// simply skip the line.
pub(crate) fn parse_opencode_stream_line(line: &str) -> Option<DelegateEvent> {
    let val: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    let part = val.get("part")?;

    match val.get("type")?.as_str()? {
        "text" => {
            let text = part.get("text").and_then(|t| t.as_str())?;
            if text.is_empty() {
                return None;
            }
            Some(DelegateEvent::TextDelta {
                text: text.to_string(),
            })
        }
        "tool_use" => {
            let name = part
                .get("tool")
                .and_then(|n| n.as_str())
                .unwrap_or("unknown");
            let id = part.get("callID").and_then(|i| i.as_str()).unwrap_or("");
            Some(DelegateEvent::ToolStart {
                name: name.to_string(),
                id: id.to_string(),
            })
        }
        "step_finish" => {
            // Emit Usage for token tracking; the actual result is extracted
            // by the fallback parser from accumulated TextDelta lines.
            let tokens = part.get("tokens")?;
            Some(DelegateEvent::Usage {
                input_tokens: tokens
                    .get("input")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0) as u32,
                output_tokens: tokens
                    .get("output")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0) as u32,
            })
        }
        // "step_start" and unknown types are skipped.
        _ => None,
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::field_reassign_with_default,
    clippy::default_trait_access,
    clippy::needless_pass_by_value,
    clippy::stable_sort_primitive
)]
mod tests {
    use super::*;

    // ── Claude Code ──────────────────────────────────────────────────────────

    #[test]
    fn claude_parses_result_line() {
        let line =
            r#"{"type":"result","subtype":"success","is_error":false,"result":"hello world"}"#;
        let event = parse_claude_stream_line(line).expect("should parse result");
        assert!(matches!(event, DelegateEvent::Result { text } if text == "hello world"));
    }

    #[test]
    fn claude_parses_assistant_text_delta() {
        let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"I will fix it."}]}}"#;
        let event = parse_claude_stream_line(line).expect("should parse text delta");
        assert!(matches!(event, DelegateEvent::TextDelta { text } if text == "I will fix it."));
    }

    #[test]
    fn claude_parses_tool_start() {
        let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_01","name":"bash","input":{"command":"cargo test"}}]}}"#;
        let event = parse_claude_stream_line(line).expect("should parse tool start");
        assert!(
            matches!(event, DelegateEvent::ToolStart { name, id } if name == "bash" && id == "toolu_01")
        );
    }

    #[test]
    fn claude_parses_tool_result() {
        let line = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_01","content":[{"type":"text","text":"ok"}],"is_error":false}]}}"#;
        let event = parse_claude_stream_line(line).expect("should parse tool result");
        assert!(
            matches!(event, DelegateEvent::ToolEnd { id, success, .. } if id == "toolu_01" && success)
        );
    }

    #[test]
    fn claude_skips_unknown_type() {
        let line = r#"{"type":"system","subtype":"init","session_id":"abc"}"#;
        assert!(parse_claude_stream_line(line).is_none());
    }

    #[test]
    fn claude_skips_malformed_json() {
        assert!(parse_claude_stream_line("not json").is_none());
    }

    // ── Codex ────────────────────────────────────────────────────────────────

    #[test]
    fn codex_parses_tool_call_item() {
        let line = r#"{"type":"item.completed","item":{"type":"tool_call","id":"call_01","name":"shell","parameters":{}}}"#;
        let event = parse_codex_stream_line(line).expect("should parse tool call");
        assert!(
            matches!(event, DelegateEvent::ToolStart { name, id } if name == "shell" && id == "call_01")
        );
    }

    #[test]
    fn codex_parses_turn_completed() {
        let line = r#"{"type":"turn.completed","items":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Task done."}]}]}"#;
        let event = parse_codex_stream_line(line).expect("should parse turn completed");
        assert!(matches!(event, DelegateEvent::Result { text } if text == "Task done."));
    }

    #[test]
    fn codex_skips_turn_started() {
        let line = r#"{"type":"turn.started"}"#;
        assert!(parse_codex_stream_line(line).is_none());
    }

    // ── OpenCode ─────────────────────────────────────────────────────────────

    #[test]
    fn opencode_parses_text_event() {
        let line = r#"{"type":"text","sessionID":"ses_abc","part":{"type":"text","text":"hello world","time":{"start":1234,"end":1234}}}"#;
        let event = parse_opencode_stream_line(line).expect("should parse text");
        assert!(matches!(event, DelegateEvent::TextDelta { text } if text == "hello world"));
    }

    #[test]
    fn opencode_parses_tool_use_event() {
        let line = r#"{"type":"tool_use","sessionID":"ses_abc","part":{"type":"tool","callID":"toolu_01","tool":"bash","state":{"status":"completed","input":{"command":"ls"},"output":"file.txt"}}}"#;
        let event = parse_opencode_stream_line(line).expect("should parse tool_use");
        assert!(
            matches!(event, DelegateEvent::ToolStart { name, id } if name == "bash" && id == "toolu_01")
        );
    }

    #[test]
    fn opencode_parses_step_finish_usage() {
        let line = r#"{"type":"step_finish","sessionID":"ses_abc","part":{"type":"step-finish","reason":"stop","cost":0.042,"tokens":{"total":100,"input":10,"output":90,"reasoning":0,"cache":{"read":0,"write":0}}}}"#;
        let event = parse_opencode_stream_line(line).expect("should parse step_finish");
        assert!(
            matches!(event, DelegateEvent::Usage { input_tokens, output_tokens } if input_tokens == 10 && output_tokens == 90)
        );
    }

    #[test]
    fn opencode_skips_step_start() {
        let line = r#"{"type":"step_start","sessionID":"ses_abc","part":{"type":"step-start","id":"prt_abc"}}"#;
        assert!(parse_opencode_stream_line(line).is_none());
    }

    #[test]
    fn opencode_skips_empty_text() {
        let line = r#"{"type":"text","part":{"type":"text","text":"","time":{}}}"#;
        assert!(parse_opencode_stream_line(line).is_none());
    }

    #[test]
    fn opencode_skips_malformed_json() {
        assert!(parse_opencode_stream_line("not json").is_none());
    }
}
