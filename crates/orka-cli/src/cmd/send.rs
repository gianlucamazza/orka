use colored::Colorize;
use futures_util::StreamExt;
use orka_core::stream::StreamChunkKind;
use tokio_tungstenite::connect_async;

use crate::client::{OrkaClient, Result};
use crate::protocol::{WsMessage, classify_ws_message};

pub async fn run(
    client: &OrkaClient,
    text: &str,
    session_id: Option<&str>,
    timeout_secs: u64,
    local_workspace: Option<crate::workspace::LocalWorkspace>,
) -> Result<()> {
    let sid = OrkaClient::resolve_session_id(session_id);

    println!("{} {}", "Session:".bold(), sid.dimmed());
    if let Some(ref ws) = local_workspace {
        println!("Workspace: {}", ws.root.display().to_string().dimmed());
    }
    println!("{} {}", "Sending:".bold(), text);

    let metadata = local_workspace.as_ref().map(|ws| ws.to_metadata());
    let resp = client.send_message(text, &sid, metadata).await?;

    if let Some(msg_id) = resp.get("message_id").and_then(|v| v.as_str()) {
        println!("{} {}", "Message ID:".bold(), msg_id.dimmed());
    }

    // Try to connect to WS and wait for a reply with a timeout
    let ws_url = client.ws_url(&sid);
    println!("{}", "Waiting for reply...".dimmed());

    let ws_result = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        wait_for_reply(&ws_url),
    )
    .await;

    match ws_result {
        Ok(Ok(reply)) => {
            println!("\n{}", "Reply:".green().bold());
            let renderer = crate::markdown::MarkdownRenderer::new();
            renderer.render_full(&reply);
        }
        Ok(Err(e)) => {
            tracing::debug!("WS error: {e}");
            println!("{}", "No reply received (connection error).".dimmed());
        }
        Err(_) => {
            println!("{}", "No reply received (timeout).".dimmed());
        }
    }

    Ok(())
}

async fn wait_for_reply(ws_url: &str) -> Result<String> {
    let (ws, _) = connect_async(ws_url).await?;
    let (_write, mut read) = ws.split();

    let mut accumulated = String::new();

    while let Some(msg) = read.next().await {
        let msg = msg?;
        if msg.is_text() {
            let text = msg.into_text()?.to_string();
            match classify_ws_message(&text) {
                WsMessage::Stream(StreamChunkKind::Delta(data)) => {
                    accumulated.push_str(&data);
                }
                WsMessage::Stream(StreamChunkKind::Done) => {
                    if !accumulated.is_empty() {
                        return Ok(accumulated);
                    }
                }
                WsMessage::Final(content) => {
                    if accumulated.is_empty() {
                        return Ok(content);
                    }
                    // Already have streamed content, prefer that
                    return Ok(accumulated);
                }
                _ => {}
            }
        }
    }

    if !accumulated.is_empty() {
        return Ok(accumulated);
    }
    Err("WebSocket closed without reply".into())
}
