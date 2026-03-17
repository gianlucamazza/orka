use std::io::Write;

use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;
use syntect::util::as_24_bit_terminal_escaped;
use termimad::MadSkin;

/// Renders markdown to the terminal with syntax highlighting for code blocks.
///
/// Supports two modes:
/// - **Streaming** (`push_delta` + `flush`): block-buffered rendering for progressive output.
/// - **Full** (`render_full`): renders a complete markdown string at once.
pub struct MarkdownRenderer {
    skin: MadSkin,
    syntax_set: SyntaxSet,
    theme_name: String,
    buffer: String,
    in_code_fence: bool,
}

impl MarkdownRenderer {
    pub fn new() -> Self {
        let mut skin = MadSkin::default();
        // Headers: green bold
        skin.headers[0].set_fg(termimad::crossterm::style::Color::Green);
        skin.headers[1].set_fg(termimad::crossterm::style::Color::Green);
        skin.headers[2].set_fg(termimad::crossterm::style::Color::Green);
        // Bold: white bold (default is already bold, just ensure fg)
        skin.bold.set_fg(termimad::crossterm::style::Color::White);
        // Inline code: cyan
        skin.inline_code
            .set_fg(termimad::crossterm::style::Color::Cyan);
        // Blockquotes: grey/dimmed
        skin.quote_mark
            .set_fg(termimad::crossterm::style::Color::DarkGrey);

        Self {
            skin,
            syntax_set: SyntaxSet::load_defaults_newlines(),
            theme_name: "base16-eighties.dark".to_string(),
            buffer: String::new(),
            in_code_fence: false,
        }
    }

    /// Feed a streaming delta chunk. Renders any completed blocks immediately.
    pub fn push_delta(&mut self, delta: &str) {
        self.buffer.push_str(delta);
        self.render_completed_blocks();
    }

    /// Flush and render whatever remains in the buffer (call on Done).
    pub fn flush(&mut self) {
        if !self.buffer.is_empty() {
            let remaining = std::mem::take(&mut self.buffer);
            self.render_block(&remaining);
        }
        self.in_code_fence = false;
    }

    /// Reset state for a new conversation turn.
    pub fn reset(&mut self) {
        self.buffer.clear();
        self.in_code_fence = false;
    }

    /// Render a complete markdown string (non-streaming path).
    pub fn render_full(&self, text: &str) {
        let blocks = split_blocks(text);
        for block in blocks {
            if block.trim_start().starts_with("```") && block.matches("```").count() >= 2 {
                self.render_code_block_inline(block);
            } else {
                let normalized = normalize_gfm_tables(block);
                self.skin.print_text(&normalized);
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
                    self.buffer.drain(..end);
                    // Skip leading newlines between blocks
                    while self.buffer.starts_with('\n') {
                        self.buffer.remove(0);
                    }
                    self.render_block(&block);
                }
                None => break,
            }
        }
    }

    /// Find the byte offset of the end of the next complete block.
    fn find_block_boundary(&self) -> Option<usize> {
        let buf = &self.buffer;

        if self.in_code_fence {
            // Look for closing fence
            // The opening fence is already consumed, so find closing ```
            if let Some(pos) = find_closing_fence(buf) {
                return Some(pos);
            }
            None
        } else {
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
            None
        }
    }

    /// Render a single completed block.
    fn render_block(&mut self, block: &str) {
        let trimmed = block.trim();
        if trimmed.is_empty() {
            return;
        }

        if trimmed.starts_with("```") && trimmed.matches("```").count() >= 2 {
            self.render_code_block_inline(trimmed);
            self.in_code_fence = false;
        } else if trimmed.starts_with("```") {
            // Unclosed code fence at flush — render as-is
            self.skin.print_text(block);
            self.in_code_fence = false;
        } else {
            let normalized = normalize_gfm_tables(block);
            self.skin.print_text(&normalized);
        }
    }

    /// Render a fenced code block with syntax highlighting via syntect.
    fn render_code_block_inline(&self, block: &str) {
        let ts = ThemeSet::load_defaults();
        let theme = &ts.themes[&self.theme_name];

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

        let syntax = if lang.is_empty() {
            self.syntax_set.find_syntax_plain_text()
        } else {
            self.syntax_set
                .find_syntax_by_token(lang)
                .unwrap_or_else(|| self.syntax_set.find_syntax_plain_text())
        };

        let mut highlighter = HighlightLines::new(syntax, theme);
        let mut out = std::io::stdout().lock();

        // Dim border top
        let _ = writeln!(out, "\x1b[90m───\x1b[0m");
        for line in code_lines {
            match highlighter.highlight_line(line, &self.syntax_set) {
                Ok(ranges) => {
                    let escaped = as_24_bit_terminal_escaped(&ranges, false);
                    let _ = writeln!(out, "  {escaped}\x1b[0m");
                }
                Err(_) => {
                    let _ = writeln!(out, "  {line}");
                }
            }
        }
        let _ = writeln!(out, "\x1b[90m───\x1b[0m");
    }
}

/// Normalize GFM tables so termimad/minimad can parse them correctly.
///
/// GFM separator rows often have spaces around dashes (e.g. `| --- | --- |`)
/// which minimad doesn't trim. This function strips those spaces and normalizes
/// data row cells too for cleaner rendering.
fn normalize_gfm_tables(text: &str) -> String {
    // Quick check: skip work if no pipe characters at all
    if !text.contains('|') {
        return text.to_string();
    }

    let mut out = String::with_capacity(text.len());
    let mut in_table = false;

    for line in text.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with('|') && trimmed.ends_with('|') {
            in_table = true;

            // Split into cells (skip first/last empty segments from leading/trailing |)
            let cells: Vec<&str> = trimmed
                .strip_prefix('|')
                .unwrap()
                .strip_suffix('|')
                .unwrap()
                .split('|')
                .collect();

            // Check if this is a separator row: all cells match :?-+:? after trimming
            let is_separator = cells.iter().all(|cell| {
                let c = cell.trim();
                if c.is_empty() {
                    return false;
                }
                let c = c.strip_prefix(':').unwrap_or(c);
                let c = c.strip_suffix(':').unwrap_or(c);
                !c.is_empty() && c.chars().all(|ch| ch == '-')
            });

            if is_separator {
                // Normalize separator: trim whitespace but keep : alignment markers
                let normalized: Vec<String> =
                    cells.iter().map(|cell| cell.trim().to_string()).collect();
                out.push('|');
                out.push_str(&normalized.join("|"));
                out.push('|');
            } else {
                // Data row: trim cell contents
                let normalized: Vec<String> = cells
                    .iter()
                    .map(|cell| {
                        let t = cell.trim();
                        format!(" {t} ")
                    })
                    .collect();
                out.push('|');
                out.push_str(&normalized.join("|"));
                out.push('|');
            }
        } else {
            if in_table {
                in_table = false;
            }
            out.push_str(line);
        }
        out.push('\n');
    }

    // Remove trailing newline if the original didn't have one
    if !text.ends_with('\n') {
        out.pop();
    }

    out
}

/// Find the byte position past a complete fenced code block (opening + closing).
fn find_closing_fence_full(s: &str) -> Option<usize> {
    // s starts with ``` — find the end of the opening line
    let after_open = s.find('\n')? + 1;
    // Find closing ``` after the opening line
    let rest = &s[after_open..];
    for (i, line) in rest.lines().enumerate() {
        if line.trim() == "```" {
            // Calculate byte offset: after_open + bytes up to end of this line
            let offset: usize = rest.lines().take(i + 1).map(|l| l.len() + 1).sum();
            return Some(after_open + offset);
        }
    }
    None
}

/// Find closing fence when we know we're already inside a code block
/// (the opening fence was in a previous block).
fn find_closing_fence(s: &str) -> Option<usize> {
    for (i, line) in s.lines().enumerate() {
        if line.trim() == "```" {
            let offset: usize = s.lines().take(i + 1).map(|l| l.len() + 1).sum();
            return Some(offset);
        }
    }
    None
}

/// Split a full markdown text into blocks for rendering.
fn split_blocks(text: &str) -> Vec<&str> {
    let mut blocks = Vec::new();
    let mut start = 0;
    let mut in_fence = false;
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

        if line.trim().starts_with("```") {
            if !in_fence {
                in_fence = true;
            } else {
                // End of fence — this line ends the block
                in_fence = false;
                let block = &text[start..i];
                if !block.trim().is_empty() {
                    blocks.push(block);
                }
                start = i;
            }
            continue;
        }

        if !in_fence && line.trim().is_empty() && i < text.len() {
            // Paragraph break
            let block = &text[start..line_start];
            if !block.trim().is_empty() {
                blocks.push(block);
            }
            start = i;
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
        let mut r = MarkdownRenderer::new();
        r.buffer = "Hello world\n\nSecond paragraph".to_string();
        let boundary = r.find_block_boundary();
        assert_eq!(boundary, Some(11)); // "Hello world" ends at 11
    }

    #[test]
    fn block_boundary_code_fence() {
        let mut r = MarkdownRenderer::new();
        r.buffer = "```rust\nfn main() {}\n```\nmore text".to_string();
        let boundary = r.find_block_boundary();
        assert!(boundary.is_some());
        let end = boundary.unwrap();
        let block = &r.buffer[..end];
        assert!(block.contains("fn main()"));
        assert!(block.contains("```"));
    }

    #[test]
    fn block_boundary_no_closing_fence() {
        let mut r = MarkdownRenderer::new();
        r.buffer = "```rust\nfn main() {}".to_string();
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
        let mut r = MarkdownRenderer::new();
        // Push partial — no render yet
        r.push_delta("Hello ");
        assert!(!r.buffer.is_empty());
        // Push paragraph break — block should be consumed
        r.push_delta("world\n\nNext");
        // "Hello world" was rendered, "Next" remains in buffer
        assert_eq!(r.buffer, "Next");
    }

    #[test]
    fn normalize_gfm_separator_row() {
        let input = "| Name | Age |\n| --- | --- |\n| Alice | 30 |";
        let out = normalize_gfm_tables(input);
        assert_eq!(out, "| Name | Age |\n|---|---|\n| Alice | 30 |");
    }

    #[test]
    fn normalize_gfm_alignment_markers() {
        let input = "| Left | Center | Right |\n| :--- | :---: | ---: |\n| a | b | c |";
        let out = normalize_gfm_tables(input);
        assert_eq!(
            out,
            "| Left | Center | Right |\n|:---|:---:|---:|\n| a | b | c |"
        );
    }

    #[test]
    fn normalize_gfm_no_table_passthrough() {
        let input = "Just some **bold** text\nand another line";
        let out = normalize_gfm_tables(input);
        assert_eq!(out, input);
    }

    #[test]
    fn normalize_gfm_trims_data_cells() {
        let input = "|  Name  |  Age  |\n|---|---|\n|  Alice  |  30  |";
        let out = normalize_gfm_tables(input);
        assert_eq!(out, "| Name | Age |\n|---|---|\n| Alice | 30 |");
    }

    #[test]
    fn reset_clears_state() {
        let mut r = MarkdownRenderer::new();
        r.buffer = "leftover".to_string();
        r.in_code_fence = true;
        r.reset();
        assert!(r.buffer.is_empty());
        assert!(!r.in_code_fence);
    }
}
