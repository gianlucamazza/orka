use crate::client::OrkaClient;
use colored::Colorize;

pub async fn list(client: &OrkaClient) -> crate::client::Result<()> {
    let resp = client.get("/api/v1/schedules").await?;
    if resp.status() == reqwest::StatusCode::SERVICE_UNAVAILABLE {
        println!("{}", "Scheduler is not enabled on this server.".yellow());
        return Ok(());
    }
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Server returned {status}: {body}").into());
    }
    let body: serde_json::Value = resp.json().await?;
    let schedules = body.as_array().map(Vec::as_slice).unwrap_or(&[]);
    if schedules.is_empty() {
        println!("{}", "No schedules found.".green());
        return Ok(());
    }
    println!("{}", format!("{} schedule(s):", schedules.len()).cyan());
    for s in schedules {
        let id = s["id"].as_str().unwrap_or("?");
        let name = s["name"].as_str().unwrap_or("?");
        let cron = s["cron"].as_str().unwrap_or("-");
        let skill = s["skill"].as_str().unwrap_or("-");
        let next_run = s["next_run"].as_i64().unwrap_or(0);
        let completed = s["completed"].as_bool().unwrap_or(false);
        let next_dt = next_run.to_string();
        let status = if completed {
            "completed".yellow().to_string()
        } else {
            "active".green().to_string()
        };
        println!(
            "  {} {} cron={} skill={} next={} [{}]",
            id.cyan(),
            name.bold(),
            cron,
            skill,
            next_dt,
            status
        );
    }
    Ok(())
}

pub async fn create(
    client: &OrkaClient,
    name: &str,
    cron: Option<&str>,
    run_at: Option<&str>,
    skill: Option<&str>,
    args: Option<&str>,
    message: Option<&str>,
) -> crate::client::Result<()> {
    let mut body = serde_json::json!({ "name": name });
    if let Some(c) = cron {
        body["cron"] = serde_json::json!(c);
    }
    if let Some(r) = run_at {
        body["run_at"] = serde_json::json!(r);
    }
    if let Some(s) = skill {
        body["skill"] = serde_json::json!(s);
    }
    if let Some(a) = args {
        let parsed: serde_json::Value =
            serde_json::from_str(a).map_err(|e| format!("invalid args JSON: {e}"))?;
        body["args"] = parsed;
    }
    if let Some(m) = message {
        body["message"] = serde_json::json!(m);
    }

    let resp = client.post("/api/v1/schedules", Some(body)).await?;
    if resp.status() == reqwest::StatusCode::SERVICE_UNAVAILABLE {
        println!("{}", "Scheduler is not enabled on this server.".yellow());
        return Ok(());
    }
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Server returned {status}: {body}").into());
    }
    let schedule: serde_json::Value = resp.json().await?;
    println!(
        "{} id={}",
        "Schedule created.".green(),
        schedule["id"].as_str().unwrap_or("?").cyan()
    );
    Ok(())
}

pub async fn delete(client: &OrkaClient, id: &str) -> crate::client::Result<()> {
    let resp = client.delete(&format!("/api/v1/schedules/{id}")).await?;
    if resp.status() == reqwest::StatusCode::SERVICE_UNAVAILABLE {
        println!("{}", "Scheduler is not enabled on this server.".yellow());
        return Ok(());
    }
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Server returned {status}: {body}").into());
    }
    let result: serde_json::Value = resp.json().await?;
    if result["deleted"].as_bool().unwrap_or(false) {
        println!("{}", format!("Schedule '{id}' deleted.").green());
    } else {
        println!("{}", format!("Schedule '{id}' not found.").yellow());
    }
    Ok(())
}
