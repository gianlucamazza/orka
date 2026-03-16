/// Extract the display text from a WebSocket message JSON payload.
///
/// Priority: `payload.data` (OutboundMessage shape), then `text`, `content`,
/// `message`.  Falls back to the raw string if no known field is found.
pub fn extract_ws_text(raw: &str) -> String {
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(raw) {
        // OutboundMessage: { payload: { data: "..." } }
        if let Some(data) = parsed
            .get("payload")
            .and_then(|p| p.get("data"))
            .and_then(|d| d.as_str())
        {
            return data.to_string();
        }
        parsed
            .get("text")
            .or_else(|| parsed.get("content"))
            .or_else(|| parsed.get("message"))
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| raw.to_string())
    } else {
        raw.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_payload_data_from_outbound_message() {
        let json = r#"{"channel":"custom","session_id":"abc","payload":{"type":"Text","data":"hello world"},"reply_to":null,"metadata":{}}"#;
        assert_eq!(extract_ws_text(json), "hello world");
    }

    #[test]
    fn extracts_legacy_text_field() {
        let json = r#"{"text":"hi there"}"#;
        assert_eq!(extract_ws_text(json), "hi there");
    }

    #[test]
    fn extracts_legacy_content_field() {
        let json = r#"{"content":"some content"}"#;
        assert_eq!(extract_ws_text(json), "some content");
    }

    #[test]
    fn extracts_legacy_message_field() {
        let json = r#"{"message":"a message"}"#;
        assert_eq!(extract_ws_text(json), "a message");
    }

    #[test]
    fn falls_back_to_raw_string_for_unknown_shape() {
        let json = r#"{"foo":"bar"}"#;
        assert_eq!(extract_ws_text(json), json);
    }

    #[test]
    fn falls_back_to_raw_for_non_json() {
        assert_eq!(extract_ws_text("plain text"), "plain text");
    }

    #[test]
    fn payload_data_takes_priority_over_text() {
        let json = r#"{"text":"ignored","payload":{"data":"preferred"}}"#;
        assert_eq!(extract_ws_text(json), "preferred");
    }
}
