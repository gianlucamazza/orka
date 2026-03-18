use std::collections::HashMap;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use colored::Colorize;
use futures_util::StreamExt;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use orka_core::parse_slash_command;
use orka_core::stream::StreamChunkKind;
use rustyline::error::ReadlineError;
use tokio_tungstenite::connect_async;

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
    let width = inner.chars().count();
    let bar = "\u{2500}".repeat(width);
    println!("{}", format!("\u{250c}{bar}\u{2510}").cyan().bold());
    println!("{}", format!("\u{2502}{inner}\u{2502}").cyan().bold());
    println!("{}", format!("\u{2514}{bar}\u{2518}").cyan().bold());
}

fn print_help() {
    print_banner();
    println!();
    println!("{}", "Shell execution:".bold());
    println!(
        "  {}       Execute shell command locally",
        "!<command>".yellow()
    );
    println!("  {}              Repeat last shell command", "!!".yellow());
    println!("  {}        Change directory", "!cd <path>".yellow());
    println!("  {}     Set environment variable", "!export K=V".yellow());
    println!(
        "  {}      Unset environment variable",
        "!unset <K>".yellow()
    );
    println!();
    println!("{}", "AI agent:".bold());
    println!(
        "  {}           Send to AI agent (default)",
        "<text>".yellow()
    );
    println!();
    println!("{}", "Commands:".bold());
    println!(
        "  {}   Invoke a skill directly",
        "/skill <name> [k=v ...]".yellow()
    );
    println!("  {}        List available skills", "/skills".yellow());
    println!("  {}         Clear conversation history", "/reset".yellow());
    println!("  {}        Show session info", "/status".yellow());
    println!(
        "  {}         Toggle extended thinking display",
        "/think".yellow()
    );
    println!(
        "  {}  Send feedback on last response",
        "/feedback [good|bad] [reason]".yellow()
    );
    println!("  {}          Show this help", "/help".yellow());
    println!("  {}          Clear screen", "/clear".yellow());
    println!("  {}          Exit", "/quit".yellow());
    println!();
    println!("{}", "File attachment:".bold());
    println!("  {}   Inline a file as a code block", "@<path>".yellow());
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
fn print_agent_header(name: &str) {
    let label = if name.is_empty() { "Agent" } else { name };
    let bar_len = 40usize.saturating_sub(label.len() + 2);
    let bar = "\u{2500}".repeat(bar_len);
    println!(
        "\n{}",
        format!("\u{2500}\u{2500}\u{2500}\u{2500} {label} {bar}").dimmed()
    );
    std::io::stdout().flush().ok();
}

/// Format token counts compactly: `1.2k` for ≥1000, else the raw number.
fn fmt_tokens(n: u32) -> String {
    if n >= 1000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
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
                // After the code block, treat position as start-of-whitespace for next token
                prev_was_whitespace_or_start = true;
            }
            Err(_) => {
                // Unreadable — leave the token as-is, re-emit the space delimiter
                result.push('@');
                result.push_str(&path_str);
                result.push(' ');
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

    // Connect WebSocket
    let ws_url = client.ws_url(&sid);
    let (ws, _) = connect_async(&ws_url)
        .await
        .map_err(|e| format!("Failed to connect WebSocket to {ws_url}: {e}"))?;
    let (_write, mut ws_read) = ws.split();

    // WS reader task: render streaming output with spinners
    let multi = MultiProgress::with_draw_target(indicatif::ProgressDrawTarget::stderr_with_hz(10));
    let multi_clone = multi.clone();

    // Channel to notify the REPL that a response is complete
    let (done_tx, mut done_rx) = tokio::sync::mpsc::unbounded_channel::<()>();

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

    let ws_task = tokio::spawn(async move {
        let multi = multi_clone;
        let spinner_style = ProgressStyle::with_template("{spinner:.cyan} {msg}")
            .unwrap()
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏");

        let mut renderer = crate::markdown::MarkdownRenderer::new();
        let mut streaming = false;
        let mut streamed_this_turn = false;
        let mut thinking_shown = false;
        let mut active_tools: HashMap<String, (String, Option<String>, Instant, ProgressBar)> =
            HashMap::new();
        // Current agent name (updated on AgentSwitch)
        let mut current_agent = String::new();

        /// Clear the waiting spinner if it's still active.
        fn clear_waiting(ws: &Arc<Mutex<Option<ProgressBar>>>) {
            if let Ok(mut guard) = ws.lock()
                && let Some(pb) = guard.take()
            {
                pb.finish_and_clear();
            }
        }

        while let Some(msg) = ws_read.next().await {
            match msg {
                Ok(msg) if msg.is_text() => {
                    let text = match msg.into_text() {
                        Ok(t) => t,
                        Err(_) => continue,
                    };

                    match classify_ws_message(&text) {
                        WsMessage::Stream(StreamChunkKind::AgentSwitch {
                            display_name, ..
                        }) => {
                            clear_waiting(&waiting_spinner_ws);
                            current_agent = display_name;
                        }
                        WsMessage::Stream(StreamChunkKind::PrinciplesUsed { count }) => {
                            clear_waiting(&waiting_spinner_ws);
                            println!(
                                "  {}",
                                format!(
                                    "\u{1f9e0} {count} principle{} applied",
                                    if count == 1 { "" } else { "s" }
                                )
                                .dimmed()
                            );
                            std::io::stdout().flush().ok();
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
                            if pct >= 85 {
                                println!("  {}", msg.red());
                                println!(
                                    "  {}",
                                    "Tip: use /reset to start a fresh context.".red().dimmed()
                                );
                            } else {
                                println!("  {}", msg.yellow());
                            }
                            std::io::stdout().flush().ok();
                        }
                        WsMessage::Stream(StreamChunkKind::Usage {
                            input_tokens,
                            output_tokens,
                            reasoning_tokens,
                            model,
                            ..
                        }) => {
                            if let Ok(mut guard) = last_usage_ws.lock() {
                                *guard =
                                    Some((input_tokens, output_tokens, reasoning_tokens, model));
                            }
                        }
                        WsMessage::Stream(StreamChunkKind::ThinkingDelta(delta)) => {
                            clear_waiting(&waiting_spinner_ws);
                            if think_enabled_ws.load(Ordering::Relaxed) {
                                if !thinking_shown {
                                    println!("  {}", "\u{25cc} thinking".dimmed());
                                    thinking_shown = true;
                                }
                                // Print thinking content indented and dimmed
                                for line in delta.lines() {
                                    println!("  {} {}", "\u{2502}".dimmed(), line.dimmed());
                                }
                            } else if !thinking_shown {
                                println!("  {}", "\u{25cc} thinking...".dimmed());
                                std::io::stdout().flush().ok();
                                thinking_shown = true;
                            }
                        }
                        WsMessage::Stream(StreamChunkKind::Delta(data)) => {
                            clear_waiting(&waiting_spinner_ws);
                            if !streaming {
                                print_agent_header(&current_agent);
                                streaming = true;
                                thinking_shown = false;
                            }
                            streamed_this_turn = true;
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
                            for (_, (name, _cat, _, pb)) in active_tools.drain() {
                                pb.finish_and_clear();
                                print!("[tool: {name}] ");
                            }
                            if streaming && !renderer.flush() {
                                println!();
                            }
                            renderer.reset();
                            streaming = false;
                            thinking_shown = false;
                            let _ = done_tx.send(());
                        }
                        WsMessage::Final(content) => {
                            clear_waiting(&waiting_spinner_ws);
                            if streamed_this_turn {
                                streamed_this_turn = false;
                                continue;
                            }
                            print_agent_header(&current_agent);
                            renderer.render_full(&content);
                            println!();
                            thinking_shown = false;
                            let _ = done_tx.send(());
                        }
                        WsMessage::Stream(_) => {
                            // Unknown stream chunk kind — ignore
                        }
                        WsMessage::Unknown(raw) => {
                            clear_waiting(&waiting_spinner_ws);
                            println!("\n{raw}");
                            let _ = done_tx.send(());
                        }
                    }
                }
                Ok(msg) if msg.is_close() => break,
                Err(e) => {
                    tracing::debug!("WS read error: {e}");
                    break;
                }
                _ => {}
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

    std::thread::spawn(move || {
        let config = rustyline::Config::builder()
            .auto_add_history(true)
            .max_history_size(10_000)
            .expect("valid history size")
            .build();
        let helper = OrkaHelper::new();
        let mut editor =
            match rustyline::Editor::<OrkaHelper, rustyline::history::DefaultHistory>::with_config(
                config,
            ) {
                Ok(mut ed) => {
                    ed.set_helper(Some(helper));
                    let _ = ed.load_history(&hist_path);
                    ed
                }
                Err(_) => return,
            };

        while let Ok(prompt) = prompt_rx.recv() {
            let result = editor.readline(&prompt);
            if line_tx.send(result).is_err() {
                break;
            }
        }
        let _ = editor.save_history(&hist_path_save);
    });

    // Shell state
    let mut cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    let mut env_overrides: HashMap<String, String> = HashMap::new();
    let mut last_exit: Option<i32> = None;
    let mut last_shell_cmd: Option<String> = None;
    let mut workspace_sent = false;

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
            Ok(line) => line,
            Err(ReadlineError::Interrupted | ReadlineError::Eof) => break,
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
                last_exit = shell::execute_shell(&cmd, &cwd, &env_overrides).await;
                last_shell_cmd = Some(cmd);
            }

            InputAction::RepeatLast => {
                if let Some(ref cmd) = last_shell_cmd {
                    println!("{} {cmd}", "!".dimmed());
                    last_exit = shell::execute_shell(cmd, &cwd, &env_overrides).await;
                } else {
                    eprintln!("No previous shell command.");
                }
            }

            InputAction::Builtin(ref b) => {
                let msg = shell::handle_builtin(b, &mut cwd, &mut env_overrides);
                if !msg.is_empty() {
                    eprintln!("{msg}");
                    if matches!(b, Builtin::Cd(_)) {
                        last_exit = Some(1);
                    }
                } else {
                    last_exit = Some(0);
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
                            // `/feedback` without server: just acknowledge locally
                            // The text after "feedback" is the reason
                            let reason = text
                                .trim_start_matches('/')
                                .trim_start_matches("feedback")
                                .trim();
                            if reason.is_empty() {
                                println!(
                                    "{}",
                                    "Usage: /feedback good | /feedback bad [reason]".dimmed()
                                );
                            } else {
                                println!(
                                    "{}",
                                    format!("  \u{2714} Feedback noted: {reason}").dimmed()
                                );
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
                // Expand @file attachments before sending
                let expanded = expand_file_attachments(text);

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
                        if let Ok(mut guard) = waiting_spinner.lock() {
                            *guard = Some(pb);
                        }

                        // Drain stale signals, then wait for this turn's response
                        while done_rx.try_recv().is_ok() {}
                        match done_rx.recv().await {
                            Some(()) => {
                                let elapsed = start.elapsed().as_millis() as u64;
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
                            None => {
                                eprintln!("{}", "Connection lost.".red());
                                break;
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
    ws_task.abort();
    println!("\n{}", "Goodbye!".cyan());

    Ok(())
}
