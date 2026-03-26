use scraper::{Html, Selector};

/// Extract readable text from HTML content.
///
/// Uses a simple approach: strips scripts/styles, extracts text from
/// semantic content elements, falls back to body text.
pub(crate) fn extract_text(html: &str) -> String {
    let document = Html::parse_document(html);

    // Try to find main content areas first
    let content_selectors = [
        "article",
        "main",
        "[role=\"main\"]",
        ".post-content",
        ".article-content",
        ".entry-content",
    ];

    for sel_str in &content_selectors {
        if let Ok(sel) = Selector::parse(sel_str) {
            let texts: Vec<String> = document
                .select(&sel)
                .flat_map(|el| el.text())
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect();
            if !texts.is_empty() {
                return clean_text(&texts.join("\n"));
            }
        }
    }

    // Fallback: extract from body, skipping script/style/nav/header/footer
    if let Ok(body_sel) = Selector::parse("body")
        && let Some(body) = document.select(&body_sel).next()
    {
        let skip_tags = ["script", "style", "nav", "header", "footer", "aside"];
        let texts: Vec<String> = body
            .descendants()
            .filter_map(|node| {
                if let Some(el) = node.value().as_element()
                    && skip_tags.contains(&el.name())
                {
                    return Some(String::new()); // marker to skip subtree
                }
                node.value().as_text().map(|t| t.trim().to_string())
            })
            .filter(|t| !t.is_empty())
            .collect();
        return clean_text(&texts.join("\n"));
    }

    // Last resort: all text
    let texts: Vec<String> = document
        .root_element()
        .text()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();
    clean_text(&texts.join("\n"))
}

/// Extract the <title> from HTML.
pub(crate) fn extract_title(html: &str) -> Option<String> {
    let document = Html::parse_document(html);
    let sel = Selector::parse("title").ok()?;
    document
        .select(&sel)
        .next()
        .map(|el| el.text().collect::<String>().trim().to_string())
        .filter(|t| !t.is_empty())
}

/// Truncate text to `max_chars`, appending a truncation marker.
pub(crate) fn truncate(text: &str, max_chars: usize) -> (String, bool) {
    if text.len() <= max_chars {
        return (text.to_string(), false);
    }
    let truncated = &text[..text.floor_char_boundary(max_chars)];
    (
        format!("{truncated}\n\n[content truncated at {max_chars} chars]"),
        true,
    )
}

/// Clean up extracted text: collapse whitespace, remove excessive blank lines.
fn clean_text(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut blank_count = 0u32;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            blank_count += 1;
            if blank_count <= 2 {
                result.push('\n');
            }
        } else {
            blank_count = 0;
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(trimmed);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_text_from_article() {
        let html = r#"<html><body>
            <nav>Menu Item</nav>
            <article><p>Hello world.</p><p>Second paragraph.</p></article>
            <footer>Copyright</footer>
        </body></html>"#;
        let text = extract_text(html);
        assert!(text.contains("Hello world."));
        assert!(text.contains("Second paragraph."));
        assert!(!text.contains("Menu Item"));
        assert!(!text.contains("Copyright"));
    }

    #[test]
    fn extracts_title() {
        let html = "<html><head><title>My Page</title></head><body></body></html>";
        assert_eq!(extract_title(html), Some("My Page".into()));
    }

    #[test]
    fn truncation_works() {
        let text = "abcdefghij";
        let (t, truncated) = truncate(text, 5);
        assert!(truncated);
        assert!(t.contains("[content truncated at 5 chars]"));
        assert!(t.starts_with("abcde"));
    }

    #[test]
    fn no_truncation_when_short() {
        let text = "short";
        let (t, truncated) = truncate(text, 100);
        assert!(!truncated);
        assert_eq!(t, "short");
    }

    #[test]
    fn fallback_to_body_text() {
        let html = "<html><body><div>Just a div</div></body></html>";
        let text = extract_text(html);
        assert!(text.contains("Just a div"));
    }
}
