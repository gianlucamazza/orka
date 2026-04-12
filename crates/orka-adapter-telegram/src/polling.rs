//! Long-polling loop for the Telegram adapter.

use std::{collections::HashMap, sync::Arc};

use orka_core::{
    CommandContent, EventContent, InboundInteraction, InteractionContent, InteractionSink,
    MediaAttachment, MemoryEntry, PlatformContext, SenderInfo, TraceContext, TrustLevel,
    traits::MemoryStore,
    types::{SessionId, backoff_delay},
};
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
    sink: InteractionSink,
    sessions: Arc<Mutex<HashMap<i64, SessionId>>>,
    memory: Option<Arc<dyn MemoryStore>>,
    mut shutdown_rx: tokio::sync::oneshot::Receiver<()>,
    auth_guard: Arc<TelegramAuthGuard>,
    trust_level: TrustLevel,
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
                    handle_update(
                        &api,
                        update,
                        &sessions,
                        memory.as_ref(),
                        &sink,
                        &auth_guard,
                        trust_level,
                    )
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
    sink: &InteractionSink,
    auth_guard: &TelegramAuthGuard,
    trust_level: TrustLevel,
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
        process_callback_query(api, cq, sessions, memory, sink, trust_level).await;
        return;
    }

    let (msg, is_edited) = match (update.message, update.edited_message) {
        (Some(m), _) => (m, false),
        (None, Some(m)) => (m, true),
        _ => return,
    };

    process_message(api, msg, sessions, memory, sink, is_edited, trust_level).await;
}

/// Process a regular or edited message.
pub(crate) async fn process_message(
    api: &Arc<TelegramApi>,
    msg: TelegramMessage,
    sessions: &Arc<Mutex<HashMap<i64, SessionId>>>,
    memory: Option<&Arc<dyn MemoryStore>>,
    sink: &InteractionSink,
    is_edited: bool,
    trust_level: TrustLevel,
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

    let content = build_content(api, &msg).await;

    let chat_type = match msg.chat.r#type.as_deref() {
        Some("private") => "direct",
        _ => "group",
    };

    let mut extensions = serde_json::Map::new();
    extensions.insert(
        "telegram_message_id".into(),
        serde_json::json!(msg.message_id),
    );
    if let Some(tid) = msg.message_thread_id {
        extensions.insert("telegram_message_thread_id".into(), serde_json::json!(tid));
    }
    if is_edited {
        extensions.insert("telegram_edited".into(), serde_json::json!(true));
    }

    let sender = msg
        .from
        .as_ref()
        .map_or_else(SenderInfo::default, |from| SenderInfo {
            platform_user_id: Some(from.id.to_string()),
            display_name: user_name.clone(),
            user_id: None,
        });

    let interaction = InboundInteraction {
        id: Uuid::now_v7(),
        source_channel: "telegram".into(),
        session_id: session_id.as_uuid(),
        timestamp: chrono::Utc::now(),
        content,
        context: PlatformContext {
            sender,
            chat_id: Some(chat_id.to_string()),
            interaction_kind: Some(chat_type.into()),
            trust_level: Some(trust_level),
            extensions: extensions.into_iter().collect(),
            ..Default::default()
        },
        trace: TraceContext::default(),
    };

    if sink.send(interaction).await.is_err() {
        debug!("sink closed, stopping Telegram message processing");
    }
}

/// Build the interaction content from a message: command > media > text.
async fn build_content(api: &Arc<TelegramApi>, msg: &TelegramMessage) -> InteractionContent {
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
        return InteractionContent::Media(MediaAttachment {
            mime_type: media.mime_type,
            url: media.url,
            filename: media.filename,
            caption: media.caption,
            size_bytes: media.size_bytes,
            data_base64: media.data_base64,
        });
    }

    // Fallback to text
    let text = msg
        .text
        .clone()
        .or_else(|| msg.caption.clone())
        .unwrap_or_default();
    InteractionContent::Text(text)
}

fn parse_command(text: &str) -> InteractionContent {
    // Strip leading '/', split on whitespace
    let stripped = text.trim_start_matches('/');
    // Handle @BotName suffix in command (e.g. /start@MyBot)
    let (cmd_raw, rest) = stripped
        .split_once(|c: char| c.is_whitespace())
        .unwrap_or((stripped, ""));
    let cmd_name = cmd_raw.split('@').next().unwrap_or(cmd_raw).to_lowercase();

    let mut args = std::collections::HashMap::new();
    if !rest.trim().is_empty() {
        args.insert("text".into(), serde_json::json!(rest.trim()));
    }
    InteractionContent::Command(CommandContent {
        name: cmd_name,
        args,
    })
}

pub(crate) async fn process_callback_query(
    api: &Arc<TelegramApi>,
    cq: CallbackQuery,
    sessions: &Arc<Mutex<HashMap<i64, SessionId>>>,
    memory: Option<&Arc<dyn MemoryStore>>,
    sink: &InteractionSink,
    trust_level: TrustLevel,
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
    let content = InteractionContent::Event(EventContent {
        kind: "callback_query".into(),
        data: serde_json::json!({ "data": data, "from_id": cq.from.id }),
    });

    let mut extensions = serde_json::Map::new();
    extensions.insert(
        "telegram_callback_query_id".into(),
        serde_json::json!(cq.id),
    );

    let interaction = InboundInteraction {
        id: Uuid::now_v7(),
        source_channel: "telegram".into(),
        session_id: session_id.as_uuid(),
        timestamp: chrono::Utc::now(),
        content,
        context: PlatformContext {
            sender: SenderInfo {
                platform_user_id: Some(cq.from.id.to_string()),
                display_name: cq.from.username.clone(),
                user_id: None,
            },
            chat_id: chat_id.map(|id| id.to_string()),
            interaction_kind: Some("callback".into()),
            trust_level: Some(trust_level),
            extensions: extensions.into_iter().collect(),
            ..Default::default()
        },
        trace: TraceContext::default(),
    };

    if sink.send(interaction).await.is_err() {
        debug!("sink closed, stopping Telegram callback query processing");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_command_simple() {
        let content = parse_command("/start");
        match content {
            InteractionContent::Command(cmd) => {
                assert_eq!(cmd.name, "start");
                assert!(cmd.args.is_empty());
            }
            _ => panic!("expected Command content"),
        }
    }

    #[test]
    fn parse_command_with_args() {
        let content = parse_command("/echo hello world");
        match content {
            InteractionContent::Command(cmd) => {
                assert_eq!(cmd.name, "echo");
                assert_eq!(
                    cmd.args.get("text").and_then(|v| v.as_str()),
                    Some("hello world")
                );
            }
            _ => panic!("expected Command content"),
        }
    }

    #[test]
    fn parse_command_with_bot_suffix() {
        let content = parse_command("/help@MyBot");
        match content {
            InteractionContent::Command(cmd) => {
                assert_eq!(cmd.name, "help");
                assert!(cmd.args.is_empty());
            }
            _ => panic!("expected Command content"),
        }
    }
}
