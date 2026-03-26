//! Telegram Bot API adapter for receiving and sending messages.
//!
//! Supports long polling (default) and webhook mode. Handles text, media,
//! slash commands, and callback queries.

#![warn(missing_docs)]

mod api;
mod markdown;
mod media;
pub mod polling;
mod types;
mod webhook;

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use api::TelegramApi;
use async_trait::async_trait;
use media::{SendMethod, select_send_method};
use orka_core::{
    Error, Result,
    config::TelegramAdapterConfig,
    traits::{ChannelAdapter, MemoryStore},
    types::{MessageSink, OutboundMessage, Payload, SessionId},
};
use serde_json::json;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

/// Authorization guard for restricting bot access to specific Telegram user
/// IDs.
#[derive(Clone, Debug)]
pub(crate) struct TelegramAuthGuard {
    allowed: Option<HashSet<i64>>,
}

impl TelegramAuthGuard {
    pub(crate) fn from_config(config: &TelegramAdapterConfig) -> Self {
        // Allow all users by default (no owner_id or allowed_users in config anymore)
        let _ = config; // Suppress unused warning
        Self { allowed: None }
    }

    pub(crate) fn is_allowed(&self, user_id: i64) -> bool {
        match &self.allowed {
            None => true,
            Some(set) => set.contains(&user_id),
        }
    }

    pub(crate) fn is_open(&self) -> bool {
        self.allowed.is_none()
    }
}

/// Telegram Bot API [`ChannelAdapter`].
pub struct TelegramAdapter {
    api: Arc<TelegramApi>,
    config: TelegramAdapterConfig,
    sink: Arc<Mutex<Option<MessageSink>>>,
    shutdown: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    sessions: Arc<Mutex<HashMap<i64, SessionId>>>,
    memory: Option<Arc<dyn MemoryStore>>,
}

impl TelegramAdapter {
    /// Create an adapter with the given config and bot token.
    pub fn new(config: TelegramAdapterConfig, bot_token: String) -> Self {
        let api = Arc::new(TelegramApi::new(bot_token));
        Self {
            api,
            config,
            sink: Arc::new(Mutex::new(None)),
            shutdown: Arc::new(Mutex::new(None)),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            memory: None,
        }
    }

    /// Attach a memory store so that chat-id → session-id mappings survive
    /// restarts.
    ///
    /// Mappings are persisted under the key
    /// `orka:adapter_session:telegram:{chat_id}`.
    pub fn with_memory(mut self, memory: Arc<dyn MemoryStore>) -> Self {
        self.memory = Some(memory);
        self
    }
}

#[async_trait]
impl ChannelAdapter for TelegramAdapter {
    fn channel_id(&self) -> &str {
        "telegram"
    }

    async fn start(&self, sink: MessageSink) -> Result<()> {
        *self.sink.lock().await = Some(sink.clone());

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        *self.shutdown.lock().await = Some(shutdown_tx);

        let api = self.api.clone();
        let sessions = self.sessions.clone();
        let memory = self.memory.clone();
        let mode = self.config.mode.as_deref().unwrap_or("polling").to_string();

        let auth_guard = Arc::new(TelegramAuthGuard::from_config(&self.config));
        if !auth_guard.is_open() {
            let n = auth_guard.allowed.as_ref().map_or(0, |s| s.len());
            info!(authorized_users = n, "Telegram auth enabled");
        }

        match mode.as_str() {
            "webhook" => {
                let webhook_url =
                    self.config.webhook_url.clone().ok_or_else(|| {
                        Error::Other("webhook_url required for webhook mode".into())
                    })?;
                let port = self.config.webhook_port.unwrap_or(8443);
                let sink_arc = self.sink.clone();
                tokio::spawn(async move {
                    webhook::run_webhook_server(
                        api,
                        sink_arc,
                        sessions,
                        memory,
                        webhook_url,
                        port,
                        shutdown_rx,
                        auth_guard,
                    )
                    .await;
                });
                info!("Telegram adapter started (webhook mode)");
            }
            _ => {
                tokio::spawn(async move {
                    polling::run_polling_loop(api, sink, sessions, memory, shutdown_rx, auth_guard)
                        .await;
                });
                info!("Telegram adapter started (long polling)");
            }
        }

        Ok(())
    }

    async fn send(&self, msg: OutboundMessage) -> Result<()> {
        let chat_id = msg
            .metadata
            .get("telegram_chat_id")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| Error::Adapter {
                source: Box::new(std::io::Error::other("missing telegram_chat_id")),
                context: "missing telegram_chat_id in outbound metadata".into(),
            })?;

        let reply_to_message_id = msg
            .metadata
            .get("telegram_message_id")
            .and_then(|v| v.as_i64());

        let message_thread_id = msg
            .metadata
            .get("telegram_message_thread_id")
            .and_then(|v| v.as_i64());

        let parse_mode = msg
            .metadata
            .get("telegram_parse_mode")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| self.config.parse_mode.clone());
        let parse_mode_ref = match parse_mode.as_deref() {
            None | Some("HTML") => Some("HTML"),
            Some("MarkdownV2") => Some("MarkdownV2"),
            Some("none") => None,
            Some(other) => {
                warn!(parse_mode = other, "unknown parse_mode, defaulting to HTML");
                Some("HTML")
            }
        };

        let inline_keyboard = msg.metadata.get("telegram_inline_keyboard").cloned();

        let reply_markup = inline_keyboard.map(|kb| json!({ "inline_keyboard": kb }));

        // Edit mode: update an existing message instead of sending a new one
        if let Some(edit_msg_id) = msg
            .metadata
            .get("telegram_edit_message_id")
            .and_then(|v| v.as_i64())
            && let Payload::Text(text) = &msg.payload
        {
            let (final_text, effective_pm) = if parse_mode_ref == Some("HTML") {
                let html = markdown::md_to_telegram_html(text);
                // edit cannot be split — truncate to first chunk if needed
                let truncated = if html.len() > 4096 {
                    let mut first = markdown::split_html(&html, 4090)
                        .into_iter()
                        .next()
                        .unwrap_or_else(|| html[..4090].to_string());
                    first.push('…');
                    first
                } else {
                    html
                };
                (truncated, Some("HTML"))
            } else {
                (text.clone(), parse_mode_ref)
            };
            self.api
                .edit_message_text(chat_id, edit_msg_id, &final_text, effective_pm)
                .await?;
            debug!(chat_id, edit_msg_id, "edited message on Telegram");
            return Ok(());
        }

        match &msg.payload {
            Payload::Text(raw_text) => {
                // Fire-and-forget typing indicator
                {
                    let api = self.api.clone();
                    tokio::spawn(async move {
                        let _ = api
                            .send_chat_action(chat_id, "typing", message_thread_id)
                            .await;
                    });
                }

                let (final_text, effective_pm) = if parse_mode_ref == Some("HTML") {
                    (markdown::md_to_telegram_html(raw_text), Some("HTML"))
                } else {
                    (raw_text.clone(), parse_mode_ref)
                };

                let chunks = markdown::split_html(&final_text, 4096);
                for (i, chunk) in chunks.iter().enumerate() {
                    let reply = if i == 0 { reply_to_message_id } else { None };
                    let markup = if i == 0 { reply_markup.as_ref() } else { None };
                    self.api
                        .send_message(
                            chat_id,
                            chunk,
                            effective_pm,
                            reply,
                            markup,
                            message_thread_id,
                        )
                        .await?;
                }
                debug!(chat_id, "sent text message to Telegram");
            }
            Payload::Media(media) => {
                let method = select_send_method(&media.mime_type);
                let caption = media.caption.as_deref();
                match method {
                    SendMethod::Photo => {
                        if let Some(data) = media.decode_data() {
                            self.api
                                .send_photo_bytes(
                                    chat_id,
                                    data,
                                    "chart.png",
                                    caption,
                                    reply_to_message_id,
                                    message_thread_id,
                                )
                                .await?;
                        } else {
                            self.api
                                .send_photo(
                                    chat_id,
                                    &media.url,
                                    caption,
                                    reply_to_message_id,
                                    message_thread_id,
                                )
                                .await?;
                        }
                    }
                    SendMethod::Audio => {
                        self.api
                            .send_audio(
                                chat_id,
                                &media.url,
                                caption,
                                reply_to_message_id,
                                message_thread_id,
                            )
                            .await?;
                    }
                    SendMethod::Video => {
                        self.api
                            .send_video(
                                chat_id,
                                &media.url,
                                caption,
                                reply_to_message_id,
                                message_thread_id,
                            )
                            .await?;
                    }
                    SendMethod::Document => {
                        self.api
                            .send_document(
                                chat_id,
                                &media.url,
                                caption,
                                reply_to_message_id,
                                message_thread_id,
                            )
                            .await?;
                    }
                }
                debug!(chat_id, mime = %media.mime_type, "sent media to Telegram");
            }
            Payload::Command(_) | Payload::Event(_) => {
                debug!("outbound Command/Event payload ignored by Telegram adapter");
            }
            _ => {
                debug!("unknown outbound payload type ignored by Telegram adapter");
            }
        }

        Ok(())
    }

    async fn shutdown(&self) -> Result<()> {
        if let Some(tx) = self.shutdown.lock().await.take() {
            let _ = tx.send(());
        }
        info!("Telegram adapter shut down");
        Ok(())
    }

    async fn register_commands(&self, commands: &[(&str, &str)]) -> Result<()> {
        let owned: Vec<(String, String)> = commands
            .iter()
            .map(|(n, d)| (n.to_string(), d.to_string()))
            .collect();
        self.api.set_my_commands(&owned).await?;
        info!(count = owned.len(), "registered Telegram bot commands");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use orka_core::{config::TelegramAdapterConfig, types::SessionId};

    use super::*;

    fn make_adapter() -> TelegramAdapter {
        TelegramAdapter::new(TelegramAdapterConfig::default(), "TEST_TOKEN".into())
    }

    #[test]
    fn channel_id_returns_telegram() {
        let adapter = make_adapter();
        assert_eq!(adapter.channel_id(), "telegram");
    }

    #[tokio::test]
    async fn send_errors_when_chat_id_missing() {
        let adapter = make_adapter();
        let msg = OutboundMessage::text("telegram", SessionId::new(), "hello", None);
        let err = adapter.send(msg).await.unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("telegram_chat_id"),
            "expected error about missing telegram_chat_id, got: {msg}"
        );
    }

    #[test]
    fn config_mode_defaults() {
        let config = TelegramAdapterConfig::default();
        assert!(config.mode.is_none());
        assert!(config.webhook_url.is_none());
        assert!(config.webhook_port.is_none());
        assert!(config.parse_mode.is_none());
        assert!(config.streaming.is_none());
    }

    #[test]
    fn auth_guard_open_when_no_config() {
        let config = TelegramAdapterConfig::default();
        let guard = TelegramAuthGuard::from_config(&config);
        assert!(guard.is_open());
        assert!(guard.is_allowed(12345));
        assert!(guard.is_allowed(0));
    }

    #[test]
    fn auth_guard_stays_open_without_acl_config() {
        let mut config = TelegramAdapterConfig::default();
        config.mode = Some("polling".into());
        let guard = TelegramAuthGuard::from_config(&config);
        assert!(guard.is_open());
        assert!(guard.is_allowed(42));
        assert!(guard.is_allowed(99));
    }
}
