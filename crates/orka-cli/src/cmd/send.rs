use colored::Colorize;
use futures_util::StreamExt;
use tokio_tungstenite::connect_async;

use crate::client::{OrkaClient, Result};

pub async fn run(
    client: &OrkaClient,
    text: &str,
    session_id: Option<&str>,
    timeout_secs: u64,
) -> Result<()> {
    let sid = OrkaClient::resolve_session_id(session_id);

    println!("{} {}", "Session:".bold(), sid.dimmed());
    println!("{} {}", "Sending:".bold(), text);

    let resp = client.send_message(text, &sid).await?;

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
            println!("\n{} {}", "Reply:".green().bold(), reply);
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

    while let Some(msg) = read.next().await {
        let msg = msg?;
        if msg.is_text() {
            let text = msg.into_text()?.to_string();
            return Ok(crate::protocol::extract_ws_text(&text));
        }
    }

    Err("WebSocket closed without reply".into())
}
