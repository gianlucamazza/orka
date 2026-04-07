use std::{
    collections::HashMap,
    fmt::Write as _,
    io::Write as _,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU16, Ordering},
    },
    time::{Duration, Instant},
};

use colored::Colorize;
use futures_util::{SinkExt, StreamExt, stream::SplitStream};
use indicatif::MultiProgress;
use orka_contracts::RealtimeEvent;
use orka_core::parse_slash_command;
use reedline::{FileBackedHistory, Reedline, Signal};
use serde_json::json;
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream,
    tungstenite::{Bytes, Message},
};
use unicode_width::UnicodeWidthStr;

use crate::{
    chat_renderer::ChatRenderer,
    client::{OrkaClient, Result},
    completion::{OrkaCompleter, OrkaHighlighter, OrkaHinter, OrkaPrompt, OrkaValidator},
    prompt::build_prompt,
    protocol::{WsMessage, classify_ws_message},
    shell::{self, Builtin, InputAction},
};

/// Accumulated token usage and cost across all LLM calls in a single turn.
#[derive(Default)]
struct TurnUsage {
    total_input_tokens: u32,
    total_output_tokens: u32,
    total_reasoning_tokens: u32,
    total_cache_read_tokens: u32,
    total_cache_creation_tokens: u32,
    total_cost_usd: f64,
    call_count: u32,
    last_model: String,
}

impl TurnUsage {
    #[allow(clippy::too_many_arguments)]
    fn accumulate(
        &mut self,
        input_tokens: u32,
        output_tokens: u32,
        reasoning_tokens: Option<u32>,
        cache_read_tokens: Option<u32>,
        cache_creation_tokens: Option<u32>,
        cost_usd: Option<f64>,
        model: String,
    ) {
        self.total_input_tokens += input_tokens;
        self.total_output_tokens += output_tokens;
        self.total_reasoning_tokens += reasoning_tokens.unwrap_or(0);
        self.total_cache_read_tokens += cache_read_tokens.unwrap_or(0);
        self.total_cache_creation_tokens += cache_creation_tokens.unwrap_or(0);
        self.total_cost_usd += cost_usd.unwrap_or(0.0);
        self.call_count += 1;
        self.last_model = model;
    }

    /// Take the accumulated usage if any calls were recorded, resetting to
    /// default.
    fn take(&mut self) -> Option<TurnUsage> {
        if self.call_count == 0 {
            return None;
        }
        Some(std::mem::take(self))
    }
}

/// Server info returned by `GET /api/v1/info`.
#[derive(serde::Deserialize)]
struct ServerInfo {
    version: String,
    git_sha: String,
    agent_name: String,
    agent_model: String,
    skill_count: usize,
    mcp_server_count: usize,
    features: ServerFeatures,
    thinking: Option<String>,
    agent_count: usize,
    auth_enabled: bool,
    #[serde(default)]
    adapters: Vec<String>,
    coding_backend: Option<String>,
    web_search: Option<String>,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(serde::Deserialize)]
struct ServerFeatures {
    knowledge: bool,
    scheduler: bool,
    experience: bool,
    guardrails: bool,
    a2a: bool,
    observe: bool,
}

fn adapter_icon(name: &str) -> &'static str {
    match name {
        "telegram" => "\u{1f4ac}", // 💬
        "discord" => "\u{1f3ae}",  // 🎮
        "slack" => "\u{1f4bc}",    // 💼
        "whatsapp" => "\u{1f4f1}", // 📱
        _ => "\u{1f517}",          // 🔗
    }
}

/// Normalize raw technical identifiers to user-friendly display names.
fn display_name(raw: &str) -> &str {
    match raw {
        "codex" => "Codex",
        "claude-code" | "claude_code" => "Claude Code",
        "tavily" => "Tavily",
        "brave" => "Brave",
        "searxng" => "SearXNG",
        "telegram" => "Telegram",
        "discord" => "Discord",
        "slack" => "Slack",
        "whatsapp" => "WhatsApp",
        other => other,
    }
}

/// Print the box-drawn welcome banner with optional server info.
#[allow(clippy::too_many_lines, clippy::items_after_statements)]
fn print_welcome(
    info: Option<&ServerInfo>,
    base_url: &str,
    session_id: &str,
    workspace: Option<&str>,
) {
    // Box header (always shown, uses CLI version)
    let version = concat!(env!("CARGO_PKG_VERSION"), " (", env!("ORKA_GIT_SHA"), ")");
    let inner = format!("  \u{25c8}  Orka Shell  v{version}  ");
    // UI-11: use display width (handles wide/ambiguous Unicode) rather than char
    // count
    let width = inner.as_str().width();
    let bar = "\u{2500}".repeat(width);
    println!("{}", format!("\u{250c}{bar}\u{2510}").cyan().bold());
    println!("{}", format!("\u{2502}{inner}\u{2502}").cyan().bold());
    println!("{}", format!("\u{2514}{bar}\u{2518}").cyan().bold());

    // Label column width for alignment
    const LBL: usize = 10;

    if let Some(info) = info {
        let short_sha = if info.git_sha.len() >= 7 {
            &info.git_sha[..7]
        } else {
            &info.git_sha
        };

        // Agent line: name · model [· N agents] [· 💡 thinking: level]
        let mut agent_parts = vec![format!("{} \u{00b7} {}", info.agent_name, info.agent_model)];
        if info.agent_count > 1 {
            agent_parts.push(format!("{} agents", info.agent_count));
        }
        if let Some(ref t) = info.thinking {
            agent_parts.push(format!("\u{1f4a1} thinking: {t}")); // 💡
        }
        println!(
            " {}{} {}",
            "Agent".dimmed(),
            " ".repeat(LBL.saturating_sub("Agent".len())),
            agent_parts.join(" \u{00b7} ")
        );

        // Server line: url  vX.Y.Z+sha [🔒]
        let display_url = base_url
            .trim_start_matches("https://")
            .trim_start_matches("http://");
        let auth_icon = if info.auth_enabled { "  \u{1f512}" } else { "" }; // 🔒
        println!(
            " {}{} {}  {}{}",
            "Server".dimmed(),
            " ".repeat(LBL.saturating_sub("Server".len())),
            display_url.dimmed(),
            format!("v{}+{short_sha}", info.version).dimmed(),
            auth_icon.dimmed()
        );
    }

    println!(
        " {}{} {}",
        "Session".dimmed(),
        " ".repeat(LBL.saturating_sub("Session".len())),
        truncate_sid(session_id).dimmed()
    );
    if let Some(ws) = workspace {
        println!(
            " {}{} {}",
            "Workspace".dimmed(),
            " ".repeat(LBL.saturating_sub("Workspace".len())),
            ws.dimmed()
        );
    }

    if let Some(info) = info {
        // Feature icons row (only when at least one feature is active)
        let mut feature_parts: Vec<&str> = Vec::new();
        if info.features.knowledge {
            feature_parts.push("\u{1f9e0} knowledge");
        } // 🧠
        if info.features.scheduler {
            feature_parts.push("\u{1f4c5} scheduler");
        } // 📅
        if info.features.experience {
            feature_parts.push("\u{1f52c} experience");
        } // 🔬
        if info.features.guardrails {
            feature_parts.push("\u{1f6e1} guardrails");
        } // 🛡
        if info.features.a2a {
            feature_parts.push("\u{1f91d} a2a");
        } // 🤝
        if info.features.observe {
            feature_parts.push("\u{1f4ca} observe");
        } // 📊
        if !feature_parts.is_empty() {
            println!(" {}", feature_parts.join("  ").dimmed());
        }

        // Skills / MCP row
        let mut parts: Vec<String> = Vec::new();
        if info.skill_count > 0 {
            parts.push(format!("\u{1f527} {} skills", info.skill_count)); // 🔧
        }
        if info.mcp_server_count > 0 {
            parts.push(format!("\u{1f50c} {} MCP servers", info.mcp_server_count)); // 🔌
        }
        if !parts.is_empty() {
            println!(" {}", parts.join(" \u{00b7} ").dimmed()); // ·
        }

        // Adapters row
        if !info.adapters.is_empty() {
            let adapter_tags: Vec<String> = info
                .adapters
                .iter()
                .map(|a| format!("{} {}", adapter_icon(a), display_name(a)))
                .collect();
            println!(" {}", adapter_tags.join("  ").dimmed());
        }

        // Coding + Web row
        let mut tool_parts: Vec<String> = Vec::new();
        if let Some(ref backend) = info.coding_backend {
            tool_parts.push(format!("\u{1f5a5} coding: {}", display_name(backend))); // 🖥
        }
        if let Some(ref provider) = info.web_search {
            tool_parts.push(format!("\u{1f50d} web: {}", display_name(provider))); // 🔍
        }
        if !tool_parts.is_empty() {
            println!(" {}", tool_parts.join(" \u{00b7} ").dimmed()); // ·
        }
    }
}

fn print_help() {
    // UI-20: use calculated column padding so descriptions align regardless of
    // command length.  COL is the visual width of the command column.
    const COL: usize = 32;
    let row = |cmd: &str, desc: &str| {
        let pad = " ".repeat(COL.saturating_sub(cmd.len()));
        println!("  {}{pad}{desc}", cmd.yellow());
    };

    print_welcome(None, "", "", None);
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
    row("/memory status", "Show memory layers");
    row("/memory clear", "Clear current session memory");
    row("/status", "Show session info");
    row("/think", "Toggle extended thinking display");
    row(
        "/feedback [good|bad] [reason]",
        "Send feedback on last response",
    );
    row("/history", "Show conversation history");
    row("/save <file>", "Save last response to file");
    row("/copy", "Copy last response to clipboard");
    row("/open <url>", "Open a URL in the browser");
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

/// Format token counts compactly: `1.2k` for ≥1000, else the raw number.
fn fmt_tokens(n: u32) -> String {
    if n >= 1000 {
        format!("{:.1}k", f64::from(n) / 1000.0)
    } else {
        n.to_string()
    }
}

/// Strip backslash line-continuation sequences (`\` + newline) from multi-line
/// input.
///
/// When rustyline's Validator returns `Incomplete` for trailing `\`, it
/// concatenates continuation lines with `\n`, leaving `\\\n` sequences in the
/// string. Removing them joins the lines as intended before the text is sent to
/// the agent.
fn strip_line_continuations(s: &str) -> String {
    s.replace("\\\n", "")
}

fn should_attach_workspace_cwd(client: &OrkaClient, include_workspace_cwd: bool) -> bool {
    include_workspace_cwd && client.targets_localhost()
}

/// Expand `@path` tokens in a message to inline file content as code fences.
///
/// `@path/to/file.rs` is replaced with a fenced code block containing the
/// file's contents.  Unknown or unreadable paths are left as-is.
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

        // Collect the path token (non-whitespace characters after `@`).
        // Manually peek so we can capture and restore the exact delimiter character
        // (space, newline, tab, …) instead of always substituting a space.
        let mut path_str = String::new();
        let mut delimiter: Option<char> = None;
        while let Some(&c) = chars.peek() {
            if c.is_whitespace() {
                delimiter = chars.next(); // consume and save the delimiter
                break;
            }
            if let Some(c) = chars.next() {
                path_str.push(c);
            }
        }

        if path_str.is_empty() {
            result.push('@');
            if let Some(d) = delimiter {
                result.push(d);
            }
            prev_was_whitespace_or_start = false;
            continue;
        }

        let path = std::path::Path::new(&path_str);
        if let Ok(content) = std::fs::read_to_string(path) {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            let _ = write!(result, "\n```{ext}\n{content}\n```\n");
            // Restore the original delimiter character (preserves newlines, tabs, etc.)
            if let Some(d) = delimiter {
                result.push(d);
            }
            prev_was_whitespace_or_start = true;
        } else {
            // Unreadable — leave the token as-is
            result.push('@');
            result.push_str(&path_str);
            if let Some(d) = delimiter {
                result.push(d);
            }
            prev_was_whitespace_or_start = true;
        }
    }
    result
}

#[allow(clippy::too_many_lines, clippy::items_after_statements)]
pub async fn run(
    client: &OrkaClient,
    server_client: &OrkaClient,
    session_id: Option<&str>,
    local_workspace: Option<crate::ws_discovery::LocalWorkspace>,
    include_workspace_cwd: bool,
) -> Result<()> {
    let sid = OrkaClient::resolve_session_id(session_id);

    // Wait for server to be ready before attempting WebSocket connection (~30s
    // total)
    client.wait_for_ready(30, Duration::from_secs(1)).await?;

    // Fetch server info from the management server (best-effort, degrades
    // gracefully)
    let server_info: Option<ServerInfo> = server_client
        .get_json("/api/v1/info")
        .await
        .ok()
        .and_then(|v| serde_json::from_value(v).ok());

    // Welcome banner
    let workspace_str = local_workspace
        .as_ref()
        .map(|ws| ws.root.display().to_string());
    print_welcome(
        server_info.as_ref(),
        server_client.base_url(),
        &sid,
        workspace_str.as_deref(),
    );
    // Show update notice from cache (no network I/O on the hot path)
    if let Some(info) = super::update::check_from_cache() {
        super::update::print_update_notice(&info);
    }
    // Refresh the update cache in the background
    tokio::spawn(async { super::update::check().await });
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

    // Shared terminal width — updated on SIGWINCH to fix resize glitches in
    // streaming markdown output (the renderer reads this instead of calling
    // crossterm::terminal::size() point-in-time).
    let term_width = Arc::new(AtomicU16::new(
        crossterm::terminal::size().map_or(80, |(w, _)| w),
    ));
    let term_width_ws = term_width.clone();

    // Detect terminal capabilities once at startup. Sets colored::control override
    // and is threaded into the renderer.
    let term_caps = crate::term_caps::TermCaps::detect();
    let color_ws = term_caps.color;

    // SIGWINCH handler: update the shared width atom whenever the terminal is
    // resized. This runs only when reedline is NOT reading (output phase), so
    // there is no conflict with reedline's own resize handling during input.
    #[cfg(unix)]
    {
        let tw = term_width.clone();
        tokio::spawn(async move {
            use tokio::signal::unix::{SignalKind, signal};
            if let Ok(mut sig) = signal(SignalKind::window_change()) {
                while sig.recv().await.is_some() {
                    if let Ok((w, _)) = crossterm::terminal::size() {
                        tw.store(w, Ordering::Relaxed);
                    }
                }
            }
        });
    }

    // Flag: set to false by the WS task when the connection drops
    let ws_alive = Arc::new(AtomicBool::new(true));
    let ws_alive_task = ws_alive.clone();

    // Flag: set to true when the user initiates graceful shutdown (e.g. /exit,
    // /quit) Use SeqCst ordering to ensure visibility across threads during
    // shutdown
    let graceful_shutdown = Arc::new(AtomicBool::new(false));
    let graceful_shutdown_task = graceful_shutdown.clone();

    // Timeout flag for the WebSocket task — set to true on timeout to suppress
    // stale message output and prevent terminal corruption. Reset on reconnect.
    let ws_timeout_flag: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
    let ws_timeout_task = ws_timeout_flag.clone();

    // Channel to notify the REPL that a response is complete
    let (done_tx, mut done_rx) = tokio::sync::mpsc::unbounded_channel::<()>();

    // Channel to pass the completed response text back to the REPL (for /save,
    // /history)
    let (response_tx, mut response_rx) = tokio::sync::mpsc::unbounded_channel::<String>();

    // Activity signals — sent on every WS message to reset the idle timeout
    let (activity_tx, mut activity_rx) = tokio::sync::mpsc::unbounded_channel::<()>();

    // Accumulated token usage for the current turn (reset after each Done).
    let last_usage: Arc<Mutex<TurnUsage>> = Arc::new(Mutex::new(TurnUsage::default()));
    let last_usage_ws = last_usage.clone();

    // Think toggle: when true, ThinkingDelta content is shown
    let think_enabled = Arc::new(AtomicBool::new(false));
    let think_enabled_ws = think_enabled.clone();

    let disconnect_notify_ws = disconnect_notify.clone();
    let mut ws_task = tokio::spawn(async move {
        let mut renderer = crate::chat_renderer::ChatRenderer::new(
            multi_clone,
            term_width_ws,
            color_ws,
            think_enabled_ws,
        );
        let mut current_ws_read = ws_read;

        'reconnect: loop {
            renderer.reset_turn();
            // Guard against duplicate Done signals in the same turn
            let mut turn_done_sent = false;
            // Track if any Delta was received (to skip on_final when streaming)
            let mut streamed_this_turn = false;

            loop {
                // Check for timeout flag before each message.
                // Use Acquire ordering to ensure visibility of the store in the main task.
                if ws_timeout_task.load(Ordering::Acquire) {
                    // Timeout occurred: clear spinners and exit inner loop.
                    // The task will wait on reconnect_ws_rx for a new connection.
                    renderer.drain_tools_on_timeout();
                    break;
                }

                let Some(msg) = current_ws_read.next().await else {
                    break; // Connection closed, wait for reconnect
                };
                activity_tx.send(()).ok();
                match msg {
                    Ok(msg) if msg.is_text() => {
                        let Ok(text) = msg.into_text() else {
                            continue;
                        };

                        match classify_ws_message(&text) {
                            WsMessage::Stream(RealtimeEvent::AgentSwitch {
                                display_name,
                                ..
                            }) => {
                                renderer.on_agent_switch(display_name);
                            }
                            WsMessage::Stream(RealtimeEvent::PrinciplesUsed { count }) => {
                                renderer.on_principles_used(count);
                            }
                            WsMessage::Stream(RealtimeEvent::ContextInfo {
                                history_tokens,
                                context_window,
                                messages_truncated,
                                summary_generated,
                            }) => {
                                renderer.on_context_info(
                                    history_tokens,
                                    context_window,
                                    messages_truncated,
                                    summary_generated,
                                );
                            }
                            WsMessage::Stream(RealtimeEvent::Usage {
                                input_tokens,
                                output_tokens,
                                reasoning_tokens,
                                cache_read_tokens,
                                cache_creation_tokens,
                                cost_usd,
                                model,
                            }) => {
                                if let Ok(mut guard) = last_usage_ws.lock() {
                                    guard.accumulate(
                                        input_tokens,
                                        output_tokens,
                                        reasoning_tokens,
                                        cache_read_tokens,
                                        cache_creation_tokens,
                                        cost_usd,
                                        model,
                                    );
                                }
                            }
                            WsMessage::Stream(RealtimeEvent::ThinkingDelta { delta }) => {
                                renderer.on_thinking_delta(&delta);
                            }
                            WsMessage::Stream(RealtimeEvent::MessageDelta { delta: data }) => {
                                streamed_this_turn = true;
                                turn_done_sent = false; // new turn is streaming
                                renderer.on_delta(&data);
                            }
                            WsMessage::Stream(RealtimeEvent::ToolExecStart {
                                name,
                                id,
                                input_summary,
                                category,
                            }) => {
                                renderer.on_tool_exec_start(name, id, input_summary, category);
                            }
                            WsMessage::Stream(RealtimeEvent::ToolExecEnd {
                                id,
                                success,
                                duration_ms,
                                error,
                                result_summary,
                            }) => {
                                renderer.on_tool_exec_end(
                                    &id,
                                    success,
                                    duration_ms,
                                    error,
                                    result_summary,
                                );
                            }
                            WsMessage::Stream(RealtimeEvent::StreamDone) => {
                                let response = renderer.on_done();
                                // CRITICAL: Check timeout flag before sending signals.
                                // The main task may have already drained the channels during
                                // timeout handling; sending now would create spurious signals
                                // that corrupt the next user interaction.
                                if !turn_done_sent && !ws_timeout_task.load(Ordering::Acquire) {
                                    // Send response before done so try_recv() in the REPL
                                    // always finds the response after receiving the done signal.
                                    let _ = response_tx.send(response);
                                    let _ = done_tx.send(());
                                    turn_done_sent = true;
                                }
                            }
                            WsMessage::Final(content, stop_reason) => {
                                // Show stop-reason warning before rendering content.
                                // Emitted even when content was already streamed via deltas.
                                if let Some(reason) = stop_reason {
                                    use orka_core::stream::AgentStopReason;
                                    let warning = match reason {
                                        AgentStopReason::MaxTurns => {
                                            "⚠ Response may be incomplete (agent reached maximum turns)"
                                        }
                                        AgentStopReason::MaxTokens => {
                                            "⚠ Response was truncated by the output token limit"
                                        }
                                        _ => "",
                                    };
                                    if !warning.is_empty() {
                                        renderer.warn(format!("  {}", warning.yellow()));
                                    }
                                }
                                if streamed_this_turn {
                                    streamed_this_turn = false;
                                    // Content already rendered via Delta
                                    // chunks; skip on_final.
                                } else {
                                    renderer.on_final(&content);
                                }
                                // CRITICAL: Check timeout flag before sending signals.
                                // Prevents race condition with main task's channel drain.
                                if !turn_done_sent && !ws_timeout_task.load(Ordering::Acquire) {
                                    // Send response before done (same ordering guarantee as Done
                                    // branch).
                                    let _ = response_tx.send(content.clone());
                                    let _ = done_tx.send(());
                                    turn_done_sent = true;
                                }
                            }
                            WsMessage::Media {
                                mime_type,
                                data_base64,
                                caption,
                            } => {
                                renderer.on_media(&mime_type, &data_base64, caption.as_deref());
                            }
                            WsMessage::Stream(
                                RealtimeEvent::ToolCallStart { .. }
                                | RealtimeEvent::ToolCallEnd { .. },
                            ) => {
                                // Internal LLM tool-block bookkeeping events —
                                // no UI action
                            }
                            WsMessage::Stream(unknown) => {
                                tracing::debug!(?unknown, "unhandled stream event");
                            }
                            WsMessage::Unknown(raw) => {
                                ChatRenderer::on_unknown(&raw);
                            }
                        }
                    }
                    Ok(msg) if msg.is_close() => break,
                    Err(e) => {
                        // Suppress error message during graceful shutdown - the server
                        // may close the connection before we complete the close handshake
                        if !graceful_shutdown_task.load(Ordering::SeqCst) {
                            eprintln!("{} {e}", "Connection error:".red());
                        }
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
                    // Use Release ordering to ensure visibility in the WS task
                    ws_alive_task.store(true, Ordering::Release);
                    // Reset the timeout flag for the new connection
                    ws_timeout_task.store(false, Ordering::Release);
                    renderer.reset_turn();
                    // per-connection state is reset at the top of 'reconnect
                }
                None => break 'reconnect,
            }
        } // end 'reconnect loop
    });

    // Keepalive: send a WebSocket Ping every 30 seconds to prevent idle
    // disconnects.
    let ws_write_ping = ws_write.clone();
    let ws_alive_ping = ws_alive.clone();
    let ping_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        interval.tick().await; // consume the immediate first tick
        loop {
            interval.tick().await;
            // Skip ping if connection is down (timeout or disconnect)
            // The task will naturally exit on next send failure
            if !ws_alive_ping.load(Ordering::Acquire) {
                continue;
            }
            let mut guard = ws_write_ping.lock().await;
            if guard.send(Message::Ping(Bytes::default())).await.is_err() {
                break;
            }
        }
    });

    // Migrate history format from rustyline (#V2 header) to reedline (plain lines).
    let hist_path = history_path();
    migrate_history_if_needed(&hist_path);

    // Set up reedline in a dedicated blocking thread, communicating via channels.
    // reedline::read_line() is blocking — same pattern as the old rustyline thread.
    // Channel sends (plain_prompt, colored_prompt) tuples; reedline uses the
    // colored version directly since it handles ANSI width measurement
    // internally.
    let (prompt_tx, prompt_rx) = std::sync::mpsc::channel::<(String, String)>();
    let (line_tx, mut line_rx) =
        tokio::sync::mpsc::unbounded_channel::<std::result::Result<String, Signal>>();

    let shell_cwd_helper = Arc::new(std::sync::Mutex::new(
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
    ));
    let shell_cwd_shared = shell_cwd_helper.clone();
    std::thread::spawn(move || {
        let history = match FileBackedHistory::with_file(10_000, hist_path) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("History init error: {e}");
                FileBackedHistory::default()
            }
        };
        let mut editor = Reedline::create()
            .with_history(Box::new(history))
            .with_completer(Box::new(OrkaCompleter::new(shell_cwd_helper)))
            .with_hinter(Box::new(OrkaHinter::new()))
            .with_highlighter(Box::new(OrkaHighlighter))
            .with_validator(Box::new(OrkaValidator))
            .with_ansi_colors(true);

        while let Ok((_plain, colored)) = prompt_rx.recv() {
            let prompt = OrkaPrompt { colored };
            match editor.read_line(&prompt) {
                Ok(Signal::Success(line)) => {
                    if line_tx.send(Ok(line)).is_err() {
                        break;
                    }
                }
                Ok(sig) => {
                    // CtrlC or CtrlD — propagate and exit the editor loop
                    let _ = line_tx.send(Err(sig));
                    break;
                }
                Err(e) => {
                    eprintln!("Editor error: {e}");
                    break;
                }
            }
        }
        // FileBackedHistory flushes on drop automatically
    });

    // Shell state — initialise cwd and sync the shared reference used by
    // tab-completion
    let mut cwd = {
        let guard = shell_cwd_shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.clone()
    };
    let mut env_overrides: HashMap<String, String> = HashMap::new();
    let mut env_removes: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut last_exit: Option<i32> = None;
    let mut last_shell_cmd: Option<String> = None;
    // Recent shell executions buffered until the next agent message.
    let mut recent_shell_runs: Vec<(String, String, Option<i32>)> = Vec::new();
    let mut workspace_sent = false;

    // Idle timeout: fires when the connection is completely silent for this
    // duration. Resets on every received chunk, so long multi-iteration tasks
    // stay alive. ORKA_WS_TIMEOUT controls this value (seconds, default 120).
    let ws_idle_timeout_secs: u64 = std::env::var("ORKA_WS_TIMEOUT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(120);

    // REPL session state
    let mut turn_history: Vec<(String, String)> = Vec::new(); // (user_input, agent_response)
    let mut last_response = String::new();
    let mut ctrl_c_count: u32 = 0;

    loop {
        let (plain_prompt, colored_prompt) = build_prompt(&cwd, last_exit);

        // Send both plain and colored prompt to the editor thread. Reedline uses
        // the colored prompt directly (it handles ANSI width internally).
        if prompt_tx.send((plain_prompt, colored_prompt)).is_err() {
            break;
        }

        // Wait for the line from the editor thread
        let Some(result) = line_rx.recv().await else {
            break;
        };

        let line = match result {
            Ok(line) => {
                ctrl_c_count = 0;
                line
            }
            // CtrlC → double-press to exit
            Err(Signal::CtrlC) => {
                ctrl_c_count += 1;
                if ctrl_c_count >= 2 {
                    break;
                }
                println!("{}", "Press Ctrl-C again or type /quit to exit.".dimmed());
                continue;
            }
            // CtrlD or any other signal → exit
            Err(Signal::CtrlD | _) => break,
        };

        match shell::classify_input(&line) {
            InputAction::Empty => {}

            InputAction::Error(msg) => {
                eprintln!("{msg}");
                last_exit = Some(1);
            }

            InputAction::ShellExec(cmd) => {
                let result = shell::execute_shell(&cmd, &cwd, &env_overrides, &env_removes).await;
                last_exit = result.exit_code;
                let combined = if result.stderr.is_empty() {
                    result.stdout.clone()
                } else if result.stdout.is_empty() {
                    result.stderr.clone()
                } else {
                    format!("{}\n{}", result.stdout, result.stderr)
                };
                recent_shell_runs.push((cmd.clone(), combined, result.exit_code));
                if recent_shell_runs.len() > 5 {
                    recent_shell_runs.remove(0);
                }
                last_shell_cmd = Some(cmd);
            }

            InputAction::RepeatLast => {
                if let Some(ref cmd) = last_shell_cmd.clone() {
                    println!("{} {cmd}", "!".dimmed());
                    let result =
                        shell::execute_shell(cmd, &cwd, &env_overrides, &env_removes).await;
                    last_exit = result.exit_code;
                    let combined = if result.stderr.is_empty() {
                        result.stdout.clone()
                    } else if result.stdout.is_empty() {
                        result.stderr.clone()
                    } else {
                        format!("{}\n{}", result.stdout, result.stderr)
                    };
                    recent_shell_runs.push((cmd.clone(), combined, result.exit_code));
                    if recent_shell_runs.len() > 5 {
                        recent_shell_runs.remove(0);
                    }
                } else {
                    eprintln!("No previous shell command.");
                }
            }

            InputAction::Builtin(ref b) => {
                let msg = shell::handle_builtin(b, &mut cwd, &mut env_overrides, &mut env_removes);
                if msg.is_empty() {
                    last_exit = Some(0);
                    // Keep the tab-completion helper in sync when the directory changes
                    if matches!(b, Builtin::Cd(_)) {
                        (*shell_cwd_shared
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner))
                        .clone_from(&cwd);
                    }
                } else {
                    eprintln!("{msg}");
                    if matches!(b, Builtin::Cd(_)) {
                        last_exit = Some(1);
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
                                    Ok(()) => println!(
                                        "{}",
                                        format!("  \u{2714} Saved to {}", target.display())
                                            .dimmed()
                                    ),
                                    Err(e) => eprintln!("Failed to save: {e}"),
                                }
                            }
                        }
                        "copy" => {
                            if last_response.is_empty() {
                                println!("{}", "  No response to copy yet.".dimmed());
                            } else {
                                match arboard::Clipboard::new()
                                    .and_then(|mut cb| cb.set_text(&last_response))
                                {
                                    Ok(()) => {
                                        println!("{}", "  \u{2714} Copied to clipboard.".dimmed());
                                    }
                                    Err(e) => eprintln!("Clipboard error: {e}"),
                                }
                            }
                        }
                        "open" => {
                            let url = text
                                .trim_start_matches('/')
                                .trim_start_matches("open")
                                .trim();
                            if url.is_empty() {
                                println!("{}", "  Usage: /open <url>".dimmed());
                            } else {
                                match open::that(url) {
                                    Ok(()) => {
                                        println!(
                                            "{}",
                                            format!("  \u{2714} Opening {url}").dimmed()
                                        );
                                    }
                                    Err(e) => eprintln!("Failed to open: {e}"),
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
                // Use Acquire ordering to see the latest store from the WS task
                if !ws_alive.load(Ordering::Acquire) {
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
                                // Drain stale signals from previous connection to prevent
                                // spurious completions in this new session
                                while done_rx.try_recv().is_ok() {}
                                while response_rx.try_recv().is_ok() {}
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

                // Re-send workspace metadata after /memory clear (server clears its context).
                if expanded == "/memory clear" || expanded.starts_with("/memory clear ") {
                    workspace_sent = false;
                }

                let metadata = {
                    let mut meta = std::collections::HashMap::new();
                    if should_attach_workspace_cwd(client, include_workspace_cwd) {
                        meta.insert(
                            "workspace:cwd".to_string(),
                            serde_json::Value::String(cwd.to_string_lossy().into_owned()),
                        );
                    }
                    if !workspace_sent {
                        workspace_sent = true;
                        if let Some(ref ws) = local_workspace {
                            meta.extend(ws.to_metadata());
                        }
                    }
                    // Attach buffered shell command outputs so the AI has context.
                    if !recent_shell_runs.is_empty() {
                        let mut ctx = String::new();
                        for (cmd, output, code) in &recent_shell_runs {
                            let _ = writeln!(ctx, "$ {cmd}");
                            if !output.is_empty() {
                                ctx.push_str(output.trim_end());
                                ctx.push('\n');
                            }
                            if let Some(c) = code
                                && *c != 0
                            {
                                let _ = writeln!(ctx, "[exit {c}]");
                            }
                            ctx.push('\n');
                        }
                        meta.insert(
                            "shell:recent_commands".to_string(),
                            serde_json::Value::String(ctx),
                        );
                        recent_shell_runs.clear();
                    }
                    Some(meta)
                };

                let start = Instant::now();
                match client.send_message(&expanded, &sid, metadata).await {
                    Ok(_) => {
                        // Drain stale signals from both channels before waiting.
                        while done_rx.try_recv().is_ok() {}
                        while activity_rx.try_recv().is_ok() {}

                        // Idle-timeout loop: the deadline resets on every received chunk
                        // so long multi-iteration tasks never hit it as long as the server
                        // keeps streaming. It only fires when the connection goes silent.
                        let idle_dur = Duration::from_secs(ws_idle_timeout_secs);
                        let idle = tokio::time::sleep(idle_dur);
                        tokio::pin!(idle);

                        let mut break_repl = false;
                        loop {
                            tokio::select! {
                                result = done_rx.recv() => {
                                    if result.is_none() {
                                        eprintln!("{}", "Connection lost.".red());
                                        break_repl = true;
                                        break;
                                    }
                                    let elapsed = start.elapsed().as_millis() as u64;
                                        let response = response_rx.try_recv().unwrap_or_default();
                                        last_response.clone_from(&response);
                                        turn_history.push((user_input, response));

                                        // Desktop notification for long responses
                                        if elapsed >= 5000
                                            && std::env::var_os("ORKA_NO_NOTIFY").is_none()
                                        {
                                            let elapsed_s = elapsed as f64 / 1000.0;
                                            let _ = notify_rust::Notification::new()
                                                .summary("Orka")
                                                .body(&format!(
                                                    "Response complete ({elapsed_s:.1}s)"
                                                ))
                                                .show();
                                        }

                                        // Show elapsed + usage if available
                                        let usage_part = if let Ok(mut guard) = last_usage.lock()
                                            && let Some(usage) = guard.take()
                                        {
                                            let model = &usage.last_model;
                                            let model_short = {
                                                let parts: Vec<&str> = model.split('-').collect();
                                                let non_date: Vec<&str> = parts
                                                    .iter()
                                                    .copied()
                                                    .filter(|s: &&str| {
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
                                            let reason_part =
                                                if usage.total_reasoning_tokens > 0 {
                                                    format!(
                                                        " {}\u{26a1}",
                                                        fmt_tokens(usage.total_reasoning_tokens)
                                                    )
                                                } else {
                                                    String::new()
                                                };
                                            let cache_total = usage.total_cache_read_tokens
                                                + usage.total_cache_creation_tokens;
                                            let cache_part = if cache_total > 0 {
                                                format!(
                                                    " \u{2502} cache: {}",
                                                    fmt_tokens(cache_total)
                                                )
                                            } else {
                                                String::new()
                                            };
                                            let cost_part = if usage.total_cost_usd > 0.0 {
                                                format!(" \u{2502} ${:.4}", usage.total_cost_usd)
                                            } else {
                                                String::new()
                                            };
                                            let calls_part = if usage.call_count > 1 {
                                                format!(" \u{2502} {} calls", usage.call_count)
                                            } else {
                                                String::new()
                                            };
                                            format!(
                                                " \u{2502} {model_short} \u{2502} {}\u{2193} {}\u{2191}{reason_part}{cache_part}{cost_part}{calls_part}",
                                                fmt_tokens(usage.total_input_tokens),
                                                fmt_tokens(usage.total_output_tokens),
                                            )
                                        } else {
                                            String::new()
                                        };
                                        println!(
                                            "{}",
                                            format!(
                                                "  \u{23f1} {}{}",
                                                crate::util::format_duration_ms(elapsed),
                                                usage_part
                                            )
                                            .dimmed()
                                        );
                                        break;
                                }
                                // Each incoming chunk resets the idle deadline
                                _ = activity_rx.recv() => {
                                    idle.as_mut().reset(tokio::time::Instant::now() + idle_dur);
                                }
                                // Idle timeout: connection silent longer than the deadline
                                () = &mut idle => {
                                    ctrl_c_count = 0;
                                    {
                                        let mut guard = ws_write.lock().await;
                                        let _ = guard.close().await;
                                    }
                                    ws_timeout_flag.store(true, Ordering::Release);
                                    ws_alive.store(false, Ordering::Release);
                                    disconnect_notify.notify_one();
                                    while done_rx.try_recv().is_ok() {}
                                    while response_rx.try_recv().is_ok() {}
                                    while activity_rx.try_recv().is_ok() {}
                                    eprintln!(
                                        "{}",
                                        format!(
                                            "No response after {ws_idle_timeout_secs}s of inactivity. \
                                             The server may be slow or the connection may be dead. \
                                             Type /quit to exit."
                                        )
                                        .yellow()
                                    );
                                    multi.clear().ok();
                                    break;
                                }
                                () = disconnect_notify.notified() => {
                                    ctrl_c_count = 0;
                                    println!("{}", "\nConnection lost. Reconnect will be attempted on next input.".yellow());
                                    break;
                                }
                                _ = tokio::signal::ctrl_c() => {
                                    println!("{}", "\nCancelled.".yellow());
                                    break;
                                }
                            }
                        }
                        if break_repl {
                            break;
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
    // Signal graceful shutdown to suppress spurious connection error messages
    graceful_shutdown.store(true, Ordering::SeqCst);
    // Stop pings first — no data frames allowed after Close per WebSocket RFC
    ping_task.abort();
    // Send a clean WS Close frame and wait for the server's Close response
    {
        let mut guard = ws_write.lock().await;
        let _ = guard.send(Message::Close(None)).await;
        // Flush to ensure the Close frame is sent immediately
        let _ = guard.flush().await;
    }
    // Give the reader up to 2s to complete the close handshake before aborting
    // The ws_task will exit when it receives the server's Close frame or EOF
    let _ = tokio::time::timeout(Duration::from_secs(2), &mut ws_task).await;
    ws_task.abort();
    println!("\n{}", "Goodbye!".cyan());

    Ok(())
}

/// Migrate history file from rustyline's `#V2` format to reedline's plain-line
/// format. Removes the `#V2\n` header line if present; reedline uses plain
/// line-per-entry storage.
fn migrate_history_if_needed(path: &std::path::Path) {
    if let Ok(content) = std::fs::read_to_string(path)
        && let Some(stripped) = content.strip_prefix("#V2\n")
    {
        let _ = std::fs::write(path, stripped);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

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

    #[test]
    fn expand_file_attachments_reads_real_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.rs");
        std::fs::write(&file_path, "fn main() {}").unwrap();
        let text = format!("check @{}", file_path.display());
        let result = expand_file_attachments(&text);
        assert!(result.contains("```rs"));
        assert!(result.contains("fn main() {}"));
        assert!(result.contains("```"));
    }

    #[test]
    fn expand_file_attachments_missing_file_left_as_is() {
        let result = expand_file_attachments("look at @/nonexistent/file.txt please");
        assert!(result.contains("@/nonexistent/file.txt"));
    }

    #[test]
    fn expand_file_attachments_multiple_files() {
        let dir = tempfile::tempdir().unwrap();
        let f1 = dir.path().join("a.py");
        let f2 = dir.path().join("b.py");
        std::fs::write(&f1, "print('a')").unwrap();
        std::fs::write(&f2, "print('b')").unwrap();
        let text = format!("@{} and @{}", f1.display(), f2.display());
        let result = expand_file_attachments(&text);
        assert!(result.contains("print('a')"));
        assert!(result.contains("print('b')"));
    }

    #[test]
    fn expand_file_attachments_bare_at_sign() {
        // A lone `@` with no path should be left as-is
        let result = expand_file_attachments("@ alone");
        assert!(result.contains('@'));
    }

    #[test]
    fn should_attach_workspace_cwd_only_for_local_targets() {
        let local = OrkaClient::new("http://127.0.0.1:8081", None);
        let remote = OrkaClient::new("http://192.168.1.103:18081", None);
        assert!(should_attach_workspace_cwd(&local, true));
        assert!(!should_attach_workspace_cwd(&local, false));
        assert!(!should_attach_workspace_cwd(&remote, true));
    }
}
