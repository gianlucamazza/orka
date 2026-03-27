//! Dead-letter queue (DLQ) endpoints.

use std::sync::Arc;

use axum::{extract::Path, response::IntoResponse};
use orka_core::traits::DeadLetterQueue;

pub(super) fn routes(queue: Arc<dyn DeadLetterQueue>) -> axum::Router {
    let q1 = queue.clone();
    let q2 = queue.clone();
    let q3 = queue;

    axum::Router::new()
        .route(
            "/api/v1/dlq",
            axum::routing::get({
                let q = q1;
                move || {
                    let q = q.clone();
                    async move {
                        match q.list().await {
                            Ok(items) => {
                                let json: Vec<serde_json::Value> = items
                                    .iter()
                                    .map(|e| {
                                        serde_json::json!({
                                            "id": e.id.to_string(),
                                            "channel": e.channel,
                                            "session_id": e.session_id.to_string(),
                                            "timestamp": e.timestamp.to_rfc3339(),
                                            "metadata": e.metadata,
                                        })
                                    })
                                    .collect();
                                axum::Json(json).into_response()
                            }
                            Err(e) => (
                                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                                format!("DLQ list failed: {e}"),
                            )
                                .into_response(),
                        }
                    }
                }
            })
            .delete({
                let q = q2;
                move || {
                    let q = q.clone();
                    async move {
                        match q.purge().await {
                            Ok(count) => {
                                axum::Json(serde_json::json!({ "purged": count })).into_response()
                            }
                            Err(e) => (
                                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                                format!("DLQ purge failed: {e}"),
                            )
                                .into_response(),
                        }
                    }
                }
            }),
        )
        .route(
            "/api/v1/dlq/{id}/replay",
            axum::routing::post({
                let q = q3;
                move |Path(id): Path<String>| {
                    let q = q.clone();
                    async move {
                        let msg_id = match uuid::Uuid::parse_str(&id) {
                            Ok(uuid) => orka_core::MessageId::from(uuid),
                            Err(_) => {
                                return (axum::http::StatusCode::BAD_REQUEST, "invalid message ID")
                                    .into_response();
                            }
                        };
                        match q.replay(&msg_id).await {
                            Ok(true) => {
                                axum::Json(serde_json::json!({ "replayed": true })).into_response()
                            }
                            Ok(false) => (
                                axum::http::StatusCode::NOT_FOUND,
                                "message not found in DLQ",
                            )
                                .into_response(),
                            Err(e) => (
                                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                                format!("DLQ replay failed: {e}"),
                            )
                                .into_response(),
                        }
                    }
                }
            }),
        )
}
