use std::{
    collections::HashMap,
    io::Write,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU16, Ordering},
    },
    time::Duration,
};

use colored::Colorize;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

use crate::{markdown::MarkdownRenderer, term_caps::ColorLevel};

/// State for a tool currently being executed (spinner + metadata).
pub(crate) struct ActiveTool {
    pub(crate) name: String,
    pub(crate) category: Option<String>,
    pub(crate) progress_bar: ProgressBar,
}

/// Write adapter that collects bytes into a shared `Vec<u8>`.
///
/// Used to capture `MarkdownRenderer` output so we can post-process it
/// (e.g. add dim prefix) before forwarding to `MultiProgress::println`.
struct VecSink(Arc<Mutex<Vec<u8>>>);

impl Write for VecSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Map a category tag to a display icon.
pub(crate) fn category_icon(category: Option<&str>) -> &'static str {
    match category {
        Some("search") => "\u{1f50d}",  // 🔍
        Some("code") => "\u{1f4bb}",    // 💻
        Some("http") => "\u{1f310}",    // 🌐
        Some("memory") => "\u{1f4c1}",  // 📁
        Some("schedule") => "\u{23f0}", // ⏰
        _ => "\u{2699}\u{fe0f}",        // ⚙️
    }
}

/// Renders a single turn of streaming chat output.
///
/// Centralises all per-turn rendering state and logic, leaving the WS task
/// responsible only for decoding messages and forwarding signals through
/// channels. One `ChatRenderer` instance is created per reconnect loop
/// iteration and reused across turns.
pub(crate) struct ChatRenderer {
    multi: MultiProgress,
    markdown: MarkdownRenderer,
    spinner_style: ProgressStyle,
    term_width: Arc<AtomicU16>,
    active_tools: HashMap<String, ActiveTool>,
    current_agent: String,
    streaming: bool,
    thinking_shown: bool,
    think_enabled: Arc<AtomicBool>,
    current_response: String,
    /// Accumulated thinking content, flushed as markdown before the first
    /// response delta.
    thinking_buffer: String,
}

impl ChatRenderer {
    pub(crate) fn new(
        multi: MultiProgress,
        term_width: Arc<AtomicU16>,
        color: ColorLevel,
        think_enabled: Arc<AtomicBool>,
    ) -> Self {
        let spinner_style = ProgressStyle::with_template("{spinner:.cyan} {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_spinner())
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏");

        let markdown = MarkdownRenderer::new(term_width.clone(), color);

        Self {
            multi,
            markdown,
            spinner_style,
            term_width,
            active_tools: HashMap::new(),
            current_agent: String::new(),
            streaming: false,
            thinking_shown: false,
            think_enabled,
            current_response: String::new(),
            thinking_buffer: String::new(),
        }
    }

    // ── Per-event handlers ────────────────────────────────────────────────

    pub(crate) fn on_agent_switch(&mut self, display_name: String) {
        self.current_agent = display_name;
    }

    pub(crate) fn on_principles_used(&mut self, count: u32) {
        self.multi
            .println(format!(
                "  {}",
                format!(
                    "\u{1f9e0} {count} principle{} applied",
                    if count == 1 { "" } else { "s" }
                )
                .dimmed()
            ))
            .ok();
    }

    pub(crate) fn on_context_info(
        &mut self,
        history_tokens: u32,
        context_window: u32,
        messages_truncated: u32,
        summary_generated: bool,
    ) {
        let pct = u64::from(history_tokens) * 100 / u64::from(context_window.max(1));
        let msg = format!(
            "\u{26a0} Context: {pct}% | {messages_truncated} message{} removed",
            if messages_truncated == 1 { "" } else { "s" }
        );
        if pct >= 85 {
            self.multi.println(format!("  {}", msg.red())).ok();
            self.multi
                .println(format!(
                    "  {}",
                    "Tip: use /memory clear to start a fresh session context."
                        .red()
                        .dimmed()
                ))
                .ok();
        } else {
            self.multi.println(format!("  {}", msg.yellow())).ok();
        }
        if summary_generated {
            self.multi
                .println(format!(
                    "  {}",
                    "\u{1f4dd} Context was automatically summarized to free space.".dimmed()
                ))
                .ok();
        }
    }

    pub(crate) fn on_thinking_delta(&mut self, delta: &str) {
        if self.think_enabled.load(Ordering::Relaxed) {
            if !self.thinking_shown {
                self.multi
                    .println(format!("  {}", "\u{25cc} thinking".dimmed()))
                    .ok();
                self.thinking_shown = true;
            }
            // Accumulate; flushed as markdown before the first response delta.
            self.thinking_buffer.push_str(delta);
        } else if !self.thinking_shown {
            self.multi
                .println(format!("  {}", "\u{25cc} thinking...".dimmed()))
                .ok();
            self.thinking_shown = true;
        }
    }

    pub(crate) fn on_delta(&mut self, data: &str) {
        if !self.streaming {
            self.flush_thinking();
            self.print_agent_header();
            self.streaming = true;
            self.thinking_shown = false;
        }
        self.current_response.push_str(data);
        self.markdown.push_delta(data);
    }

    pub(crate) fn on_tool_exec_start(
        &mut self,
        name: String,
        id: String,
        input_summary: Option<String>,
        category: Option<String>,
    ) {
        let icon = category_icon(category.as_deref());
        let label = match input_summary {
            Some(s) => format!("{icon} {name}: {s}"),
            None => format!("{icon} {name}..."),
        };
        let pb = self.multi.add(ProgressBar::new_spinner());
        pb.set_style(self.spinner_style.clone());
        pb.set_message(label);
        pb.enable_steady_tick(Duration::from_millis(80));
        self.active_tools.insert(
            id,
            ActiveTool {
                name,
                category,
                progress_bar: pb,
            },
        );
    }

    pub(crate) fn on_tool_exec_end(
        &mut self,
        id: &str,
        success: bool,
        duration_ms: u64,
        error: Option<String>,
        result_summary: Option<String>,
    ) {
        let dur = crate::util::format_duration_ms(duration_ms);
        if let Some(tool) = self.active_tools.remove(id) {
            let icon = category_icon(tool.category.as_deref());
            if success {
                let suffix = result_summary
                    .map(|s| format!(" \u{2014} {s}"))
                    .unwrap_or_default();
                tool.progress_bar.finish_with_message(format!(
                    "{} {icon} {} done ({dur}){suffix}",
                    "\u{2713}".green(),
                    tool.name,
                ));
            } else {
                let suffix = error
                    .or(result_summary)
                    .map(|s| format!(" \u{2014} {s}"))
                    .unwrap_or_default();
                tool.progress_bar.finish_with_message(format!(
                    "{} {icon} {} failed ({dur}){suffix}",
                    "\u{2717}".red(),
                    tool.name,
                ));
            }
        } else {
            // Orphan end event — no matching spinner
            if success {
                self.multi
                    .println(format!("  {} {id} done ({dur})", "\u{2713}".green()))
                    .ok();
            } else {
                self.multi
                    .println(format!("  {} {id} failed ({dur})", "\u{2717}".red()))
                    .ok();
            }
        }
    }

    /// Handle a `Done` event. Drains orphan spinners, flushes the markdown
    /// renderer, and returns the accumulated response text for this turn.
    pub(crate) fn on_done(&mut self) -> String {
        let had_tools = !self.active_tools.is_empty();
        for (_, tool) in self.active_tools.drain() {
            tool.progress_bar.finish_and_clear();
            self.multi.println(format!("[tool: {}]", tool.name)).ok();
        }
        if self.streaming {
            if !self.markdown.flush() {
                self.multi.println("").ok();
            }
        } else if !had_tools {
            self.multi
                .println(format!("  {}", "(empty response)".dimmed()))
                .ok();
        }
        self.reset_turn();
        std::mem::take(&mut self.current_response)
    }

    /// Render a non-streaming final message.
    pub(crate) fn on_final(&mut self, content: &str) {
        self.print_agent_header();
        self.markdown.render_full(content);
        self.multi.println("").ok();
        self.thinking_shown = false;
    }

    /// Print a warning line (e.g. stop-reason notice).
    pub(crate) fn warn(&mut self, msg: String) {
        self.multi.println(msg).ok();
    }

    /// Handle an unrecognised WebSocket message (silent).
    pub(crate) fn on_unknown(_raw: &str) {}

    /// Render an inline media attachment.
    pub(crate) fn on_media(&mut self, mime_type: &str, data_base64: &str, caption: Option<&str>) {
        crate::media::render_media(mime_type, data_base64, caption, Some(&self.multi));
    }

    /// Clear all active tool spinners silently (called on WS timeout).
    pub(crate) fn drain_tools_on_timeout(&mut self) {
        for (_, tool) in self.active_tools.drain() {
            tool.progress_bar.finish_and_clear();
        }
    }

    // ── Private helpers ───────────────────────────────────────────────────

    /// Render accumulated thinking content as dim-indented markdown, then clear
    /// the buffer. Called just before the first response delta is processed.
    fn flush_thinking(&mut self) {
        let content = std::mem::take(&mut self.thinking_buffer);
        if content.is_empty() {
            return;
        }
        let raw = Arc::new(Mutex::new(Vec::<u8>::new()));
        let sink = VecSink(raw.clone());
        let renderer = MarkdownRenderer::with_output(
            self.term_width.clone(),
            // No-color so the rendered output contains no ANSI codes — dim is
            // applied below when forwarding through MultiProgress::println.
            ColorLevel::None,
            Arc::new(Mutex::new(Box::new(sink))),
        );
        renderer.render_full(&content);
        drop(renderer);
        let bytes = raw
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let text = String::from_utf8_lossy(&bytes);
        for line in text.lines() {
            self.multi
                .println(format!("  {} {}", "\u{2502}".dimmed(), line.dimmed()))
                .ok();
        }
    }

    fn print_agent_header(&self) {
        let label = if self.current_agent.is_empty() {
            "Agent"
        } else {
            &self.current_agent
        };
        let width = self.term_width.load(Ordering::Relaxed) as usize;
        let bar_len = width.saturating_sub(label.len() + 6);
        let bar = "\u{2500}".repeat(bar_len);
        self.multi
            .println(format!(
                "\n{}",
                format!("\u{2500}\u{2500}\u{2500}\u{2500} {label} {bar}").dimmed()
            ))
            .ok();
    }

    /// Reset per-turn state (called from `on_done` and externally after
    /// reconnect).
    pub(crate) fn reset_turn(&mut self) {
        self.markdown.reset();
        self.streaming = false;
        self.thinking_shown = false;
        self.thinking_buffer.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_icon_known_categories() {
        assert_eq!(category_icon(Some("search")), "\u{1f50d}");
        assert_eq!(category_icon(Some("code")), "\u{1f4bb}");
        assert_eq!(category_icon(Some("http")), "\u{1f310}");
        assert_eq!(category_icon(Some("memory")), "\u{1f4c1}");
        assert_eq!(category_icon(Some("schedule")), "\u{23f0}");
    }

    #[test]
    fn category_icon_unknown_returns_gear() {
        assert_eq!(category_icon(None), "\u{2699}\u{fe0f}");
        assert_eq!(category_icon(Some("other")), "\u{2699}\u{fe0f}");
    }
}
