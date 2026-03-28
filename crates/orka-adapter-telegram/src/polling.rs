//! Long-polling loop for the Telegram adapter.

use std::{collections::HashMap, sync::Arc};

use orka_core::{
    MemoryEntry,
    traits::MemoryStore,
    types::{
        CommandPayload, Envelope, EventPayload, MessageId, MessageSink, Payload, SessionId,
        backoff_delay,
    },
};
use serde_json::json;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::{
    TelegramAuthGuard,
    api::TelegramApi,
    media::resolve_inbound_media,
    types::{CallbackQuery, TelegramMessage, Update},
};

/// Resolve the session ID for a given Telegram chat ID.
///
/// Resolution order:
/// 1. In-memory map (hot path, zero-cost after the first lookup)
/// 2. Persistent memory store (survives restarts)
/// 3. Generate a new ID and persist it
#[allow(clippy::implicit_hasher)]
pub async fn resolve_session(
    chat_id: i64,
    sessions: &Arc<Mutex<HashMap<i64, SessionId>>>,
    memory: Option<&Arc<dyn MemoryStore>>,
) -> SessionId {
    // Fast path: already in memory
    if let Some(sid) = sessions.lock().await.get(&chat_id).copied() {
        return sid;
    }

    // Slow path: check persistent store
    let key = format!("orka:adapter_session:telegram:{chat_id}");
    if let Some(mem) = memory
        && let Ok(Some(entry)) = mem.recall(&key).await
        && let Some(sid_str) = entry.value.as_str()
        && let Ok(uuid) = Uuid::parse_str(sid_str)
    {
        let sid = SessionId::from(uuid);
        sessions.lock().await.insert(chat_id, sid);
        return sid;
    }

    // Not found anywhere — create a new one and persist it
    let sid = SessionId::new();
    sessions.lock().await.insert(chat_id, sid);
    if let Some(mem) = memory {
        let entry = MemoryEntry::new(&key, serde_json::json!(sid.to_string()))
            .with_tags(vec!["adapter_session".to_string()]);
        if let Err(e) = mem.store(&key, entry, None).await {
            warn!(%e, chat_id, "failed to persist Telegram session mapping");
        }
    }
    sid
}

/// Extract `(user_id, username)` from any update type for auth checking.
pub(crate) fn extract_user_info(update: &Update) -> Option<(i64, Option<String>)> {
    if let Some(ref cq) = update.callback_query {
        return Some((cq.from.id, cq.from.username.clone()));
    }
    let msg = update.message.as_ref().or(update.edited_message.as_ref())?;
    let from = msg.from.as_ref()?;
    Some((from.id, from.username.clone()))
}

/// Run the long-polling loop until `shutdown_rx` fires.
pub(crate) async fn run_polling_loop(
    api: Arc<TelegramApi>,
    sink: MessageSink,
    sessions: Arc<Mutex<HashMap<i64, SessionId>>>,
    memory: Option<Arc<dyn MemoryStore>>,
    mut shutdown_rx: tokio::sync::oneshot::Receiver<()>,
    auth_guard: Arc<TelegramAuthGuard>,
) {
    let mut offset: i64 = 0;
    let mut error_count: u32 = 0;

    info!("Telegram long-polling started");

    loop {
        let updates_fut =
            api.get_updates(offset, 30, &["message", "edited_message", "callback_query"]);

        let updates = tokio::select! {
            _ = &mut shutdown_rx => {
                info!("Telegram adapter shutting down");
                break;
            }
            result = updates_fut => result,
        };

        match updates {
            Ok(updates) => {
                error_count = 0;
                for update in updates {
                    offset = update.update_id + 1;
                    handle_update(&api, update, &sessions, memory.as_ref(), &sink, &auth_guard)
                        .await;
                }
            }
            Err(e) => {
                error_count = error_count.saturating_add(1);
                error!(
                    error = %e,
                    consecutive_failures = error_count,
                    "Telegram getUpdates failed"
                );
                let delay = backoff_delay(error_count.saturating_sub(1), 1, 60);
                debug!(
                    delay_ms = delay.as_millis() as u64,
                    "backing off before retry"
                );
                tokio::time::sleep(delay).await;
            }
        }
    }
}

async fn handle_update(
    api: &Arc<TelegramApi>,
    update: Update,
    sessions: &Arc<Mutex<HashMap<i64, SessionId>>>,
    memory: Option<&Arc<dyn MemoryStore>>,
    sink: &MessageSink,
    auth_guard: &TelegramAuthGuard,
) {
    if let Some((user_id, username)) = extract_user_info(&update) {
        if !auth_guard.is_allowed(user_id) {
            warn!(
                user_id,
                username = username.as_deref().unwrap_or("<none>"),
                "unauthorized Telegram user, dropping message"
            );
            return;
        }
    } else if !auth_guard.is_open() {
        return;
    }

    if let Some(cq) = update.callback_query {
        process_callback_query(api, cq, sessions, memory, sink).await;
        return;
    }

    let (msg, is_edited) = match (update.message, update.edited_message) {
        (Some(m), _) => (m, false),
        (None, Some(m)) => (m, true),
        _ => return,
    };

    process_message(api, msg, sessions, memory, sink, is_edited).await;
}

/// Process a regular or edited message.
pub(crate) async fn process_message(
    api: &Arc<TelegramApi>,
    msg: TelegramMessage,
    sessions: &Arc<Mutex<HashMap<i64, SessionId>>>,
    memory: Option<&Arc<dyn MemoryStore>>,
    sink: &MessageSink,
    is_edited: bool,
) {
    let chat_id = msg.chat.id;

    let session_id = resolve_session(chat_id, sessions, memory).await;

    // Fire-and-forget typing indicator
    {
        let api = api.clone();
        let thread_id = msg.message_thread_id;
        tokio::spawn(async move {
            let _ = api.send_chat_action(chat_id, "typing", thread_id).await;
        });
    }

    // Build user display name
    let user_name = msg.from.as_ref().map(|u| {
        let mut name = u.first_name.clone();
        if let Some(ln) = &u.last_name {
            name.push(' ');
            name.push_str(ln);
        }
        name
    });

    let payload = build_payload(api, &msg).await;

    let mut envelope = Envelope::text("telegram", session_id, "");
    envelope.id = MessageId::new();
    envelope.payload = payload;
    envelope.timestamp = chrono::Utc::now();

    // Core metadata
    envelope
        .metadata
        .insert("telegram_chat_id".into(), json!(chat_id));

    let chat_type = match msg.chat.r#type.as_deref() {
        Some("private") => "direct",
        _ => "group",
    };
    envelope
        .metadata
        .insert("chat_type".into(), json!(chat_type));
    envelope
        .metadata
        .insert("telegram_message_id".into(), json!(msg.message_id));

    if let Some(from) = &msg.from {
        envelope
            .metadata
            .insert("telegram_user_id".into(), json!(from.id));
        if let Some(ref name) = user_name {
            envelope
                .metadata
                .insert("telegram_user_name".into(), json!(name));
        }
        if let Some(ref uname) = from.username {
            envelope
                .metadata
                .insert("telegram_username".into(), json!(uname));
        }
    }
    if let Some(tid) = msg.message_thread_id {
        envelope
            .metadata
            .insert("telegram_message_thread_id".into(), json!(tid));
    }
    if is_edited {
        envelope
            .metadata
            .insert("telegram_edited".into(), json!(true));
    }

    if sink.send(envelope).await.is_err() {
        debug!("sink closed, stopping Telegram message processing");
    }
}

/// Build the payload from a message: command > media > text.
async fn build_payload(api: &Arc<TelegramApi>, msg: &TelegramMessage) -> Payload {
    // Check for bot command entity at offset 0
    let is_command = msg
        .entities
        .iter()
        .any(|e| e.r#type == "bot_command" && e.offset == 0);

    if is_command && let Some(text) = &msg.text {
        return parse_command(text);
    }

    // Check for media
    if let Some(media) = resolve_inbound_media(api, msg).await {
        return Payload::Media(media);
    }

    // Fallback to text
    let text = msg
        .text
        .clone()
        .or_else(|| msg.caption.clone())
        .unwrap_or_default();
    Payload::Text(text)
}

fn parse_command(text: &str) -> Payload {
    // Strip leading '/', split on whitespace
    let stripped = text.trim_start_matches('/');
    // Handle @BotName suffix in command (e.g. /start@MyBot)
    let (cmd_raw, rest) = stripped
        .split_once(|c: char| c.is_whitespace())
        .unwrap_or((stripped, ""));
    let cmd_name = cmd_raw.split('@').next().unwrap_or(cmd_raw).to_lowercase();

    let mut args = HashMap::new();
    if !rest.trim().is_empty() {
        args.insert("text".into(), json!(rest.trim()));
    }
    Payload::Command(CommandPayload::new(cmd_name, args))
}

async fn process_callback_query(
    api: &Arc<TelegramApi>,
    cq: CallbackQuery,
    sessions: &Arc<Mutex<HashMap<i64, SessionId>>>,
    memory: Option<&Arc<dyn MemoryStore>>,
    sink: &MessageSink,
) {
    let chat_id = cq.message.as_ref().map(|m| m.chat.id);

    let session_id = if let Some(cid) = chat_id {
        resolve_session(cid, sessions, memory).await
    } else {
        SessionId::new()
    };

    // Acknowledge the callback query immediately
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
    if let Some(uname) = &cq.from.username {
        envelope
            .metadata
            .insert("telegram_username".into(), json!(uname));
    }

    if sink.send(envelope).await.is_err() {
        debug!("sink closed, stopping Telegram callback query processing");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_command_simple() {
        let payload = parse_command("/start");
        match payload {
            Payload::Command(cmd) => {
                assert_eq!(cmd.name, "start");
                assert!(cmd.args.is_empty());
            }
            _ => panic!("expected Command payload"),
        }
    }

    #[test]
    fn parse_command_with_args() {
        let payload = parse_command("/echo hello world");
        match payload {
            Payload::Command(cmd) => {
                assert_eq!(cmd.name, "echo");
                assert_eq!(
                    cmd.args.get("text").and_then(|v| v.as_str()),
                    Some("hello world")
                );
            }
            _ => panic!("expected Command payload"),
        }
    }

    #[test]
    fn parse_command_with_bot_suffix() {
        let payload = parse_command("/help@MyBot");
        match payload {
            Payload::Command(cmd) => {
                assert_eq!(cmd.name, "help");
                assert!(cmd.args.is_empty());
            }
            _ => panic!("expected Command payload"),
        }
    }
}
