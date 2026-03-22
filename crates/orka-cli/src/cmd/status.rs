use colored::Colorize;

use crate::client::{OrkaClient, Result};

pub async fn run(client: &OrkaClient, short: bool) -> Result<()> {
    if short {
        // Minimal output + exit code 1 if not healthy/ready (for scripting/probes)
        match client.get_json("/health").await {
            Ok(body) => {
                let status = body
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                if status == "ok" {
                    println!("{}", "ok".green());
                } else {
                    eprintln!("{}", status.red());
                    return Err(status.to_string().into());
                }
            }
            Err(e) => {
                eprintln!("{}: {e}", "unreachable".red());
                return Err(e);
            }
        }
        return Ok(());
    }

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
                println!(
                    "{} {}",
                    "Uptime:".bold(),
                    crate::util::format_uptime(uptime)
                );
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
            return Ok(());
        }
    }

    // Readiness checks
    if let Ok(body) = client.get_json("/health/ready").await {
        // The response uses `"status": "ready"` (not a bool `"ready"` field)
        let status_str = body
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let ready = status_str == "ready";
        if ready {
            println!("{} {}", "Ready:".bold(), "yes".green());
        } else {
            println!("{} {}", "Ready:".bold(), "no".red());
        }

        if let Some(checks) = body.get("checks").and_then(|v| v.as_object()) {
            println!("{}:", "Checks".bold());
            for (name, value) in checks {
                // Values can be a plain string ("ok") or an object {"status":"ok","depth":0}
                let status = value
                    .as_str()
                    .or_else(|| value.get("status").and_then(|v| v.as_str()))
                    .unwrap_or("unknown");
                let mut line = format!("  {name}: {status}");
                if let Some(depth) = value.get("depth").and_then(|v| v.as_u64()) {
                    line.push_str(&format!(" (depth: {depth})"));
                }
                if status == "ok" {
                    println!("{}", line.green());
                } else {
                    println!("{}", line.red());
                }
            }
        }
    }

    Ok(())
}
