//! Markdown → Telegram HTML converter.
//!
//! Telegram supports a strict subset of HTML: `<b>`, `<i>`, `<s>`, `<u>`,
//! `<code>`, `<pre>`, `<a href>`, and `<blockquote>`.
//! This module converts standard CommonMark (+ strikethrough) to that subset
//! and handles the 4096-character message length limit via `split_html`.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

/// Convert CommonMark `input` to Telegram-compatible HTML.
pub(crate) fn md_to_telegram_html(input: &str) -> String {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_STRIKETHROUGH);

    let parser = Parser::new_ext(input, opts);

    let mut out = String::with_capacity(input.len() * 2);
    // Stack of (tag_name, close_string) for inline tags
    let mut tag_stack: Vec<&'static str> = Vec::new();
    // List tracking: (is_ordered, counter)
    let mut list_stack: Vec<(bool, u64)> = Vec::new();

    for event in parser {
        match event {
            // ── Block-level open ──────────────────────────────────────────
            Event::Start(Tag::Paragraph) => {}
            Event::End(TagEnd::Paragraph) => {
                out.push('\n');
            }

            Event::Start(Tag::Heading { .. }) => {
                out.push_str("<b>");
                tag_stack.push("b");
            }
            Event::End(TagEnd::Heading(_)) => {
                if tag_stack.last() == Some(&"b") {
                    tag_stack.pop();
                }
                out.push_str("</b>\n");
            }

            Event::Start(Tag::BlockQuote(_)) => {
                out.push_str("<blockquote>");
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                out.push_str("</blockquote>\n");
            }

            Event::Start(Tag::CodeBlock(kind)) => match kind {
                pulldown_cmark::CodeBlockKind::Fenced(lang) if !lang.is_empty() => {
                    out.push_str("<pre><code class=\"language-");
                    out.push_str(&escape_html(&lang));
                    out.push_str("\">");
                }
                _ => {
                    out.push_str("<pre><code>");
                }
            },
            Event::End(TagEnd::CodeBlock) => {
                out.push_str("</code></pre>\n");
            }

            // ── Lists ─────────────────────────────────────────────────────
            Event::Start(Tag::List(start)) => {
                list_stack.push((start.is_some(), start.unwrap_or(1)));
            }
            Event::End(TagEnd::List(_)) => {
                list_stack.pop();
                out.push('\n');
            }

            Event::Start(Tag::Item) => {
                let depth = list_stack.len().saturating_sub(1);
                let indent = "  ".repeat(depth);
                if let Some((ordered, counter)) = list_stack.last_mut() {
                    if *ordered {
                        out.push_str(&format!("{indent}{}. ", counter));
                        *counter += 1;
                    } else {
                        out.push_str(&format!("{indent}- "));
                    }
                }
            }
            Event::End(TagEnd::Item) => {
                out.push('\n');
            }

            // ── Inline open ───────────────────────────────────────────────
            Event::Start(Tag::Strong) => {
                out.push_str("<b>");
                tag_stack.push("b");
            }
            Event::End(TagEnd::Strong) => {
                if tag_stack.last() == Some(&"b") {
                    tag_stack.pop();
                }
                out.push_str("</b>");
            }

            Event::Start(Tag::Emphasis) => {
                out.push_str("<i>");
                tag_stack.push("i");
            }
            Event::End(TagEnd::Emphasis) => {
                if tag_stack.last() == Some(&"i") {
                    tag_stack.pop();
                }
                out.push_str("</i>");
            }

            Event::Start(Tag::Strikethrough) => {
                out.push_str("<s>");
                tag_stack.push("s");
            }
            Event::End(TagEnd::Strikethrough) => {
                if tag_stack.last() == Some(&"s") {
                    tag_stack.pop();
                }
                out.push_str("</s>");
            }

            Event::Start(Tag::Link {
                dest_url,
                title: _,
                id: _,
                ..
            }) => {
                out.push_str("<a href=\"");
                out.push_str(&escape_html(&dest_url));
                out.push_str("\">");
                tag_stack.push("a");
            }
            Event::End(TagEnd::Link) => {
                if tag_stack.last() == Some(&"a") {
                    tag_stack.pop();
                }
                out.push_str("</a>");
            }

            Event::Start(Tag::Image {
                dest_url,
                title: _,
                id: _,
                ..
            }) => {
                // Render images as links with alt text
                out.push_str("<a href=\"");
                out.push_str(&escape_html(&dest_url));
                out.push_str("\">");
                tag_stack.push("a");
            }
            Event::End(TagEnd::Image) => {
                if tag_stack.last() == Some(&"a") {
                    tag_stack.pop();
                }
                out.push_str("</a>");
            }

            // ── Leaf events ───────────────────────────────────────────────
            Event::Code(text) => {
                out.push_str("<code>");
                out.push_str(&escape_html(&text));
                out.push_str("</code>");
            }

            Event::Text(text) => {
                out.push_str(&escape_html(&text));
            }

            Event::SoftBreak => {
                out.push('\n');
            }
            Event::HardBreak => {
                out.push('\n');
            }

            Event::Rule => {
                out.push_str("───────────────\n");
            }

            // Ignore everything else (HTML passthrough, footnotes, etc.)
            _ => {}
        }
    }

    // Trim trailing whitespace/newlines
    out.trim_end().to_string()
}

/// Escape `&`, `<`, `>` for use in Telegram HTML.
fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}

/// Split `html` into chunks of at most `max_len` bytes, attempting to break on
/// newline boundaries and keeping track of open tags so each chunk is
/// self-contained.
///
/// Tags tracked for re-opening/closing: `b`, `i`, `s`, `u`, `code`, `pre`, `a`.
pub(crate) fn split_html(html: &str, max_len: usize) -> Vec<String> {
    if html.len() <= max_len {
        return vec![html.to_string()];
    }

    let lines: Vec<&str> = html.split('\n').collect();
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    // Stack of open tag strings (just the opening tag, e.g. `<b>` or `<a
    // href="...">`)
    let mut open_tags: Vec<String> = Vec::new();

    let prefix_from_stack = |stack: &Vec<String>| stack.join("");

    for (i, line) in lines.iter().enumerate() {
        let separator = if i == 0 { "" } else { "\n" };
        let candidate_len = current.len() + separator.len() + line.len();

        if candidate_len > max_len && !current.is_empty() {
            // Close open tags in reverse order
            let closing: String = open_tags
                .iter()
                .rev()
                .map(|t| close_tag(t))
                .collect::<Vec<_>>()
                .join("");
            chunks.push(format!("{current}{closing}"));

            // Re-open the same tags in the next chunk
            let prefix = prefix_from_stack(&open_tags);
            current = format!("{prefix}{line}");
        } else {
            if !current.is_empty() {
                current.push('\n');
            }
            current.push_str(line);
        }

        // Update open_tags based on tags found in this line
        update_open_tags(&mut open_tags, line);
    }

    if !current.is_empty() {
        let closing: String = open_tags
            .iter()
            .rev()
            .map(|t| close_tag(t))
            .collect::<Vec<_>>()
            .join("");
        // Only append closing if there are actually open tags (avoids trailing noise)
        if closing.is_empty() {
            chunks.push(current);
        } else {
            chunks.push(format!("{current}{closing}"));
        }
    }

    if chunks.is_empty() {
        chunks.push(html.to_string());
    }

    chunks
}

/// Parse opening/closing tags in `line` and update the `open_tags` stack.
fn update_open_tags(stack: &mut Vec<String>, line: &str) {
    let mut pos = 0;
    let bytes = line.as_bytes();
    while pos < bytes.len() {
        if bytes[pos] == b'<' {
            // Find closing `>`
            if let Some(end) = line[pos..].find('>') {
                let tag_content = &line[pos + 1..pos + end];
                if let Some(stripped) = tag_content.strip_prefix('/') {
                    // Closing tag — pop matching open tag
                    let name = stripped.trim();
                    if let Some(idx) = stack.iter().rposition(|t| tag_name(t) == name) {
                        stack.remove(idx);
                    }
                } else if !tag_content.ends_with('/') {
                    // Opening tag
                    let full_tag = format!("<{}>", tag_content);
                    stack.push(full_tag);
                }
                pos += end + 1;
            } else {
                pos += 1;
            }
        } else {
            pos += 1;
        }
    }
}

/// Extract the tag name from an opening tag string like `<b>` or `<a
/// href="...">`.
fn tag_name(open_tag: &str) -> &str {
    // open_tag is like `<b>` or `<a href="url">`
    let inner = open_tag.trim_start_matches('<').trim_end_matches('>');
    inner.split_whitespace().next().unwrap_or("")
}

/// Build the closing tag for an opening tag string.
fn close_tag(open_tag: &str) -> String {
    format!("</{}>", tag_name(open_tag))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── md_to_telegram_html ───────────────────────────────────────────────

    #[test]
    fn bold() {
        assert_eq!(md_to_telegram_html("**hello**"), "<b>hello</b>");
    }

    #[test]
    fn italic() {
        assert_eq!(md_to_telegram_html("*world*"), "<i>world</i>");
    }

    #[test]
    fn strikethrough() {
        assert_eq!(md_to_telegram_html("~~strike~~"), "<s>strike</s>");
    }

    #[test]
    fn inline_code() {
        assert_eq!(md_to_telegram_html("`code`"), "<code>code</code>");
    }

    #[test]
    fn code_block_no_lang() {
        let input = "```\nfn main() {}\n```";
        let out = md_to_telegram_html(input);
        assert!(out.contains("<pre><code>"), "got: {out}");
        assert!(out.contains("fn main()"), "got: {out}");
        assert!(out.contains("</code></pre>"), "got: {out}");
    }

    #[test]
    fn code_block_with_lang() {
        let input = "```rust\nlet x = 1;\n```";
        let out = md_to_telegram_html(input);
        assert!(out.contains(r#"class="language-rust""#), "got: {out}");
        assert!(out.contains("let x = 1;"), "got: {out}");
    }

    #[test]
    fn link() {
        let input = "[click](https://example.com)";
        let out = md_to_telegram_html(input);
        assert_eq!(out, r#"<a href="https://example.com">click</a>"#);
    }

    #[test]
    fn image_rendered_as_link() {
        let input = "![alt text](https://example.com/img.png)";
        let out = md_to_telegram_html(input);
        assert!(
            out.contains(r#"href="https://example.com/img.png""#),
            "got: {out}"
        );
        assert!(out.contains("alt text"), "got: {out}");
    }

    #[test]
    fn heading_all_levels() {
        for level in 1..=6 {
            let input = format!("{} Heading", "#".repeat(level));
            let out = md_to_telegram_html(&input);
            assert!(out.starts_with("<b>"), "level {level}: got {out}");
            assert!(out.contains("Heading"), "level {level}: got {out}");
        }
    }

    #[test]
    fn blockquote() {
        let input = "> quoted text";
        let out = md_to_telegram_html(input);
        assert!(out.contains("<blockquote>"), "got: {out}");
        assert!(out.contains("quoted text"), "got: {out}");
    }

    #[test]
    fn unordered_list() {
        let input = "- one\n- two\n- three";
        let out = md_to_telegram_html(input);
        assert!(out.contains("- one"), "got: {out}");
        assert!(out.contains("- two"), "got: {out}");
        assert!(out.contains("- three"), "got: {out}");
    }

    #[test]
    fn ordered_list() {
        let input = "1. first\n2. second\n3. third";
        let out = md_to_telegram_html(input);
        assert!(out.contains("1. first"), "got: {out}");
        assert!(out.contains("2. second"), "got: {out}");
        assert!(out.contains("3. third"), "got: {out}");
    }

    #[test]
    fn html_escaping() {
        let input = "a < b & c > d";
        let out = md_to_telegram_html(input);
        assert!(out.contains("&lt;"), "got: {out}");
        assert!(out.contains("&amp;"), "got: {out}");
        assert!(out.contains("&gt;"), "got: {out}");
    }

    #[test]
    fn html_escaping_in_code() {
        let input = "`<script>&`";
        let out = md_to_telegram_html(input);
        assert!(out.contains("&lt;script&gt;"), "got: {out}");
        assert!(out.contains("&amp;"), "got: {out}");
    }

    #[test]
    fn mixed_formatting() {
        let input = "**bold** and *italic* and `code`";
        let out = md_to_telegram_html(input);
        assert!(out.contains("<b>bold</b>"), "got: {out}");
        assert!(out.contains("<i>italic</i>"), "got: {out}");
        assert!(out.contains("<code>code</code>"), "got: {out}");
    }

    #[test]
    fn plain_text_passthrough() {
        let input = "just plain text";
        let out = md_to_telegram_html(input);
        assert_eq!(out, "just plain text");
    }

    // ── split_html ────────────────────────────────────────────────────────

    #[test]
    fn split_short_text_single_chunk() {
        let html = "<b>hello</b>";
        let chunks = split_html(html, 4096);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], html);
    }

    #[test]
    fn split_long_text_multiple_chunks() {
        let line = "a".repeat(100);
        let html = (0..50).map(|_| line.clone()).collect::<Vec<_>>().join("\n");
        let chunks = split_html(&html, 1000);
        assert!(chunks.len() > 1, "expected multiple chunks");
        for chunk in &chunks {
            assert!(chunk.len() <= 1100, "chunk too long: {}", chunk.len()); // allow slight overage on single long lines
        }
    }

    #[test]
    fn split_reopens_tags() {
        // Two lines, first opens <b>, split forces a new chunk
        let html = "<b>line one\nline two</b>";
        let chunks = split_html(html, 15);
        // The second chunk should reopen <b>
        if chunks.len() > 1 {
            assert!(
                chunks[1].contains("<b>"),
                "second chunk should reopen <b>: {:?}",
                chunks
            );
        }
    }

    #[test]
    fn tag_name_extraction() {
        assert_eq!(tag_name("<b>"), "b");
        assert_eq!(tag_name("<a href=\"url\">"), "a");
    }

    #[test]
    fn close_tag_generation() {
        assert_eq!(close_tag("<b>"), "</b>");
        assert_eq!(close_tag("<i>"), "</i>");
    }
}
