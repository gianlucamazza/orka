#![allow(missing_docs)]

mod common;

use axum::body::Body;
use http::{Request, StatusCode};
use tower::ServiceExt;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn request(builder: http::request::Builder, body: Body) -> TestResult<Request<Body>> {
    Ok(builder.body(body)?)
}

async fn json_body(response: axum::response::Response) -> TestResult<serde_json::Value> {
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await?;
    Ok(serde_json::from_slice(&body)?)
}

fn field_str<'a>(value: &'a serde_json::Value, key: &str) -> TestResult<&'a str> {
    value[key]
        .as_str()
        .ok_or_else(|| format!("missing string field `{key}`").into())
}

fn as_array(value: &serde_json::Value) -> TestResult<&Vec<serde_json::Value>> {
    value
        .as_array()
        .ok_or_else(|| "expected array response".into())
}

#[tokio::test]
async fn research_not_enabled_returns_503() -> TestResult {
    let app = common::test_router();

    let req = request(
        Request::builder().uri("/api/v1/research/campaigns"),
        Body::empty(),
    )?;
    let resp = app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    Ok(())
}

#[tokio::test]
async fn get_nonexistent_campaign_returns_404() -> TestResult {
    let app = common::test_router_with_research();

    let req = request(
        Request::builder().uri("/api/v1/research/campaigns/does-not-exist"),
        Body::empty(),
    )?;
    let resp = app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
async fn create_campaign_with_invalid_body_returns_error() -> TestResult {
    let app = common::test_router_with_research();

    // Missing required fields (name, task, etc.)
    let req = request(
        Request::builder()
            .method("POST")
            .uri("/api/v1/research/campaigns")
            .header("content-type", "application/json"),
        Body::from(r#"{"not_a_field": true}"#),
    )?;
    let resp = app.oneshot(req).await?;
    // Axum rejects missing required fields with 422 (typed extractor) or
    // the service returns 400 for validation errors.
    assert!(
        resp.status() == StatusCode::UNPROCESSABLE_ENTITY
            || resp.status() == StatusCode::BAD_REQUEST,
        "expected 400 or 422, got {}",
        resp.status()
    );
    Ok(())
}

#[tokio::test]
async fn delete_campaign_removes_it() -> TestResult {
    let app = common::test_router_with_research();

    let create = request(
        Request::builder()
            .method("POST")
            .uri("/api/v1/research/campaigns")
            .header("content-type", "application/json"),
        Body::from(
            serde_json::json!({
                "name": "to-delete",
                "workspace": "default",
                "repo_path": ".",
                "baseline_ref": "HEAD",
                "task": "A task.",
                "verification_command": "cargo test",
                "editable_paths": ["crates/orka-research"],
                "target_branch": "main"
            })
            .to_string(),
        ),
    )?;
    let resp = app.clone().oneshot(create).await?;
    let created = json_body(resp).await?;
    let id = field_str(&created, "id")?;

    let delete = request(
        Request::builder()
            .method("DELETE")
            .uri(format!("/api/v1/research/campaigns/{id}")),
        Body::empty(),
    )?;
    let resp = app.clone().oneshot(delete).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let get = request(
        Request::builder().uri(format!("/api/v1/research/campaigns/{id}")),
        Body::empty(),
    )?;
    let resp = app.oneshot(get).await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
async fn reject_promotion_request() -> TestResult {
    let app = common::test_router_with_research();

    // Create campaign
    let create = request(
        Request::builder()
            .method("POST")
            .uri("/api/v1/research/campaigns")
            .header("content-type", "application/json"),
        Body::from(
            serde_json::json!({
                "name": "reject-flow",
                "workspace": "default",
                "repo_path": ".",
                "baseline_ref": "HEAD",
                "task": "Do a safe refactor.",
                "verification_command": "cargo test -p orka-research",
                "editable_paths": ["crates/orka-research"],
                "target_branch": "main"
            })
            .to_string(),
        ),
    )?;
    let resp = app.clone().oneshot(create).await?;
    let campaign = json_body(resp).await?;
    let campaign_id = field_str(&campaign, "id")?;

    // Trigger a run (returns 202 with status=Running, candidate_id=null)
    let run_req = request(
        Request::builder()
            .method("POST")
            .uri(format!("/api/v1/research/campaigns/{campaign_id}/runs")),
        Body::from("{}"),
    )?;
    let resp = app.clone().oneshot(run_req).await?;
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let run_init = json_body(resp).await?;
    let run_id = field_str(&run_init, "id")?;

    // Poll until the background task completes
    let run = poll_run_completion(app.clone(), run_id).await?;
    assert_eq!(run["status"], "completed", "run failed: {:?}", run["error"]);
    let candidate_id = field_str(&run, "candidate_id")?;

    // Submit promotion (requires approval → 202)
    let promote = request(
        Request::builder()
            .method("POST")
            .uri(format!(
                "/api/v1/research/candidates/{candidate_id}/promote"
            ))
            .header("content-type", "application/json"),
        Body::from(serde_json::json!({ "approved": false }).to_string()),
    )?;
    let resp = app.clone().oneshot(promote).await?;
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let promotion_request = json_body(resp).await?;
    let request_id = field_str(&promotion_request, "id")?;

    // Reject the request
    let reject = request(
        Request::builder()
            .method("POST")
            .uri(format!("/api/v1/research/promotions/{request_id}/reject"))
            .header("content-type", "application/json"),
        Body::from(serde_json::json!({ "reason": "not good enough" }).to_string()),
    )?;
    let resp = app.oneshot(reject).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let rejected = json_body(resp).await?;
    assert_eq!(rejected["status"], "rejected");
    Ok(())
}

#[tokio::test]
async fn create_and_list_research_campaigns() -> TestResult {
    let app = common::test_router_with_research();

    let create = request(
        Request::builder()
            .method("POST")
            .uri("/api/v1/research/campaigns")
            .header("content-type", "application/json"),
        Body::from(
            serde_json::json!({
                "name": "nightly-tune",
                "workspace": "default",
                "repo_path": ".",
                "baseline_ref": "HEAD",
                "task": "Improve verification coverage in a constrained way.",
                "verification_command": "cargo test -p orka-research",
                "editable_paths": ["crates/orka-research"],
                "target_branch": "main"
            })
            .to_string(),
        ),
    )?;
    let resp = app.clone().oneshot(create).await?;
    assert_eq!(resp.status(), StatusCode::CREATED);

    let list = request(
        Request::builder().uri("/api/v1/research/campaigns"),
        Body::empty(),
    )?;
    let resp = app.oneshot(list).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await?;
    let arr = as_array(&json)?;
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["name"], "nightly-tune");
    Ok(())
}

#[tokio::test]
async fn pause_research_campaign_updates_state() -> TestResult {
    let app = common::test_router_with_research();

    let create = request(
        Request::builder()
            .method("POST")
            .uri("/api/v1/research/campaigns")
            .header("content-type", "application/json"),
        Body::from(
            serde_json::json!({
                "name": "pause-me",
                "workspace": "default",
                "repo_path": ".",
                "baseline_ref": "HEAD",
                "task": "Do a safe refactor.",
                "verification_command": "cargo test -p orka-research",
                "editable_paths": ["crates/orka-research"],
                "target_branch": "main"
            })
            .to_string(),
        ),
    )?;
    let resp = app.clone().oneshot(create).await?;
    let created = json_body(resp).await?;
    let id = field_str(&created, "id")?;

    let pause = request(
        Request::builder()
            .method("POST")
            .uri(format!("/api/v1/research/campaigns/{id}/pause")),
        Body::empty(),
    )?;
    let resp = app.oneshot(pause).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let paused = json_body(resp).await?;
    assert_eq!(paused["active"], false);
    Ok(())
}

#[tokio::test]
async fn promotion_request_can_be_created_and_approved() -> TestResult {
    let app = common::test_router_with_research();

    let create = request(
        Request::builder()
            .method("POST")
            .uri("/api/v1/research/campaigns")
            .header("content-type", "application/json"),
        Body::from(
            serde_json::json!({
                "name": "promote-flow",
                "workspace": "default",
                "repo_path": ".",
                "baseline_ref": "HEAD",
                "task": "Do a safe refactor.",
                "verification_command": "cargo test -p orka-research",
                "editable_paths": ["crates/orka-research"],
                "target_branch": "main"
            })
            .to_string(),
        ),
    )?;
    let resp = app.clone().oneshot(create).await?;
    let created = json_body(resp).await?;
    let campaign_id = field_str(&created, "id")?;

    let run_req = request(
        Request::builder()
            .method("POST")
            .uri(format!("/api/v1/research/campaigns/{campaign_id}/runs")),
        Body::from("{}"),
    )?;
    let resp = app.clone().oneshot(run_req).await?;
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let run_init = json_body(resp).await?;
    let run_id = field_str(&run_init, "id")?;
    let run = poll_run_completion(app.clone(), run_id).await?;
    assert_eq!(run["status"], "completed", "run failed: {:?}", run["error"]);
    let candidate_id = field_str(&run, "candidate_id")?;

    let promote = request(
        Request::builder()
            .method("POST")
            .uri(format!(
                "/api/v1/research/candidates/{candidate_id}/promote"
            ))
            .header("content-type", "application/json"),
        Body::from(serde_json::json!({ "approved": false }).to_string()),
    )?;
    let resp = app.clone().oneshot(promote).await?;
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let promotion_request = json_body(resp).await?;
    let request_id = field_str(&promotion_request, "id")?;
    assert_eq!(promotion_request["status"], "pending");

    let approve = request(
        Request::builder()
            .method("POST")
            .uri(format!("/api/v1/research/promotions/{request_id}/approve")),
        Body::empty(),
    )?;
    let resp = app.oneshot(approve).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let candidate = json_body(resp).await?;
    assert_eq!(candidate["status"], "promoted");
    Ok(())
}

/// Poll `GET /api/v1/research/runs/{run_id}` until `status != "Running"` and
/// return the final run JSON. Panics after 50 attempts (~500 ms).
async fn poll_run_completion(app: axum::Router, run_id: &str) -> TestResult<serde_json::Value> {
    for _ in 0..50 {
        let req = request(
            Request::builder().uri(format!("/api/v1/research/runs/{run_id}")),
            Body::empty(),
        )?;
        let resp = app.clone().oneshot(req).await?;
        let run = json_body(resp).await?;
        if run["status"].as_str() != Some("running") {
            return Ok(run);
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    Err("research run did not complete within timeout".into())
}
