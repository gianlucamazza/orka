//! Axum-based webhook server for Telegram Bot API updates.

use std::{collections::HashMap, sync::Arc};

use axum::{Json, Router, extract::State, routing::post};
use orka_core::{
    traits::MemoryStore,
    types::{MessageSink, SessionId},
};
use serde_json::json;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use crate::{
    TelegramAuthGuard,
    api::TelegramApi,
    polling::{extract_user_info, process_message, resolve_session},
    types::{CallbackQuery, Update},
};

#[derive(Clone)]
struct WebhookState {
    api: Arc<TelegramApi>,
    sink: Arc<Mutex<Option<MessageSink>>>,
    sessions: Arc<Mutex<HashMap<i64, SessionId>>>,
    memory: Option<Arc<dyn MemoryStore>>,
    auth_guard: Arc<TelegramAuthGuard>,
}

async fn handle_update(
    State(state): State<WebhookState>,
    Json(update): Json<Update>,
) -> axum::http::StatusCode {
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

    let sink = match state.sink.lock().await.clone() {
        Some(s) => s,
        None => return axum::http::StatusCode::OK,
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
            &state.memory,
            &sink,
            is_edited,
        )
        .await;
    } else if let Some(cq) = update.callback_query {
        handle_callback_query(&state.api, cq, &state.sessions, &state.memory, &sink).await;
    }

    axum::http::StatusCode::OK
}

async fn handle_callback_query(
    api: &Arc<TelegramApi>,
    cq: CallbackQuery,
    sessions: &Arc<Mutex<HashMap<i64, SessionId>>>,
    memory: &Option<Arc<dyn MemoryStore>>,
    sink: &MessageSink,
) {
    use orka_core::types::{Envelope, EventPayload, MessageId, Payload};

    let chat_id = cq.message.as_ref().map(|m| m.chat.id);

    let session_id = if let Some(cid) = chat_id {
        resolve_session(cid, sessions, memory).await
    } else {
        SessionId::new()
    };

    {
        let api = api.clone();
        let cq_id = cq.id.clone();
        tokio::spawn(async move {
            if let Err(e) = api.answer_callback_query(&cq_id, None).await {
                warn!(%e, "failed to answer callback query");
            }
        });
    }

    let data = cq.data.clone().unwrap_or_default();
    let payload = Payload::Event(EventPayload::new(
        "callback_query",
        json!({ "data": data, "from_id": cq.from.id }),
    ));

    let mut envelope = Envelope::text("telegram", session_id, "");
    envelope.id = MessageId::new();
    envelope.payload = payload;
    envelope.timestamp = chrono::Utc::now();

    envelope
        .metadata
        .insert("telegram_callback_query_id".into(), json!(cq.id));
    if let Some(cid) = chat_id {
        envelope
            .metadata
            .insert("telegram_chat_id".into(), json!(cid));
    }
    envelope
        .metadata
        .insert("telegram_user_id".into(), json!(cq.from.id));

    if let Err(e) = sink.send(envelope).await {
        warn!(%e, "failed to send callback query envelope");
    }
}

/// Start the webhook HTTP server.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_webhook_server(
    api: Arc<TelegramApi>,
    sink: Arc<Mutex<Option<MessageSink>>>,
    sessions: Arc<Mutex<HashMap<i64, SessionId>>>,
    memory: Option<Arc<dyn MemoryStore>>,
    webhook_url: String,
    port: u16,
    shutdown_rx: tokio::sync::oneshot::Receiver<()>,
    auth_guard: Arc<TelegramAuthGuard>,
) {
    // Register webhook with Telegram
    match api
        .set_webhook(
            &webhook_url,
            &["message", "edited_message", "callback_query"],
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
