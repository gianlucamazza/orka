#![allow(clippy::unwrap_used)]

use orka_core::Result;

use super::DocumentParser;

/// Parser that strips HTML tags and returns the visible text content.
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
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join("\n");

        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_strips_nested_tags() {
        let result = HtmlParser.parse(b"<div><p>hello</p></div>").unwrap();
        assert_eq!(result, "hello");
    }

    #[test]
    fn html_collapses_whitespace() {
        let input = b"<div>  foo  </div>\n\n\n<p>  bar  </p>";
        let result = HtmlParser.parse(input).unwrap();
        assert_eq!(result, "foo\nbar");
    }

    #[test]
    fn html_empty_input() {
        let result = HtmlParser.parse(b"").unwrap();
        assert_eq!(result, "");
    }
}
