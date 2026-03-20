use crate::client::OrkaClient;
use colored::Colorize;

pub async fn show(
    client: &OrkaClient,
    filter: Option<&str>,
    json: bool,
) -> crate::client::Result<()> {
    let resp = client.get("/metrics").await?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        println!(
            "{}",
            "Metrics endpoint not available (Prometheus not enabled).".yellow()
        );
        return Ok(());
    }
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Server returned {status}: {body}").into());
    }
    let text = resp.text().await?;

    let lines: Vec<&str> = text
        .lines()
        .filter(|line| {
            if let Some(f) = filter {
                line.contains(f)
            } else {
                true
            }
        })
        .collect();

    if json {
        let metrics: Vec<serde_json::Value> = lines
            .iter()
            .filter(|line| !line.starts_with('#') && !line.is_empty())
            .filter_map(|line| {
                let (name_part, value_str) = line.rsplit_once(' ')?;
                let value: f64 = value_str.parse().ok()?;
                Some(serde_json::json!({
                    "metric": name_part.trim(),
                    "value": value,
                }))
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&metrics).unwrap_or_default()
        );
    } else {
        for line in lines {
            if line.starts_with("# HELP") || line.starts_with("# TYPE") {
                println!("{}", line.dimmed());
            } else if !line.is_empty() {
                println!("{line}");
            }
        }
    }
    Ok(())
}
