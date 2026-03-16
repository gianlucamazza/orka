use orka_core::Result;

use super::DocumentParser;

pub struct MarkdownParser;

impl DocumentParser for MarkdownParser {
    fn parse(&self, data: &[u8]) -> Result<String> {
        // Markdown is already text-friendly, just return as-is
        Ok(String::from_utf8_lossy(data).to_string())
    }
}
