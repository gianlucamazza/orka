/// Derive a category tag and human-readable input summary from the tool name
/// and its JSON input.
pub(super) fn tool_metadata(
    name: &str,
    input: &serde_json::Value,
) -> (Option<String>, Option<String>) {
    match name {
        "web_search" => {
            let summary = input
                .get("query")
                .and_then(|v| v.as_str())
                .map(|q| format!("query: '{q}'"));
            (Some("search".into()), summary)
        }
        "http_request" => {
            let method = input
                .get("method")
                .and_then(|v| v.as_str())
                .unwrap_or("GET");
            let summary = input
                .get("url")
                .and_then(|v| v.as_str())
                .map(|u| format!("{method} {u}"));
            (Some("http".into()), summary)
        }
        "sandbox" | "code_exec" | "code_interpreter" => (Some("code".into()), None),
        "remember_fact" | "search_facts" | "list_facts" | "forget_fact" | "ingest_document"
        | "list_documents" => (Some("memory".into()), None),
        n if n.starts_with("schedule_") => (Some("schedule".into()), None),
        _ => (None, None),
    }
}

/// Produce a brief output summary for known tools.
pub(super) fn summarize_result(name: &str, content: &str, is_error: bool) -> Option<String> {
    if is_error {
        // Truncate long error messages
        let msg = if content.len() > 80 {
            format!("{}…", &content[..80])
        } else {
            content.to_string()
        };
        return Some(msg);
    }
    match name {
        "web_search" => {
            // Try to count results from JSON array
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(content)
                && let Some(arr) = v.as_array()
            {
                return Some(format!("Found {} results", arr.len()));
            }
            Some("Search complete".into())
        }
        "http_request" => {
            let len = content.len();
            if len > 1024 {
                Some(format!("{:.1} KB response", len as f64 / 1024.0))
            } else {
                Some(format!("{len} bytes"))
            }
        }
        _ => None,
    }
}
