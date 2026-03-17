use orka_core::stream::StreamChunkKind;

/// Classified WebSocket message.
pub enum WsMessage {
    /// A streaming chunk (delta, tool event, done).
    Stream(StreamChunkKind),
    /// A final message with extracted display text.
    Final(String),
    /// Unrecognized payload (fallback).
    Unknown(String),
}

/// Classify a raw WebSocket text frame into a typed message.
///
/// 1. Try deserializing as `StreamChunkKind` (`{"type":"Delta","data":"..."}`)
/// 2. Try extracting text from OutboundMessage shape or legacy fields
/// 3. Fall back to `Unknown`
pub fn classify_ws_message(raw: &str) -> WsMessage {
    // Stream chunk: top-level { "type": "Delta"|"ToolStart"|... }
    if let Ok(kind) = serde_json::from_str::<StreamChunkKind>(raw) {
        return WsMessage::Stream(kind);
    }

    // OutboundMessage or legacy shapes
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(raw) {
        // OutboundMessage: { payload: { data: "..." } }
        if let Some(data) = parsed
            .get("payload")
            .and_then(|p| p.get("data"))
            .and_then(|d| d.as_str())
        {
            return WsMessage::Final(data.to_string());
        }
        // Legacy fields
        if let Some(text) = parsed
            .get("text")
            .or_else(|| parsed.get("content"))
            .or_else(|| parsed.get("message"))
            .and_then(|v| v.as_str())
        {
            return WsMessage::Final(text.to_string());
        }
    }

    WsMessage::Unknown(raw.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_delta_chunk() {
        let raw = r#"{"type":"Delta","data":"Hello "}"#;
        match classify_ws_message(raw) {
            WsMessage::Stream(StreamChunkKind::Delta(s)) => assert_eq!(s, "Hello "),
            other => panic!("expected Stream(Delta), got {other:?}"),
        }
    }

    #[test]
    fn classifies_done_chunk() {
        let raw = r#"{"type":"Done"}"#;
        assert!(matches!(
            classify_ws_message(raw),
            WsMessage::Stream(StreamChunkKind::Done)
        ));
    }

    #[test]
    fn classifies_tool_exec_start() {
        let raw = r#"{"type":"ToolExecStart","data":{"name":"web_search","id":"t1"}}"#;
        assert!(matches!(
            classify_ws_message(raw),
            WsMessage::Stream(StreamChunkKind::ToolExecStart { .. })
        ));
    }

    #[test]
    fn classifies_tool_exec_end_success() {
        let raw = r#"{"type":"ToolExecEnd","data":{"id":"t1","success":true,"duration_ms":1200}}"#;
        match classify_ws_message(raw) {
            WsMessage::Stream(StreamChunkKind::ToolExecEnd {
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
            WsMessage::Final(text) => assert_eq!(text, "hello world"),
            other => panic!("expected Final, got {other:?}"),
        }
    }

    #[test]
    fn classifies_legacy_text_as_final() {
        let raw = r#"{"text":"hi there"}"#;
        match classify_ws_message(raw) {
            WsMessage::Final(text) => assert_eq!(text, "hi there"),
            other => panic!("expected Final, got {other:?}"),
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

    // Allow debug formatting in panic messages
    impl std::fmt::Debug for WsMessage {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                WsMessage::Stream(kind) => write!(f, "Stream({kind:?})"),
                WsMessage::Final(text) => write!(f, "Final({text:?})"),
                WsMessage::Unknown(raw) => write!(f, "Unknown({raw:?})"),
            }
        }
    }
}
