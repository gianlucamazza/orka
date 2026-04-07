//! Secret management endpoints: CRUD for runtime secrets.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query},
    http::StatusCode,
    response::IntoResponse,
};
use orka_core::{SecretValue, traits::SecretManager};

pub(super) fn routes(secrets: Arc<dyn SecretManager>) -> axum::Router {
    let s1 = secrets.clone();
    let s2 = secrets.clone();
    let s3 = secrets.clone();
    let s4 = secrets;

    axum::Router::new()
        .route(
            "/api/v1/secrets",
            axum::routing::get(move || {
                let secrets = s1.clone();
                async move {
                    match secrets.list_secrets().await {
                        Ok(keys) => axum::Json(keys).into_response(),
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
            "/api/v1/secrets/{*path}",
            axum::routing::get(
                move |Path(path): Path<String>,
                      Query(params): Query<std::collections::HashMap<String, String>>| {
                    let secrets = s2.clone();
                    async move {
                        let reveal = params.get("reveal").is_some_and(|v| v == "true");
                        match secrets.get_secret(&path).await {
                            Ok(secret) => {
                                let raw = secret.expose_str().unwrap_or("").to_string();
                                let value = if reveal {
                                    raw
                                } else if raw.chars().count() <= 4 {
                                    "****".to_string()
                                } else {
                                    let prefix: String = raw.chars().take(4).collect();
                                    format!("{prefix}****")
                                };
                                axum::Json(serde_json::json!({ "path": path, "value": value }))
                                    .into_response()
                            }
                            Err(e) => {
                                let msg = e.to_string();
                                if msg.contains("not found") {
                                    (StatusCode::NOT_FOUND, msg).into_response()
                                } else {
                                    (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
                                }
                            }
                        }
                    }
                },
            )
            .post(move |Path(path): Path<String>, Json(body): Json<serde_json::Value>| {
                let secrets = s3.clone();
                async move {
                    let Some(value) = body["value"].as_str() else {
                        return (StatusCode::BAD_REQUEST, "'value' is required").into_response();
                    };
                    if value.is_empty() {
                        return (StatusCode::BAD_REQUEST, "value must not be empty")
                            .into_response();
                    }
                    let secret = SecretValue::new(value.as_bytes().to_vec());
                    match secrets.set_secret(&path, &secret).await {
                        Ok(()) => (
                            StatusCode::OK,
                            axum::Json(serde_json::json!({ "path": path, "set": true })),
                        )
                            .into_response(),
                        Err(e) => (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("set failed: {e}"),
                        )
                            .into_response(),
                    }
                }
            })
            .delete(move |Path(path): Path<String>| {
                let secrets = s4.clone();
                async move {
                    match secrets.delete_secret(&path).await {
                        Ok(()) => axum::Json(serde_json::json!({ "path": path, "deleted": true }))
                            .into_response(),
                        Err(e) => {
                            let msg = e.to_string();
                            if msg.contains("not found") {
                                (StatusCode::NOT_FOUND, msg).into_response()
                            } else {
                                (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
                            }
                        }
                    }
                }
            }),
        )
}
