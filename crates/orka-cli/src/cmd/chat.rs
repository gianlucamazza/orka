use std::collections::HashMap;
use std::io::Write as _;
use std::path::PathBuf;
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

fn print_help() {
    println!("{}", "=== Orka Shell ===".bold().cyan());
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
    println!("  {}          Show this help", "/help".yellow());
    println!("  {}          Clear screen", "/clear".yellow());
    println!("  {}          Exit", "/quit".yellow());
    println!();
}

/// Resolve the history file path, creating parent dirs as needed.
fn history_path() -> PathBuf {
    let dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("orka");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("history")
}

pub async fn run(
    client: &OrkaClient,
    session_id: Option<&str>,
    local_workspace: Option<crate::workspace::LocalWorkspace>,
) -> Result<()> {
    let sid = OrkaClient::resolve_session_id(session_id);

    // Wait for server to be ready before attempting WebSocket connection
    client.wait_for_ready(300, Duration::from_secs(1)).await?;

    println!("{}", "=== Orka Shell ===".bold().cyan());
    println!("Session: {}", sid.dimmed());
    if let Some(ref ws) = local_workspace {
        println!("Workspace: {}", ws.root.display().to_string().dimmed());
    }
    println!(
        "Type messages for AI, {} for shell, {} for commands, {} to exit.\n",
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

    let ws_task = tokio::spawn(async move {
        let multi = multi_clone;
        let spinner_style = ProgressStyle::with_template("{spinner:.cyan} {msg}")
            .unwrap()
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏");

        let mut renderer = crate::markdown::MarkdownRenderer::new();
        let mut streaming = false;
        let mut streamed_this_turn = false;
        let mut active_tools: HashMap<String, (String, Option<String>, Instant, ProgressBar)> =
            HashMap::new();

        while let Some(msg) = ws_read.next().await {
            match msg {
                Ok(msg) if msg.is_text() => {
                    let text = match msg.into_text() {
                        Ok(t) => t,
                        Err(_) => continue,
                    };

                    match classify_ws_message(&text) {
                        WsMessage::Stream(StreamChunkKind::Delta(data)) => {
                            if !streaming {
                                println!("\n{}", "Agent:".green().bold());
                                std::io::stdout().flush().ok();
                                streaming = true;
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
                                        .map(|s| format!(" — {s}"))
                                        .unwrap_or_default();
                                    pb.finish_with_message(format!(
                                        "{} {icon} {name} done ({dur}){suffix}",
                                        "✓".green()
                                    ));
                                } else {
                                    let suffix = error
                                        .or(result_summary)
                                        .map(|s| format!(" — {s}"))
                                        .unwrap_or_default();
                                    pb.finish_with_message(format!(
                                        "{} {icon} {name} failed ({dur}){suffix}",
                                        "✗".red()
                                    ));
                                }
                            } else {
                                let label = id;
                                if success {
                                    multi
                                        .println(format!("  {} {label} done ({dur})", "✓".green()))
                                        .ok();
                                } else {
                                    multi
                                        .println(format!("  {} {label} failed ({dur})", "✗".red()))
                                        .ok();
                                }
                            }
                        }
                        WsMessage::Stream(StreamChunkKind::ToolStart { .. })
                        | WsMessage::Stream(StreamChunkKind::ToolEnd { .. }) => {
                            // Internal LLM events — silent
                        }
                        WsMessage::Stream(StreamChunkKind::Done) => {
                            for (_, (name, _cat, _, pb)) in active_tools.drain() {
                                pb.finish_and_clear();
                                print!("[tool: {name}] ");
                            }
                            if streaming && !renderer.flush() {
                                println!();
                            }
                            renderer.reset();
                            streaming = false;
                            let _ = done_tx.send(());
                        }
                        WsMessage::Final(content) => {
                            if streamed_this_turn {
                                streamed_this_turn = false;
                                continue;
                            }
                            println!("\n{}", "Agent:".green().bold());
                            std::io::stdout().flush().ok();
                            renderer.render_full(&content);
                            println!();
                            let _ = done_tx.send(());
                        }
                        WsMessage::Stream(_) => {
                            // Unknown stream chunk kind — ignore
                        }
                        WsMessage::Unknown(raw) => {
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
                        unknown => {
                            println!("Unknown command: /{unknown}. Type /help for available commands.");
                        }
                    }
                }
            }

            InputAction::SlashServer(ref text) | InputAction::AgentMessage(ref text) => {
                let metadata = if !workspace_sent {
                    workspace_sent = true;
                    local_workspace.as_ref().map(|ws| ws.to_metadata())
                } else {
                    None
                };

                match client.send_message(text, &sid, metadata).await {
                    Ok(_) => {
                        // Drain stale signals, then wait for this turn's response
                        while done_rx.try_recv().is_ok() {}
                        match done_rx.recv().await {
                            Some(()) => {}
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
