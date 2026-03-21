use crate::client::OrkaClient;
use crate::table::make_table;
use colored::Colorize;

pub async fn list(client: &OrkaClient) -> crate::client::Result<()> {
    let body = client.get_json("/api/v1/workspaces").await?;
    let workspaces = body.as_array().map(Vec::as_slice).unwrap_or(&[]);
    if workspaces.is_empty() {
        println!("{}", "No workspaces found.".yellow());
        return Ok(());
    }
    let mut table = make_table(&["Name", "Agent", "Tools", "Description"]);
    for ws in workspaces {
        table.add_row([
            ws["name"].as_str().unwrap_or("?"),
            ws["agent_name"].as_str().unwrap_or("-"),
            if ws["has_tools"].as_bool().unwrap_or(false) {
                "yes"
            } else {
                "no"
            },
            ws["description"].as_str().unwrap_or(""),
        ]);
    }
    println!("{table}");
    Ok(())
}

pub async fn show(client: &OrkaClient, name: &str) -> crate::client::Result<()> {
    let resp = client.get(&format!("/api/v1/workspaces/{name}")).await?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(format!("workspace '{name}' not found").into());
    }
    let resp = OrkaClient::ensure_ok(resp).await?;
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
