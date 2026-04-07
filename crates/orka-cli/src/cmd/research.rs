use colored::Colorize;

use crate::{client::OrkaClient, table::make_table};

const NOT_ENABLED_MSG: &str = "Research is not enabled on this server.";

fn check_503(resp: reqwest::Response) -> Option<reqwest::Response> {
    if resp.status() == reqwest::StatusCode::SERVICE_UNAVAILABLE {
        println!("{}", NOT_ENABLED_MSG.yellow());
        None
    } else {
        Some(resp)
    }
}

pub async fn campaign_list(client: &OrkaClient) -> crate::client::Result<()> {
    let resp = client.get("/api/v1/research/campaigns").await?;
    let Some(resp) = check_503(resp) else {
        return Ok(());
    };
    let body: serde_json::Value = OrkaClient::ensure_ok(resp).await?.json().await?;
    let campaigns = body.as_array().map_or(&[][..], Vec::as_slice);
    if campaigns.is_empty() {
        println!("{}", "No research campaigns found.".yellow());
        return Ok(());
    }
    let mut table = make_table(&["ID", "Name", "Workspace", "Active", "Best Candidate"]);
    for campaign in campaigns {
        table.add_row([
            campaign["id"].as_str().unwrap_or("?"),
            campaign["name"].as_str().unwrap_or("?"),
            campaign["workspace"].as_str().unwrap_or("?"),
            if campaign["active"].as_bool().unwrap_or(false) {
                "yes"
            } else {
                "no"
            },
            campaign["best_candidate_id"].as_str().unwrap_or("-"),
        ]);
    }
    println!("{table}");
    Ok(())
}

pub async fn campaign_show(client: &OrkaClient, id: &str) -> crate::client::Result<()> {
    let resp = client
        .get(&format!("/api/v1/research/campaigns/{id}"))
        .await?;
    let Some(resp) = check_503(resp) else {
        return Ok(());
    };
    let body: serde_json::Value = OrkaClient::ensure_ok(resp).await?.json().await?;
    println!("{}", serde_json::to_string_pretty(&body)?);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn campaign_create(
    client: &OrkaClient,
    name: &str,
    workspace: &str,
    repo_path: &str,
    baseline_ref: &str,
    task: &str,
    verify: &str,
    context: Option<&str>,
    editable_paths: &[String],
    metric_name: Option<&str>,
    metric_regex: Option<&str>,
    direction: &str,
    baseline_metric: Option<f64>,
    min_improvement: Option<f64>,
    cron: Option<&str>,
    target_branch: &str,
) -> crate::client::Result<()> {
    let metric = metric_name.map(|name| {
        serde_json::json!({
            "name": name,
            "regex": metric_regex,
            "direction": direction,
            "baseline_value": baseline_metric,
            "min_improvement": min_improvement,
        })
    });
    let body = serde_json::json!({
        "name": name,
        "workspace": workspace,
        "repo_path": repo_path,
        "baseline_ref": baseline_ref,
        "task": task,
        "context": context,
        "verification_command": verify,
        "editable_paths": editable_paths,
        "metric": metric,
        "cron": cron,
        "target_branch": target_branch,
    });
    let resp = client
        .post("/api/v1/research/campaigns", Some(body))
        .await?;
    let Some(resp) = check_503(resp) else {
        return Ok(());
    };
    let created: serde_json::Value = OrkaClient::ensure_ok(resp).await?.json().await?;
    println!(
        "{} id={}",
        "Research campaign created.".green(),
        created["id"].as_str().unwrap_or("?").cyan()
    );
    Ok(())
}

pub async fn campaign_delete(
    client: &OrkaClient,
    id: &str,
    yes: bool,
) -> crate::client::Result<()> {
    if !yes {
        let confirmed = dialoguer::Confirm::new()
            .with_prompt(format!("Delete campaign '{id}'?"))
            .default(false)
            .interact()
            .unwrap_or(false);
        if !confirmed {
            println!("{}", "Aborted.".yellow());
            return Ok(());
        }
    }
    let resp = client
        .delete(&format!("/api/v1/research/campaigns/{id}"))
        .await?;
    let Some(resp) = check_503(resp) else {
        return Ok(());
    };
    OrkaClient::ensure_ok(resp).await?;
    println!("{}", format!("Campaign '{id}' deleted.").green());
    Ok(())
}

pub async fn campaign_pause(client: &OrkaClient, id: &str) -> crate::client::Result<()> {
    let resp = client
        .post(
            &format!("/api/v1/research/campaigns/{id}/pause"),
            Some(serde_json::json!({})),
        )
        .await?;
    let Some(resp) = check_503(resp) else {
        return Ok(());
    };
    OrkaClient::ensure_ok(resp).await?;
    println!("{}", format!("Campaign '{id}' paused.").green());
    Ok(())
}

pub async fn campaign_resume(client: &OrkaClient, id: &str) -> crate::client::Result<()> {
    let resp = client
        .post(
            &format!("/api/v1/research/campaigns/{id}/resume"),
            Some(serde_json::json!({})),
        )
        .await?;
    let Some(resp) = check_503(resp) else {
        return Ok(());
    };
    OrkaClient::ensure_ok(resp).await?;
    println!("{}", format!("Campaign '{id}' resumed.").green());
    Ok(())
}

pub async fn campaign_run(client: &OrkaClient, id: &str) -> crate::client::Result<()> {
    let resp = client
        .post(
            &format!("/api/v1/research/campaigns/{id}/runs"),
            Some(serde_json::json!({})),
        )
        .await?;
    let Some(resp) = check_503(resp) else {
        return Ok(());
    };
    let run: serde_json::Value = OrkaClient::ensure_ok(resp).await?.json().await?;
    println!(
        "{} run_id={}",
        "Research campaign started.".green(),
        run["id"].as_str().unwrap_or("?").cyan()
    );
    Ok(())
}

pub async fn run_list(client: &OrkaClient, campaign_id: Option<&str>) -> crate::client::Result<()> {
    let path = if let Some(campaign_id) = campaign_id {
        format!("/api/v1/research/runs?campaign_id={campaign_id}")
    } else {
        "/api/v1/research/runs".to_string()
    };
    let resp = client.get(&path).await?;
    let Some(resp) = check_503(resp) else {
        return Ok(());
    };
    let body: serde_json::Value = OrkaClient::ensure_ok(resp).await?.json().await?;
    let runs = body.as_array().map_or(&[][..], Vec::as_slice);
    if runs.is_empty() {
        println!("{}", "No research runs found.".yellow());
        return Ok(());
    }
    let mut table = make_table(&["ID", "Campaign", "Status", "Candidate"]);
    for run in runs {
        table.add_row([
            run["id"].as_str().unwrap_or("?"),
            run["campaign_id"].as_str().unwrap_or("?"),
            run["status"].as_str().unwrap_or("?"),
            run["candidate_id"].as_str().unwrap_or("-"),
        ]);
    }
    println!("{table}");
    Ok(())
}

pub async fn run_show(client: &OrkaClient, id: &str) -> crate::client::Result<()> {
    let resp = client.get(&format!("/api/v1/research/runs/{id}")).await?;
    let Some(resp) = check_503(resp) else {
        return Ok(());
    };
    let body: serde_json::Value = OrkaClient::ensure_ok(resp).await?.json().await?;
    println!("{}", serde_json::to_string_pretty(&body)?);
    Ok(())
}

pub async fn candidate_list(
    client: &OrkaClient,
    campaign_id: Option<&str>,
) -> crate::client::Result<()> {
    let path = if let Some(campaign_id) = campaign_id {
        format!("/api/v1/research/candidates?campaign_id={campaign_id}")
    } else {
        "/api/v1/research/candidates".to_string()
    };
    let resp = client.get(&path).await?;
    let Some(resp) = check_503(resp) else {
        return Ok(());
    };
    let body: serde_json::Value = OrkaClient::ensure_ok(resp).await?.json().await?;
    let candidates = body.as_array().map_or(&[][..], Vec::as_slice);
    if candidates.is_empty() {
        println!("{}", "No research candidates found.".yellow());
        return Ok(());
    }
    let mut table = make_table(&["ID", "Campaign", "Status", "Improvement", "Branch"]);
    for candidate in candidates {
        let improvement = candidate["improvement"]
            .as_f64()
            .map_or_else(|| "-".to_string(), |value| format!("{value:.6}"));
        table.add_row([
            candidate["id"].as_str().unwrap_or("?"),
            candidate["campaign_id"].as_str().unwrap_or("?"),
            candidate["status"].as_str().unwrap_or("?"),
            &improvement,
            candidate["branch"].as_str().unwrap_or("?"),
        ]);
    }
    println!("{table}");
    Ok(())
}

pub async fn candidate_show(client: &OrkaClient, id: &str) -> crate::client::Result<()> {
    let resp = client
        .get(&format!("/api/v1/research/candidates/{id}"))
        .await?;
    let Some(resp) = check_503(resp) else {
        return Ok(());
    };
    let body: serde_json::Value = OrkaClient::ensure_ok(resp).await?.json().await?;
    println!("{}", serde_json::to_string_pretty(&body)?);
    Ok(())
}

pub async fn candidate_promote(
    client: &OrkaClient,
    id: &str,
    approve: bool,
) -> crate::client::Result<()> {
    let resp = client
        .post(
            &format!("/api/v1/research/candidates/{id}/promote"),
            Some(serde_json::json!({ "approved": approve })),
        )
        .await?;
    let Some(resp) = check_503(resp) else {
        return Ok(());
    };
    let body: serde_json::Value = OrkaClient::ensure_ok(resp).await?.json().await?;
    if body.get("target_branch").is_some() {
        let request_id = body["id"].as_str().unwrap_or("?");
        println!(
            "{} request_id={}",
            "Promotion request created.".yellow(),
            request_id.cyan()
        );
    } else {
        println!("{}", format!("Candidate '{id}' promoted.").green());
    }
    Ok(())
}

pub async fn promotion_list(
    client: &OrkaClient,
    campaign_id: Option<&str>,
) -> crate::client::Result<()> {
    let path = if let Some(campaign_id) = campaign_id {
        format!("/api/v1/research/promotions?campaign_id={campaign_id}")
    } else {
        "/api/v1/research/promotions".to_string()
    };
    let resp = client.get(&path).await?;
    let Some(resp) = check_503(resp) else {
        return Ok(());
    };
    let body: serde_json::Value = OrkaClient::ensure_ok(resp).await?.json().await?;
    let requests = body.as_array().map_or(&[][..], Vec::as_slice);
    if requests.is_empty() {
        println!("{}", "No promotion requests found.".yellow());
        return Ok(());
    }
    let mut table = make_table(&["ID", "Campaign", "Candidate", "Status", "Target"]);
    for request in requests {
        table.add_row([
            request["id"].as_str().unwrap_or("?"),
            request["campaign_id"].as_str().unwrap_or("?"),
            request["candidate_id"].as_str().unwrap_or("?"),
            request["status"].as_str().unwrap_or("?"),
            request["target_branch"].as_str().unwrap_or("?"),
        ]);
    }
    println!("{table}");
    Ok(())
}

pub async fn promotion_show(client: &OrkaClient, id: &str) -> crate::client::Result<()> {
    let resp = client
        .get(&format!("/api/v1/research/promotions/{id}"))
        .await?;
    let Some(resp) = check_503(resp) else {
        return Ok(());
    };
    let body: serde_json::Value = OrkaClient::ensure_ok(resp).await?.json().await?;
    println!("{}", serde_json::to_string_pretty(&body)?);
    Ok(())
}

pub async fn promotion_approve(client: &OrkaClient, id: &str) -> crate::client::Result<()> {
    let resp = client
        .post(
            &format!("/api/v1/research/promotions/{id}/approve"),
            Some(serde_json::json!({})),
        )
        .await?;
    let Some(resp) = check_503(resp) else {
        return Ok(());
    };
    OrkaClient::ensure_ok(resp).await?;
    println!("{}", format!("Promotion request '{id}' approved.").green());
    Ok(())
}

pub async fn promotion_reject(
    client: &OrkaClient,
    id: &str,
    reason: Option<&str>,
) -> crate::client::Result<()> {
    let resp = client
        .post(
            &format!("/api/v1/research/promotions/{id}/reject"),
            Some(serde_json::json!({ "reason": reason })),
        )
        .await?;
    let Some(resp) = check_503(resp) else {
        return Ok(());
    };
    OrkaClient::ensure_ok(resp).await?;
    println!("{}", format!("Promotion request '{id}' rejected.").green());
    Ok(())
}
