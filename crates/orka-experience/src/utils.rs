/// Extract a JSON array from text that may contain markdown code fences.
pub(crate) fn extract_json_array(text: &str) -> String {
    // Try to find JSON between code fences first
    if let Some(start) = text.find("```json") {
        let after = &text[start + 7..];
        if let Some(end) = after.find("```") {
            return after[..end].trim().to_string();
        }
    }
    if let Some(start) = text.find("```") {
        let after = &text[start + 3..];
        if let Some(end) = after.find("```") {
            let inner = after[..end].trim();
            if inner.starts_with('[') {
                return inner.to_string();
            }
        }
    }
    // Try to find a bare JSON array
    if let Some(start) = text.find('[')
        && let Some(end) = text.rfind(']')
    {
        return text[start..=end].to_string();
    }
    text.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_json_from_code_fence() {
        let input =
            "Here are the principles:\n```json\n[{\"text\": \"hello\", \"kind\": \"do\"}]\n```\n";
        assert_eq!(
            extract_json_array(input),
            "[{\"text\": \"hello\", \"kind\": \"do\"}]"
        );
    }

    #[test]
    fn extract_json_bare_array() {
        let input = "Some text [{\"text\": \"test\"}] more text";
        assert_eq!(extract_json_array(input), "[{\"text\": \"test\"}]");
    }
}
