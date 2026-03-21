use crate::client::OrkaClient;
use crate::table::make_table;
use colored::Colorize;
use serde_json::json;

pub async fn list(client: &OrkaClient) -> crate::client::Result<()> {
    let body = client.get_json("/api/v1/skills").await?;
    let skills = body.as_array().map(Vec::as_slice).unwrap_or(&[]);
    if skills.is_empty() {
        println!("{}", "No skills registered.".yellow());
    } else {
        let mut table = make_table(&["Name", "Category", "Status", "Description"]);
        for skill in skills {
            let status_raw = skill["status"].as_str().unwrap_or("ok");
            let status_colored = match status_raw {
                "ok" => status_raw.green().to_string(),
                "degraded" => status_raw.yellow().to_string(),
                "disabled" => status_raw.red().to_string(),
                other => other.to_string(),
            };
            table.add_row([
                skill["name"].as_str().unwrap_or("?"),
                skill["category"].as_str().unwrap_or("general"),
                &status_colored,
                skill["description"].as_str().unwrap_or(""),
            ]);
        }
        println!("{table}");
    }

    // Soft skills section
    let soft_body = client.get_json("/api/v1/soft-skills").await?;
    let soft_skills = soft_body.as_array().map(Vec::as_slice).unwrap_or(&[]);
    if !soft_skills.is_empty() {
        let mut sorted: Vec<&serde_json::Value> = soft_skills.iter().collect();
        sorted.sort_by_key(|s| s["name"].as_str().unwrap_or(""));
        println!("\n{}", "Soft Skills".bold());
        let mut table = make_table(&["Name", "Tags", "Description"]);
        for skill in sorted {
            let tags = skill["tags"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|t| t.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            table.add_row([
                skill["name"].as_str().unwrap_or("?"),
                &tags,
                skill["description"].as_str().unwrap_or(""),
            ]);
        }
        println!("{table}");
    }

    Ok(())
}

pub async fn eval(
    client: &OrkaClient,
    skill: Option<&str>,
    dir: Option<&str>,
    as_json: bool,
) -> crate::client::Result<()> {
    let body = json!({
        "skill": skill,
        "dir": dir,
    });
    let resp = client.post_json("/api/v1/eval", &body).await?;

    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&resp).unwrap_or_default()
        );
        return Ok(());
    }

    // Pretty-print
    let results = resp["results"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let total = results.len();
    let passed = results
        .iter()
        .filter(|r| r["passed"].as_bool().unwrap_or(false))
        .count();
    let failed = total - passed;

    for r in results {
        let skill_name = r["skill"].as_str().unwrap_or("?");
        let scenario = r["scenario"].as_str().unwrap_or("?");
        let ok = r["passed"].as_bool().unwrap_or(false);
        let marker = if ok { "✓".green() } else { "✗".red() };
        println!("{marker} {skill_name} :: {scenario}");
        if !ok && let Some(failures) = r["failures"].as_array() {
            for f in failures {
                println!("    {}", f.as_str().unwrap_or("?").red());
            }
        }
    }

    println!("\n{passed}/{total} passed",);
    if failed > 0 {
        std::process::exit(1);
    }
    Ok(())
}

pub async fn describe(client: &OrkaClient, name: &str) -> crate::client::Result<()> {
    let resp = client.get(&format!("/api/v1/skills/{name}")).await?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(format!("skill '{name}' not found").into());
    }
    let resp = OrkaClient::ensure_ok(resp).await?;
    let skill: serde_json::Value = resp.json().await?;
    println!("{}", skill["name"].as_str().unwrap_or("?").green().bold());
    println!("{}", skill["description"].as_str().unwrap_or(""));
    println!("\n{}", "Schema:".cyan());
    let schema = serde_json::to_string_pretty(&skill["schema"]).unwrap_or_default();
    println!("{schema}");
    Ok(())
}
