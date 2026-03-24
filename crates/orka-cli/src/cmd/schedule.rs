use colored::Colorize;

use crate::{client::OrkaClient, table::make_table};

pub async fn list(client: &OrkaClient) -> crate::client::Result<()> {
    let resp = client.get("/api/v1/schedules").await?;
    if resp.status() == reqwest::StatusCode::SERVICE_UNAVAILABLE {
        println!("{}", "Scheduler is not enabled on this server.".yellow());
        return Ok(());
    }
    let resp = OrkaClient::ensure_ok(resp).await?;
    let body: serde_json::Value = resp.json().await?;
    let schedules = body.as_array().map(Vec::as_slice).unwrap_or(&[]);
    if schedules.is_empty() {
        println!("{}", "No schedules found.".green());
        return Ok(());
    }
    let mut table = make_table(&["ID", "Name", "Cron", "Skill", "Next Run", "Status"]);
    for s in schedules {
        let next_run = s["next_run"].as_i64().unwrap_or(0).to_string();
        let status = if s["completed"].as_bool().unwrap_or(false) {
            "completed"
        } else {
            "active"
        };
        table.add_row([
            s["id"].as_str().unwrap_or("?"),
            s["name"].as_str().unwrap_or("?"),
            s["cron"].as_str().unwrap_or("-"),
            s["skill"].as_str().unwrap_or("-"),
            &next_run,
            status,
        ]);
    }
    println!("{table}");
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
    let resp = OrkaClient::ensure_ok(resp).await?;
    let schedule: serde_json::Value = resp.json().await?;
    println!(
        "{} id={}",
        "Schedule created.".green(),
        schedule["id"].as_str().unwrap_or("?").cyan()
    );
    Ok(())
}

pub async fn delete(client: &OrkaClient, id: &str, yes: bool) -> crate::client::Result<()> {
    if !yes
        && !dialoguer::Confirm::new()
            .with_prompt(format!("Delete schedule '{id}'?"))
            .default(false)
            .interact()?
    {
        println!("Aborted.");
        return Ok(());
    }
    let resp = client.delete(&format!("/api/v1/schedules/{id}")).await?;
    if resp.status() == reqwest::StatusCode::SERVICE_UNAVAILABLE {
        println!("{}", "Scheduler is not enabled on this server.".yellow());
        return Ok(());
    }
    let resp = OrkaClient::ensure_ok(resp).await?;
    let result: serde_json::Value = resp.json().await?;
    if result["deleted"].as_bool().unwrap_or(false) {
        println!("{}", format!("Schedule '{id}' deleted.").green());
    } else {
        println!("{}", format!("Schedule '{id}' not found.").yellow());
    }
    Ok(())
}
