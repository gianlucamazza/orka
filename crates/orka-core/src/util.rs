/// Truncate a tool result string if it exceeds the configured limit, respecting
/// UTF-8 character boundaries.
pub fn truncate_tool_result(content: &str, max_chars: usize) -> String {
    if content.len() <= max_chars {
        return content.to_string();
    }
    let boundary = content.floor_char_boundary(max_chars);
    let truncated = &content[..boundary];
    format!(
        "{truncated}\n\n[truncated, showing first {boundary} chars of {} total]",
        content.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_short_content_unchanged() {
        let s = "short content";
        assert_eq!(truncate_tool_result(s, 100), s);
    }

    #[test]
    fn truncate_long_content_shows_boundary() {
        let s = "a".repeat(1000);
        let result = truncate_tool_result(&s, 100);
        assert!(result.contains("[truncated"));
        assert!(result.contains("1000 total"));
        assert!(result.len() < s.len());
    }

    #[test]
    fn truncate_multibyte_respects_char_boundary() {
        // 4-byte emoji repeated; truncation must not panic
        let s = "🦀".repeat(100);
        let result = truncate_tool_result(&s, 50);
        assert!(result.contains("[truncated"));
    }
}
