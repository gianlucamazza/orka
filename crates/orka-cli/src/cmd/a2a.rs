use crate::client::OrkaClient;
use colored::Colorize;

pub async fn card(client: &OrkaClient) -> crate::client::Result<()> {
    let resp = client.get("/.well-known/agent.json").await?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        println!("{}", "A2A is not enabled on this server.".yellow());
        return Ok(());
    }
    let resp = OrkaClient::ensure_ok(resp).await?;
    let card: serde_json::Value = resp.json().await?;
    println!(
        "{}",
        serde_json::to_string_pretty(&card).unwrap_or_default()
    );
    Ok(())
}

pub async fn send(client: &OrkaClient, task: &str) -> crate::client::Result<()> {
    let task_value: serde_json::Value = serde_json::from_str(task)
        .unwrap_or_else(|_| serde_json::json!({ "message": { "parts": [{ "text": task }] } }));

    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "tasks/send",
        "params": task_value,
        "id": 1,
    });

    let resp = client.post("/a2a", Some(body)).await?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        println!("{}", "A2A is not enabled on this server.".yellow());
        return Ok(());
    }
    let resp = OrkaClient::ensure_ok(resp).await?;
    let result: serde_json::Value = resp.json().await?;
    if let Some(error) = result["error"].as_object() {
        let msg = error["message"].as_str().unwrap_or("unknown error");
        println!("{} {}", "A2A error:".red(), msg);
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&result["result"]).unwrap_or_default()
        );
    }
    Ok(())
}
