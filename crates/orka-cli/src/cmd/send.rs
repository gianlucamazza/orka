use colored::Colorize;
use futures_util::StreamExt;
use orka_core::stream::StreamChunkKind;

use crate::{
    client::{OrkaClient, Result},
    markdown::MarkdownRenderer,
    protocol::{WsMessage, classify_ws_message},
};

pub async fn run(
    client: &OrkaClient,
    text: &str,
    session_id: Option<&str>,
    timeout_secs: u64,
    local_workspace: Option<crate::workspace::LocalWorkspace>,
) -> Result<()> {
    let sid = OrkaClient::resolve_session_id(session_id);

    eprintln!("{} {}", "Session:".bold(), sid.dimmed());
    if let Some(ref ws) = local_workspace {
        eprintln!("Workspace: {}", ws.root.display().to_string().dimmed());
    }
    eprintln!("{} {}", "Sending:".bold(), text);

    let mut metadata = local_workspace
        .as_ref()
        .map(|ws| ws.to_metadata())
        .unwrap_or_default();
    metadata.insert(
        "workspace:cwd".to_string(),
        serde_json::Value::String(
            std::env::current_dir()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
        ),
    );
    let metadata = Some(metadata);

    // Connect WebSocket BEFORE sending the HTTP message to avoid missing fast
    // replies. For quick responses the server may stream the reply before the
    // HTTP call returns, so the WS connection must be established first.
    eprintln!("{}", "Waiting for reply...".dimmed());

    let mut renderer = MarkdownRenderer::default();
    let idle_dur = std::time::Duration::from_secs(timeout_secs);

    let ws = client.ws_connect(&sid).await?;
    let (_write, read) = ws.split();
    let resp = client.send_message(text, &sid, metadata).await?;
    if let Some(msg_id) = resp.get("message_id").and_then(|v| v.as_str()) {
        eprintln!("{} {}", "Message ID:".bold(), msg_id.dimmed());
    }

    match stream_reply(read, &mut renderer, idle_dur).await {
        Ok(()) => {}
        Err(e) => {
            let msg = e.to_string();
            if msg.starts_with("idle:") {
                eprintln!(
                    "{}",
                    format!(
                        "No response after {timeout_secs}s of inactivity (use --timeout to increase)."
                    )
                    .dimmed()
                );
            } else {
                tracing::debug!("WS error: {e}");
                eprintln!("{}", "No reply received (connection error).".dimmed());
            }
        }
    }

    Ok(())
}

async fn stream_reply<S>(
    mut read: S,
    renderer: &mut MarkdownRenderer,
    idle_timeout: std::time::Duration,
) -> Result<()>
where
    S: futures_util::Stream<
            Item = std::result::Result<
                tokio_tungstenite::tungstenite::Message,
                tokio_tungstenite::tungstenite::Error,
            >,
        > + Unpin,
{
    let mut got_content = false;
    let mut thinking_shown = false;

    loop {
        let msg = match tokio::time::timeout(idle_timeout, read.next()).await {
            Ok(Some(Ok(msg))) => msg,
            Ok(Some(Err(e))) => return Err(e.into()),
            Ok(None) => break,
            Err(_) => {
                return Err(format!(
                    "idle: no activity for {}s",
                    idle_timeout.as_secs()
                )
                .into())
            }
        };
        if !msg.is_text() {
            continue;
        }
        // Use Deref<Target=str> on Utf8Bytes directly — no allocation needed.
        let raw = msg.into_text()?;
        match classify_ws_message(&raw) {
            WsMessage::Stream(StreamChunkKind::Delta(data)) => {
                if !got_content {
                    println!("\n{}", "Reply:".green().bold());
                    got_content = true;
                }
                renderer.push_delta(&data);
            }
            WsMessage::Stream(StreamChunkKind::ToolExecStart {
                name,
                input_summary,
                ..
            }) => {
                let label = match &input_summary {
                    Some(s) => format!("{name}: {s}"),
                    None => name.clone(),
                };
                eprintln!("  {} {}...", "⚙".dimmed(), label.dimmed());
            }
            WsMessage::Stream(StreamChunkKind::ToolExecEnd {
                success,
                duration_ms,
                error,
                result_summary,
                ..
            }) => {
                let dur = crate::util::format_duration_ms(duration_ms);
                if success {
                    let suffix = result_summary
                        .map(|s| format!(" — {s}"))
                        .unwrap_or_default();
                    eprintln!("  {} ({dur}){suffix}", "✓".green());
                } else {
                    let suffix = error
                        .or(result_summary)
                        .map(|s| format!(" — {s}"))
                        .unwrap_or_default();
                    eprintln!("  {} ({dur}){suffix}", "✗".red());
                }
            }
            WsMessage::Stream(StreamChunkKind::ThinkingDelta(_)) => {
                if !thinking_shown {
                    eprintln!("  {}", "thinking...".dimmed());
                    thinking_shown = true;
                }
            }
            WsMessage::Stream(StreamChunkKind::AgentSwitch { display_name, .. }) => {
                eprintln!("  {}", format!("[{display_name}]").dimmed());
            }
            WsMessage::Stream(StreamChunkKind::Done) => {
                if got_content {
                    renderer.flush();
                } else {
                    println!("{}", "(empty response)".dimmed());
                }
                return Ok(());
            }
            WsMessage::Final(content) => {
                if !got_content {
                    println!("\n{}", "Reply:".green().bold());
                }
                renderer.render_full(&content);
                return Ok(());
            }
            // ToolStart, ToolEnd, Usage, ContextInfo, PrinciplesUsed — silent
            _ => {}
        }
    }

    if got_content {
        renderer.flush();
        Ok(())
    } else {
        Err("WebSocket closed without reply".into())
    }
}
