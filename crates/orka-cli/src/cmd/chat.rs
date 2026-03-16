use std::time::Duration;

use colored::Colorize;
use futures_util::StreamExt;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_tungstenite::connect_async;

use crate::client::{OrkaClient, Result};

pub async fn run(client: &OrkaClient, session_id: Option<&str>) -> Result<()> {
    let sid = OrkaClient::resolve_session_id(session_id);

    // Wait for server to be ready before attempting WebSocket connection
    client.wait_for_ready(300, Duration::from_secs(1)).await?;

    println!("{}", "=== Orka Chat ===".bold().cyan());
    println!("Session: {}", sid.dimmed());
    println!(
        "Type your messages below. Use {} or {} to exit.\n",
        "/quit".yellow(),
        "Ctrl+C".yellow()
    );

    // Connect WebSocket
    let ws_url = client.ws_url(&sid);
    let (ws, _) = connect_async(&ws_url)
        .await
        .map_err(|e| format!("Failed to connect WebSocket to {ws_url}: {e}"))?;
    let (_write, mut ws_read) = ws.split();

    // WS reader task: print incoming messages
    let ws_task = tokio::spawn(async move {
        while let Some(msg) = ws_read.next().await {
            match msg {
                Ok(msg) if msg.is_text() => {
                    let text = match msg.into_text() {
                        Ok(t) => t.to_string(),
                        Err(_) => continue,
                    };
                    let content = crate::protocol::extract_ws_text(&text);
                    println!("\n{} {}", "Agent:".green().bold(), content);
                    print!("{} ", "You:".cyan().bold());
                    // Flush to show the prompt
                    use std::io::Write;
                    let _ = std::io::stdout().flush();
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

    // Stdin reader: read lines and POST them
    let stdin = BufReader::new(tokio::io::stdin());
    let mut lines = stdin.lines();

    print!("{} ", "You:".cyan().bold());
    use std::io::Write;
    std::io::stdout().flush()?;

    loop {
        let line = tokio::select! {
            line = lines.next_line() => {
                match line {
                    Ok(Some(line)) => line,
                    Ok(None) => break, // EOF
                    Err(e) => {
                        eprintln!("Input error: {e}");
                        break;
                    }
                }
            }
            _ = tokio::signal::ctrl_c() => {
                break;
            }
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            print!("{} ", "You:".cyan().bold());
            std::io::stdout().flush()?;
            continue;
        }
        if trimmed == "/quit" {
            break;
        }

        match client.send_message(trimmed, &sid).await {
            Ok(_) => {
                // Message sent, wait for WS reply (handled by ws_task)
            }
            Err(e) => {
                eprintln!("{} {e}", "Send failed:".red());
                print!("{} ", "You:".cyan().bold());
                std::io::stdout().flush()?;
            }
        }
    }

    ws_task.abort();
    println!("\n{}", "Goodbye!".cyan());

    Ok(())
}
