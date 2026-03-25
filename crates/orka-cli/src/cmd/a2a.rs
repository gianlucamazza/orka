use colored::Colorize;
use futures_util::StreamExt;
use uuid::Uuid;

use crate::client::OrkaClient;

/// Print the agent card from `GET /.well-known/agent.json`.
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

/// Send a message via `POST /a2a` using the A2A v1.0 `message/send` method.
pub async fn send(client: &OrkaClient, task: &str) -> crate::client::Result<()> {
    // Accept raw JSON or treat the argument as plain text.
    let message = if let Ok(v) = serde_json::from_str::<serde_json::Value>(task) {
        // If the caller already provided a full message object, use it as-is.
        if v.get("parts").is_some() || v.get("kind").is_some() {
            v
        } else {
            // Treat it as params with a message field.
            v
        }
    } else {
        // Plain text — wrap in a v1.0 Message object.
        serde_json::json!({
            "kind": "message",
            "role": "ROLE_USER",
            "parts": [{"kind": "text", "text": task}],
            "messageId": Uuid::now_v7().to_string()
        })
    };

    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "message/send",
        "params": {
            "message": message
        },
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
        let code = error["code"].as_i64().unwrap_or(0);
        println!("{} [{}] {}", "A2A error:".red(), code, msg);
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&result["result"]).unwrap_or_default()
        );
    }
    Ok(())
}

/// Stream a task via `POST /a2a` using `message/stream` (SSE output).
pub async fn stream(client: &OrkaClient, task: &str) -> crate::client::Result<()> {
    let message = serde_json::json!({
        "kind": "message",
        "role": "ROLE_USER",
        "parts": [{"kind": "text", "text": task}],
        "messageId": Uuid::now_v7().to_string()
    });
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "message/stream",
        "params": { "message": message },
        "id": 1,
    });

    let resp = client.post("/a2a", Some(body)).await?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        println!("{}", "A2A is not enabled on this server.".yellow());
        return Ok(());
    }
    let resp = OrkaClient::ensure_ok(resp).await?;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let bytes = chunk.map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
        if let Ok(text) = std::str::from_utf8(&bytes) {
            for line in text.lines() {
                if let Some(data) = line.strip_prefix("data: ")
                    && let Ok(v) = serde_json::from_str::<serde_json::Value>(data)
                {
                    // Print the text parts of each event as they arrive.
                    if let Some(parts) = v["result"]["artifact"]["parts"].as_array() {
                        for part in parts {
                            if let Some(text) = part["text"].as_str() {
                                print!("{text}");
                            }
                        }
                    } else if let Some(status) = v["result"]["status"]["state"].as_str()
                        && matches!(status, "completed" | "failed" | "canceled")
                    {
                        println!("\n{}", format!("[{status}]").dimmed());
                    }
                }
            }
        }
    }
    Ok(())
}

/// Get a task by ID via `tasks/get`.
pub async fn tasks_get(client: &OrkaClient, task_id: &str) -> crate::client::Result<()> {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "tasks/get",
        "params": { "taskId": task_id },
        "id": 1,
    });
    rpc_print(client, body).await
}

/// List tasks with optional state filter via `tasks/list`.
pub async fn tasks_list(client: &OrkaClient, state: Option<&str>) -> crate::client::Result<()> {
    let filter = match state {
        Some(s) => serde_json::json!({ "states": [s] }),
        None => serde_json::json!({}),
    };
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "tasks/list",
        "params": { "filter": filter },
        "id": 1,
    });
    rpc_print(client, body).await
}

/// Cancel a task by ID via `tasks/cancel`.
pub async fn tasks_cancel(client: &OrkaClient, task_id: &str) -> crate::client::Result<()> {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "tasks/cancel",
        "params": { "taskId": task_id },
        "id": 1,
    });
    rpc_print(client, body).await
}

/// Helper: POST to /a2a, print result or error.
async fn rpc_print(client: &OrkaClient, body: serde_json::Value) -> crate::client::Result<()> {
    let resp = client.post("/a2a", Some(body)).await?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        println!("{}", "A2A is not enabled on this server.".yellow());
        return Ok(());
    }
    let resp = OrkaClient::ensure_ok(resp).await?;
    let result: serde_json::Value = resp.json().await?;
    if let Some(error) = result["error"].as_object() {
        let msg = error["message"].as_str().unwrap_or("unknown error");
        let code = error["code"].as_i64().unwrap_or(0);
        println!("{} [{}] {}", "A2A error:".red(), code, msg);
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&result["result"]).unwrap_or_default()
        );
    }
    Ok(())
}
