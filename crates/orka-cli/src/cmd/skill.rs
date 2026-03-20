use crate::client::OrkaClient;
use colored::Colorize;

pub async fn list(client: &OrkaClient) -> crate::client::Result<()> {
    let body = client.get_json("/api/v1/skills").await?;
    let skills = body.as_array().map(Vec::as_slice).unwrap_or(&[]);
    if skills.is_empty() {
        println!("{}", "No skills registered.".yellow());
        return Ok(());
    }
    println!("{}", format!("{} skills:", skills.len()).cyan());
    for skill in skills {
        let name = skill["name"].as_str().unwrap_or("?");
        let desc = skill["description"].as_str().unwrap_or("");
        println!("  {}  {}", name.green(), desc);
    }
    Ok(())
}

pub async fn describe(client: &OrkaClient, name: &str) -> crate::client::Result<()> {
    let resp = client.get(&format!("/api/v1/skills/{name}")).await?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(format!("skill '{name}' not found").into());
    }
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Server returned {status}: {body}").into());
    }
    let skill: serde_json::Value = resp.json().await?;
    println!("{}", skill["name"].as_str().unwrap_or("?").green().bold());
    println!("{}", skill["description"].as_str().unwrap_or(""));
    println!("\n{}", "Schema:".cyan());
    let schema = serde_json::to_string_pretty(&skill["schema"]).unwrap_or_default();
    println!("{schema}");
    Ok(())
}
