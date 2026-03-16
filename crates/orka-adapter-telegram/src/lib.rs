use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use orka_core::traits::ChannelAdapter;
use orka_core::types::{backoff_delay, Envelope, MessageSink, OutboundMessage, Payload, SessionId};
use orka_core::{Error, Result};
use reqwest::Client;
use serde::Deserialize;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

pub struct TelegramAdapter {
    bot_token: String,
    client: Client,
    sink: Arc<Mutex<Option<MessageSink>>>,
    shutdown: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    /// Maps chat_id to SessionId for consistent session routing
    sessions: Arc<Mutex<HashMap<i64, SessionId>>>,
}

impl TelegramAdapter {
    pub fn new(bot_token: String) -> Self {
        Self {
            bot_token,
            client: Client::new(),
            sink: Arc::new(Mutex::new(None)),
            shutdown: Arc::new(Mutex::new(None)),
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn api_url(&self, method: &str) -> String {
        format!("https://api.telegram.org/bot{}/{}", self.bot_token, method)
    }
}

#[derive(Debug, Deserialize)]
struct TelegramResponse<T> {
    ok: bool,
    result: Option<T>,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Update {
    update_id: i64,
    message: Option<TelegramMessage>,
}

#[derive(Debug, Deserialize)]
struct TelegramMessage {
    #[allow(dead_code)]
    message_id: i64,
    chat: Chat,
    text: Option<String>,
    #[allow(dead_code)]
    from: Option<User>,
}

#[derive(Debug, Deserialize)]
struct Chat {
    id: i64,
    #[serde(default)]
    r#type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct User {
    #[allow(dead_code)]
    id: i64,
    #[allow(dead_code)]
    first_name: String,
}

#[async_trait]
impl ChannelAdapter for TelegramAdapter {
    fn channel_id(&self) -> &str {
        "telegram"
    }

    async fn start(&self, sink: MessageSink) -> Result<()> {
        *self.sink.lock().await = Some(sink.clone());

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
        *self.shutdown.lock().await = Some(shutdown_tx);

        let client = self.client.clone();
        let api_url = self.api_url("getUpdates");
        let send_sink = sink;
        let sessions = self.sessions.clone();

        tokio::spawn(async move {
            let mut offset: i64 = 0;
            let mut error_count: u32 = 0;

            loop {
                // Check shutdown
                if shutdown_rx.try_recv().is_ok() {
                    info!("Telegram adapter shutting down");
                    break;
                }

                let params = serde_json::json!({
                    "offset": offset,
                    "timeout": 30,
                    "allowed_updates": ["message"],
                });

                let result = client.post(api_url.as_str()).json(&params).send().await;

                match result {
                    Ok(response) => {
                        match response.json::<TelegramResponse<Vec<Update>>>().await {
                            Ok(resp) if resp.ok => {
                                error_count = 0;
                                if let Some(updates) = resp.result {
                                    for update in updates {
                                        offset = update.update_id + 1;

                                        if let Some(msg) = update.message {
                                            if let Some(text) = msg.text {
                                                let session_id = {
                                                    let mut s = sessions.lock().await;
                                                    s.entry(msg.chat.id)
                                                        .or_insert_with(SessionId::new)
                                                        .clone()
                                                };

                                                let mut envelope =
                                                    Envelope::text("telegram", session_id, text);

                                                // Store chat_id in metadata for outbound routing
                                                envelope.metadata.insert(
                                                    "telegram_chat_id".to_string(),
                                                    serde_json::json!(msg.chat.id),
                                                );

                                                // Propagate chat type for priority routing
                                                let chat_type = match msg.chat.r#type.as_deref() {
                                                    Some("private") => "direct",
                                                    Some("group" | "supergroup" | "channel") => {
                                                        "group"
                                                    }
                                                    _ => "group",
                                                };
                                                envelope.metadata.insert(
                                                    "chat_type".to_string(),
                                                    serde_json::json!(chat_type),
                                                );

                                                if send_sink.send(envelope).await.is_err() {
                                                    debug!(
                                                        "sink closed, stopping Telegram polling"
                                                    );
                                                    return;
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            Ok(resp) => {
                                warn!(
                                    description = resp.description.as_deref().unwrap_or("unknown"),
                                    "Telegram API error"
                                );
                                tokio::time::sleep(backoff_delay(error_count, 1, 60)).await;
                                error_count = error_count.saturating_add(1);
                            }
                            Err(e) => {
                                error!(%e, "failed to parse Telegram response");
                                tokio::time::sleep(backoff_delay(error_count, 1, 60)).await;
                                error_count = error_count.saturating_add(1);
                            }
                        }
                    }
                    Err(e) => {
                        error!(%e, "Telegram getUpdates request failed");
                        tokio::time::sleep(backoff_delay(error_count, 1, 60)).await;
                        error_count = error_count.saturating_add(1);
                    }
                }
            }
        });

        info!("Telegram adapter started (long polling)");
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

        let text = match &msg.payload {
            Payload::Text(t) => t.clone(),
            _ => "[unsupported payload type]".into(),
        };

        let params = serde_json::json!({
            "chat_id": chat_id,
            "text": text,
        });

        let response = self
            .client
            .post(self.api_url("sendMessage"))
            .json(&params)
            .send()
            .await
            .map_err(|e| Error::Adapter {
                source: Box::new(e),
                context: "Telegram sendMessage request failed".into(),
            })?;

        if !response.status().is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Adapter {
                source: Box::new(std::io::Error::other(body.clone())),
                context: format!("Telegram sendMessage failed: {body}"),
            });
        }

        debug!(chat_id, "sent message to Telegram");
        Ok(())
    }

    async fn shutdown(&self) -> Result<()> {
        if let Some(tx) = self.shutdown.lock().await.take() {
            let _ = tx.send(());
        }
        info!("Telegram adapter shut down");
        Ok(())
    }
}
