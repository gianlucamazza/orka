use std::{
    io::Write,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU16, Ordering},
    },
};

use comfy_table::{ContentArrangement, Table, presets};
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use syntect::{
    easy::HighlightLines, highlighting::ThemeSet, parsing::SyntaxSet,
    util::as_24_bit_terminal_escaped,
};

use crate::term_caps::ColorLevel;

/// Renders markdown to the terminal with syntax highlighting for code blocks.
///
/// Supports two modes:
/// - **Streaming** (`push_delta` + `flush`): block-buffered rendering for
///   progressive output.
/// - **Full** (`render_full`): renders a complete markdown string at once.
pub struct MarkdownRenderer {
    syntax_set: SyntaxSet,
    theme_set: ThemeSet,
    theme_name: String,
    buffer: String,
    /// Terminal color capability, set at construction time.
    color: ColorLevel,
    /// Shared terminal width updated on SIGWINCH — used for horizontal rules
    /// and other width-dependent output so they reflow after resize.
    term_width: Arc<AtomicU16>,
    /// Output sink — defaults to stdout, injectable for testing.
    output: Arc<Mutex<Box<dyn Write + Send>>>,
}

impl MarkdownRenderer {
    /// Create a renderer writing to stdout.
    pub fn new(term_width: Arc<AtomicU16>, color: ColorLevel) -> Self {
        Self::with_output(
            term_width,
            color,
            Arc::new(Mutex::new(Box::new(std::io::stdout()))),
        )
    }

    /// Create a renderer with a custom output sink (for testing).
    pub fn with_output(
        term_width: Arc<AtomicU16>,
        color: ColorLevel,
        output: Arc<Mutex<Box<dyn Write + Send>>>,
    ) -> Self {
        Self {
            syntax_set: SyntaxSet::load_defaults_newlines(),
            theme_set: ThemeSet::load_defaults(),
            theme_name: "base16-eighties.dark".to_string(),
            buffer: String::new(),
            color,
            term_width,
            output,
        }
    }

    /// Feed a streaming delta chunk. Renders any completed blocks immediately.
    pub fn push_delta(&mut self, delta: &str) {
        self.buffer.push_str(delta);
        self.render_completed_blocks();
    }

    /// Flush and render whatever remains in the buffer (call on Done).
    /// Returns `true` if anything was rendered.
    pub fn flush(&mut self) -> bool {
        if self.buffer.is_empty() {
            false
        } else {
            let remaining = std::mem::take(&mut self.buffer);
            // Use render_full so that mixed content (prose then code fence) is
            // handled correctly even when they arrive in the same final chunk.
            self.render_full(&remaining);
            true
        }
    }

    /// Reset state for a new conversation turn.
    pub fn reset(&mut self) {
        self.buffer.clear();
    }

    /// Render a complete markdown string (non-streaming path).
    pub fn render_full(&self, text: &str) {
        let blocks = split_blocks(text);
        for block in blocks {
            // UI-8: use find_closing_fence_full instead of naive backtick count
            let block_trimmed = block.trim_start();
            if block_trimmed.starts_with("```") && find_closing_fence_full(block_trimmed).is_some()
            {
                self.render_code_block_inline(block);
            } else {
                self.render_prose_pulldown(block);
            }
        }
    }

    /// Scan the buffer for completed blocks and render them.
    fn render_completed_blocks(&mut self) {
        loop {
            let boundary = self.find_block_boundary();
            match boundary {
                Some(end) => {
                    let block: String = self.buffer[..end].to_string();
                    // Skip the block and any leading newlines between blocks in one drain
                    let skip_nl = self.buffer[end..]
                        .bytes()
                        .take_while(|&b| b == b'\n')
                        .count();
                    self.buffer.drain(..end + skip_nl);
                    self.render_block(&block);
                }
                None => break,
            }
        }
    }

    /// Find the byte offset of the end of the next complete block.
    fn find_block_boundary(&self) -> Option<usize> {
        let buf = &self.buffer;

        // Check if we're entering a code fence
        let trimmed = buf.trim_start();
        if trimmed.starts_with("```") {
            // We're starting a code block — look for closing fence
            if let Some(pos) = find_closing_fence_full(buf) {
                return Some(pos);
            }
            // Not closed yet — wait
            return None;
        }

        // Outside code fence: look for double newline (paragraph break)
        if let Some(pos) = buf.find("\n\n") {
            return Some(pos);
        }

        // UI-7: incremental flush threshold — render up to the last newline when
        // the buffer grows large, so long paragraphs stream smoothly instead of
        // waiting for a paragraph break.
        #[allow(clippy::items_after_statements)]
        const INCREMENTAL_THRESHOLD: usize = 200;
        if buf.len() >= INCREMENTAL_THRESHOLD
            && let Some(pos) = buf.rfind('\n')
            && pos > 0
        {
            return Some(pos);
        }

        None
    }

    /// Render a single completed block.
    fn render_block(&mut self, block: &str) {
        let trimmed = block.trim();
        if trimmed.is_empty() {
            return;
        }

        // UI-8: use find_closing_fence_full for reliable complete-fence detection
        if trimmed.starts_with("```") && find_closing_fence_full(trimmed).is_some() {
            self.render_code_block_inline(trimmed);
        } else if trimmed.starts_with("```") {
            // Unclosed code fence at flush — print raw to avoid markdown mangling
            let mut guard = self.output.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let out = &mut **guard;
            for line in trimmed.lines() {
                let _ = writeln!(out, "{line}");
            }
        } else {
            self.render_prose_pulldown(block);
        }
    }

    /// Render a prose markdown block using pulldown-cmark with ANSI terminal
    /// output.
    ///
    /// Supports: headings (h1–h6), bold, italic, strikethrough, task lists,
    /// blockquotes, ordered/unordered lists, GFM tables (via comfy-table),
    /// inline code, links with OSC 8 hyperlinks, and horizontal rules.
    #[allow(clippy::too_many_lines, clippy::items_after_statements, clippy::fn_params_excessive_bools)]
    fn render_prose_pulldown(&self, text: &str) {
        let no_color = self.color.is_none();
        let opts = Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_TASKLISTS
            | Options::ENABLE_TABLES
            | Options::ENABLE_GFM;
        let parser = Parser::new_ext(text, opts);

        // Inline formatting flags
        let mut in_strong = false;
        let mut in_em = false;
        let mut in_strike = false;

        // Current inline text accumulator
        let mut buf = String::new();

        // Blockquote nesting depth
        let mut blockquote_depth: u32 = 0;

        // List state — stack tracks whether each level is ordered and the next ordinal
        struct ListEntry {
            ordered: bool,
            ordinal: u64,
        }
        let mut list_stack: Vec<ListEntry> = Vec::new();
        let mut indent_depth: usize = 0;

        // Link state
        let mut link_url: Option<String> = None;
        let mut link_text = String::new();
        let mut in_link = false;

        // Table state
        let mut in_table = false;
        let mut table_headers: Vec<String> = Vec::new();
        let mut table_rows: Vec<Vec<String>> = Vec::new();
        let mut current_row: Vec<String> = Vec::new();
        let mut cell_buf = String::new();
        let mut in_thead = false;

        let mut guard = self.output.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let out = &mut **guard;

        fn apply_fmt(s: &str, strong: bool, em: bool, strike: bool, no_color: bool) -> String {
            if no_color || s.is_empty() {
                return s.to_string();
            }
            let mut r = s.to_string();
            if strong {
                r = format!("\x1b[1m{r}\x1b[22m");
            }
            if em {
                r = format!("\x1b[3m{r}\x1b[23m");
            }
            if strike {
                r = format!("\x1b[9m{r}\x1b[29m");
            }
            r
        }

        fn fmt_inline_code(s: &str, no_color: bool) -> String {
            if no_color {
                format!("`{s}`")
            } else {
                format!("\x1b[36m{s}\x1b[0m")
            }
        }

        fn osc8_link(url: &str, text: &str, no_color: bool) -> String {
            if no_color || url.is_empty() {
                return text.to_string();
            }
            format!("\x1b]8;;{url}\x1b\\{text}\x1b]8;;\x1b\\")
        }

        fn bq_prefix(depth: u32, no_color: bool) -> String {
            if depth == 0 {
                return String::new();
            }
            if no_color {
                "| ".repeat(depth as usize)
            } else {
                format!("{} ", "\x1b[90m│\x1b[0m").repeat(depth as usize)
            }
        }

        for event in parser {
            match event {
                // ── Paragraphs ──────────────────────────────────────────────────
                Event::Start(Tag::Paragraph | Tag::Item) => {
                    buf.clear();
                }
                Event::End(TagEnd::Paragraph) => {
                    let prefix = bq_prefix(blockquote_depth, no_color);
                    if !buf.trim().is_empty() {
                        let _ = writeln!(out, "{prefix}{}", buf.trim_start());
                    }
                    let _ = writeln!(out);
                    buf.clear();
                }

                // ── Headings ─────────────────────────────────────────────────────
                Event::Start(Tag::Heading { .. }) => {
                    in_strong = true;
                    buf.clear();
                }
                Event::End(TagEnd::Heading(level)) => {
                    let text = if no_color {
                        buf.clone()
                    } else {
                        let color = match level {
                            HeadingLevel::H1 => "\x1b[32m",
                            HeadingLevel::H2 => "\x1b[36m",
                            _ => "\x1b[33m",
                        };
                        format!("{color}\x1b[1m{buf}\x1b[0m")
                    };
                    let _ = writeln!(out, "\n{text}");
                    buf.clear();
                    in_strong = false;
                }

                // ── Inline formatting ───────────────────────────────────────────
                Event::Start(Tag::Strong) => in_strong = true,
                Event::End(TagEnd::Strong) => in_strong = false,
                Event::Start(Tag::Emphasis) => in_em = true,
                Event::End(TagEnd::Emphasis) => in_em = false,
                Event::Start(Tag::Strikethrough) => in_strike = true,
                Event::End(TagEnd::Strikethrough) => in_strike = false,

                // ── Blockquote ───────────────────────────────────────────────────
                Event::Start(Tag::BlockQuote(_)) => blockquote_depth += 1,
                Event::End(TagEnd::BlockQuote(_)) => {
                    blockquote_depth = blockquote_depth.saturating_sub(1);
                }

                // ── Lists ─────────────────────────────────────────────────────────
                Event::Start(Tag::List(start)) => {
                    list_stack.push(ListEntry {
                        ordered: start.is_some(),
                        ordinal: start.unwrap_or(1),
                    });
                    indent_depth += 1;
                }
                Event::End(TagEnd::List(_)) => {
                    list_stack.pop();
                    indent_depth = indent_depth.saturating_sub(1);
                    if list_stack.is_empty() {
                        let _ = writeln!(out);
                    }
                }
                Event::End(TagEnd::Item) => {
                    let item_indent = "  ".repeat(indent_depth.saturating_sub(1));
                    let bullet = if let Some(entry) = list_stack.last_mut() {
                        if entry.ordered {
                            let n = entry.ordinal;
                            entry.ordinal += 1;
                            format!("{n}. ")
                        } else {
                            "- ".to_string()
                        }
                    } else {
                        "- ".to_string()
                    };
                    if !buf.trim().is_empty() {
                        let _ = writeln!(out, "{item_indent}{bullet}{}", buf.trim_start());
                    }
                    buf.clear();
                }
                Event::TaskListMarker(checked) => {
                    let marker = if checked { "☑ " } else { "☐ " };
                    buf.push_str(marker);
                }

                // ── Links ─────────────────────────────────────────────────────────
                Event::Start(Tag::Link { dest_url, .. }) => {
                    in_link = true;
                    link_url = Some(dest_url.to_string());
                    link_text.clear();
                }
                Event::End(TagEnd::Link) => {
                    let url = link_url.take().unwrap_or_default();
                    let display = if link_text.is_empty() {
                        url.clone()
                    } else {
                        link_text.clone()
                    };
                    let formatted = osc8_link(&url, &display, no_color);
                    if in_table {
                        cell_buf.push_str(&formatted);
                    } else {
                        buf.push_str(&formatted);
                    }
                    in_link = false;
                    link_text.clear();
                }

                // ── Tables (rendered via comfy-table) ─────────────────────────────
                Event::Start(Tag::Table(_)) => {
                    in_table = true;
                    table_headers.clear();
                    table_rows.clear();
                    current_row.clear();
                }
                Event::End(TagEnd::Table) => {
                    let preset = if no_color {
                        presets::ASCII_FULL_CONDENSED
                    } else {
                        presets::UTF8_FULL_CONDENSED
                    };
                    let mut table = Table::new();
                    table
                        .load_preset(preset)
                        .set_content_arrangement(ContentArrangement::Dynamic)
                        .set_header(table_headers.clone());
                    for row in &table_rows {
                        table.add_row(row);
                    }
                    let _ = writeln!(out, "{table}");
                    let _ = writeln!(out);
                    in_table = false;
                    table_headers.clear();
                    table_rows.clear();
                    current_row.clear();
                }
                Event::Start(Tag::TableHead) => {
                    in_thead = true;
                }
                Event::End(TagEnd::TableHead) => {
                    in_thead = false;
                    table_headers = std::mem::take(&mut current_row);
                }
                Event::Start(Tag::TableRow) => {
                    current_row.clear();
                }
                Event::End(TagEnd::TableRow) => {
                    if !in_thead && !current_row.is_empty() {
                        table_rows.push(std::mem::take(&mut current_row));
                    }
                }
                Event::Start(Tag::TableCell) => {
                    cell_buf.clear();
                }
                Event::End(TagEnd::TableCell) => {
                    current_row.push(std::mem::take(&mut cell_buf));
                }

                // ── Horizontal rule ────────────────────────────────────────────────
                Event::Rule => {
                    let width = self.term_width.load(Ordering::Relaxed) as usize;
                    let rule = if no_color {
                        "-".repeat(width)
                    } else {
                        format!("\x1b[90m{}\x1b[0m", "\u{2500}".repeat(width))
                    };
                    let _ = writeln!(out, "{rule}");
                }

                // ── Text and inline events ────────────────────────────────────────
                Event::Text(s) => {
                    let formatted = apply_fmt(&s, in_strong, in_em, in_strike, no_color);
                    if in_link {
                        link_text.push_str(&s);
                    } else if in_table {
                        cell_buf.push_str(&formatted);
                    } else {
                        buf.push_str(&formatted);
                    }
                }
                Event::Code(s) => {
                    let formatted = fmt_inline_code(&s, no_color);
                    if in_table {
                        cell_buf.push_str(&formatted);
                    } else {
                        buf.push_str(&formatted);
                    }
                }
                Event::SoftBreak => {
                    if in_table {
                        cell_buf.push(' ');
                    } else {
                        buf.push(' ');
                    }
                }
                Event::HardBreak => {
                    if !buf.trim().is_empty() {
                        let prefix = bq_prefix(blockquote_depth, no_color);
                        let _ = writeln!(out, "{prefix}{buf}");
                        buf.clear();
                    }
                }

                // Code blocks within prose are handled by split_blocks /
                // render_code_block_inline; ignore any that slip through here.
                _ => {}
            }
        }

        // Flush any remaining inline content not followed by a paragraph end
        if !buf.trim().is_empty() {
            let prefix = bq_prefix(blockquote_depth, no_color);
            let _ = writeln!(out, "{prefix}{}", buf.trim_start());
        }
    }

    /// Render a fenced code block with syntax highlighting via syntect.
    fn render_code_block_inline(&self, block: &str) {
        let no_color = self.color.is_none();

        // Parse the fence: ```lang\n...\n```
        let mut lines = block.lines();
        let first_line = lines.next().unwrap_or("");
        let lang = first_line.trim_start_matches('`').trim();

        // Collect code lines (skip closing ```)
        let code_lines: Vec<&str> = lines.collect();
        let code_lines = if code_lines.last().is_some_and(|l| l.trim() == "```") {
            &code_lines[..code_lines.len() - 1]
        } else {
            &code_lines[..]
        };

        let border = if no_color {
            "---"
        } else {
            "\x1b[90m───\x1b[0m"
        };

        let mut guard = self.output.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let out = &mut **guard;

        let Some(theme) = self.theme_set.themes.get(&self.theme_name) else {
            // Theme missing — print without highlighting
            let _ = writeln!(out, "{border}");
            for line in code_lines {
                let _ = writeln!(out, "  {line}");
            }
            let _ = writeln!(out, "{border}");
            return;
        };

        let syntax = if lang.is_empty() {
            self.syntax_set.find_syntax_plain_text()
        } else {
            self.syntax_set
                .find_syntax_by_token(lang)
                .unwrap_or_else(|| self.syntax_set.find_syntax_plain_text())
        };

        let mut highlighter = HighlightLines::new(syntax, theme);

        let _ = writeln!(out, "{border}");
        for line in code_lines {
            if no_color || !self.color.supports_256() {
                let _ = writeln!(out, "  {line}");
            } else {
                match highlighter.highlight_line(line, &self.syntax_set) {
                    Ok(ranges) => {
                        // Use 24-bit escape sequences only for true-color terminals;
                        // fall back to the same sequence for 256-color (most terminals
                        // accept 24-bit even when reporting 256color via TERM).
                        let escaped = as_24_bit_terminal_escaped(&ranges, false);
                        let _ = writeln!(out, "  {escaped}\x1b[0m");
                    }
                    Err(_) => {
                        let _ = writeln!(out, "  {line}");
                    }
                }
            }
        }
        let _ = writeln!(out, "{border}");
    }
}

impl Default for MarkdownRenderer {
    fn default() -> Self {
        Self::new(Arc::new(AtomicU16::new(80)), ColorLevel::default())
    }
}

/// Find the byte position past a complete fenced code block (opening +
/// closing).
///
/// Tracks the opening fence length so that inner fences of a different length
/// (e.g. 4 backticks inside a 3-backtick block) are not treated as the close.
fn find_closing_fence_full(s: &str) -> Option<usize> {
    // s starts with ``` — find the opening fence delimiter
    let first_line_end = s.find('\n')? + 1;
    let first_line = &s[..first_line_end - 1];
    let fence_len = first_line.chars().take_while(|&c| c == '`').count().max(3);
    let closing = "`".repeat(fence_len);

    let after_open = first_line_end;
    let rest = &s[after_open..];
    let mut prev = 0;
    for (nl_pos, _) in rest.match_indices('\n') {
        let line = rest[prev..nl_pos].trim();
        // Closing fence: exactly `fence_len` backticks, nothing else (no more backticks
        // after)
        if line == closing
            || (line.starts_with(&closing) && !line[closing.len()..].starts_with('`'))
        {
            return Some(after_open + nl_pos + 1);
        }
        prev = nl_pos + 1;
    }
    // Check last line (no trailing newline)
    let last = rest[prev..].trim();
    if last == closing {
        return Some(after_open + rest.len());
    }
    None
}

/// Return true if `trimmed` (already stripped of leading whitespace) starts a
/// Markdown list item: unordered (`- `, `* `, `+ `) or ordered (`1. `).
fn has_list_start(trimmed: &str) -> bool {
    if trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ") {
        return true;
    }
    // Ordered list: one or more digits followed by ". "
    if let Some((prefix, _)) = trimmed.split_once(". ") {
        return !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_digit());
    }
    false
}

/// Split a full markdown text into blocks for rendering.
fn split_blocks(text: &str) -> Vec<&str> {
    let mut blocks = Vec::new();
    let mut start = 0;
    let mut fence_len: Option<usize> = None; // Some(n) when inside a fence of n backticks
    let bytes = text.as_bytes();

    let mut i = 0;
    while i < text.len() {
        let line_start = i;
        // Find end of line
        while i < bytes.len() && bytes[i] != b'\n' {
            i += 1;
        }
        let line = &text[line_start..i];
        if i < bytes.len() {
            i += 1; // skip \n
        }

        let trimmed = line.trim();
        if let Some(open_len) = fence_len {
            // Inside a code fence — look for matching close
            let close_len = trimmed.chars().take_while(|&c| c == '`').count();
            let is_close =
                close_len >= open_len && trimmed.chars().skip(close_len).all(char::is_whitespace);
            if is_close {
                fence_len = None;
                let block = &text[start..i];
                if !block.trim().is_empty() {
                    blocks.push(block);
                }
                start = i;
            }
        } else if trimmed.starts_with("```") {
            let open_len = trimmed.chars().take_while(|&c| c == '`').count();
            fence_len = Some(open_len);
        } else if trimmed.is_empty() && i < text.len() {
            // If the current block contains list items, check whether the
            // content after this blank line continues the same list.  If so,
            // keep accumulating to avoid splitting a numbered list across blocks
            // (which would reset item numbering on each fragment).
            let current_block = &text[start..line_start];
            let block_has_list = current_block
                .lines()
                .any(|l| has_list_start(l.trim_start()));

            if block_has_list {
                let rest = &text[i..];
                let next_nonempty = rest.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
                let next_t = next_nonempty.trim_start();
                // Keep accumulating if the next non-empty line is a list item or
                // indented continuation (e.g. wrapped paragraph inside a list item).
                let continues_list = has_list_start(next_t)
                    || next_nonempty.starts_with("  ")
                    || next_nonempty.starts_with('\t');
                if !continues_list {
                    // Different content — split here
                    if !current_block.trim().is_empty() {
                        blocks.push(current_block);
                    }
                    start = i;
                }
                // else: intra-list blank line — continue accumulating
            } else {
                // Regular paragraph break (outside fence, no list)
                let block = &text[start..line_start];
                if !block.trim().is_empty() {
                    blocks.push(block);
                }
                start = i;
            }
        }
    }

    // Remainder
    if start < text.len() {
        let block = &text[start..];
        if !block.trim().is_empty() {
            blocks.push(block);
        }
    }

    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_boundary_paragraph() {
        let r = MarkdownRenderer {
            buffer: "Hello world\n\nSecond paragraph".to_string(),
            ..Default::default()
        };
        let boundary = r.find_block_boundary();
        assert_eq!(boundary, Some(11)); // "Hello world" ends at 11
    }

    #[test]
    fn block_boundary_code_fence() {
        let r = MarkdownRenderer {
            buffer: "```rust\nfn main() {}\n```\nmore text".to_string(),
            ..Default::default()
        };
        let boundary = r.find_block_boundary();
        assert!(boundary.is_some());
        let end = boundary.unwrap();
        let block = &r.buffer[..end];
        assert!(block.contains("fn main()"));
        assert!(block.contains("```"));
    }

    #[test]
    fn block_boundary_no_closing_fence() {
        let r = MarkdownRenderer {
            buffer: "```rust\nfn main() {}".to_string(),
            ..Default::default()
        };
        let boundary = r.find_block_boundary();
        assert_eq!(boundary, None); // No closing fence yet
    }

    #[test]
    fn split_blocks_basic() {
        let text = "# Title\n\nSome paragraph\n\n```rust\nlet x = 1;\n```\n\nEnd";
        let blocks = split_blocks(text);
        assert_eq!(blocks.len(), 4);
        assert!(blocks[0].contains("Title"));
        assert!(blocks[1].contains("paragraph"));
        assert!(blocks[2].contains("let x = 1"));
        assert!(blocks[3].contains("End"));
    }

    #[test]
    fn streaming_renders_completed_blocks() {
        let mut r = MarkdownRenderer::default();
        // Push partial — no render yet
        r.push_delta("Hello ");
        assert!(!r.buffer.is_empty());
        // Push paragraph break — block should be consumed
        r.push_delta("world\n\nNext");
        // "Hello world" was rendered, "Next" remains in buffer
        assert_eq!(r.buffer, "Next");
    }

    #[test]
    fn reset_clears_state() {
        let mut r = MarkdownRenderer {
            buffer: "leftover".to_string(),
            ..Default::default()
        };
        r.reset();
        assert!(r.buffer.is_empty());
    }

    #[test]
    fn find_closing_fence_4_backtick() {
        let s = "````rust\ncode here\n````\n";
        let pos = find_closing_fence_full(s);
        assert!(pos.is_some());
        assert_eq!(pos.unwrap(), s.len());
    }

    #[test]
    fn find_closing_fence_inner_3_does_not_close_4() {
        // A 3-backtick fence inside a 4-backtick block should not close it
        let s = "````\ninner ```\nstill inside\n````\n";
        let pos = find_closing_fence_full(s);
        assert!(pos.is_some());
        let block = &s[..pos.unwrap()];
        assert!(block.contains("still inside"));
    }

    #[test]
    fn find_closing_fence_no_newline_returns_none() {
        // No newline at all — can't even find the opening line end
        assert_eq!(find_closing_fence_full("```rust"), None);
    }

    #[test]
    fn split_blocks_numbered_list_across_blank_lines() {
        let text = "1. First item\n\n2. Second item\n\n3. Third item";
        let blocks = split_blocks(text);
        // Should be kept as one block (list continuity)
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].contains("First"));
        assert!(blocks[0].contains("Third"));
    }

    #[test]
    fn has_list_start_all_variants() {
        assert!(has_list_start("- item"));
        assert!(has_list_start("* item"));
        assert!(has_list_start("+ item"));
        assert!(has_list_start("1. item"));
        assert!(has_list_start("42. item"));
    }

    #[test]
    fn has_list_start_non_list() {
        assert!(!has_list_start("just text"));
        assert!(!has_list_start(""));
        assert!(!has_list_start("-no space"));
        assert!(!has_list_start("a. not a number"));
    }

    #[test]
    fn streaming_partial_code_fence_not_rendered() {
        let mut r = MarkdownRenderer::default();
        // Push an opening fence without closing — should not render
        r.push_delta("```rust\nfn main() {}\n");
        assert!(r.buffer.contains("```rust"));
        // Now close it
        r.push_delta("```\n\nAfter code");
        // The code block should be consumed, "After code" remains
        assert!(!r.buffer.contains("```rust"));
        assert!(r.buffer.contains("After code"));
    }
}
