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
    theme_set: ThemeSet,
    theme_name: String,
    buffer: String,
    /// Cached value of `NO_COLOR` env var at construction time.
    no_color: bool,
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
            theme_set: ThemeSet::load_defaults(),
            theme_name: "base16-eighties.dark".to_string(),
            buffer: String::new(),
            no_color: std::env::var_os("NO_COLOR").is_some(),
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
        if !self.buffer.is_empty() {
            let remaining = std::mem::take(&mut self.buffer);
            // Use render_full so that mixed content (prose then code fence) is
            // handled correctly even when they arrive in the same final chunk.
            self.render_full(&remaining);
            true
        } else {
            false
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
            // UI-12: unclosed code fence at flush — print raw to avoid termimad mangling
            // (termimad would interpret the opening ``` as an unterminated code block)
            for line in trimmed.lines() {
                println!("{line}");
            }
        } else {
            let normalized = normalize_gfm_tables(block);
            self.skin.print_text(&normalized);
        }
    }

    /// Render a fenced code block with syntax highlighting via syntect.
    fn render_code_block_inline(&self, block: &str) {
        // UI-5: respect NO_COLOR — cached at construction time to avoid per-call env lookup
        let no_color = self.no_color;

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

        let Some(theme) = self.theme_set.themes.get(&self.theme_name) else {
            // Theme missing — print without highlighting
            let mut out = std::io::stdout().lock();
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
        let mut out = std::io::stdout().lock();

        let _ = writeln!(out, "{border}");
        for line in code_lines {
            if no_color {
                let _ = writeln!(out, "  {line}");
            } else {
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
        }
        let _ = writeln!(out, "{border}");
    }
}

impl Default for MarkdownRenderer {
    fn default() -> Self {
        Self::new()
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
                // UI-15: add a blank line at the table→prose transition so
                // content doesn't get glued directly to the table's last row.
                in_table = false;
                out.push('\n');
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
        // Closing fence: exactly `fence_len` backticks, nothing else (no more backticks after)
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
                close_len >= open_len && trimmed.chars().skip(close_len).all(|c| c.is_whitespace());
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
            // UI-21: if the current block contains list items, check whether the
            // content after this blank line continues the same list.  If so, keep
            // accumulating to avoid handing termimad a split numbered list (which
            // would reset item numbering on each fragment).
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
    fn normalize_gfm_table_to_prose_transition() {
        // UI-15: blank line should be inserted between table and prose
        let input = "| A | B |\n|---|---|\n| 1 | 2 |\nSome text after";
        let out = normalize_gfm_tables(input);
        // The prose should be preceded by an extra blank line
        assert!(out.contains("\n\nSome text after"));
    }

    #[test]
    fn streaming_partial_code_fence_not_rendered() {
        let mut r = MarkdownRenderer::new();
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
