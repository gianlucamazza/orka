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
    let lower = path.to_lowercase();
    if lower.ends_with(".pdf") {
        Box::new(pdf::PdfParser)
    } else if lower.ends_with(".html") || lower.ends_with(".htm") {
        Box::new(html::HtmlParser)
    } else if lower.ends_with(".md") || lower.ends_with(".markdown") {
        Box::new(markdown::MarkdownParser)
    } else {
        Box::new(plaintext::PlaintextParser)
    }
}
