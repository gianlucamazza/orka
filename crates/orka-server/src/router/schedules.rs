//! Schedule management endpoints: CRUD for cron/one-shot tasks.

use std::{collections::HashMap, sync::Arc};

use axum::{Json, extract::Path, http::StatusCode, response::IntoResponse};
use orka_scheduler::ScheduleStore;

pub(super) fn routes(scheduler_store: Option<Arc<dyn ScheduleStore>>) -> axum::Router {
    let sc1 = scheduler_store.clone();
    let sc2 = scheduler_store.clone();
    let sc3 = scheduler_store;

    axum::Router::new()
        .route(
            "/api/v1/schedules",
            axum::routing::get(move || {
                let store = sc1.clone();
                async move {
                    let Some(store) = store else {
                        return (StatusCode::SERVICE_UNAVAILABLE, "scheduler not enabled")
                            .into_response();
                    };
                    match store.list(false).await {
                        Ok(s) => axum::Json(s).into_response(),
                        Err(e) => (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("list failed: {e}"),
                        )
                            .into_response(),
                    }
                }
            })
            .post(move |Json(body): Json<serde_json::Value>| {
                let store = sc2.clone();
                async move {
                    let Some(store) = store else {
                        return (StatusCode::SERVICE_UNAVAILABLE, "scheduler not enabled")
                            .into_response();
                    };
                    let name = match body["name"].as_str() {
                        Some(n) => n.to_string(),
                        None => {
                            return (StatusCode::BAD_REQUEST, "'name' is required").into_response();
                        }
                    };
                    let cron_expr = body["cron"].as_str().map(String::from);
                    let run_at_str = body["run_at"].as_str().map(String::from);
                    if cron_expr.is_none() && run_at_str.is_none() {
                        return (
                            StatusCode::BAD_REQUEST,
                            "either 'cron' or 'run_at' is required",
                        )
                            .into_response();
                    }
                    let next_run = if let Some(ref cron_str) = cron_expr {
                        use std::str::FromStr as _;
                        match cron::Schedule::from_str(cron_str) {
                            Ok(sched) => {
                                match sched.upcoming(chrono::Utc).next().map(|t| t.timestamp()) {
                                    Some(ts) => ts,
                                    None => {
                                        return (
                                            StatusCode::BAD_REQUEST,
                                            "no upcoming run for cron",
                                        )
                                            .into_response();
                                    }
                                }
                            }
                            Err(e) => {
                                return (StatusCode::BAD_REQUEST, format!("invalid cron: {e}"))
                                    .into_response();
                            }
                        }
                    } else if let Some(ref run_at) = run_at_str {
                        match chrono::DateTime::parse_from_rfc3339(run_at) {
                            Ok(dt) => dt.timestamp(),
                            Err(e) => {
                                return (StatusCode::BAD_REQUEST, format!("invalid run_at: {e}"))
                                    .into_response();
                            }
                        }
                    } else {
                        0
                    };
                    let args: Option<HashMap<String, serde_json::Value>> = body["args"]
                        .as_object()
                        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
                    let schedule = orka_scheduler::types::Schedule {
                        id: uuid::Uuid::now_v7().to_string(),
                        name,
                        cron: cron_expr,
                        run_at: run_at_str,
                        timezone: body["timezone"].as_str().map(String::from),
                        skill: body["skill"].as_str().map(String::from),
                        args,
                        message: body["message"].as_str().map(String::from),
                        next_run,
                        created_at: chrono::Utc::now().to_rfc3339(),
                        completed: false,
                    };
                    match store.add(&schedule).await {
                        Ok(()) => (StatusCode::CREATED, axum::Json(schedule)).into_response(),
                        Err(e) => (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("create failed: {e}"),
                        )
                            .into_response(),
                    }
                }
            }),
        )
        .route(
            "/api/v1/schedules/{id}",
            axum::routing::delete(move |Path(id): Path<String>| {
                let store = sc3.clone();
                async move {
                    let Some(store) = store else {
                        return (StatusCode::SERVICE_UNAVAILABLE, "scheduler not enabled")
                            .into_response();
                    };
                    match store.remove(&id).await {
                        Ok(found) => {
                            axum::Json(serde_json::json!({ "deleted": found })).into_response()
                        }
                        Err(e) => (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("delete failed: {e}"),
                        )
                            .into_response(),
                    }
                }
            }),
        )
}
