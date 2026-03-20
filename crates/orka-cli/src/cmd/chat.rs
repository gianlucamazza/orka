use std::collections::HashMap;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use colored::Colorize;
use futures_util::stream::SplitStream;
use futures_util::{SinkExt, StreamExt};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use orka_core::parse_slash_command;
use orka_core::stream::StreamChunkKind;
use rustyline::error::ReadlineError;
use serde_json::json;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use unicode_width::UnicodeWidthStr;

use crate::client::{OrkaClient, Result};
use crate::completion::OrkaHelper;
use crate::prompt::build_prompt;
use crate::protocol::{WsMessage, classify_ws_message};
use crate::shell::{self, Builtin, InputAction};

/// Map a category tag to a display icon.
fn category_icon(category: Option<&str>) -> &'static str {
    match category {
        Some("search") => "\u{1f50d}",  // 🔍
        Some("code") => "\u{1f4bb}",    // 💻
        Some("http") => "\u{1f310}",    // 🌐
        Some("memory") => "\u{1f4c1}",  // 📁
        Some("schedule") => "\u{23f0}", // ⏰
        _ => "\u{2699}\u{fe0f}",        // ⚙️
    }
}

/// Format a duration smartly: `< 1s` → `142ms`, `≥ 1s` → `1.2s`.
fn format_duration(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms}ms")
    } else {
        format!("{:.1}s", ms as f64 / 1000.0)
    }
}

/// Print the box-drawn welcome banner.
fn print_banner() {
    let version = env!("CARGO_PKG_VERSION");
    let inner = format!("  \u{25c8}  Orka Shell  v{version}  ");
    // UI-11: use display width (handles wide/ambiguous Unicode) rather than char count
    let width = inner.as_str().width();
    let bar = "\u{2500}".repeat(width);
    println!("{}", format!("\u{250c}{bar}\u{2510}").cyan().bold());
    println!("{}", format!("\u{2502}{inner}\u{2502}").cyan().bold());
    println!("{}", format!("\u{2514}{bar}\u{2518}").cyan().bold());
}

fn print_help() {
    // UI-20: use calculated column padding so descriptions align regardless of
    // command length.  COL is the visual width of the command column.
    const COL: usize = 32;
    let row = |cmd: &str, desc: &str| {
        let pad = " ".repeat(COL.saturating_sub(cmd.len()));
        println!("  {}{pad}{desc}", cmd.yellow());
    };

    print_banner();
    println!();
    println!("{}", "Shell execution:".bold());
    row("!<command>", "Execute shell command locally");
    row("!!", "Repeat last shell command");
    row("!cd <path>", "Change directory");
    row("!export K=V", "Set environment variable");
    row("!unset <K>", "Unset environment variable");
    println!();
    println!("{}", "AI agent:".bold());
    row("<text>", "Send to AI agent (default)");
    println!();
    println!("{}", "Commands:".bold());
    row("/skill <name> [k=v ...]", "Invoke a skill directly");
    row("/skills", "List available skills");
    row("/reset", "Clear conversation history");
    row("/status", "Show session info");
    row("/think", "Toggle extended thinking display");
    row(
        "/feedback [good|bad] [reason]",
        "Send feedback on last response",
    );
    row("/history", "Show conversation history");
    row("/save <file>", "Save last response to file");
    row("/help", "Show this help");
    row("/clear", "Clear screen");
    row("/quit", "Exit");
    println!();
    println!("{}", "File attachment:".bold());
    row("@<path>", "Inline a file as a code block");
    println!();
    println!("{}", "Multi-line input:".bold());
    row("\\", "trailing backslash for line continuation");
    println!();
}

/// Truncate a session ID to `first8…last5` for display.
fn truncate_sid(sid: &str) -> String {
    let chars: Vec<char> = sid.chars().collect();
    if chars.len() > 14 {
        let head: String = chars[..8].iter().collect();
        let tail: String = chars[chars.len() - 5..].iter().collect();
        format!("{head}\u{2026}{tail}")
    } else {
        sid.to_string()
    }
}

/// Resolve the history file path, creating parent dirs as needed.
fn history_path() -> PathBuf {
    let dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("orka");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("history")
}

/// Print the agent turn separator with an optional display name.
///
/// UI-1: routed through MultiProgress so it serialises with spinner redraws.
/// UI-4: adapts to the actual terminal width instead of a hardcoded 40-char bar.
fn print_agent_header(name: &str, multi: &MultiProgress) {
    let label = if name.is_empty() { "Agent" } else { name };
    let term_width = termimad::crossterm::terminal::size()
        .map(|(w, _)| w as usize)
        .unwrap_or(80);
    // 4 leading dashes + 2 spaces around the label
    let bar_len = term_width.saturating_sub(label.len() + 6);
    let bar = "\u{2500}".repeat(bar_len);
    multi
        .println(format!(
            "\n{}",
            format!("\u{2500}\u{2500}\u{2500}\u{2500} {label} {bar}").dimmed()
        ))
        .ok();
}

/// Format token counts compactly: `1.2k` for ≥1000, else the raw number.
fn fmt_tokens(n: u32) -> String {
    if n >= 1000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
}

/// Strip backslash line-continuation sequences (`\` + newline) from multi-line input.
///
/// When rustyline's Validator returns `Incomplete` for trailing `\`, it concatenates
/// continuation lines with `\n`, leaving `\\\n` sequences in the string. Removing them
/// joins the lines as intended before the text is sent to the agent.
fn strip_line_continuations(s: &str) -> String {
    s.replace("\\\n", "")
}

/// Expand `@path` tokens in a message to inline file content as code fences.
///
/// `@path/to/file.rs` is replaced with a fenced code block containing the file's
/// contents.  Unknown or unreadable paths are left as-is.
fn expand_file_attachments(text: &str) -> String {
    // Fast path: no `@` in text
    if !text.contains('@') {
        return text.to_string();
    }

    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut prev_was_whitespace_or_start = true;

    while let Some(ch) = chars.next() {
        if ch != '@' {
            prev_was_whitespace_or_start = ch.is_whitespace();
            result.push(ch);
            continue;
        }

        // Only trigger @-expansion when `@` is at position 0 or preceded by whitespace
        // This avoids matching email addresses like user@example.com
        if !prev_was_whitespace_or_start {
            result.push('@');
            prev_was_whitespace_or_start = false;
            continue;
        }

        // Collect the path token (non-whitespace characters after `@`)
        let path_str: String = chars.by_ref().take_while(|c| !c.is_whitespace()).collect();

        if path_str.is_empty() {
            result.push('@');
            prev_was_whitespace_or_start = false;
            continue;
        }

        let path = std::path::Path::new(&path_str);
        match std::fs::read_to_string(path) {
            Ok(content) => {
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                result.push_str(&format!("\n```{ext}\n{content}\n```\n"));
                // UI-18: only restore the space delimiter when take_while actually
                // consumed one (i.e. there is more text following @path).
                if chars.peek().is_some() {
                    result.push(' ');
                }
                prev_was_whitespace_or_start = true;
            }
            Err(_) => {
                // Unreadable — leave the token as-is
                result.push('@');
                result.push_str(&path_str);
                if chars.peek().is_some() {
                    result.push(' ');
                }
                prev_was_whitespace_or_start = true;
            }
        }
    }
    result
}

pub async fn run(
    client: &OrkaClient,
    session_id: Option<&str>,
    local_workspace: Option<crate::workspace::LocalWorkspace>,
) -> Result<()> {
    let sid = OrkaClient::resolve_session_id(session_id);

    // Wait for server to be ready before attempting WebSocket connection
    client.wait_for_ready(300, Duration::from_secs(1)).await?;

    // Welcome banner
    print_banner();
    // Show update notice from cache (no network I/O on the hot path)
    if let Some(info) = super::update::check_from_cache() {
        super::update::print_update_notice(&info);
    }
    // Refresh the update cache in the background
    tokio::spawn(async { super::update::check().await });
    println!(" {}  {}", "Session".dimmed(), truncate_sid(&sid).dimmed());
    if let Some(ref ws) = local_workspace {
        println!(
            " {}  {}",
            "Workspace".dimmed(),
            ws.root.display().to_string().dimmed()
        );
    }
    println!(
        "\nType messages for AI, {} for shell, {} for commands, {} to exit.\n",
        "!cmd".yellow(),
        "/help".yellow(),
        "/quit".yellow()
    );

    // Type alias for readability
    type WsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
    type WsRead = SplitStream<WsStream>;

    // Connect WebSocket — API key is sent as X-Api-Key header, not in the URL
    let ws = client.ws_connect(&sid).await?;
    let (ws_write, ws_read) = ws.split();
    let ws_write = Arc::new(tokio::sync::Mutex::new(ws_write));

    // Channel to feed new ws_read streams into the task on reconnect
    let (reconnect_ws_tx, mut reconnect_ws_rx) = tokio::sync::mpsc::unbounded_channel::<WsRead>();
    // Notified when the WS connection drops (so the REPL wait-loop can react)
    let disconnect_notify = Arc::new(tokio::sync::Notify::new());

    // WS reader task: render streaming output with spinners.
    // UI-2: use stdout so that indicatif's cursor management coordinates with
    // all other stdout writes (piping, tee, etc.) — stderr target causes
    // cross-stream interleaving artefacts.
    let multi = MultiProgress::with_draw_target(indicatif::ProgressDrawTarget::stdout_with_hz(10));
    let multi_clone = multi.clone();

    // Flag: set to false by the WS task when the connection drops
    let ws_alive = Arc::new(AtomicBool::new(true));
    let ws_alive_task = ws_alive.clone();

    // Channel to notify the REPL that a response is complete
    let (done_tx, mut done_rx) = tokio::sync::mpsc::unbounded_channel::<()>();

    // Channel to pass the completed response text back to the REPL (for /save, /history)
    let (response_tx, mut response_rx) = tokio::sync::mpsc::unbounded_channel::<String>();

    // Shared waiting spinner — set by REPL loop after send, cleared by WS task on first event
    let waiting_spinner: Arc<Mutex<Option<ProgressBar>>> = Arc::new(Mutex::new(None));
    let waiting_spinner_ws = waiting_spinner.clone();

    // Shared last usage (overwritten by each Usage chunk; read after Done)
    // (input_tokens, output_tokens, reasoning_tokens, model)
    type UsageInfo = Option<(u32, u32, Option<u32>, String)>;
    let last_usage: Arc<Mutex<UsageInfo>> = Arc::new(Mutex::new(None));
    let last_usage_ws = last_usage.clone();

    // Think toggle: when true, ThinkingDelta content is shown
    let think_enabled = Arc::new(AtomicBool::new(false));
    let think_enabled_ws = think_enabled.clone();

    let disconnect_notify_ws = disconnect_notify.clone();
    let ws_task = tokio::spawn(async move {
        let multi = multi_clone;
        let spinner_style = ProgressStyle::with_template("{spinner:.cyan} {msg}")
            .unwrap()
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏");

        let mut renderer = crate::markdown::MarkdownRenderer::new();
        let mut current_ws_read = ws_read;

        /// Clear the waiting spinner if it's still active.
        fn clear_waiting(ws: &Arc<Mutex<Option<ProgressBar>>>) {
            let mut guard = ws.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(pb) = guard.take() {
                pb.finish_and_clear();
            }
        }

        'reconnect: loop {
            let mut streaming = false;
            let mut streamed_this_turn = false;
            let mut thinking_shown = false;
            let mut active_tools: HashMap<String, (String, Option<String>, Instant, ProgressBar)> =
                HashMap::new();
            // Current agent name (updated on AgentSwitch)
            let mut current_agent = String::new();
            // Guard against duplicate Done signals in the same turn
            let mut turn_done_sent = false;
            // Accumulate response text for /save and /history
            let mut current_response = String::new();

            while let Some(msg) = current_ws_read.next().await {
                match msg {
                    Ok(msg) if msg.is_text() => {
                        let text = match msg.into_text() {
                            Ok(t) => t,
                            Err(_) => continue,
                        };

                        match classify_ws_message(&text) {
                            WsMessage::Stream(StreamChunkKind::AgentSwitch {
                                display_name,
                                ..
                            }) => {
                                clear_waiting(&waiting_spinner_ws);
                                current_agent = display_name;
                            }
                            WsMessage::Stream(StreamChunkKind::PrinciplesUsed { count }) => {
                                clear_waiting(&waiting_spinner_ws);
                                // UI-1: route through multi so output serialises with spinners
                                multi
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
                            WsMessage::Stream(StreamChunkKind::ContextInfo {
                                history_tokens,
                                context_window,
                                messages_truncated,
                                ..
                            }) => {
                                let pct =
                                    (history_tokens as u64) * 100 / (context_window.max(1) as u64);
                                let msg = format!(
                                    "\u{26a0} Context: {pct}% | {messages_truncated} message{} removed",
                                    if messages_truncated == 1 { "" } else { "s" }
                                );
                                // UI-1: route through multi
                                if pct >= 85 {
                                    multi.println(format!("  {}", msg.red())).ok();
                                    multi
                                        .println(format!(
                                            "  {}",
                                            "Tip: use /reset to start a fresh context."
                                                .red()
                                                .dimmed()
                                        ))
                                        .ok();
                                } else {
                                    multi.println(format!("  {}", msg.yellow())).ok();
                                }
                            }
                            WsMessage::Stream(StreamChunkKind::Usage {
                                input_tokens,
                                output_tokens,
                                reasoning_tokens,
                                model,
                                ..
                            }) => {
                                if let Ok(mut guard) = last_usage_ws.lock() {
                                    *guard = Some((
                                        input_tokens,
                                        output_tokens,
                                        reasoning_tokens,
                                        model,
                                    ));
                                }
                            }
                            WsMessage::Stream(StreamChunkKind::ThinkingDelta(delta)) => {
                                clear_waiting(&waiting_spinner_ws);
                                if think_enabled_ws.load(Ordering::Relaxed) {
                                    if !thinking_shown {
                                        // UI-1: route through multi
                                        multi
                                            .println(format!("  {}", "\u{25cc} thinking".dimmed()))
                                            .ok();
                                        thinking_shown = true;
                                    }
                                    // Print thinking content indented and dimmed
                                    for line in delta.lines() {
                                        multi
                                            .println(format!(
                                                "  {} {}",
                                                "\u{2502}".dimmed(),
                                                line.dimmed()
                                            ))
                                            .ok();
                                    }
                                } else if !thinking_shown {
                                    // UI-1: route through multi
                                    multi
                                        .println(format!("  {}", "\u{25cc} thinking...".dimmed()))
                                        .ok();
                                    thinking_shown = true;
                                }
                            }
                            WsMessage::Stream(StreamChunkKind::Delta(data)) => {
                                clear_waiting(&waiting_spinner_ws);
                                if !streaming {
                                    print_agent_header(&current_agent, &multi);
                                    streaming = true;
                                    thinking_shown = false;
                                }
                                streamed_this_turn = true;
                                turn_done_sent = false; // new turn is streaming
                                current_response.push_str(&data);
                                renderer.push_delta(&data);
                            }
                            WsMessage::Stream(StreamChunkKind::ToolExecStart {
                                name,
                                id,
                                input_summary,
                                category,
                            }) => {
                                clear_waiting(&waiting_spinner_ws);
                                let icon = category_icon(category.as_deref());
                                let label = match &input_summary {
                                    Some(s) => format!("{icon} {name}: {s}"),
                                    None => format!("{icon} {name}..."),
                                };
                                let pb = multi.add(ProgressBar::new_spinner());
                                pb.set_style(spinner_style.clone());
                                pb.set_message(label);
                                pb.enable_steady_tick(Duration::from_millis(80));
                                active_tools.insert(id, (name, category, Instant::now(), pb));
                            }
                            WsMessage::Stream(StreamChunkKind::ToolExecEnd {
                                id,
                                success,
                                duration_ms,
                                error,
                                result_summary,
                            }) => {
                                let dur = format_duration(duration_ms);
                                if let Some((name, category, _, pb)) = active_tools.remove(&id) {
                                    let icon = category_icon(category.as_deref());
                                    if success {
                                        let suffix = result_summary
                                            .map(|s| format!(" \u{2014} {s}"))
                                            .unwrap_or_default();
                                        pb.finish_with_message(format!(
                                            "{} {icon} {name} done ({dur}){suffix}",
                                            "\u{2713}".green()
                                        ));
                                    } else {
                                        let suffix = error
                                            .or(result_summary)
                                            .map(|s| format!(" \u{2014} {s}"))
                                            .unwrap_or_default();
                                        pb.finish_with_message(format!(
                                            "{} {icon} {name} failed ({dur}){suffix}",
                                            "\u{2717}".red()
                                        ));
                                    }
                                } else {
                                    let label = id;
                                    if success {
                                        multi
                                            .println(format!(
                                                "  {} {label} done ({dur})",
                                                "\u{2713}".green()
                                            ))
                                            .ok();
                                    } else {
                                        multi
                                            .println(format!(
                                                "  {} {label} failed ({dur})",
                                                "\u{2717}".red()
                                            ))
                                            .ok();
                                    }
                                }
                            }
                            WsMessage::Stream(StreamChunkKind::ToolStart { .. })
                            | WsMessage::Stream(StreamChunkKind::ToolEnd { .. }) => {
                                // Internal LLM events — silent
                            }
                            WsMessage::Stream(StreamChunkKind::Done) => {
                                clear_waiting(&waiting_spinner_ws);
                                // UI-3: drain orphaned tool spinners with a full line each
                                // (original used print! without flush, risking glued output).
                                // UI-1: route through multi to serialise with progress bars.
                                let had_tools = !active_tools.is_empty();
                                for (_, (name, _cat, _, pb)) in active_tools.drain() {
                                    pb.finish_and_clear();
                                    multi.println(format!("[tool: {name}]")).ok();
                                }
                                if streaming {
                                    if !renderer.flush() {
                                        // UI-1: trailing newline through multi
                                        multi.println("").ok();
                                    }
                                } else if !had_tools {
                                    // UI-19: truly empty response (no deltas, no tools) —
                                    // show a placeholder so the user isn't left confused
                                    multi
                                        .println(format!("  {}", "(empty response)".dimmed()))
                                        .ok();
                                }
                                renderer.reset();
                                streaming = false;
                                streamed_this_turn = false; // UI-10: reset between agent switches
                                thinking_shown = false;
                                if !turn_done_sent {
                                    let _ = done_tx.send(());
                                    let _ = response_tx.send(std::mem::take(&mut current_response));
                                    turn_done_sent = true;
                                }
                            }
                            WsMessage::Final(content) => {
                                clear_waiting(&waiting_spinner_ws);
                                if streamed_this_turn {
                                    streamed_this_turn = false;
                                    continue;
                                }
                                print_agent_header(&current_agent, &multi);
                                renderer.render_full(&content);
                                // UI-1: trailing newline through multi
                                multi.println("").ok();
                                thinking_shown = false;
                                if !turn_done_sent {
                                    let _ = done_tx.send(());
                                    let _ = response_tx.send(content.clone());
                                    turn_done_sent = true;
                                }
                            }
                            WsMessage::Stream(_) => {
                                // Unknown stream chunk kind — ignore
                            }
                            WsMessage::Unknown(raw) => {
                                clear_waiting(&waiting_spinner_ws);
                                // UI-1: route through multi
                                multi.println(format!("\n{raw}")).ok();
                                // Intentionally no done_tx.send() — Unknown events are not turn completions
                            }
                        }
                    }
                    Ok(msg) if msg.is_close() => break,
                    Err(e) => {
                        eprintln!("{} {e}", "Connection error:".red());
                        break;
                    }
                    _ => {}
                }
            }
            // Mark the connection as dead and notify the REPL
            ws_alive_task.store(false, Ordering::Relaxed);
            disconnect_notify_ws.notify_one();

            // Wait for a reconnected stream, or exit when the sender is dropped
            match reconnect_ws_rx.recv().await {
                Some(new_read) => {
                    current_ws_read = new_read;
                    ws_alive_task.store(true, Ordering::Relaxed);
                    renderer.reset();
                    // per-connection state is reset at the top of 'reconnect
                }
                None => break 'reconnect,
            }
        } // end 'reconnect loop
    });

    // Keepalive: send a WebSocket Ping every 30 seconds to prevent idle disconnects.
    let ws_write_ping = ws_write.clone();
    let ping_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        interval.tick().await; // consume the immediate first tick
        loop {
            interval.tick().await;
            let mut guard = ws_write_ping.lock().await;
            if guard.send(Message::Ping(Default::default())).await.is_err() {
                break;
            }
        }
    });

    // Set up rustyline in a dedicated blocking thread, communicating via channels.
    // The editor thread sends lines to us, and we send prompts back to it.
    let (prompt_tx, prompt_rx) = std::sync::mpsc::channel::<String>();
    let (line_tx, mut line_rx) =
        tokio::sync::mpsc::unbounded_channel::<std::result::Result<String, ReadlineError>>();

    let hist_path = history_path();
    let hist_path_save = hist_path.clone();

    let shell_cwd_helper = Arc::new(std::sync::Mutex::new(
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
    ));
    let shell_cwd_shared = shell_cwd_helper.clone();
    std::thread::spawn(move || {
        let config = rustyline::Config::builder()
            .auto_add_history(true)
            .max_history_size(10_000)
            .expect("valid history size")
            .build();
        let helper = OrkaHelper::new(shell_cwd_helper);
        let mut editor =
            match rustyline::Editor::<OrkaHelper, rustyline::history::DefaultHistory>::with_config(
                config,
            ) {
                Ok(mut ed) => {
                    ed.set_helper(Some(helper));
                    let _ = ed.load_history(&hist_path);
                    ed
                }
                Err(e) => {
                    let _ = line_tx.send(Err(e));
                    return;
                }
            };

        while let Ok(prompt) = prompt_rx.recv() {
            let result = editor.readline(&prompt);
            if line_tx.send(result).is_err() {
                break;
            }
        }
        let _ = editor.save_history(&hist_path_save);
    });

    // Shell state — initialise cwd and sync the shared reference used by tab-completion
    let mut cwd = {
        let guard = shell_cwd_shared.lock().unwrap_or_else(|e| e.into_inner());
        guard.clone()
    };
    let mut env_overrides: HashMap<String, String> = HashMap::new();
    let mut env_removes: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut last_exit: Option<i32> = None;
    let mut last_shell_cmd: Option<String> = None;
    let mut workspace_sent = false;

    // WS response timeout — configurable via ORKA_WS_TIMEOUT env var (seconds)
    let ws_response_timeout_secs: u64 = std::env::var("ORKA_WS_TIMEOUT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(120);

    // REPL session state
    let mut turn_history: Vec<(String, String)> = Vec::new(); // (user_input, agent_response)
    let mut last_response = String::new();
    let mut ctrl_c_count: u32 = 0;

    let waiting_spinner_style = ProgressStyle::with_template("{spinner:.cyan} {msg}")
        .unwrap()
        .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏");

    loop {
        let prompt = build_prompt(&cwd, last_exit);

        // Send prompt to the editor thread
        if prompt_tx.send(prompt).is_err() {
            break;
        }

        // Wait for the line from the editor thread
        let result = match line_rx.recv().await {
            Some(r) => r,
            None => break,
        };

        let line = match result {
            Ok(line) => {
                ctrl_c_count = 0;
                line
            }
            Err(ReadlineError::Eof) => break,
            Err(ReadlineError::Interrupted) => {
                ctrl_c_count += 1;
                if ctrl_c_count >= 2 {
                    break;
                }
                println!("{}", "Press Ctrl-C again or type /quit to exit.".dimmed());
                continue;
            }
            Err(e) => {
                eprintln!("Input error: {e}");
                break;
            }
        };

        match shell::classify_input(&line) {
            InputAction::Empty => continue,

            InputAction::Error(msg) => {
                eprintln!("{msg}");
                last_exit = Some(1);
            }

            InputAction::ShellExec(cmd) => {
                last_exit = shell::execute_shell(&cmd, &cwd, &env_overrides, &env_removes).await;
                last_shell_cmd = Some(cmd);
            }

            InputAction::RepeatLast => {
                if let Some(ref cmd) = last_shell_cmd {
                    println!("{} {cmd}", "!".dimmed());
                    last_exit = shell::execute_shell(cmd, &cwd, &env_overrides, &env_removes).await;
                } else {
                    eprintln!("No previous shell command.");
                }
            }

            InputAction::Builtin(ref b) => {
                let msg = shell::handle_builtin(b, &mut cwd, &mut env_overrides, &mut env_removes);
                if !msg.is_empty() {
                    eprintln!("{msg}");
                    if matches!(b, Builtin::Cd(_)) {
                        last_exit = Some(1);
                    }
                } else {
                    last_exit = Some(0);
                    // Keep the tab-completion helper in sync when the directory changes
                    if matches!(b, Builtin::Cd(_)) {
                        *shell_cwd_shared.lock().unwrap_or_else(|e| e.into_inner()) = cwd.clone();
                    }
                }
            }

            InputAction::SlashLocal(ref text) => {
                if let Some(cmd) = parse_slash_command(text) {
                    match cmd.name.as_str() {
                        "quit" | "exit" => break,
                        "help" => print_help(),
                        "clear" => {
                            print!("\x1B[2J\x1B[1;1H");
                            std::io::stdout().flush().ok();
                            // UI-6: reset indicatif's cursor tracking — without this it
                            // thinks spinners are still at their old screen positions.
                            multi.clear().ok();
                        }
                        "think" => {
                            let enabled = !think_enabled.load(Ordering::Relaxed);
                            think_enabled.store(enabled, Ordering::Relaxed);
                            let status = if enabled { "on" } else { "off" };
                            println!(
                                "{}",
                                format!("  \u{1f9e0} Thinking display: {status}").dimmed()
                            );
                        }
                        "feedback" => {
                            let args = text
                                .trim_start_matches('/')
                                .trim_start_matches("feedback")
                                .trim();
                            if args.is_empty() {
                                println!(
                                    "{}",
                                    "Usage: /feedback good | /feedback bad [reason]".dimmed()
                                );
                            } else {
                                let (sentiment, reason) = if let Some(r) = args.strip_prefix("good")
                                {
                                    ("good", r.trim())
                                } else if let Some(r) = args.strip_prefix("bad") {
                                    ("bad", r.trim())
                                } else {
                                    ("", args)
                                };
                                let mut meta = HashMap::new();
                                meta.insert("type".to_string(), json!("feedback"));
                                meta.insert("sentiment".to_string(), json!(sentiment));
                                if !reason.is_empty() {
                                    meta.insert("reason".to_string(), json!(reason));
                                }
                                match client
                                    .send_message(&format!("/feedback {args}"), &sid, Some(meta))
                                    .await
                                {
                                    Ok(_) => println!(
                                        "{}",
                                        format!("  \u{2714} Feedback sent: {args}").dimmed()
                                    ),
                                    Err(e) => eprintln!("Failed to send feedback: {e}"),
                                }
                            }
                        }
                        "history" => {
                            if turn_history.is_empty() {
                                println!("{}", "  No conversation history yet.".dimmed());
                            } else {
                                for (i, (input, response)) in turn_history.iter().enumerate() {
                                    let user_preview: String = input
                                        .lines()
                                        .next()
                                        .unwrap_or("")
                                        .chars()
                                        .take(80)
                                        .collect();
                                    let agent_preview: String = response
                                        .lines()
                                        .next()
                                        .unwrap_or("")
                                        .chars()
                                        .take(80)
                                        .collect();
                                    println!(
                                        "{}",
                                        format!("  [{}] You: {user_preview}", i + 1).yellow()
                                    );
                                    println!(
                                        "{}",
                                        format!("       Agent: {agent_preview}").dimmed()
                                    );
                                }
                            }
                        }
                        "save" => {
                            let path = text
                                .trim_start_matches('/')
                                .trim_start_matches("save")
                                .trim();
                            if path.is_empty() {
                                println!("{}", "  Usage: /save <filename>".dimmed());
                            } else if last_response.is_empty() {
                                println!("{}", "  No response to save yet.".dimmed());
                            } else {
                                // Resolve relative to the shell's tracked CWD (not the process CWD)
                                let target = if std::path::Path::new(path).is_absolute() {
                                    PathBuf::from(path)
                                } else {
                                    cwd.join(path)
                                };
                                match std::fs::write(&target, &last_response) {
                                    Ok(_) => println!(
                                        "{}",
                                        format!("  \u{2714} Saved to {}", target.display())
                                            .dimmed()
                                    ),
                                    Err(e) => eprintln!("Failed to save: {e}"),
                                }
                            }
                        }
                        unknown => {
                            println!(
                                "Unknown command: /{unknown}. Type /help for available commands."
                            );
                        }
                    }
                }
            }

            InputAction::SlashServer(ref text) | InputAction::AgentMessage(ref text) => {
                // Reconnect with exponential backoff if the WebSocket is down
                if !ws_alive.load(Ordering::Relaxed) {
                    let mut delay = 1u64;
                    let mut reconnected = false;
                    for attempt in 1u32..=5 {
                        eprintln!(
                            "  {} Reconnecting in {delay}s (attempt {attempt}/5)...",
                            "\u{21bb}".yellow()
                        );
                        tokio::time::sleep(Duration::from_secs(delay)).await;
                        delay = (delay * 2).min(16);
                        match client.ws_connect(&sid).await {
                            Ok(new_ws) => {
                                let (new_write, new_read) = new_ws.split();
                                {
                                    let mut guard = ws_write.lock().await;
                                    *guard = new_write;
                                }
                                let _ = reconnect_ws_tx.send(new_read);
                                workspace_sent = false;
                                while done_rx.try_recv().is_ok() {}
                                println!("  {} Reconnected.", "\u{2713}".green());
                                reconnected = true;
                                break;
                            }
                            Err(e) => {
                                eprintln!("  {} Attempt {attempt} failed: {e}", "\u{2717}".red());
                            }
                        }
                    }
                    if !reconnected {
                        eprintln!("{}", "Failed to reconnect after 5 attempts.".red());
                        break;
                    }
                }

                // Strip backslash continuations, then expand @file attachments before sending
                let stripped = strip_line_continuations(text);
                let expanded = expand_file_attachments(&stripped);
                let user_input = expanded.clone();

                // Re-send workspace metadata after /reset (server clears its context)
                if expanded.starts_with("/reset") {
                    workspace_sent = false;
                }

                let metadata = if !workspace_sent {
                    workspace_sent = true;
                    local_workspace.as_ref().map(|ws| ws.to_metadata())
                } else {
                    None
                };

                let start = Instant::now();
                match client.send_message(&expanded, &sid, metadata).await {
                    Ok(_) => {
                        // Start waiting spinner
                        let pb = multi.add(ProgressBar::new_spinner());
                        pb.set_style(waiting_spinner_style.clone());
                        pb.set_message("Waiting for response...");
                        pb.enable_steady_tick(Duration::from_millis(80));
                        {
                            let mut guard =
                                waiting_spinner.lock().unwrap_or_else(|e| e.into_inner());
                            *guard = Some(pb);
                        }

                        // Drain stale signals, then wait for this turn's response.
                        // Ctrl-C cancels the local wait; disconnect_notify fires if WS drops.
                        while done_rx.try_recv().is_ok() {}
                        tokio::select! {
                            result = tokio::time::timeout(
                                Duration::from_secs(ws_response_timeout_secs),
                                done_rx.recv(),
                            ) => {
                                match result {
                                    Ok(None) => {
                                        // done_tx dropped — should not happen with reconnect loop
                                        eprintln!("{}", "Connection lost.".red());
                                        break;
                                    }
                                    Err(_) => {
                                        eprintln!(
                                            "{}",
                                            format!(
                                                "No response after {ws_response_timeout_secs}s. \
                                                 The server may be slow or the connection may be dead. \
                                                 Type /quit to exit."
                                            )
                                            .yellow()
                                        );
                                        // Don't break — let the user decide what to do next.
                                    }
                                    Ok(Some(())) => {
                                        let elapsed = start.elapsed().as_millis() as u64;
                                        let response = response_rx.try_recv().unwrap_or_default();
                                        last_response = response.clone();
                                        turn_history.push((user_input, response));

                                        // Show elapsed + usage if available
                                        let usage_part = if let Ok(mut guard) = last_usage.lock()
                                            && let Some((inp, out, reason, model)) = guard.take()
                                        {
                                            // Strip trailing date-like suffixes (e.g. "-20241022"),
                                            // keep up to 3 dash-segments (e.g. "claude-sonnet-4-6").
                                            let model_short = {
                                                let parts: Vec<&str> = model.split('-').collect();
                                                let non_date: Vec<&str> = parts
                                                    .iter()
                                                    .copied()
                                                    .filter(|s| {
                                                        !(s.len() == 8
                                                            && s.chars().all(|c| c.is_ascii_digit()))
                                                    })
                                                    .collect();
                                                let joined = non_date.join("-");
                                                if joined.len() > 24 {
                                                    format!("{}\u{2026}", &joined[..24])
                                                } else {
                                                    joined
                                                }
                                            };
                                            let reason_part = reason
                                                .map(|r| format!(" {}\u{26a1}", fmt_tokens(r)))
                                                .unwrap_or_default();
                                            format!(
                                                " \u{2502} {model_short} \u{2502} {}\u{2193} {}\u{2191}{reason_part}",
                                                fmt_tokens(inp),
                                                fmt_tokens(out),
                                            )
                                        } else {
                                            String::new()
                                        };
                                        println!(
                                            "{}",
                                            format!(
                                                "  \u{23f1} {}{}",
                                                format_duration(elapsed),
                                                usage_part
                                            )
                                            .dimmed()
                                        );
                                    }
                                }
                            }
                            _ = disconnect_notify.notified() => {
                                // WS dropped while we were waiting for a response
                                {
                                    let mut guard = waiting_spinner
                                        .lock()
                                        .unwrap_or_else(|e| e.into_inner());
                                    if let Some(pb) = guard.take() {
                                        pb.finish_and_clear();
                                    }
                                }
                                println!("{}", "\nConnection lost. Reconnect will be attempted on next input.".yellow());
                                // Don't break — ws_alive is now false; reconnect triggers on next send
                            }
                            _ = tokio::signal::ctrl_c() => {
                                // Cancel in-flight wait — clear spinner and return to prompt
                                {
                                    let mut guard = waiting_spinner
                                        .lock()
                                        .unwrap_or_else(|e| e.into_inner());
                                    if let Some(pb) = guard.take() {
                                        pb.finish_and_clear();
                                    }
                                }
                                println!("{}", "\nCancelled.".yellow());
                                // Don't break — return to prompt
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("{} {e}", "Send failed:".red());
                    }
                }
            }
        }
    }

    drop(prompt_tx); // signal the editor thread to exit and save history
    // Send a clean WS Close frame before aborting the tasks
    {
        let mut guard = ws_write.lock().await;
        let _ = guard.send(Message::Close(None)).await;
    }
    ws_task.abort();
    ping_task.abort();
    println!("\n{}", "Goodbye!".cyan());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_duration_sub_second() {
        assert_eq!(format_duration(0), "0ms");
        assert_eq!(format_duration(142), "142ms");
        assert_eq!(format_duration(999), "999ms");
    }

    #[test]
    fn format_duration_seconds() {
        assert_eq!(format_duration(1000), "1.0s");
        assert_eq!(format_duration(1200), "1.2s");
        assert_eq!(format_duration(10000), "10.0s");
    }

    #[test]
    fn truncate_sid_short_unchanged() {
        assert_eq!(truncate_sid("short"), "short");
        assert_eq!(truncate_sid("exactly14chars"), "exactly14chars");
    }

    #[test]
    fn truncate_sid_long_is_truncated() {
        let sid = "0190abcd-ef01-7abc-8def-123456789012"; // 36-char UUID
        let result = truncate_sid(sid);
        assert_eq!(result, "0190abcd\u{2026}89012");
    }

    #[test]
    fn fmt_tokens_below_1000() {
        assert_eq!(fmt_tokens(0), "0");
        assert_eq!(fmt_tokens(999), "999");
    }

    #[test]
    fn fmt_tokens_1000_and_above() {
        assert_eq!(fmt_tokens(1000), "1.0k");
        assert_eq!(fmt_tokens(1500), "1.5k");
        assert_eq!(fmt_tokens(10000), "10.0k");
    }

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

    #[test]
    fn strip_line_continuations_removes_backslash_newline() {
        assert_eq!(strip_line_continuations("line1\\\nline2"), "line1line2");
        assert_eq!(strip_line_continuations("a\\\nb\\\nc"), "abc");
    }

    #[test]
    fn strip_line_continuations_leaves_normal_newlines() {
        // A plain newline (no preceding backslash) is kept as-is
        assert_eq!(strip_line_continuations("a\nb"), "a\nb");
    }

    #[test]
    fn strip_line_continuations_noop_on_no_continuations() {
        let s = "hello world";
        assert_eq!(strip_line_continuations(s), s);
    }

    #[test]
    fn expand_file_attachments_no_at_sign_unchanged() {
        let text = "hello world";
        assert_eq!(expand_file_attachments(text), text);
    }

    #[test]
    fn expand_file_attachments_email_not_expanded() {
        let text = "send to user@example.com please";
        assert_eq!(expand_file_attachments(text), text);
    }
}
