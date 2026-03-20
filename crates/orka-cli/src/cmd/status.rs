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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_uptime_seconds_only() {
        assert_eq!(format_uptime(0), "0s");
        assert_eq!(format_uptime(59), "59s");
    }

    #[test]
    fn format_uptime_minutes_and_seconds() {
        assert_eq!(format_uptime(60), "1m 00s");
        assert_eq!(format_uptime(90), "1m 30s");
        assert_eq!(format_uptime(3599), "59m 59s");
    }

    #[test]
    fn format_uptime_hours() {
        assert_eq!(format_uptime(3600), "1h 00m 00s");
        assert_eq!(format_uptime(3661), "1h 01m 01s");
        assert_eq!(format_uptime(7384), "2h 03m 04s");
    }
}
