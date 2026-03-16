use colored::Colorize;

use crate::client::{OrkaClient, Result};

pub async fn run(client: &OrkaClient) -> Result<()> {
    match client.get_json("/health").await {
        Ok(body) => {
            let status = body
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            if status == "ok" {
                println!("{}", "Server is healthy".green());
            } else {
                println!("{}: {}", "Server is unhealthy".red(), status);
            }
        }
        Err(e) => {
            println!("{}: {e}", "Cannot reach server".red());
        }
    }
    Ok(())
}
