/// HTML document parser — strips tags and collapses whitespace.
pub mod html;
/// Markdown document parser — passes content through as plain text.
pub mod markdown;
/// PDF document parser using `pdf-extract`.
pub mod pdf;
/// Plain-text document parser — returns content as-is.
pub mod plaintext;

use orka_core::Result;

/// Trait for document parsers.
pub trait DocumentParser: Send + Sync {
    /// Parse document bytes into plain text.
    fn parse(&self, data: &[u8]) -> Result<String>;
}

/// Detect format from file extension and return the appropriate parser.
pub fn detect_format(path: &str) -> Box<dyn DocumentParser> {
    let p = std::path::Path::new(path);
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
    if ext.eq_ignore_ascii_case("pdf") {
        Box::new(pdf::PdfParser)
    } else if ext.eq_ignore_ascii_case("html") || ext.eq_ignore_ascii_case("htm") {
        Box::new(html::HtmlParser)
    } else if ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown") {
        Box::new(markdown::MarkdownParser)
    } else {
        Box::new(plaintext::PlaintextParser)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_format_html() {
        let parser = detect_format("page.html");
        let result = parser.parse(b"<b>hi</b>").unwrap();
        assert_eq!(result, "hi");
    }

    #[test]
    fn detect_format_htm() {
        let parser = detect_format("page.htm");
        let result = parser.parse(b"<p>hello</p>").unwrap();
        assert_eq!(result, "hello");
    }

    #[test]
    fn detect_format_md() {
        let parser = detect_format("readme.md");
        let result = parser.parse(b"# title").unwrap();
        assert_eq!(result, "# title");
    }

    #[test]
    fn detect_format_markdown_ext() {
        let parser = detect_format("doc.markdown");
        let result = parser.parse(b"text").unwrap();
        assert_eq!(result, "text");
    }

    #[test]
    fn detect_format_txt() {
        let parser = detect_format("notes.txt");
        let result = parser.parse(b"plain").unwrap();
        assert_eq!(result, "plain");
    }

    #[test]
    fn detect_format_unknown_falls_to_plaintext() {
        let parser = detect_format("file.docx");
        let result = parser.parse(b"data").unwrap();
        assert_eq!(result, "data");
    }

    #[test]
    fn detect_format_case_insensitive() {
        let parser = detect_format("PAGE.HTML");
        let result = parser.parse(b"<i>ok</i>").unwrap();
        assert_eq!(result, "ok");
    }
}
