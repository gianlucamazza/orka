use crate::client::OrkaClient;
use colored::Colorize;

pub async fn list(client: &OrkaClient, limit: usize) -> crate::client::Result<()> {
    let body = client
        .get_json(&format!("/api/v1/sessions?limit={limit}"))
        .await?;
    let sessions = body.as_array().map(Vec::as_slice).unwrap_or(&[]);
    if sessions.is_empty() {
        println!("{}", "No active sessions.".green());
        return Ok(());
    }
    println!("{}", format!("{} session(s):", sessions.len()).cyan());
    for s in sessions {
        let id = s["id"].as_str().unwrap_or("?");
        let channel = s["channel"].as_str().unwrap_or("?");
        let user_id = s["user_id"].as_str().unwrap_or("?");
        let updated_at = s["updated_at"].as_str().unwrap_or("?");
        println!(
            "  {} channel={} user={} updated={}",
            id.cyan(),
            channel,
            user_id,
            updated_at
        );
    }
    Ok(())
}

pub async fn show(client: &OrkaClient, id: &str) -> crate::client::Result<()> {
    let resp = client.get(&format!("/api/v1/sessions/{id}")).await?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(format!("session '{id}' not found").into());
    }
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Server returned {status}: {body}").into());
    }
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

pub async fn delete(client: &OrkaClient, id: &str) -> crate::client::Result<()> {
    let resp = client.delete(&format!("/api/v1/sessions/{id}")).await?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        println!("{}", format!("Session '{id}' not found.").yellow());
        return Ok(());
    }
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Server returned {status}: {body}").into());
    }
    println!("{}", format!("Session '{id}' deleted.").green());
    Ok(())
}
