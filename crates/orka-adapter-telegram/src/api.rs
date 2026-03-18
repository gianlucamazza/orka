//! HTTP client wrapper for the Telegram Bot API.

use orka_core::{Error, Result};
use reqwest::Client;
use serde_json::{Value, json};
use tracing::warn;

use crate::types::{TelegramFile, TelegramMessage, TelegramResponse, Update};

const BASE_URL: &str = "https://api.telegram.org";

/// Thin async HTTP client for the Telegram Bot API.
pub(crate) struct TelegramApi {
    client: Client,
    bot_token: String,
}

impl TelegramApi {
    pub fn new(bot_token: String) -> Self {
        Self {
            client: Client::new(),
            bot_token,
        }
    }

    /// Build a full download URL for a resolved file path.
    pub fn file_download_url(&self, file_path: &str) -> String {
        format!("{}/file/bot{}/{}", BASE_URL, self.bot_token, file_path)
    }

    fn url(&self, method: &str) -> String {
        format!("{}/bot{}/{}", BASE_URL, self.bot_token, method)
    }

    /// Call a Telegram API method with automatic 429 retry-once.
    async fn call<T: serde::de::DeserializeOwned>(&self, method: &str, body: &Value) -> Result<T> {
        for attempt in 0u8..2 {
            let resp = self
                .client
                .post(self.url(method))
                .json(body)
                .send()
                .await
                .map_err(|e| Error::Adapter {
                    source: Box::new(e),
                    context: format!("Telegram {method} request failed"),
                })?;

            if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS && attempt == 0 {
                let text = resp.text().await.unwrap_or_default();
                let retry_secs: u64 = serde_json::from_str::<Value>(&text)
                    .ok()
                    .and_then(|v| {
                        v.get("parameters")
                            .and_then(|p| p.get("retry_after"))
                            .and_then(|r| r.as_u64())
                    })
                    .unwrap_or(5);
                warn!(
                    retry_after = retry_secs,
                    method, "Telegram rate limited, retrying"
                );
                tokio::time::sleep(tokio::time::Duration::from_secs(retry_secs)).await;
                continue;
            }

            let tg_resp: TelegramResponse<T> = resp.json().await.map_err(|e| Error::Adapter {
                source: Box::new(e),
                context: format!("Telegram {method}: failed to parse response"),
            })?;

            if tg_resp.ok {
                return tg_resp.result.ok_or_else(|| Error::Adapter {
                    source: Box::new(std::io::Error::other("missing result")),
                    context: format!("Telegram {method}: ok=true but result is null"),
                });
            } else {
                return Err(Error::Adapter {
                    source: Box::new(std::io::Error::other(
                        tg_resp
                            .description
                            .unwrap_or_else(|| "unknown error".into()),
                    )),
                    context: format!("Telegram {method} API error"),
                });
            }
        }

        Err(Error::Adapter {
            source: Box::new(std::io::Error::other("rate limited after retry")),
            context: format!("Telegram {method} still rate limited"),
        })
    }

    /// Fetch pending updates from Telegram (long polling).
    pub async fn get_updates(
        &self,
        offset: i64,
        timeout: u64,
        allowed_updates: &[&str],
    ) -> Result<Vec<Update>> {
        self.call(
            "getUpdates",
            &json!({
                "offset": offset,
                "timeout": timeout,
                "allowed_updates": allowed_updates,
            }),
        )
        .await
    }

    /// Send a text message.
    pub async fn send_message(
        &self,
        chat_id: i64,
        text: &str,
        parse_mode: Option<&str>,
        reply_to_message_id: Option<i64>,
        reply_markup: Option<&Value>,
        message_thread_id: Option<i64>,
    ) -> Result<TelegramMessage> {
        let mut body = json!({
            "chat_id": chat_id,
            "text": text,
        });
        if let Some(pm) = parse_mode {
            body["parse_mode"] = json!(pm);
        }
        if let Some(id) = reply_to_message_id {
            body["reply_parameters"] = json!({"message_id": id});
        }
        if let Some(rm) = reply_markup {
            body["reply_markup"] = rm.clone();
        }
        if let Some(tid) = message_thread_id {
            body["message_thread_id"] = json!(tid);
        }
        self.call("sendMessage", &body).await
    }

    /// Send a photo by URL.
    pub async fn send_photo(
        &self,
        chat_id: i64,
        photo_url: &str,
        caption: Option<&str>,
        reply_to_message_id: Option<i64>,
        message_thread_id: Option<i64>,
    ) -> Result<TelegramMessage> {
        let mut body = json!({
            "chat_id": chat_id,
            "photo": photo_url,
        });
        if let Some(c) = caption {
            body["caption"] = json!(c);
        }
        if let Some(id) = reply_to_message_id {
            body["reply_parameters"] = json!({"message_id": id});
        }
        if let Some(tid) = message_thread_id {
            body["message_thread_id"] = json!(tid);
        }
        self.call("sendPhoto", &body).await
    }

    /// Send a document by URL.
    pub async fn send_document(
        &self,
        chat_id: i64,
        doc_url: &str,
        caption: Option<&str>,
        reply_to_message_id: Option<i64>,
        message_thread_id: Option<i64>,
    ) -> Result<TelegramMessage> {
        let mut body = json!({
            "chat_id": chat_id,
            "document": doc_url,
        });
        if let Some(c) = caption {
            body["caption"] = json!(c);
        }
        if let Some(id) = reply_to_message_id {
            body["reply_parameters"] = json!({"message_id": id});
        }
        if let Some(tid) = message_thread_id {
            body["message_thread_id"] = json!(tid);
        }
        self.call("sendDocument", &body).await
    }

    /// Send an audio file by URL.
    pub async fn send_audio(
        &self,
        chat_id: i64,
        audio_url: &str,
        caption: Option<&str>,
        reply_to_message_id: Option<i64>,
        message_thread_id: Option<i64>,
    ) -> Result<TelegramMessage> {
        let mut body = json!({
            "chat_id": chat_id,
            "audio": audio_url,
        });
        if let Some(c) = caption {
            body["caption"] = json!(c);
        }
        if let Some(id) = reply_to_message_id {
            body["reply_parameters"] = json!({"message_id": id});
        }
        if let Some(tid) = message_thread_id {
            body["message_thread_id"] = json!(tid);
        }
        self.call("sendAudio", &body).await
    }

    /// Send a video by URL.
    pub async fn send_video(
        &self,
        chat_id: i64,
        video_url: &str,
        caption: Option<&str>,
        reply_to_message_id: Option<i64>,
        message_thread_id: Option<i64>,
    ) -> Result<TelegramMessage> {
        let mut body = json!({
            "chat_id": chat_id,
            "video": video_url,
        });
        if let Some(c) = caption {
            body["caption"] = json!(c);
        }
        if let Some(id) = reply_to_message_id {
            body["reply_parameters"] = json!({"message_id": id});
        }
        if let Some(tid) = message_thread_id {
            body["message_thread_id"] = json!(tid);
        }
        self.call("sendVideo", &body).await
    }

    /// Send a chat action (e.g. "typing").
    pub async fn send_chat_action(&self, chat_id: i64, action: &str) -> Result<bool> {
        self.call(
            "sendChatAction",
            &json!({
                "chat_id": chat_id,
                "action": action,
            }),
        )
        .await
    }

    /// Resolve a file_id to a download URL.
    pub async fn get_file_url(&self, file_id: &str) -> Result<String> {
        let file: TelegramFile = self.call("getFile", &json!({ "file_id": file_id })).await?;
        let path = file.file_path.ok_or_else(|| Error::Adapter {
            source: Box::new(std::io::Error::other(
                "file_path missing from getFile response",
            )),
            context: "getFile returned no file_path".into(),
        })?;
        Ok(self.file_download_url(&path))
    }

    /// Edit a previously sent message's text.
    pub async fn edit_message_text(
        &self,
        chat_id: i64,
        message_id: i64,
        text: &str,
        parse_mode: Option<&str>,
    ) -> Result<TelegramMessage> {
        let mut body = json!({
            "chat_id": chat_id,
            "message_id": message_id,
            "text": text,
        });
        if let Some(pm) = parse_mode {
            body["parse_mode"] = json!(pm);
        }
        self.call("editMessageText", &body).await
    }

    /// Acknowledge a callback query.
    pub async fn answer_callback_query(
        &self,
        callback_query_id: &str,
        text: Option<&str>,
    ) -> Result<bool> {
        let mut body = json!({ "callback_query_id": callback_query_id });
        if let Some(t) = text {
            body["text"] = json!(t);
        }
        self.call("answerCallbackQuery", &body).await
    }

    /// Register the webhook URL with Telegram.
    pub async fn set_webhook(&self, url: &str, allowed_updates: &[&str]) -> Result<bool> {
        self.call(
            "setWebhook",
            &json!({
                "url": url,
                "allowed_updates": allowed_updates,
            }),
        )
        .await
    }

    /// Remove the registered webhook.
    pub async fn delete_webhook(&self) -> Result<bool> {
        self.call("deleteWebhook", &json!({})).await
    }

    /// Register bot commands with Telegram.
    pub(crate) async fn set_my_commands(&self, commands: &[(String, String)]) -> Result<bool> {
        let cmds: Vec<Value> = commands
            .iter()
            .map(|(cmd, desc)| json!({"command": cmd, "description": desc}))
            .collect();
        self.call("setMyCommands", &json!({ "commands": cmds }))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_url_format() {
        let api = TelegramApi::new("TEST_TOKEN".into());
        assert_eq!(
            api.url("sendMessage"),
            "https://api.telegram.org/botTEST_TOKEN/sendMessage"
        );
    }

    #[test]
    fn file_download_url_format() {
        let api = TelegramApi::new("TEST_TOKEN".into());
        assert_eq!(
            api.file_download_url("documents/file_0.pdf"),
            "https://api.telegram.org/file/botTEST_TOKEN/documents/file_0.pdf"
        );
    }
}
