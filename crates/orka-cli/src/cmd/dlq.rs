use crate::client::OrkaClient;
use crate::table::make_table;
use colored::Colorize;

pub async fn list(client: &OrkaClient) -> crate::client::Result<()> {
    let resp = client.get("/api/v1/dlq").await?;
    let resp = OrkaClient::ensure_ok(resp).await?;
    let body: serde_json::Value = resp.json().await?;

    if let Some(messages) = body.as_array() {
        if messages.is_empty() {
            println!("{}", "DLQ is empty".green());
            return Ok(());
        }
        let mut table = make_table(&["ID", "Channel", "Retries", "Timestamp"]);
        for msg in messages {
            let retry_count = msg["metadata"]["retry_count"]
                .as_u64()
                .unwrap_or(0)
                .to_string();
            table.add_row([
                msg["id"].as_str().unwrap_or("?"),
                msg["channel"].as_str().unwrap_or("?"),
                &retry_count,
                msg["timestamp"].as_str().unwrap_or("?"),
            ]);
        }
        println!("{table}");
    } else {
        println!("{body}");
    }
    Ok(())
}

pub async fn replay(client: &OrkaClient, id: &str) -> crate::client::Result<()> {
    let resp = client
        .post(&format!("/api/v1/dlq/{id}/replay"), None)
        .await?;
    if resp.status().is_success() {
        println!("{}", format!("Message {id} replayed successfully").green());
    } else {
        let body = resp.text().await?;
        println!("{}", format!("Failed to replay: {body}").red());
    }
    Ok(())
}

pub async fn purge(client: &OrkaClient, yes: bool) -> crate::client::Result<()> {
    if !yes
        && !dialoguer::Confirm::new()
            .with_prompt("Purge all DLQ messages?")
            .default(false)
            .interact()?
    {
        println!("Aborted.");
        return Ok(());
    }
    let resp = client.delete("/api/v1/dlq").await?;
    if resp.status().is_success() {
        println!("{}", "DLQ purged successfully".green());
    } else {
        let body = resp.text().await?;
        println!("{}", format!("Failed to purge: {body}").red());
    }
    Ok(())
}
