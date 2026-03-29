//! HTTP client wrapper for the Telegram Bot API.

use orka_core::{Error, Result, SecretStr};
use reqwest::Client;
use serde_json::{Value, json};
use tracing::warn;

use crate::types::{TelegramFile, TelegramMessage, TelegramResponse, Update};

const BASE_URL: &str = "https://api.telegram.org";

/// Thin async HTTP client for the Telegram Bot API.
///
/// The bot token is embedded once into pre-built URL prefixes at construction
/// time so it never appears in per-request method bodies or log traces.
pub(crate) struct TelegramApi {
    client: Client,
    /// `https://api.telegram.org/bot{token}` — no trailing slash.
    api_prefix: String,
    /// `https://api.telegram.org/file/bot{token}` — no trailing slash.
    file_prefix: String,
}

impl TelegramApi {
    pub(crate) fn new(bot_token: &SecretStr) -> Self {
        let api_prefix = format!("{BASE_URL}/bot{}", bot_token.expose());
        let file_prefix = format!("{BASE_URL}/file/bot{}", bot_token.expose());
        Self {
            client: Client::new(),
            api_prefix,
            file_prefix,
        }
    }

    /// Build a full download URL for a resolved file path.
    pub(crate) fn file_download_url(&self, file_path: &str) -> String {
        format!("{}/{}", self.file_prefix, file_path)
    }

    fn url(&self, method: &str) -> String {
        format!("{}/{}", self.api_prefix, method)
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
                            .and_then(serde_json::Value::as_u64)
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
            }
            let description = tg_resp
                .description
                .unwrap_or_else(|| "unknown error".into());
            let error_code = tg_resp
                .error_code
                .map(|c| format!(" (error_code={c})"))
                .unwrap_or_default();
            return Err(Error::Adapter {
                source: Box::new(std::io::Error::other(description)),
                context: format!("Telegram {method} API error{error_code}"),
            });
        }

        Err(Error::Adapter {
            source: Box::new(std::io::Error::other("rate limited after retry")),
            context: format!("Telegram {method} still rate limited"),
        })
    }

    /// Fetch pending updates from Telegram (long polling).
    pub(crate) async fn get_updates(
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
    pub(crate) async fn send_message(
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
    pub(crate) async fn send_photo(
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

    /// Send a photo from raw bytes via multipart upload.
    pub(crate) async fn send_photo_bytes(
        &self,
        chat_id: i64,
        data: Vec<u8>,
        filename: &str,
        caption: Option<&str>,
        reply_to_message_id: Option<i64>,
        message_thread_id: Option<i64>,
    ) -> Result<TelegramMessage> {
        let url = self.url("sendPhoto");
        let caption_str = caption.map(ToString::to_string);
        let reply_params =
            reply_to_message_id.map(|id| serde_json::json!({"message_id": id}).to_string());

        // Retry up to 2 times on 429 (rate limit) or 5xx, mirroring `call()`.
        let mut last_err: Option<Error> = None;
        for attempt in 0..2u8 {
            if attempt > 0 {
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            }

            let mut form = reqwest::multipart::Form::new()
                .text("chat_id", chat_id.to_string())
                .part(
                    "photo",
                    reqwest::multipart::Part::bytes(data.clone())
                        .file_name(filename.to_string())
                        .mime_str("image/png")
                        .map_err(|e| Error::Adapter {
                            source: Box::new(e),
                            context: "Telegram multipart MIME error".into(),
                        })?,
                );
            if let Some(ref c) = caption_str {
                form = form.text("caption", c.clone());
            }
            if let Some(ref rp) = reply_params {
                form = form.text("reply_parameters", rp.clone());
            }
            if let Some(tid) = message_thread_id {
                form = form.text("message_thread_id", tid.to_string());
            }

            let resp = match self.client.post(&url).multipart(form).send().await {
                Ok(r) => r,
                Err(e) => {
                    last_err = Some(Error::Adapter {
                        source: Box::new(e),
                        context: "Telegram photo bytes send failed".into(),
                    });
                    continue;
                }
            };

            if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS && attempt == 0 {
                let text = resp.text().await.unwrap_or_default();
                let retry_secs: u64 = serde_json::from_str::<Value>(&text)
                    .ok()
                    .and_then(|v| {
                        v.get("parameters")
                            .and_then(|p| p.get("retry_after"))
                            .and_then(serde_json::Value::as_u64)
                    })
                    .unwrap_or(5);
                warn!(
                    retry_after = retry_secs,
                    "Telegram sendPhoto rate limited, retrying"
                );
                tokio::time::sleep(tokio::time::Duration::from_secs(retry_secs)).await;
                last_err = Some(Error::Adapter {
                    source: Box::new(std::io::Error::other("rate limited")),
                    context: "Telegram sendPhoto rate limited".into(),
                });
                continue;
            }

            let tg_resp: TelegramResponse<TelegramMessage> =
                resp.json().await.map_err(|e| Error::Adapter {
                    source: Box::new(e),
                    context: "Telegram photo bytes response parse failed".into(),
                })?;
            if !tg_resp.ok {
                return Err(Error::Adapter {
                    source: Box::new(std::io::Error::other(
                        tg_resp.description.unwrap_or_default(),
                    )),
                    context: "Telegram sendPhoto (bytes) returned ok=false".into(),
                });
            }
            return tg_resp.result.ok_or_else(|| Error::Adapter {
                source: Box::new(std::io::Error::other("missing result")),
                context: "Telegram sendPhoto (bytes) returned no result".into(),
            });
        }
        Err(last_err.unwrap_or_else(|| Error::Adapter {
            source: Box::new(std::io::Error::other("max retries exceeded")),
            context: "Telegram sendPhoto (bytes) failed after retries".into(),
        }))
    }

    /// Send a document by URL.
    pub(crate) async fn send_document(
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
    pub(crate) async fn send_audio(
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
    pub(crate) async fn send_video(
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
    pub(crate) async fn send_chat_action(
        &self,
        chat_id: i64,
        action: &str,
        message_thread_id: Option<i64>,
    ) -> Result<bool> {
        let mut body = json!({
            "chat_id": chat_id,
            "action": action,
        });
        if let Some(tid) = message_thread_id {
            body["message_thread_id"] = json!(tid);
        }
        self.call("sendChatAction", &body).await
    }

    /// Resolve a `file_id` to a download URL.
    pub(crate) async fn get_file_url(&self, file_id: &str) -> Result<String> {
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
    pub(crate) async fn edit_message_text(
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
    pub(crate) async fn answer_callback_query(
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
    ///
    /// If `secret_token` is provided it will be sent back by Telegram in the
    /// `X-Telegram-Bot-Api-Secret-Token` header on every webhook POST, allowing
    /// the server to authenticate incoming requests.
    pub(crate) async fn set_webhook(
        &self,
        url: &str,
        allowed_updates: &[&str],
        secret_token: Option<&str>,
    ) -> Result<bool> {
        let mut body = json!({
            "url": url,
            "allowed_updates": allowed_updates,
        });
        if let Some(secret) = secret_token {
            body["secret_token"] = json!(secret);
        }
        self.call("setWebhook", &body).await
    }

    /// Remove the registered webhook.
    pub(crate) async fn delete_webhook(&self) -> Result<bool> {
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
        let api = TelegramApi::new(&SecretStr::new("TEST_TOKEN"));
        assert_eq!(
            api.url("sendMessage"),
            "https://api.telegram.org/botTEST_TOKEN/sendMessage"
        );
    }

    #[test]
    fn file_download_url_format() {
        let api = TelegramApi::new(&SecretStr::new("TEST_TOKEN"));
        assert_eq!(
            api.file_download_url("documents/file_0.pdf"),
            "https://api.telegram.org/file/botTEST_TOKEN/documents/file_0.pdf"
        );
    }
}
