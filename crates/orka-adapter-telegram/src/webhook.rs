//! Axum-based webhook server for Telegram Bot API updates.

use std::{collections::HashMap, sync::Arc};

use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
};
use orka_contracts::TrustLevel;
use orka_core::{
    InteractionSink,
    traits::MemoryStore,
    types::SessionId,
};
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use crate::{
    TelegramAuthGuard,
    api::TelegramApi,
    polling::{extract_user_info, process_callback_query, process_message},
    types::Update,
};

#[derive(Clone)]
struct WebhookState {
    api: Arc<TelegramApi>,
    sink: Arc<Mutex<Option<InteractionSink>>>,
    sessions: Arc<Mutex<HashMap<i64, SessionId>>>,
    memory: Option<Arc<dyn MemoryStore>>,
    auth_guard: Arc<TelegramAuthGuard>,
    /// Expected value of `X-Telegram-Bot-Api-Secret-Token`, if configured.
    webhook_secret: Option<String>,
    trust_level: TrustLevel,
}

/// Verify `X-Telegram-Bot-Api-Secret-Token` using constant-time comparison to
/// prevent timing-based secret recovery.
fn verify_telegram_secret(headers: &HeaderMap, expected: &str) -> bool {
    match headers.get("X-Telegram-Bot-Api-Secret-Token") {
        Some(value) => {
            // Constant-time comparison: same length check first, then byte-by-byte.
            let provided = value.as_bytes();
            let expected_bytes = expected.as_bytes();
            if provided.len() != expected_bytes.len() {
                return false;
            }
            provided
                .iter()
                .zip(expected_bytes.iter())
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0
        }
        None => false,
    }
}

async fn handle_update(
    State(state): State<WebhookState>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    // Verify webhook secret token when configured.
    if let Some(ref secret) = state.webhook_secret
        && !verify_telegram_secret(&headers, secret)
    {
        warn!("Telegram webhook: missing or invalid X-Telegram-Bot-Api-Secret-Token");
        return StatusCode::UNAUTHORIZED;
    }

    let update: Update = match serde_json::from_slice(&body) {
        Ok(u) => u,
        Err(e) => {
            warn!(%e, "Telegram webhook: failed to parse update payload");
            return StatusCode::BAD_REQUEST;
        }
    };
    if let Some((user_id, username)) = extract_user_info(&update) {
        if !state.auth_guard.is_allowed(user_id) {
            warn!(
                user_id,
                username = username.as_deref().unwrap_or("<none>"),
                "unauthorized Telegram user, dropping message"
            );
            return axum::http::StatusCode::OK;
        }
    } else if !state.auth_guard.is_open() {
        return axum::http::StatusCode::OK;
    }

    let Some(sink) = state.sink.lock().await.clone() else {
        return axum::http::StatusCode::OK;
    };

    let (msg_opt, is_edited) = match (update.message, update.edited_message) {
        (Some(m), _) => (Some(m), false),
        (None, Some(m)) => (Some(m), true),
        _ => (None, false),
    };

    if let Some(msg) = msg_opt {
        process_message(
            &state.api,
            msg,
            &state.sessions,
            state.memory.as_ref(),
            &sink,
            is_edited,
            state.trust_level,
        )
        .await;
    } else if let Some(cq) = update.callback_query {
        process_callback_query(
            &state.api,
            cq,
            &state.sessions,
            state.memory.as_ref(),
            &sink,
            state.trust_level,
        )
        .await;
    }

    axum::http::StatusCode::OK
}

/// Start the webhook HTTP server.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_webhook_server(
    api: Arc<TelegramApi>,
    sink: Arc<Mutex<Option<InteractionSink>>>,
    sessions: Arc<Mutex<HashMap<i64, SessionId>>>,
    memory: Option<Arc<dyn MemoryStore>>,
    webhook_url: String,
    port: u16,
    shutdown_rx: tokio::sync::oneshot::Receiver<()>,
    auth_guard: Arc<TelegramAuthGuard>,
    webhook_secret: Option<String>,
    trust_level: TrustLevel,
) {
    // Register webhook with Telegram
    match api
        .set_webhook(
            &webhook_url,
            &["message", "edited_message", "callback_query"],
            webhook_secret.as_deref(),
        )
        .await
    {
        Ok(_) => info!(url = %webhook_url, "Telegram webhook registered"),
        Err(e) => {
            error!(%e, "failed to register Telegram webhook");
            return;
        }
    }

    let state = WebhookState {
        api: api.clone(),
        sink,
        sessions,
        memory,
        auth_guard,
        webhook_secret,
        trust_level,
    };

    let app = Router::new()
        .route("/telegram/webhook", post(handle_update))
        .with_state(state);

    let addr = format!("0.0.0.0:{port}");
    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            error!(%e, addr, "failed to bind webhook listener");
            return;
        }
    };

    info!(addr, "Telegram webhook server listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = shutdown_rx.await;
        })
        .await
        .unwrap_or_else(|e| error!(%e, "webhook server error"));

    // Clean up webhook on shutdown
    if let Err(e) = api.delete_webhook().await {
        warn!(%e, "failed to delete Telegram webhook on shutdown");
    }
    info!("Telegram webhook server stopped");
}
