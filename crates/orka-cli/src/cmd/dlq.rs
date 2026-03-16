use crate::client::OrkaClient;
use colored::Colorize;

pub async fn list(client: &OrkaClient) -> crate::client::Result<()> {
    let resp = client.get("/api/v1/dlq").await?;
    let body: serde_json::Value = resp.json().await?;

    if let Some(messages) = body.as_array() {
        if messages.is_empty() {
            println!("{}", "DLQ is empty".green());
            return Ok(());
        }
        println!("{}", format!("{} messages in DLQ:", messages.len()).yellow());
        for msg in messages {
            let id = msg["id"].as_str().unwrap_or("?");
            let channel = msg["channel"].as_str().unwrap_or("?");
            let timestamp = msg["timestamp"].as_str().unwrap_or("?");
            let retry_count = msg["metadata"]["retry_count"].as_u64().unwrap_or(0);
            println!(
                "  {} channel={} retries={} ts={}",
                id.cyan(),
                channel,
                retry_count,
                timestamp
            );
        }
    } else {
        println!("{}", body);
    }
    Ok(())
}

pub async fn replay(client: &OrkaClient, id: &str) -> crate::client::Result<()> {
    let resp = client.post(&format!("/api/v1/dlq/{id}/replay"), None).await?;
    if resp.status().is_success() {
        println!("{}", format!("Message {id} replayed successfully").green());
    } else {
        let body = resp.text().await?;
        println!("{}", format!("Failed to replay: {body}").red());
    }
    Ok(())
}

pub async fn purge(client: &OrkaClient) -> crate::client::Result<()> {
    let resp = client.delete("/api/v1/dlq").await?;
    if resp.status().is_success() {
        println!("{}", "DLQ purged successfully".green());
    } else {
        let body = resp.text().await?;
        println!("{}", format!("Failed to purge: {body}").red());
    }
    Ok(())
}
