//! REST endpoints for graph-execution checkpoints.
//!
//! | Method | Path | Description |
//! |--------|------|-------------|
//! | GET | `/api/v1/runs/{run_id}/checkpoints` | List all checkpoint IDs for a run |
//! | GET | `/api/v1/runs/{run_id}/checkpoints/latest` | Load the most recent checkpoint |
//! | GET | `/api/v1/runs/{run_id}/status` | Return the status field of the latest checkpoint |
//! | POST | `/api/v1/runs/{run_id}/approve` | Approve an interrupted run and re-enqueue it |
//! | POST | `/api/v1/runs/{run_id}/reject` | Reject an interrupted run and mark it failed |
//!
//! All endpoints return `503 Service Unavailable` when checkpointing is not
//! configured (store is `None`).

use std::sync::Arc;

use axum::{extract::Path, http::StatusCode, response::IntoResponse};
use orka_checkpoint::{CheckpointId, CheckpointStore, RunStatus};
use orka_core::traits::PriorityQueue;

#[allow(clippy::too_many_lines)]
pub(super) fn routes(
    store: Option<Arc<dyn CheckpointStore>>,
    queue: Arc<dyn PriorityQueue>,
) -> axum::Router {
    let s1 = store.clone();
    let s2 = store.clone();
    let s3 = store.clone();
    let s4 = store.clone();
    let s5 = store;
    let q4 = queue.clone();
    let q5 = queue;

    axum::Router::new()
        .route(
            "/api/v1/runs/{run_id}/checkpoints",
            axum::routing::get(move |Path(run_id): Path<String>| {
                let store = s1.clone();
                async move {
                    let Some(store) = store else {
                        return (StatusCode::SERVICE_UNAVAILABLE, "checkpointing not enabled")
                            .into_response();
                    };
                    match store.list(&run_id).await {
                        Ok(ids) => {
                            let id_strings: Vec<String> =
                                ids.iter().map(std::string::ToString::to_string).collect();
                            axum::Json(id_strings).into_response()
                        }
                        Err(e) => (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("list failed: {e}"),
                        )
                            .into_response(),
                    }
                }
            }),
        )
        .route(
            "/api/v1/runs/{run_id}/checkpoints/latest",
            axum::routing::get(move |Path(run_id): Path<String>| {
                let store = s2.clone();
                async move {
                    let Some(store) = store else {
                        return (StatusCode::SERVICE_UNAVAILABLE, "checkpointing not enabled")
                            .into_response();
                    };
                    match store.load_latest(&run_id).await {
                        Ok(Some(ckpt)) => axum::Json(ckpt).into_response(),
                        Ok(None) => (StatusCode::NOT_FOUND, "no checkpoint found").into_response(),
                        Err(e) => (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("load failed: {e}"),
                        )
                            .into_response(),
                    }
                }
            }),
        )
        .route(
            "/api/v1/runs/{run_id}/status",
            axum::routing::get(move |Path(run_id): Path<String>| {
                let store = s3.clone();
                async move {
                    let Some(store) = store else {
                        return (StatusCode::SERVICE_UNAVAILABLE, "checkpointing not enabled")
                            .into_response();
                    };
                    match store.load_latest(&run_id).await {
                        Ok(Some(ckpt)) => axum::Json(serde_json::json!({
                            "run_id": ckpt.run_id,
                            "status": ckpt.status,
                            "completed_node": ckpt.completed_node,
                            "resume_node": ckpt.resume_node,
                            "total_tokens": ckpt.total_tokens,
                            "total_iterations": ckpt.total_iterations,
                            "created_at": ckpt.created_at,
                        }))
                        .into_response(),
                        Ok(None) => (StatusCode::NOT_FOUND, "no checkpoint found").into_response(),
                        Err(e) => (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("status load failed: {e}"),
                        )
                            .into_response(),
                    }
                }
            }),
        )
        .route(
            "/api/v1/runs/{run_id}/approve",
            axum::routing::post(move |Path(run_id): Path<String>| {
                let store = s4.clone();
                let queue = q4.clone();
                async move {
                    let Some(store) = store else {
                        return (StatusCode::SERVICE_UNAVAILABLE, "checkpointing not enabled")
                            .into_response();
                    };
                    let ckpt = match store.load_latest(&run_id).await {
                        Ok(Some(c)) => c,
                        Ok(None) => {
                            return (StatusCode::NOT_FOUND, "no checkpoint found").into_response();
                        }
                        Err(e) => {
                            return (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                format!("load failed: {e}"),
                            )
                                .into_response();
                        }
                    };
                    if !matches!(ckpt.status, RunStatus::Interrupted { .. }) {
                        return (StatusCode::CONFLICT, "run is not in Interrupted state")
                            .into_response();
                    }
                    // Re-enqueue the original envelope with resume metadata
                    let mut envelope = ckpt.trigger.clone();
                    envelope
                        .metadata
                        .insert("__resume_run_id".to_string(), serde_json::json!(run_id));
                    match queue.push(&envelope).await {
                        Ok(()) => (
                            StatusCode::ACCEPTED,
                            axum::Json(serde_json::json!({
                                "run_id": run_id,
                                "status": "queued_for_resume",
                            })),
                        )
                            .into_response(),
                        Err(e) => (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("enqueue failed: {e}"),
                        )
                            .into_response(),
                    }
                }
            }),
        )
        .route(
            "/api/v1/runs/{run_id}/reject",
            axum::routing::post(move |Path(run_id): Path<String>| {
                let store = s5.clone();
                let _queue = q5.clone();
                async move {
                    let Some(store) = store else {
                        return (StatusCode::SERVICE_UNAVAILABLE, "checkpointing not enabled")
                            .into_response();
                    };
                    let ckpt = match store.load_latest(&run_id).await {
                        Ok(Some(c)) => c,
                        Ok(None) => {
                            return (StatusCode::NOT_FOUND, "no checkpoint found").into_response();
                        }
                        Err(e) => {
                            return (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                format!("load failed: {e}"),
                            )
                                .into_response();
                        }
                    };
                    if !matches!(ckpt.status, RunStatus::Interrupted { .. }) {
                        return (StatusCode::CONFLICT, "run is not in Interrupted state")
                            .into_response();
                    }
                    // Write a terminal Failed checkpoint
                    let mut terminal = ckpt.clone();
                    terminal.id = CheckpointId::new();
                    terminal.status = RunStatus::Failed {
                        error: "rejected by human operator".to_string(),
                    };
                    terminal.resume_node = None;
                    match store.save(&terminal).await {
                        Ok(()) => axum::Json(serde_json::json!({
                            "run_id": run_id,
                            "status": "rejected",
                        }))
                        .into_response(),
                        Err(e) => (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("save failed: {e}"),
                        )
                            .into_response(),
                    }
                }
            }),
        )
}
