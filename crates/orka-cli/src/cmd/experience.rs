use crate::client::OrkaClient;
use colored::Colorize;

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
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Server returned {status}: {body}").into());
    }
    let body: serde_json::Value = resp.json().await?;
    let principles = body.as_array().map(Vec::as_slice).unwrap_or(&[]);
    if principles.is_empty() {
        println!("{}", "No principles found.".yellow());
        return Ok(());
    }
    println!("{}", format!("{} principle(s):", principles.len()).cyan());
    for (i, p) in principles.iter().enumerate() {
        let kind = p["kind"].as_str().unwrap_or("do");
        let text = p["text"].as_str().unwrap_or("?");
        let scope = p["scope"].as_str().unwrap_or("?");
        let reinforcements = p["reinforcement_count"].as_u64().unwrap_or(0);
        let prefix = if kind == "avoid" {
            "AVOID".red().to_string()
        } else {
            "DO".green().to_string()
        };
        println!(
            "  {}. [{}] {} (scope={}, reinforced={})",
            i + 1,
            prefix,
            text,
            scope.cyan(),
            reinforcements
        );
    }
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
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Server returned {status}: {body}").into());
    }
    let result: serde_json::Value = resp.json().await?;
    let created = result["created"].as_u64().unwrap_or(0);
    println!(
        "{} {} new principle(s) created.",
        "Distillation complete.".green(),
        created
    );
    Ok(())
}
