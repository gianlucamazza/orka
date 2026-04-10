use colored::Colorize;
use serde_json::json;

use crate::{client::OrkaClient, table::make_table};

pub async fn list(client: &OrkaClient) -> crate::client::Result<()> {
    let body = client.get_json("/api/v1/workspaces").await?;
    let workspaces = body.as_array().map_or(&[][..], Vec::as_slice);
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

pub async fn create(
    client: &OrkaClient,
    name: &str,
    agent_name: Option<String>,
    description: Option<String>,
    version: Option<String>,
) -> crate::client::Result<()> {
    let mut body = json!({ "name": name });
    if let Some(v) = agent_name {
        body["agent_name"] = json!(v);
    }
    if let Some(v) = description {
        body["description"] = json!(v);
    }
    if let Some(v) = version {
        body["version"] = json!(v);
    }
    let resp = client.post("/api/v1/workspaces", Some(body)).await?;
    if resp.status() == reqwest::StatusCode::CONFLICT {
        return Err(format!("workspace '{name}' already exists").into());
    }
    if resp.status() == reqwest::StatusCode::BAD_REQUEST {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("invalid request: {text}").into());
    }
    let ws: serde_json::Value = OrkaClient::ensure_ok(resp).await?.json().await?;
    println!(
        "Workspace {} created.",
        ws["name"].as_str().unwrap_or(name).green().bold()
    );
    Ok(())
}

pub async fn update(
    client: &OrkaClient,
    name: &str,
    agent_name: Option<String>,
    description: Option<String>,
    version: Option<String>,
) -> crate::client::Result<()> {
    let mut body = json!({});
    if let Some(v) = agent_name {
        body["agent_name"] = json!(v);
    }
    if let Some(v) = description {
        body["description"] = json!(v);
    }
    if let Some(v) = version {
        body["version"] = json!(v);
    }
    let resp = client
        .patch_json(&format!("/api/v1/workspaces/{name}"), &body)
        .await;
    match resp {
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("404") {
                return Err(format!("workspace '{name}' not found").into());
            }
            if msg.contains("400") {
                return Err("at least one field must be provided".into());
            }
            return Err(e);
        }
        Ok(ws) => {
            println!(
                "Workspace {} updated.",
                ws["name"].as_str().unwrap_or(name).green().bold()
            );
        }
    }
    Ok(())
}

pub async fn delete(client: &OrkaClient, name: &str, yes: bool) -> crate::client::Result<()> {
    if !yes {
        use std::io::{BufRead, Write};
        print!("Are you sure you want to delete workspace '{name}'? [y/N] ");
        std::io::stdout().flush().ok();
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line).ok();
        if !matches!(line.trim(), "y" | "Y") {
            println!("Aborted.");
            return Ok(());
        }
    }
    let resp = client.delete(&format!("/api/v1/workspaces/{name}")).await?;
    match resp.status() {
        reqwest::StatusCode::NO_CONTENT => {
            println!("Workspace '{}' deleted.", name.green().bold());
        }
        reqwest::StatusCode::BAD_REQUEST => {
            return Err(format!("cannot delete workspace '{name}': it may be the default").into());
        }
        reqwest::StatusCode::NOT_FOUND => {
            return Err(format!("workspace '{name}' not found").into());
        }
        _ => {
            OrkaClient::ensure_ok(resp).await?;
        }
    }
    Ok(())
}
