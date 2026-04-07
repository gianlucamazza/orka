use orka_contracts::RealtimeEvent;
use orka_core::stream::AgentStopReason;

/// Classified WebSocket message.
#[derive(Debug)]
pub enum WsMessage {
    /// A streaming chunk (delta, tool event, done).
    Stream(RealtimeEvent),
    /// A final text message, with optional stop reason from execution metadata.
    Final(String, Option<AgentStopReason>),
    /// An inlined media attachment (e.g. a generated chart).
    Media {
        /// MIME type of the media (e.g. `image/png`).
        mime_type: String,
        /// Base64-encoded media bytes.
        data_base64: String,
        /// Optional caption.
        caption: Option<String>,
    },
    /// Unrecognized payload (fallback).
    Unknown(String),
}

/// Classify a raw WebSocket text frame into a typed message.
///
/// 1. Parse as `serde_json::Value` (single parse for the whole message)
/// 2. Try deserializing that value as `RealtimeEvent` (`snake_case` tagged)
/// 3. Try extracting text from `OutboundMessage` shape (`payload.data`)
/// 4. Fall back to `Unknown`
pub fn classify_ws_message(raw: &str) -> WsMessage {
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(raw) else {
        return WsMessage::Unknown(raw.to_string());
    };

    // Stream chunk: top-level { "type": "message_delta"|"stream_done"|... }
    if let Ok(event) = serde_json::from_value::<RealtimeEvent>(parsed.clone()) {
        return WsMessage::Stream(event);
    }

    // OutboundMessage shape: { payload: { type: "...", data: ... }, metadata: { ...
    // } }
    if let Some(payload) = parsed.get("payload") {
        let stop_reason = parsed
            .get("metadata")
            .and_then(|m| m.get("stop_reason"))
            .and_then(|v| serde_json::from_value::<AgentStopReason>(v.clone()).ok());

        match payload.get("type").and_then(|t| t.as_str()) {
            Some("Text") => {
                if let Some(text) = payload.get("data").and_then(|d| d.as_str()) {
                    return WsMessage::Final(text.to_string(), stop_reason);
                }
            }
            Some("Media") => {
                if let Some(media) = payload.get("data") {
                    let mime_type = media
                        .get("mime_type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("application/octet-stream")
                        .to_string();
                    let caption = media
                        .get("caption")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string);
                    if let Some(data_base64) = media.get("data_base64").and_then(|v| v.as_str()) {
                        return WsMessage::Media {
                            mime_type,
                            data_base64: data_base64.to_string(),
                            caption,
                        };
                    }
                }
            }
            _ => {}
        }
    }

    WsMessage::Unknown(raw.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_delta_chunk() {
        let raw = r#"{"type":"message_delta","data":{"delta":"Hello "}}"#;
        match classify_ws_message(raw) {
            WsMessage::Stream(RealtimeEvent::MessageDelta { delta }) => {
                assert_eq!(delta, "Hello ");
            }
            other => panic!("expected Stream(MessageDelta), got {other:?}"),
        }
    }

    #[test]
    fn classifies_done_chunk() {
        let raw = r#"{"type":"stream_done"}"#;
        assert!(matches!(
            classify_ws_message(raw),
            WsMessage::Stream(RealtimeEvent::StreamDone)
        ));
    }

    #[test]
    fn classifies_tool_exec_start() {
        let raw = r#"{"type":"tool_exec_start","data":{"name":"web_search","id":"t1"}}"#;
        assert!(matches!(
            classify_ws_message(raw),
            WsMessage::Stream(RealtimeEvent::ToolExecStart { .. })
        ));
    }

    #[test]
    fn classifies_tool_exec_end_success() {
        let raw =
            r#"{"type":"tool_exec_end","data":{"id":"t1","success":true,"duration_ms":1200}}"#;
        match classify_ws_message(raw) {
            WsMessage::Stream(RealtimeEvent::ToolExecEnd {
                success,
                duration_ms,
                ..
            }) => {
                assert!(success);
                assert_eq!(duration_ms, 1200);
            }
            other => panic!("expected ToolExecEnd, got {other:?}"),
        }
    }

    #[test]
    fn classifies_outbound_message_as_final() {
        let raw = r#"{"channel":"custom","session_id":"abc","payload":{"type":"Text","data":"hello world"},"reply_to":null,"metadata":{}}"#;
        match classify_ws_message(raw) {
            WsMessage::Final(text, stop_reason) => {
                assert_eq!(text, "hello world");
                assert!(stop_reason.is_none());
            }
            other => panic!("expected Final, got {other:?}"),
        }
    }

    #[test]
    fn classifies_final_with_stop_reason() {
        let raw = r#"{"channel":"custom","session_id":"abc","payload":{"type":"Text","data":"partial"},"reply_to":null,"metadata":{"stop_reason":"max_turns"}}"#;
        match classify_ws_message(raw) {
            WsMessage::Final(text, Some(AgentStopReason::MaxTurns)) => {
                assert_eq!(text, "partial");
            }
            other => panic!("expected Final with MaxTurns, got {other:?}"),
        }
    }

    #[test]
    fn classifies_final_with_max_tokens_stop_reason() {
        let raw = r#"{"channel":"custom","session_id":"abc","payload":{"type":"Text","data":"truncated"},"reply_to":null,"metadata":{"stop_reason":"max_tokens"}}"#;
        match classify_ws_message(raw) {
            WsMessage::Final(_, Some(AgentStopReason::MaxTokens)) => {}
            other => panic!("expected Final with MaxTokens, got {other:?}"),
        }
    }

    #[test]
    fn classifies_media_payload() {
        let raw = r#"{"channel":"custom","session_id":"abc","payload":{"type":"Media","data":{"mime_type":"image/png","url":"","caption":"Revenue chart","data_base64":"abc123"}},"reply_to":null,"metadata":{}}"#;
        match classify_ws_message(raw) {
            WsMessage::Media {
                mime_type,
                data_base64,
                caption,
            } => {
                assert_eq!(mime_type, "image/png");
                assert_eq!(data_base64, "abc123");
                assert_eq!(caption.as_deref(), Some("Revenue chart"));
            }
            other => panic!("expected Media, got {other:?}"),
        }
    }

    #[test]
    fn classifies_unknown_json() {
        let raw = r#"{"foo":"bar"}"#;
        assert!(matches!(classify_ws_message(raw), WsMessage::Unknown(_)));
    }

    #[test]
    fn classifies_non_json_as_unknown() {
        assert!(matches!(
            classify_ws_message("plain text"),
            WsMessage::Unknown(_)
        ));
    }

    #[test]
    fn classifies_usage_chunk() {
        let raw = r#"{"type":"usage","data":{"input_tokens":1500,"output_tokens":300,"model":"claude-sonnet-4-20250514"}}"#;
        match classify_ws_message(raw) {
            WsMessage::Stream(RealtimeEvent::Usage {
                input_tokens,
                output_tokens,
                model,
                ..
            }) => {
                assert_eq!(input_tokens, 1500);
                assert_eq!(output_tokens, 300);
                assert_eq!(model, "claude-sonnet-4-20250514");
            }
            other => panic!("expected Usage, got {other:?}"),
        }
    }

    #[test]
    fn classifies_thinking_delta_chunk() {
        let raw = r#"{"type":"thinking_delta","data":{"delta":"Let me think..."}}"#;
        match classify_ws_message(raw) {
            WsMessage::Stream(RealtimeEvent::ThinkingDelta { delta }) => {
                assert_eq!(delta, "Let me think...");
            }
            other => panic!("expected ThinkingDelta, got {other:?}"),
        }
    }

    #[test]
    fn classifies_agent_switch_chunk() {
        let raw = r#"{"type":"agent_switch","data":{"agent_id":"a1","display_name":"Researcher"}}"#;
        match classify_ws_message(raw) {
            WsMessage::Stream(RealtimeEvent::AgentSwitch { display_name, .. }) => {
                assert_eq!(display_name, "Researcher");
            }
            other => panic!("expected AgentSwitch, got {other:?}"),
        }
    }

    #[test]
    fn classifies_context_info_chunk() {
        let raw = r#"{"type":"context_info","data":{"history_tokens":50000,"context_window":128000,"messages_truncated":3,"summary_generated":true}}"#;
        match classify_ws_message(raw) {
            WsMessage::Stream(RealtimeEvent::ContextInfo {
                history_tokens,
                context_window,
                messages_truncated,
                ..
            }) => {
                assert_eq!(history_tokens, 50000);
                assert_eq!(context_window, 128_000);
                assert_eq!(messages_truncated, 3);
            }
            other => panic!("expected ContextInfo, got {other:?}"),
        }
    }

    #[test]
    fn classifies_principles_used_chunk() {
        let raw = r#"{"type":"principles_used","data":{"count":5}}"#;
        match classify_ws_message(raw) {
            WsMessage::Stream(RealtimeEvent::PrinciplesUsed { count }) => {
                assert_eq!(count, 5);
            }
            other => panic!("expected PrinciplesUsed, got {other:?}"),
        }
    }
}
