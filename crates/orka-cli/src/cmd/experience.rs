use colored::Colorize;

use crate::{client::OrkaClient, table::make_table};

pub async fn status(client: &OrkaClient) -> crate::client::Result<()> {
    let body = client.get_json("/api/v1/experience/status").await?;
    let enabled = body["enabled"].as_bool().unwrap_or(false);
    if enabled {
        println!("{}", "Experience system: enabled".green());
    } else {
        println!("{}", "Experience system: disabled".yellow());
    }
    Ok(())
}

pub async fn principles(
    client: &OrkaClient,
    workspace: &str,
    query: &str,
    limit: usize,
) -> crate::client::Result<()> {
    let path =
        format!("/api/v1/experience/principles?workspace={workspace}&query={query}&limit={limit}");
    let resp = client.get(&path).await?;
    if resp.status() == reqwest::StatusCode::SERVICE_UNAVAILABLE {
        println!(
            "{}",
            "Experience system is not enabled on this server.".yellow()
        );
        return Ok(());
    }
    let resp = OrkaClient::ensure_ok(resp).await?;
    let body: serde_json::Value = resp.json().await?;
    let principles = body.as_array().map_or(&[] as &[_], Vec::as_slice);
    if principles.is_empty() {
        println!("{}", "No principles found.".yellow());
        return Ok(());
    }
    let mut table = make_table(&["#", "Kind", "Text", "Scope", "Reinforced"]);
    for (i, p) in principles.iter().enumerate() {
        let kind = p["kind"].as_str().unwrap_or("do");
        let reinforcements = p["reinforcement_count"].as_u64().unwrap_or(0).to_string();
        let idx = (i + 1).to_string();
        table.add_row([
            idx.as_str(),
            kind,
            p["text"].as_str().unwrap_or("?"),
            p["scope"].as_str().unwrap_or("?"),
            &reinforcements,
        ]);
    }
    println!("{table}");
    Ok(())
}

pub async fn distill(client: &OrkaClient, workspace: &str) -> crate::client::Result<()> {
    let body = serde_json::json!({ "workspace": workspace });
    let resp = client
        .post("/api/v1/experience/distill", Some(body))
        .await?;
    if resp.status() == reqwest::StatusCode::SERVICE_UNAVAILABLE {
        println!(
            "{}",
            "Experience system is not enabled on this server.".yellow()
        );
        return Ok(());
    }
    let resp = OrkaClient::ensure_ok(resp).await?;
    let result: serde_json::Value = resp.json().await?;
    let created = result["created"].as_u64().unwrap_or(0);
    println!(
        "{} {} new principle(s) created.",
        "Distillation complete.".green(),
        created
    );
    Ok(())
}
