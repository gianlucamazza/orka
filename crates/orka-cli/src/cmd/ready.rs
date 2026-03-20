use colored::Colorize;

use crate::client::{OrkaClient, Result};

pub async fn run(client: &OrkaClient) -> Result<()> {
    let body = client
        .get_json("/health/ready")
        .await
        .map_err(|e| format!("{}: {e}", "Not ready".red()))?;

    let ready = body.get("ready").and_then(|v| v.as_bool()).unwrap_or(false);

    if ready {
        println!("{} {}", "Status:".bold(), "ready".green());
    } else {
        println!("{} {}", "Status:".bold(), "not ready".red());
    }

    if let Some(checks) = body.get("checks").and_then(|v| v.as_object()) {
        println!("{}:", "Checks".bold());
        for (name, value) in checks {
            let status = value
                .get("status")
                .and_then(|v| v.as_str())
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

    if !ready {
        return Err("server is not ready".into());
    }

    Ok(())
}
