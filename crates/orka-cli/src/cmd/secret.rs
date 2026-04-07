use std::io::{BufRead as _, IsTerminal as _};

use crate::client::{OrkaClient, Result};

/// Read the secret value.
/// - Interactive TTY: uses a masked password prompt via dialoguer.
/// - Pipe: reads the first line from stdin (e.g. `echo -n "$SECRET" | orka
///   secret set path`).
fn read_value() -> Result<String> {
    if std::io::stdin().is_terminal() {
        let value = dialoguer::Password::new()
            .with_prompt("Secret value")
            .interact()?;
        if value.is_empty() {
            return Err("secret value must not be empty".into());
        }
        return Ok(value);
    }
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    while line.ends_with('\n') || line.ends_with('\r') {
        line.pop();
    }
    if line.is_empty() {
        return Err("secret value must not be empty".into());
    }
    Ok(line)
}

pub async fn set(client: &OrkaClient, path: &str) -> Result<()> {
    let value = read_value()?;
    let url = format!("/api/v1/secrets/{path}");
    let resp = client
        .post(&url, Some(serde_json::json!({ "value": value })))
        .await?;
    OrkaClient::ensure_ok(resp).await?;
    println!("secret '{path}' set");
    Ok(())
}

pub async fn get(client: &OrkaClient, path: &str, reveal: bool) -> Result<()> {
    let url = if reveal {
        format!("/api/v1/secrets/{path}?reveal=true")
    } else {
        format!("/api/v1/secrets/{path}")
    };
    let body = client.get_json(&url).await?;
    println!("{}", body["value"].as_str().unwrap_or("<binary>"));
    Ok(())
}

pub async fn list(client: &OrkaClient) -> Result<()> {
    let body = client.get_json("/api/v1/secrets").await?;
    let keys = body.as_array().ok_or("expected JSON array")?;
    if keys.is_empty() {
        println!("no secrets found");
    } else {
        for key in keys {
            println!("{}", key.as_str().unwrap_or(""));
        }
    }
    Ok(())
}

pub async fn delete(client: &OrkaClient, path: &str) -> Result<()> {
    let url = format!("/api/v1/secrets/{path}");
    let resp = client.delete(&url).await?;
    OrkaClient::ensure_ok(resp).await?;
    println!("secret '{path}' deleted");
    Ok(())
}
