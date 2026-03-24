use colored::Colorize;

use crate::{client::OrkaClient, table::make_table};

pub async fn list(client: &OrkaClient, limit: usize) -> crate::client::Result<()> {
    let body = client
        .get_json(&format!("/api/v1/sessions?limit={limit}"))
        .await?;
    let sessions = body.as_array().map(Vec::as_slice).unwrap_or(&[]);
    if sessions.is_empty() {
        println!("{}", "No active sessions.".green());
        return Ok(());
    }
    let mut table = make_table(&["ID", "Channel", "User", "Updated"]);
    for s in sessions {
        table.add_row([
            s["id"].as_str().unwrap_or("?"),
            s["channel"].as_str().unwrap_or("?"),
            s["user_id"].as_str().unwrap_or("?"),
            s["updated_at"].as_str().unwrap_or("?"),
        ]);
    }
    println!("{table}");
    Ok(())
}

pub async fn show(client: &OrkaClient, id: &str) -> crate::client::Result<()> {
    let resp = client.get(&format!("/api/v1/sessions/{id}")).await?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(format!("session '{id}' not found").into());
    }
    let resp = OrkaClient::ensure_ok(resp).await?;
    let session: serde_json::Value = resp.json().await?;
    println!(
        "{} {}",
        "Session:".cyan(),
        session["id"].as_str().unwrap_or("?").bold()
    );
    println!("Channel: {}", session["channel"].as_str().unwrap_or("?"));
    println!("User:    {}", session["user_id"].as_str().unwrap_or("?"));
    println!("Created: {}", session["created_at"].as_str().unwrap_or("?"));
    println!("Updated: {}", session["updated_at"].as_str().unwrap_or("?"));
    if let Some(state) = session["state"].as_object()
        && !state.is_empty()
    {
        println!("\n{}", "State:".cyan());
        println!(
            "{}",
            serde_json::to_string_pretty(&session["state"]).unwrap_or_default()
        );
    }
    Ok(())
}

pub async fn delete(client: &OrkaClient, id: &str, yes: bool) -> crate::client::Result<()> {
    if !yes
        && !dialoguer::Confirm::new()
            .with_prompt(format!("Delete session '{id}'?"))
            .default(false)
            .interact()?
    {
        println!("Aborted.");
        return Ok(());
    }
    let resp = client.delete(&format!("/api/v1/sessions/{id}")).await?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        println!("{}", format!("Session '{id}' not found.").yellow());
        return Ok(());
    }
    OrkaClient::ensure_ok(resp).await?;
    println!("{}", format!("Session '{id}' deleted.").green());
    Ok(())
}
