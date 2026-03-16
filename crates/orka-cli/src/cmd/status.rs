use colored::Colorize;

use crate::client::{OrkaClient, Result};

fn format_uptime(secs: u64) -> String {
    let hours = secs / 3600;
    let minutes = (secs % 3600) / 60;
    let seconds = secs % 60;
    if hours > 0 {
        format!("{hours}h {minutes:02}m {seconds:02}s")
    } else if minutes > 0 {
        format!("{minutes}m {seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}

pub async fn run(client: &OrkaClient) -> Result<()> {
    println!("{} {}", "Server:".bold(), client.base_url());

    match client.get_json("/health").await {
        Ok(body) => {
            let status = body
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");

            if status == "ok" {
                println!("{} {}", "Health:".bold(), "ok".green());
            } else {
                println!("{} {}", "Health:".bold(), status.red());
            }

            if let Some(uptime) = body.get("uptime_secs").and_then(|v| v.as_u64()) {
                println!("{} {}", "Uptime:".bold(), format_uptime(uptime));
            }

            if let Some(workers) = body.get("workers").and_then(|v| v.as_u64()) {
                println!("{} {}", "Workers:".bold(), workers);
            }

            if let Some(queue) = body.get("queue_depth").and_then(|v| v.as_u64()) {
                println!("{} {}", "Queue:".bold(), queue);
            }
        }
        Err(e) => {
            println!("{} {} ({e})", "Health:".bold(), "unreachable".red());
        }
    }

    Ok(())
}
