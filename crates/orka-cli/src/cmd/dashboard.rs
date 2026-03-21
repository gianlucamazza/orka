//! Real-time TUI dashboard for Orka server monitoring.
//!
//! Renders a terminal UI (via `ratatui`) that polls the server at a
//! configurable interval and displays health, dependency readiness,
//! Prometheus metrics, active sessions, and DLQ depth.
//!
//! Keybindings:
//! - `q` / `Esc` — quit
//! - `r` — force-refresh immediately

use std::time::{Duration, Instant};

use crossterm::event::{Event, EventStream, KeyCode, KeyEventKind};
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use futures_util::StreamExt as _;
use ratatui::Frame;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};

use crate::client::OrkaClient;
use crate::util::{format_duration_ms, format_uptime, truncate_id};

// ── State ─────────────────────────────────────────────────────────────────────

struct SessionRow {
    id: String,
    channel: String,
    user_id: String,
    updated_at: String,
}

struct DashboardState {
    server_url: String,
    interval_secs: u64,
    // /health
    health_status: String,
    uptime_secs: u64,
    workers: u64,
    queue_depth: u64,
    // /api/v1/version
    version: String,
    git_sha: String,
    // /health/ready
    checks: Vec<(String, String)>,
    // /metrics (Prometheus counters / histogram averages)
    messages_total: f64,
    llm_completions_total: f64,
    errors_total: f64,
    skill_invocations_total: f64,
    llm_input_tokens: f64,
    llm_output_tokens: f64,
    llm_cost_microdollars: f64,
    handler_duration_avg: f64,
    llm_duration_avg: f64,
    // /api/v1/sessions  /api/v1/dlq
    sessions: Vec<SessionRow>,
    dlq_count: usize,
    // meta
    last_refresh: Option<Instant>,
    last_error: Option<String>,
}

impl DashboardState {
    fn new(server_url: String, interval_secs: u64) -> Self {
        Self {
            server_url,
            interval_secs,
            health_status: "—".to_string(),
            uptime_secs: 0,
            workers: 0,
            queue_depth: 0,
            version: String::new(),
            git_sha: String::new(),
            checks: Vec::new(),
            messages_total: 0.0,
            llm_completions_total: 0.0,
            errors_total: 0.0,
            skill_invocations_total: 0.0,
            llm_input_tokens: 0.0,
            llm_output_tokens: 0.0,
            llm_cost_microdollars: 0.0,
            handler_duration_avg: 0.0,
            llm_duration_avg: 0.0,
            sessions: Vec::new(),
            dlq_count: 0,
            last_refresh: None,
            last_error: None,
        }
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

pub async fn run(client: &OrkaClient, interval: u64) -> crate::client::Result<()> {
    // Restore terminal on panic
    let orig_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = crossterm::execute!(std::io::stderr(), LeaveAlternateScreen);
        let _ = crossterm::terminal::disable_raw_mode();
        orig_hook(info);
    }));

    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_loop(&mut terminal, client, interval).await;

    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

// ── Main loop ─────────────────────────────────────────────────────────────────

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    client: &OrkaClient,
    interval: u64,
) -> crate::client::Result<()> {
    let mut state = DashboardState::new(client.base_url().to_string(), interval);
    let mut tick = tokio::time::interval(Duration::from_secs(interval));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut events = EventStream::new();
    let mut quit = false;
    let mut refresh = false;

    // Initial poll before first draw
    poll_all(client, &mut state).await;
    terminal.draw(|f| ui(f, &state))?;

    loop {
        tokio::select! {
            _ = tick.tick() => {
                poll_all(client, &mut state).await;
            }
            maybe_event = events.next() => {
                if let Some(Ok(event)) = maybe_event {
                    handle_input(event, &mut quit, &mut refresh);
                }
            }
            _ = tokio::signal::ctrl_c() => {
                quit = true;
            }
        }

        if refresh {
            poll_all(client, &mut state).await;
            refresh = false;
        }

        terminal.draw(|f| ui(f, &state))?;

        if quit {
            break;
        }
    }

    Ok(())
}

// ── Input ─────────────────────────────────────────────────────────────────────

fn handle_input(event: Event, quit: &mut bool, refresh: &mut bool) {
    if let Event::Key(key) = event
        && key.kind == KeyEventKind::Press
    {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => *quit = true,
            KeyCode::Char('r') => *refresh = true,
            _ => {}
        }
    }
}

// ── Polling ───────────────────────────────────────────────────────────────────

async fn poll_all(client: &OrkaClient, state: &mut DashboardState) {
    let (health, ready, version, metrics_resp, sessions, dlq) = tokio::join!(
        client.get_json("/health"),
        client.get_json("/health/ready"),
        client.get_json("/api/v1/version"),
        client.get("/metrics"),
        client.get_json("/api/v1/sessions?limit=20"),
        client.get_json("/api/v1/dlq"),
    );

    state.last_error = None;

    // /health
    match health {
        Ok(body) => {
            state.health_status = body["status"].as_str().unwrap_or("unknown").to_string();
            state.uptime_secs = body["uptime_secs"].as_u64().unwrap_or(0);
            state.workers = body["workers"].as_u64().unwrap_or(0);
            state.queue_depth = body["queue_depth"].as_u64().unwrap_or(0);
        }
        Err(e) => {
            state.health_status = "unreachable".to_string();
            state.last_error = Some(format!("health: {e}"));
        }
    }

    // /api/v1/version (only overwrite on success)
    if let Ok(body) = version {
        if let Some(v) = body["version"].as_str() {
            state.version = v.to_string();
        }
        if let Some(sha) = body["git_sha"].as_str() {
            state.git_sha = sha.chars().take(7).collect();
        }
    }

    // /health/ready
    match ready {
        Ok(body) => {
            state.checks.clear();
            if let Some(checks) = body["checks"].as_object() {
                for (name, val) in checks {
                    let status = if val.is_string() {
                        val.as_str().unwrap_or("?").to_string()
                    } else if let Some(obj) = val.as_object() {
                        let s = obj.get("status").and_then(|v| v.as_str()).unwrap_or("?");
                        if let Some(d) = obj.get("depth").and_then(|v| v.as_u64()) {
                            format!("{s} ({d})")
                        } else {
                            s.to_string()
                        }
                    } else {
                        "?".to_string()
                    };
                    state.checks.push((name.clone(), status));
                }
                state.checks.sort_by(|a, b| a.0.cmp(&b.0));
            }
        }
        Err(e) => {
            if state.last_error.is_none() {
                state.last_error = Some(format!("ready: {e}"));
            }
        }
    }

    // /metrics
    match metrics_resp {
        Ok(resp) => {
            if let Ok(text) = resp.text().await {
                state.messages_total = parse_metric_sum(&text, "orka_messages_received_total");
                state.llm_completions_total = parse_metric_sum(&text, "orka_llm_completions_total");
                state.errors_total = parse_metric_sum(&text, "orka_errors_total");
                state.skill_invocations_total =
                    parse_metric_sum(&text, "orka_skill_invocations_total");
                state.llm_input_tokens = parse_metric_sum(&text, "orka_llm_input_tokens_total");
                state.llm_output_tokens = parse_metric_sum(&text, "orka_llm_output_tokens_total");
                state.llm_cost_microdollars =
                    parse_metric_sum(&text, "orka_llm_cost_dollars_total");
                state.handler_duration_avg =
                    parse_histogram_avg(&text, "orka_handler_duration_seconds");
                state.llm_duration_avg = parse_histogram_avg(&text, "orka_llm_duration_seconds");
            }
        }
        Err(e) => {
            if state.last_error.is_none() {
                state.last_error = Some(format!("metrics: {e}"));
            }
        }
    }

    // /api/v1/sessions
    match sessions {
        Ok(body) => {
            state.sessions = body
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or(&[])
                .iter()
                .map(|s| SessionRow {
                    id: s["id"].as_str().unwrap_or("?").to_string(),
                    channel: s["channel"].as_str().unwrap_or("?").to_string(),
                    user_id: s["user_id"].as_str().unwrap_or("?").to_string(),
                    updated_at: s["updated_at"].as_str().unwrap_or("?").to_string(),
                })
                .collect();
        }
        Err(e) => {
            if state.last_error.is_none() {
                state.last_error = Some(format!("sessions: {e}"));
            }
        }
    }

    // /api/v1/dlq
    match dlq {
        Ok(body) => {
            state.dlq_count = body.as_array().map(|a| a.len()).unwrap_or(0);
        }
        Err(e) => {
            if state.last_error.is_none() {
                state.last_error = Some(format!("dlq: {e}"));
            }
        }
    }

    state.last_refresh = Some(Instant::now());
}

// ── Metric parsing ────────────────────────────────────────────────────────────

fn parse_metric_sum(text: &str, prefix: &str) -> f64 {
    text.lines()
        .filter(|l| !l.starts_with('#'))
        .filter(|l| l.starts_with(prefix) && l[prefix.len()..].starts_with(['{', ' ']))
        .filter_map(|l| l.rsplit_once(' ')?.1.parse::<f64>().ok())
        .sum()
}

fn parse_histogram_avg(text: &str, name: &str) -> f64 {
    let sum = parse_metric_sum(text, &format!("{name}_sum"));
    let count = parse_metric_sum(text, &format!("{name}_count"));
    if count > 0.0 { sum / count } else { 0.0 }
}

// ── Rendering ─────────────────────────────────────────────────────────────────

fn ui(frame: &mut Frame, state: &DashboardState) {
    let chunks = Layout::vertical([
        Constraint::Length(4),
        Constraint::Min(8),
        Constraint::Min(5),
        Constraint::Length(1),
    ])
    .split(frame.area());

    render_header(frame, chunks[0], state);

    let mid = Layout::horizontal([Constraint::Percentage(30), Constraint::Percentage(70)])
        .split(chunks[1]);

    render_deps(frame, mid[0], state);
    render_metrics(frame, mid[1], state);
    render_sessions(frame, chunks[2], state);
    render_footer(frame, chunks[3], state);
}

// ── Header ────────────────────────────────────────────────────────────────────

fn render_header(frame: &mut Frame, area: Rect, state: &DashboardState) {
    let health_style = if state.health_status == "ok" {
        Style::default().fg(Color::Green)
    } else {
        Style::default().fg(Color::Red)
    };

    let version_part = if state.version.is_empty() {
        String::new()
    } else if state.git_sha.is_empty() {
        format!("  v{}", state.version)
    } else {
        format!("  v{} ({})", state.version, state.git_sha)
    };

    let line1 = Line::from(vec![
        Span::styled(
            "ORKA DASHBOARD",
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(Color::Cyan),
        ),
        Span::styled(version_part, Style::default().fg(Color::DarkGray)),
        Span::raw("   "),
        Span::styled(state.server_url.clone(), Style::default().fg(Color::Yellow)),
        Span::styled(
            format!("   every {}s", state.interval_secs),
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    let line2 = Line::from(vec![
        Span::raw("Health: "),
        Span::styled(state.health_status.clone(), health_style),
        Span::raw(format!(
            "   Uptime: {}   Workers: {}   Queue: {}",
            format_uptime(state.uptime_secs),
            state.workers,
            state.queue_depth
        )),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    frame.render_widget(Paragraph::new(vec![line1, line2]).block(block), area);
}

// ── Dependencies ──────────────────────────────────────────────────────────────

fn render_deps(frame: &mut Frame, area: Rect, state: &DashboardState) {
    let lines: Vec<Line> = if state.checks.is_empty() {
        vec![Line::from(Span::styled(
            "—",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        state
            .checks
            .iter()
            .map(|(name, status)| {
                let ok = status.starts_with("ok") || status.starts_with("ready");
                let dot_style = if ok {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default().fg(Color::Red)
                };
                let val_style = if ok {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default().fg(Color::Red)
                };
                Line::from(vec![
                    Span::styled("● ", dot_style),
                    Span::raw(format!("{name}: ")),
                    Span::styled(status.clone(), val_style),
                ])
            })
            .collect()
    };

    let block = Block::default()
        .title(" Dependencies ")
        .borders(Borders::ALL);
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

// ── Metrics ───────────────────────────────────────────────────────────────────

fn render_metrics(frame: &mut Frame, area: Rect, state: &DashboardState) {
    let mut lines: Vec<Line> = vec![
        metric_line("Messages:", format_count(state.messages_total), false),
        metric_line(
            "LLM calls:",
            format_count(state.llm_completions_total),
            false,
        ),
        metric_line(
            "Skills:",
            format_count(state.skill_invocations_total),
            false,
        ),
        metric_line(
            "Errors:",
            format_count(state.errors_total),
            state.errors_total > 0.0,
        ),
        Line::from(vec![
            Span::styled(
                format!("{:<18}", "Tokens:"),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                format!("{}↓", format_count(state.llm_input_tokens)),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(" / "),
            Span::styled(
                format!("{}↑", format_count(state.llm_output_tokens)),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]),
        metric_line("LLM cost:", format_cost(state.llm_cost_microdollars), false),
    ];

    if state.handler_duration_avg > 0.0 {
        lines.push(metric_line(
            "Avg handler:",
            format_duration_ms((state.handler_duration_avg * 1000.0) as u64),
            false,
        ));
    }
    if state.llm_duration_avg > 0.0 {
        lines.push(metric_line(
            "Avg LLM:",
            format_duration_ms((state.llm_duration_avg * 1000.0) as u64),
            false,
        ));
    }

    let block = Block::default().title(" Metrics ").borders(Borders::ALL);
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn metric_line(label: &'static str, value: String, error: bool) -> Line<'static> {
    let val_style = if error {
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
    } else {
        Style::default().add_modifier(Modifier::BOLD)
    };
    Line::from(vec![
        Span::styled(format!("{label:<18}"), Style::default().fg(Color::DarkGray)),
        Span::styled(value, val_style),
    ])
}

// ── Sessions ──────────────────────────────────────────────────────────────────

fn render_sessions(frame: &mut Frame, area: Rect, state: &DashboardState) {
    let title = format!(
        " Sessions ({})   DLQ: {} ",
        state.sessions.len(),
        state.dlq_count
    );
    let block = Block::default().title(title).borders(Borders::ALL);

    if state.sessions.is_empty() {
        frame.render_widget(
            Paragraph::new("No active sessions.")
                .style(Style::default().fg(Color::DarkGray))
                .block(block),
            area,
        );
        return;
    }

    let header = Row::new(vec![
        Cell::from("ID").style(
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(Color::Cyan),
        ),
        Cell::from("Channel").style(
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(Color::Cyan),
        ),
        Cell::from("User").style(
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(Color::Cyan),
        ),
        Cell::from("Updated").style(
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(Color::Cyan),
        ),
    ]);

    let rows: Vec<Row> = state
        .sessions
        .iter()
        .map(|s| {
            Row::new(vec![
                Cell::from(truncate_id(&s.id, 16)),
                Cell::from(s.channel.clone()),
                Cell::from(s.user_id.clone()),
                Cell::from(s.updated_at.clone()),
            ])
        })
        .collect();

    let widths = [
        Constraint::Percentage(25),
        Constraint::Percentage(15),
        Constraint::Percentage(25),
        Constraint::Percentage(35),
    ];

    let table = Table::new(rows, widths).header(header).block(block);
    frame.render_widget(table, area);
}

// ── Footer ────────────────────────────────────────────────────────────────────

fn render_footer(frame: &mut Frame, area: Rect, state: &DashboardState) {
    let line = if let Some(ref err) = state.last_error {
        Line::from(Span::styled(
            format!(" Error: {err}"),
            Style::default().fg(Color::Red),
        ))
    } else {
        let ago = state
            .last_refresh
            .map(|t| format!("{}s ago", t.elapsed().as_secs()))
            .unwrap_or_else(|| "never".to_string());
        Line::from(Span::styled(
            format!(" Last refresh: {ago}   q: quit   r: refresh"),
            Style::default().fg(Color::DarkGray),
        ))
    };
    frame.render_widget(Paragraph::new(line), area);
}

// ── Formatting helpers ────────────────────────────────────────────────────────

fn format_count(n: f64) -> String {
    let n = if n == 0.0 { 0.0 } else { n };
    if n >= 1_000_000.0 {
        format!("{:.1}M", n / 1_000_000.0)
    } else if n >= 1_000.0 {
        format!("{:.1}k", n / 1_000.0)
    } else {
        format!("{n:.0}")
    }
}

fn format_cost(microdollars: f64) -> String {
    let v = microdollars / 1_000_000.0;
    let v = if v == 0.0 { 0.0 } else { v };
    format!("${v:.2}")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_METRICS: &str = "\
# HELP orka_messages_received_total Messages received
# TYPE orka_messages_received_total counter
orka_messages_received_total{channel=\"telegram\"} 42
orka_messages_received_total{channel=\"discord\"} 10
# HELP orka_errors_total Errors
# TYPE orka_errors_total counter
orka_errors_total{source=\"handler\"} 3
# HELP orka_handler_duration_seconds Handler duration
# TYPE orka_handler_duration_seconds histogram
orka_handler_duration_seconds_sum 120.0
orka_handler_duration_seconds_count 60";

    #[test]
    fn parse_metric_sum_sums_all_labels() {
        assert_eq!(
            parse_metric_sum(SAMPLE_METRICS, "orka_messages_received_total"),
            52.0
        );
    }

    #[test]
    fn parse_metric_sum_returns_zero_for_missing() {
        assert_eq!(parse_metric_sum(SAMPLE_METRICS, "nonexistent_metric"), 0.0);
    }

    #[test]
    fn parse_metric_sum_does_not_match_prefix_substring() {
        // "orka_errors_total" must not match "orka_errors_total_something"
        assert_eq!(parse_metric_sum(SAMPLE_METRICS, "orka_errors_total"), 3.0);
    }

    #[test]
    fn parse_histogram_avg_computes_ratio() {
        let avg = parse_histogram_avg(SAMPLE_METRICS, "orka_handler_duration_seconds");
        assert!((avg - 2.0).abs() < 0.001, "expected 2.0, got {avg}");
    }

    #[test]
    fn parse_histogram_avg_zero_when_no_count() {
        assert_eq!(
            parse_histogram_avg(SAMPLE_METRICS, "orka_llm_duration_seconds"),
            0.0
        );
    }

    #[test]
    fn format_count_below_thousand() {
        assert_eq!(format_count(0.0), "0");
        assert_eq!(format_count(999.0), "999");
    }

    #[test]
    fn format_count_thousands() {
        assert_eq!(format_count(1234.0), "1.2k");
    }

    #[test]
    fn format_count_millions() {
        assert_eq!(format_count(1_000_000.0), "1.0M");
    }

    #[test]
    fn format_cost_converts_microdollars() {
        assert_eq!(format_cost(1_230_000.0), "$1.23");
        assert_eq!(format_cost(0.0), "$0.00");
    }
}
