//! Health check endpoints: /health, /health/live, /health/ready.

use std::sync::Arc;

use orka_core::traits::PriorityQueue;

pub(super) fn routes(
    queue: Arc<dyn PriorityQueue>,
    start_time: std::time::Instant,
    concurrency: usize,
    redis_url: String,
    qdrant_url: Option<String>,
) -> axum::Router {
    let queue_for_health = queue.clone();
    let queue_for_ready = queue.clone();

    axum::Router::new()
        .route(
            "/health",
            axum::routing::get(move || {
                let queue = queue_for_health.clone();
                async move {
                    let uptime_secs = start_time.elapsed().as_secs();
                    let queue_depth = queue.len().await.unwrap_or(0);
                    axum::Json(serde_json::json!({
                        "status": "ok",
                        "uptime_secs": uptime_secs,
                        "workers": concurrency,
                        "queue_depth": queue_depth,
                    }))
                }
            }),
        )
        .route(
            "/health/live",
            axum::routing::get(|| async { axum::Json(serde_json::json!({"status": "ok"})) }),
        )
        .route(
            "/health/ready",
            axum::routing::get({
                let redis_url = redis_url.clone();
                let qdrant_url = qdrant_url.clone();
                move || {
                    let queue = queue_for_ready.clone();
                    let redis_url = redis_url.clone();
                    let qdrant_url = qdrant_url.clone();
                    async move {
                        let mut checks = serde_json::Map::new();
                        let mut all_ok = true;

                        match redis::Client::open(redis_url.as_str()) {
                            Ok(client) => match client.get_multiplexed_async_connection().await {
                                Ok(mut conn) => {
                                    match redis::cmd("PING").query_async::<String>(&mut conn).await
                                    {
                                        Ok(_) => {
                                            checks.insert("redis".into(), serde_json::json!("ok"));
                                        }
                                        Err(e) => {
                                            checks.insert(
                                                "redis".into(),
                                                serde_json::json!(format!("error: {e}")),
                                            );
                                            all_ok = false;
                                        }
                                    }
                                }
                                Err(e) => {
                                    checks.insert(
                                        "redis".into(),
                                        serde_json::json!(format!("error: {e}")),
                                    );
                                    all_ok = false;
                                }
                            },
                            Err(e) => {
                                checks.insert(
                                    "redis".into(),
                                    serde_json::json!(format!("error: {e}")),
                                );
                                all_ok = false;
                            }
                        }

                        match queue.len().await {
                            Ok(depth) => {
                                checks.insert(
                                    "queue".into(),
                                    serde_json::json!({"status": "ok", "depth": depth}),
                                );
                            }
                            Err(e) => {
                                checks.insert(
                                    "queue".into(),
                                    serde_json::json!(format!("error: {e}")),
                                );
                                all_ok = false;
                            }
                        }

                        if let Some(ref url) = qdrant_url {
                            match qdrant_client::Qdrant::from_url(url).build() {
                                Ok(client) => {
                                    match tokio::time::timeout(
                                        std::time::Duration::from_secs(2),
                                        client.health_check(),
                                    )
                                    .await
                                    {
                                        Ok(Ok(_)) => {
                                            checks.insert("qdrant".into(), serde_json::json!("ok"));
                                        }
                                        Ok(Err(e)) => {
                                            checks.insert(
                                                "qdrant".into(),
                                                serde_json::json!(format!("error: {e}")),
                                            );
                                            all_ok = false;
                                        }
                                        Err(_) => {
                                            checks.insert(
                                                "qdrant".into(),
                                                serde_json::json!("error: health check timed out"),
                                            );
                                            all_ok = false;
                                        }
                                    }
                                }
                                Err(e) => {
                                    checks.insert(
                                        "qdrant".into(),
                                        serde_json::json!(format!("error: {e}")),
                                    );
                                    all_ok = false;
                                }
                            }
                        }

                        let status = if all_ok { "ready" } else { "not_ready" };
                        let code = if all_ok {
                            axum::http::StatusCode::OK
                        } else {
                            axum::http::StatusCode::SERVICE_UNAVAILABLE
                        };
                        (
                            code,
                            axum::Json(serde_json::json!({
                                "status": status,
                                "checks": checks,
                            })),
                        )
                    }
                }
            }),
        )
}
