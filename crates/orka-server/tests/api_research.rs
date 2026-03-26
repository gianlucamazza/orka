#![allow(missing_docs)]

mod common;

use axum::body::Body;
use http::{Request, StatusCode};
use tower::ServiceExt;

#[tokio::test]
async fn research_not_enabled_returns_503() {
    let app = common::test_router();

    let req = Request::builder()
        .uri("/api/v1/research/campaigns")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn get_nonexistent_campaign_returns_404() {
    let app = common::test_router_with_research();

    let req = Request::builder()
        .uri("/api/v1/research/campaigns/does-not-exist")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn create_campaign_with_invalid_body_returns_error() {
    let app = common::test_router_with_research();

    // Missing required fields (name, task, etc.)
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/research/campaigns")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"not_a_field": true}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    // Axum rejects missing required fields with 422 (typed extractor) or
    // the service returns 400 for validation errors.
    assert!(
        resp.status() == StatusCode::UNPROCESSABLE_ENTITY
            || resp.status() == StatusCode::BAD_REQUEST,
        "expected 400 or 422, got {}",
        resp.status()
    );
}

#[tokio::test]
async fn delete_campaign_removes_it() {
    let app = common::test_router_with_research();

    let create = Request::builder()
        .method("POST")
        .uri("/api/v1/research/campaigns")
        .header("content-type", "application/json")
        .body(Body::from(
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
        ))
        .unwrap();
    let resp = app.clone().oneshot(create).await.unwrap();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let id = created["id"].as_str().unwrap();

    let delete = Request::builder()
        .method("DELETE")
        .uri(format!("/api/v1/research/campaigns/{id}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(delete).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let get = Request::builder()
        .uri(format!("/api/v1/research/campaigns/{id}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(get).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn reject_promotion_request() {
    let app = common::test_router_with_research();

    // Create campaign
    let create = Request::builder()
        .method("POST")
        .uri("/api/v1/research/campaigns")
        .header("content-type", "application/json")
        .body(Body::from(
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
        ))
        .unwrap();
    let resp = app.clone().oneshot(create).await.unwrap();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let campaign: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let campaign_id = campaign["id"].as_str().unwrap();

    // Trigger a run (returns 202 with status=Running, candidate_id=null)
    let run_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/research/campaigns/{campaign_id}/runs"))
        .body(Body::from("{}"))
        .unwrap();
    let resp = app.clone().oneshot(run_req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let run_init: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let run_id = run_init["id"].as_str().unwrap();

    // Poll until the background task completes
    let run = poll_run_completion(app.clone(), run_id).await;
    assert_eq!(run["status"], "completed", "run failed: {:?}", run["error"]);
    let candidate_id = run["candidate_id"].as_str().unwrap();

    // Submit promotion (requires approval → 202)
    let promote = Request::builder()
        .method("POST")
        .uri(format!(
            "/api/v1/research/candidates/{candidate_id}/promote"
        ))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({ "approved": false }).to_string(),
        ))
        .unwrap();
    let resp = app.clone().oneshot(promote).await.unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let request_id = request["id"].as_str().unwrap();

    // Reject the request
    let reject = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/research/promotions/{request_id}/reject"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({ "reason": "not good enough" }).to_string(),
        ))
        .unwrap();
    let resp = app.oneshot(reject).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let rejected: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(rejected["status"], "rejected");
}

#[tokio::test]
async fn create_and_list_research_campaigns() {
    let app = common::test_router_with_research();

    let create = Request::builder()
        .method("POST")
        .uri("/api/v1/research/campaigns")
        .header("content-type", "application/json")
        .body(Body::from(
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
        ))
        .unwrap();
    let resp = app.clone().oneshot(create).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let list = Request::builder()
        .uri("/api/v1/research/campaigns")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(list).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let arr = json.as_array().expect("expected array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["name"], "nightly-tune");
}

#[tokio::test]
async fn pause_research_campaign_updates_state() {
    let app = common::test_router_with_research();

    let create = Request::builder()
        .method("POST")
        .uri("/api/v1/research/campaigns")
        .header("content-type", "application/json")
        .body(Body::from(
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
        ))
        .unwrap();
    let resp = app.clone().oneshot(create).await.unwrap();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let id = created["id"].as_str().unwrap();

    let pause = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/research/campaigns/{id}/pause"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(pause).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let paused: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(paused["active"], false);
}

#[tokio::test]
async fn promotion_request_can_be_created_and_approved() {
    let app = common::test_router_with_research();

    let create = Request::builder()
        .method("POST")
        .uri("/api/v1/research/campaigns")
        .header("content-type", "application/json")
        .body(Body::from(
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
        ))
        .unwrap();
    let resp = app.clone().oneshot(create).await.unwrap();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let campaign_id = created["id"].as_str().unwrap();

    let run_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/research/campaigns/{campaign_id}/runs"))
        .body(Body::from("{}"))
        .unwrap();
    let resp = app.clone().oneshot(run_req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let run_init: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let run_id = run_init["id"].as_str().unwrap();
    let run = poll_run_completion(app.clone(), run_id).await;
    assert_eq!(run["status"], "completed", "run failed: {:?}", run["error"]);
    let candidate_id = run["candidate_id"].as_str().unwrap();

    let promote = Request::builder()
        .method("POST")
        .uri(format!(
            "/api/v1/research/candidates/{candidate_id}/promote"
        ))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({ "approved": false }).to_string(),
        ))
        .unwrap();
    let resp = app.clone().oneshot(promote).await.unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let request_id = request["id"].as_str().unwrap();
    assert_eq!(request["status"], "pending");

    let approve = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/research/promotions/{request_id}/approve"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(approve).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let candidate: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(candidate["status"], "promoted");
}

/// Poll `GET /api/v1/research/runs/{run_id}` until `status != "Running"` and
/// return the final run JSON. Panics after 50 attempts (~500 ms).
async fn poll_run_completion(app: axum::Router, run_id: &str) -> serde_json::Value {
    for _ in 0..50 {
        let req = Request::builder()
            .uri(format!("/api/v1/research/runs/{run_id}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let run: serde_json::Value = serde_json::from_slice(&body).unwrap();
        if run["status"].as_str() != Some("running") {
            return run;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("research run did not complete within timeout");
}
