use orka_core::Result;

use super::DocumentParser;

pub struct HtmlParser;

impl DocumentParser for HtmlParser {
    fn parse(&self, data: &[u8]) -> Result<String> {
        let html = String::from_utf8_lossy(data);
        let mut text = String::with_capacity(html.len());
        let mut in_tag = false;

        for c in html.chars() {
            match c {
                '<' => in_tag = true,
                '>' => in_tag = false,
                _ if in_tag => {}
                _ => text.push(c),
            }
        }

        // Clean up whitespace
        let text = text
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join("\n");

        Ok(text)
    }
}
