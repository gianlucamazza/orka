use crate::client::OrkaClient;
use colored::Colorize;

pub async fn list(client: &OrkaClient) -> crate::client::Result<()> {
    let body = client.get_json("/api/v1/workspaces").await?;
    let workspaces = body.as_array().map(Vec::as_slice).unwrap_or(&[]);
    if workspaces.is_empty() {
        println!("{}", "No workspaces found.".yellow());
        return Ok(());
    }
    println!("{}", format!("{} workspace(s):", workspaces.len()).cyan());
    for ws in workspaces {
        let name = ws["name"].as_str().unwrap_or("?");
        let agent_name = ws["agent_name"].as_str().unwrap_or("-");
        let desc = ws["description"].as_str().unwrap_or("");
        let has_tools = ws["has_tools"].as_bool().unwrap_or(false);
        println!(
            "  {} (agent: {}){}  {}",
            name.green().bold(),
            agent_name.cyan(),
            if has_tools { " [tools]" } else { "" },
            desc
        );
    }
    Ok(())
}

pub async fn show(client: &OrkaClient, name: &str) -> crate::client::Result<()> {
    let resp = client.get(&format!("/api/v1/workspaces/{name}")).await?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(format!("workspace '{name}' not found").into());
    }
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Server returned {status}: {body}").into());
    }
    let ws: serde_json::Value = resp.json().await?;
    println!("{}", ws["name"].as_str().unwrap_or("?").green().bold());
    if let Some(agent) = ws["agent_name"].as_str() {
        println!("Agent: {}", agent.cyan());
    }
    if let Some(desc) = ws["description"].as_str() {
        println!("Description: {desc}");
    }
    if let Some(ver) = ws["version"].as_str() {
        println!("Version: {ver}");
    }
    if let Some(soul) = ws["soul_body"].as_str()
        && !soul.is_empty()
    {
        println!("\n{}", "SOUL:".cyan());
        println!("{soul}");
    }
    if let Some(tools) = ws["tools_body"].as_str()
        && !tools.is_empty()
    {
        println!("\n{}", "TOOLS:".cyan());
        println!("{tools}");
    }
    Ok(())
}
