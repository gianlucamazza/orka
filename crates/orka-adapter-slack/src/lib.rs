//! Slack Events API adapter for receiving and sending messages.

#![warn(missing_docs)]

pub mod config;

use std::{collections::HashMap, future::IntoFuture, sync::Arc};

use async_trait::async_trait;
use axum::{Json, Router, body::Bytes, extract::State, http::HeaderMap, routing::post};
pub use config::SlackAdapterConfig;
use hmac::{Hmac, Mac};
use orka_core::{
    Error, Result, SecretStr,
    traits::ChannelAdapter,
    types::{
        Envelope, MediaPayload, MessageSink, OutboundMessage, Payload, SessionId, backoff_delay,
    },
};
use reqwest::Client;
use serde::Deserialize;
use sha2::Sha256;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

type HmacSha256 = Hmac<Sha256>;

/// Slack Events API [`ChannelAdapter`] using HTTP webhooks.
pub struct SlackAdapter {
    bot_token: Arc<SecretStr>,
    /// Slack signing secret used to verify `X-Slack-Signature` on incoming
    /// events.  When `None`, signature verification is skipped and a warning
    /// is logged at startup.
    signing_secret: Option<Arc<SecretStr>>,
    client: Client,
    sink: Arc<Mutex<Option<MessageSink>>>,
    shutdown: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    sessions: Arc<Mutex<HashMap<String, SessionId>>>,
    listen_port: u16,
}

impl SlackAdapter {
    /// Create an adapter with the given bot token, optional signing secret, and
    /// webhook listen port.
    pub fn new(bot_token: SecretStr, signing_secret: Option<SecretStr>, listen_port: u16) -> Self {
        if signing_secret.is_none() {
            warn!(
                "Slack signing_secret not configured — incoming events will not be authenticated"
            );
        }
        Self {
            bot_token: Arc::new(bot_token),
            signing_secret: signing_secret.map(Arc::new),
            client: Client::new(),
            sink: Arc::new(Mutex::new(None)),
            shutdown: Arc::new(Mutex::new(None)),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            listen_port,
        }
    }
}

/// Verify `X-Slack-Signature` using HMAC-SHA256.
///
/// Slack signs each request as `v0={hex(HMAC-SHA256(signing_secret,
/// "v0:{timestamp}:{body}")))}` and includes the timestamp in
/// `X-Slack-Request-Timestamp`.  Requests older than 5 minutes are rejected to
/// prevent replay attacks.
/// POST JSON to `url` with up to 3 attempts (retry on 429 or 5xx).
/// Returns the successful `reqwest::Response` or the last error encountered.
async fn post_json_retried(
    client: &reqwest::Client,
    url: &str,
    auth: &str,
    body: &serde_json::Value,
    context: &str,
) -> Result<reqwest::Response> {
    let mut last_err: Option<Error> = None;
    for attempt in 0..3u32 {
        if attempt > 0 {
            let ms = 500u64.saturating_mul(1u64 << (attempt - 1));
            tokio::time::sleep(std::time::Duration::from_millis(ms.min(10_000))).await;
        }
        match client
            .post(url)
            .header("Authorization", auth)
            .header("Content-Type", "application/json; charset=utf-8")
            .json(body)
            .send()
            .await
        {
            Err(e) if e.is_timeout() || e.is_connect() => {
                last_err = Some(Error::Adapter {
                    source: Box::new(e),
                    context: format!("{context} (transient)"),
                });
            }
            Err(e) => {
                return Err(Error::Adapter {
                    source: Box::new(e),
                    context: context.to_string(),
                });
            }
            Ok(r) if r.status() == 429 || r.status().is_server_error() => {
                let status = r.status();
                let text = r.text().await.unwrap_or_default();
                last_err = Some(Error::Adapter {
                    source: Box::new(std::io::Error::other(text.clone())),
                    context: format!("{context}: HTTP {status}: {text}"),
                });
            }
            Ok(r) => return Ok(r),
        }
    }
    Err(last_err.unwrap_or_else(|| Error::Adapter {
        source: Box::new(std::io::Error::other("max retries exceeded")),
        context: format!("{context}: max retries exceeded"),
    }))
}

fn verify_slack_signature(headers: &HeaderMap, body: &[u8], signing_secret: &str) -> bool {
    let timestamp = match headers
        .get("X-Slack-Request-Timestamp")
        .and_then(|v| v.to_str().ok())
    {
        Some(ts) => ts.to_owned(),
        None => {
            warn!("Slack webhook: missing X-Slack-Request-Timestamp");
            return false;
        }
    };

    // Reject requests older than 5 minutes to prevent replay attacks.
    if let Ok(ts_secs) = timestamp.parse::<i64>() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        if (now - ts_secs).abs() > 300 {
            warn!("Slack webhook: request timestamp too old (possible replay attack)");
            return false;
        }
    }

    let provided_sig = match headers
        .get("X-Slack-Signature")
        .and_then(|v| v.to_str().ok())
    {
        Some(s) => s.to_owned(),
        None => {
            warn!("Slack webhook: missing X-Slack-Signature");
            return false;
        }
    };

    let base_string = format!("v0:{}:{}", timestamp, String::from_utf8_lossy(body));
    let mut mac = match HmacSha256::new_from_slice(signing_secret.as_bytes()) {
        Ok(m) => m,
        Err(_) => return false,
    };
    mac.update(base_string.as_bytes());
    let expected = format!("v0={}", hex::encode(mac.finalize().into_bytes()));

    // Constant-time comparison.
    let expected_b = expected.as_bytes();
    let provided_b = provided_sig.as_bytes();
    if expected_b.len() != provided_b.len() {
        return false;
    }
    expected_b
        .iter()
        .zip(provided_b.iter())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

#[derive(Debug, Deserialize)]
struct SlackEventPayload {
    #[serde(rename = "type")]
    event_type: String,
    challenge: Option<String>,
    event: Option<SlackEvent>,
}

#[derive(Debug, Deserialize)]
struct SlackEvent {
    #[serde(rename = "type")]
    event_type: String,
    channel: Option<String>,
    text: Option<String>,
    user: Option<String>,
    #[serde(default)]
    bot_id: Option<String>,
    #[serde(default)]
    channel_type: Option<String>,
    #[serde(default)]
    files: Vec<SlackFile>,
}

#[derive(Debug, Deserialize, Clone)]
struct SlackFile {
    /// File ID from Slack API (used for completeness in deserialization).
    /// Note: File download uses `url_private` directly; upload uses
    /// `files.getUploadURLExternal`/`completeUploadExternal` flow.
    // Note: ID is required by Slack API schema for deserialization completeness.
    #[allow(dead_code)]
    id: String,
    mimetype: Option<String>,
    name: Option<String>,
    url_private: Option<String>,
    size: Option<u64>,
}

#[derive(Clone)]
struct AppState {
    sink: Arc<Mutex<Option<MessageSink>>>,
    sessions: Arc<Mutex<HashMap<String, SessionId>>>,
    signing_secret: Option<Arc<SecretStr>>,
}

async fn handle_event(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    // Verify Slack request signature when a signing secret is configured.
    if let Some(ref secret) = state.signing_secret {
        if !verify_slack_signature(&headers, &body, (**secret).expose()) {
            warn!("Slack webhook: signature verification failed, rejecting request");
            return axum::response::IntoResponse::into_response(
                axum::http::StatusCode::UNAUTHORIZED,
            );
        }
    }

    let payload: SlackEventPayload = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(e) => {
            warn!(%e, "Slack webhook: failed to parse event payload");
            return axum::response::IntoResponse::into_response(
                axum::http::StatusCode::BAD_REQUEST,
            );
        }
    };

    // URL verification challenge
    if payload.event_type == "url_verification"
        && let Some(challenge) = payload.challenge
    {
        return axum::response::IntoResponse::into_response(Json(
            serde_json::json!({ "challenge": challenge }),
        ));
    }

    if payload.event_type == "event_callback"
        && let Some(event) = payload.event
    {
        // Skip bot messages
        if event.bot_id.is_some() {
            return axum::response::IntoResponse::into_response(axum::http::StatusCode::OK);
        }

        if event.event_type == "message"
            && let Some(channel) = event.channel.clone()
        {
            let session_id = {
                let mut sessions = state.sessions.lock().await;
                *sessions
                    .entry(channel.clone())
                    .or_insert_with(SessionId::new)
            };

            let chat_type = match event.channel_type.as_deref() {
                Some("im") => "direct",
                _ => "group",
            };

            // Media files attached to the message
            for file in &event.files {
                let url = match &file.url_private {
                    Some(u) => u.clone(),
                    None => continue,
                };
                let mime = file
                    .mimetype
                    .clone()
                    .unwrap_or_else(|| "application/octet-stream".into());
                let caption = file.name.clone();

                let mut media = MediaPayload::new(mime, url);
                media.caption = caption;
                media.size_bytes = file.size;

                let mut envelope = Envelope::text("slack", session_id, "");
                envelope.payload = Payload::Media(media);
                envelope
                    .metadata
                    .insert("slack_channel".into(), serde_json::json!(channel));
                envelope
                    .metadata
                    .insert("chat_type".into(), serde_json::json!(chat_type));
                if let Some(ref user) = event.user {
                    envelope
                        .metadata
                        .insert("slack_user".into(), serde_json::json!(user));
                }

                let sink = state.sink.lock().await;
                if let Some(ref tx) = *sink
                    && tx.send(envelope).await.is_err()
                {
                    error!("Slack: sink closed");
                }
            }

            // Text message
            if let Some(text) = event.text {
                let mut envelope = Envelope::text("slack", session_id, &text);
                envelope
                    .metadata
                    .insert("slack_channel".into(), serde_json::json!(channel));
                if let Some(user) = event.user {
                    envelope
                        .metadata
                        .insert("slack_user".into(), serde_json::json!(user));
                }
                envelope
                    .metadata
                    .insert("chat_type".into(), serde_json::json!(chat_type));

                let sink = state.sink.lock().await;
                if let Some(ref tx) = *sink
                    && tx.send(envelope).await.is_err()
                {
                    error!("Slack: sink closed");
                }
            }
        }
    }

    axum::response::IntoResponse::into_response(axum::http::StatusCode::OK)
}

#[async_trait]
#[allow(clippy::unnecessary_literal_bound, clippy::too_many_lines)]
impl ChannelAdapter for SlackAdapter {
    fn channel_id(&self) -> &str {
        "slack"
    }

    async fn start(&self, sink: MessageSink) -> Result<()> {
        *self.sink.lock().await = Some(sink);

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        *self.shutdown.lock().await = Some(shutdown_tx);

        let state = AppState {
            sink: self.sink.clone(),
            sessions: self.sessions.clone(),
            signing_secret: self.signing_secret.clone(),
        };

        let state_for_restart = state.clone();
        let app = Router::new()
            .route("/slack/events", post(handle_event))
            .with_state(state);

        let addr = format!("0.0.0.0:{}", self.listen_port);
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .map_err(|e| Error::Adapter {
                source: Box::new(e),
                context: format!("failed to bind Slack event listener on {addr}"),
            })?;

        let listen_port = self.listen_port;
        tokio::spawn(async move {
            let mut reconnect_count: u32 = 0;
            let server = axum::serve(listener, app);
            tokio::select! {
                result = server.into_future() => {
                    if let Err(e) = result {
                        error!(%e, "Slack event server error, attempting restart");
                        loop {
                            let delay = backoff_delay(reconnect_count, 1, 60);
                            warn!(attempt = reconnect_count + 1, ?delay, "Slack server reconnecting");
                            tokio::time::sleep(delay).await;
                            reconnect_count = reconnect_count.saturating_add(1);
                            match tokio::net::TcpListener::bind(format!("0.0.0.0:{listen_port}")).await {
                                Ok(new_listener) => {
                                    let new_state = state_for_restart.clone();
                                    let new_app = Router::new()
                                        .route("/slack/events", post(handle_event))
                                        .with_state(new_state);
                                    info!("Slack server restarted");
                                    let _ = axum::serve(new_listener, new_app).into_future().await;
                                    break;
                                }
                                Err(e) => {
                                    error!(%e, "Slack rebind failed");
                                }
                            }
                        }
                    }
                }
                () = async {
                    let _ = shutdown_rx.await;
                } => {
                    info!("Slack adapter shutting down");
                }
            }
        });

        info!(
            port = self.listen_port,
            "Slack adapter started (Events API)"
        );
        Ok(())
    }

    async fn send(&self, msg: OutboundMessage) -> Result<()> {
        let channel = msg
            .metadata
            .get("slack_channel")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Adapter {
                source: Box::new(std::io::Error::other("missing slack_channel")),
                context: "missing slack_channel in outbound metadata".into(),
            })?;

        let auth = format!("Bearer {}", self.bot_token.expose());

        match &msg.payload {
            Payload::Text(text) => {
                let body = serde_json::json!({
                    "channel": channel,
                    "text": text,
                });
                let response = post_json_retried(
                    &self.client,
                    "https://slack.com/api/chat.postMessage",
                    &auth,
                    &body,
                    "Slack chat.postMessage failed",
                )
                .await?;
                if !response.status().is_success() {
                    let body = response.text().await.unwrap_or_default();
                    return Err(Error::Adapter {
                        source: Box::new(std::io::Error::other(body.clone())),
                        context: format!("Slack API error: {body}"),
                    });
                }
                debug!(channel, "sent text message to Slack");
            }
            Payload::Media(media) => {
                // Inline data (e.g. generated charts): always use file upload.
                // URL-based images: use Block Kit image block (lighter path).
                // URL-based non-images: download then file upload.
                if media.mime_type.starts_with("image/") && media.data_base64.is_none() {
                    // Image with URL → Block Kit image block
                    let blocks = serde_json::json!([{
                        "type": "image",
                        "image_url": media.url,
                        "alt_text": media.caption.as_deref().unwrap_or("image"),
                    }]);
                    let body = serde_json::json!({
                        "channel": channel,
                        "blocks": blocks,
                    });
                    let response = post_json_retried(
                        &self.client,
                        "https://slack.com/api/chat.postMessage",
                        &auth,
                        &body,
                        "Slack image block send failed",
                    )
                    .await?;
                    if !response.status().is_success() {
                        let body = response.text().await.unwrap_or_default();
                        return Err(Error::Adapter {
                            source: Box::new(std::io::Error::other(body.clone())),
                            context: format!("Slack API error (image): {body}"),
                        });
                    }
                    debug!(channel, "sent image block to Slack");
                } else {
                    // Inline data or non-image URL:
                    // files.getUploadURLExternal → upload → completeUploadExternal
                    let filename = media.caption.clone().unwrap_or_else(|| "attachment".into());
                    let file_bytes: Vec<u8> = if let Some(data) = media.decode_data() {
                        data
                    } else {
                        self.client
                            .get(&media.url)
                            .send()
                            .await
                            .map_err(|e| Error::Adapter {
                                source: Box::new(e),
                                context: "Slack media download failed".into(),
                            })?
                            .bytes()
                            .await
                            .map_err(|e| Error::Adapter {
                                source: Box::new(e),
                                context: "Slack media read failed".into(),
                            })?
                            .to_vec()
                    };

                    // Step 1: Get upload URL (with retry)
                    let url_resp: serde_json::Value = {
                        let mut last_err: Option<Error> = None;
                        let mut result: Option<serde_json::Value> = None;
                        for attempt in 0..3u32 {
                            if attempt > 0 {
                                let ms = 500u64.saturating_mul(1u64 << (attempt - 1));
                                tokio::time::sleep(std::time::Duration::from_millis(
                                    ms.min(10_000),
                                ))
                                .await;
                            }
                            match self
                                .client
                                .get("https://slack.com/api/files.getUploadURLExternal")
                                .header("Authorization", &auth)
                                .query(&[
                                    ("filename", &filename),
                                    ("length", &file_bytes.len().to_string()),
                                ])
                                .send()
                                .await
                            {
                                Err(e) if e.is_timeout() || e.is_connect() => {
                                    last_err = Some(Error::Adapter {
                                        source: Box::new(e),
                                        context: "Slack getUploadURLExternal failed (transient)"
                                            .into(),
                                    });
                                }
                                Err(e) => {
                                    return Err(Error::Adapter {
                                        source: Box::new(e),
                                        context: "Slack getUploadURLExternal failed".into(),
                                    });
                                }
                                Ok(r) if r.status() == 429 || r.status().is_server_error() => {
                                    let status = r.status();
                                    let text = r.text().await.unwrap_or_default();
                                    last_err = Some(Error::Adapter {
                                        source: Box::new(std::io::Error::other(text.clone())),
                                        context: format!(
                                            "Slack getUploadURLExternal HTTP {status}: {text}"
                                        ),
                                    });
                                }
                                Ok(r) => {
                                    let v: serde_json::Value =
                                        r.json().await.map_err(|e| Error::Adapter {
                                            source: Box::new(e),
                                            context: "Slack getUploadURLExternal parse failed"
                                                .into(),
                                        })?;
                                    result = Some(v);
                                    last_err = None;
                                    break;
                                }
                            }
                        }
                        result.ok_or_else(|| {
                            last_err.unwrap_or_else(|| Error::Adapter {
                                source: Box::new(std::io::Error::other("max retries exceeded")),
                                context: "Slack getUploadURLExternal max retries exceeded".into(),
                            })
                        })?
                    };

                    if url_resp["ok"].as_bool() != Some(true) {
                        return Err(Error::Adapter {
                            source: Box::new(std::io::Error::other(url_resp.to_string())),
                            context: "Slack getUploadURLExternal returned ok=false".into(),
                        });
                    }

                    let upload_url = url_resp["upload_url"].as_str().unwrap_or("").to_string();
                    let file_id = url_resp["file_id"].as_str().unwrap_or("").to_string();

                    // Step 2: Upload the file (with retry — clone bytes on each attempt)
                    let mut last_err: Option<Error> = None;
                    for attempt in 0..3u32 {
                        if attempt > 0 {
                            let ms = 500u64.saturating_mul(1u64 << (attempt - 1));
                            tokio::time::sleep(std::time::Duration::from_millis(ms.min(10_000)))
                                .await;
                        }
                        match self
                            .client
                            .post(&upload_url)
                            .body(file_bytes.clone())
                            .send()
                            .await
                        {
                            Err(e) if e.is_timeout() || e.is_connect() => {
                                last_err = Some(Error::Adapter {
                                    source: Box::new(e),
                                    context: "Slack file upload failed (transient)".into(),
                                });
                            }
                            Err(e) => {
                                return Err(Error::Adapter {
                                    source: Box::new(e),
                                    context: "Slack file upload failed".into(),
                                });
                            }
                            Ok(r) if r.status() == 429 || r.status().is_server_error() => {
                                let status = r.status();
                                let text = r.text().await.unwrap_or_default();
                                last_err = Some(Error::Adapter {
                                    source: Box::new(std::io::Error::other(text.clone())),
                                    context: format!("Slack file upload HTTP {status}: {text}"),
                                });
                            }
                            Ok(_) => {
                                last_err = None;
                                break;
                            }
                        }
                    }
                    if let Some(e) = last_err {
                        return Err(e);
                    }

                    // Step 3: Complete upload (with retry via helper)
                    let complete_body = serde_json::json!({
                        "files": [{ "id": file_id }],
                        "channel_id": channel,
                    });
                    let complete_resp: serde_json::Value = post_json_retried(
                        &self.client,
                        "https://slack.com/api/files.completeUploadExternal",
                        &auth,
                        &complete_body,
                        "Slack completeUploadExternal failed",
                    )
                    .await?
                    .json()
                    .await
                    .map_err(|e| Error::Adapter {
                        source: Box::new(e),
                        context: "Slack completeUploadExternal parse failed".into(),
                    })?;

                    if complete_resp["ok"].as_bool() != Some(true) {
                        return Err(Error::Adapter {
                            source: Box::new(std::io::Error::other(complete_resp.to_string())),
                            context: "Slack completeUploadExternal returned ok=false".into(),
                        });
                    }

                    debug!(channel, "uploaded file to Slack");
                }
            }
            _ => {
                warn!("Slack adapter: unsupported payload type, skipping");
            }
        }

        Ok(())
    }

    async fn register_commands(&self, _commands: &[(&str, &str)]) -> Result<()> {
        warn!(
            "Slack register_commands: Slack slash commands require app dashboard configuration and cannot be registered at runtime"
        );
        Ok(())
    }

    async fn shutdown(&self) -> Result<()> {
        if let Some(tx) = self.shutdown.lock().await.take() {
            let _ = tx.send(());
        }
        info!("Slack adapter shut down");
        Ok(())
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::field_reassign_with_default,
    clippy::default_trait_access,
    clippy::needless_pass_by_value,
    clippy::stable_sort_primitive
)]
mod tests {
    use serde_json::json;

    use super::*;

    fn make_adapter() -> SlackAdapter {
        SlackAdapter::new(SecretStr::new("xoxb-test-token"), None, 3000)
    }

    #[test]
    fn channel_id_returns_slack() {
        let adapter = make_adapter();
        assert_eq!(adapter.channel_id(), "slack");
    }

    #[tokio::test]
    async fn send_errors_when_slack_channel_missing() {
        let adapter = make_adapter();
        let msg = orka_core::types::OutboundMessage::text("slack", SessionId::new(), "hello", None);
        let err = adapter.send(msg).await.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("slack_channel"),
            "expected error about missing slack_channel, got: {msg}"
        );
    }

    #[test]
    fn deserialize_url_verification() {
        let raw = json!({
            "type": "url_verification",
            "challenge": "abc123",
        });
        let payload: SlackEventPayload = serde_json::from_value(raw).unwrap();
        assert_eq!(payload.event_type, "url_verification");
        assert_eq!(payload.challenge.as_deref(), Some("abc123"));
        assert!(payload.event.is_none());
    }

    #[test]
    fn deserialize_event_callback_message() {
        let raw = json!({
            "type": "event_callback",
            "event": {
                "type": "message",
                "channel": "C123",
                "text": "hello bot",
                "user": "U456",
            }
        });
        let payload: SlackEventPayload = serde_json::from_value(raw).unwrap();
        assert_eq!(payload.event_type, "event_callback");
        let event = payload.event.unwrap();
        assert_eq!(event.event_type, "message");
        assert_eq!(event.channel.as_deref(), Some("C123"));
        assert_eq!(event.text.as_deref(), Some("hello bot"));
        assert_eq!(event.user.as_deref(), Some("U456"));
        assert!(event.bot_id.is_none());
    }

    #[test]
    fn deserialize_event_callback_with_bot_id() {
        let raw = json!({
            "type": "event_callback",
            "event": {
                "type": "message",
                "channel": "C123",
                "text": "bot echo",
                "bot_id": "B789",
            }
        });
        let payload: SlackEventPayload = serde_json::from_value(raw).unwrap();
        let event = payload.event.unwrap();
        assert_eq!(event.bot_id.as_deref(), Some("B789"));
    }

    #[test]
    fn deserialize_event_with_channel_type() {
        let raw = json!({
            "type": "event_callback",
            "event": {
                "type": "message",
                "channel": "D999",
                "text": "dm text",
                "user": "U111",
                "channel_type": "im",
            }
        });
        let payload: SlackEventPayload = serde_json::from_value(raw).unwrap();
        let event = payload.event.unwrap();
        assert_eq!(event.channel_type.as_deref(), Some("im"));
    }

    #[test]
    fn deserialize_event_with_files() {
        let raw = json!({
            "type": "event_callback",
            "event": {
                "type": "message",
                "channel": "C123",
                "user": "U456",
                "files": [{
                    "id": "F123",
                    "mimetype": "application/pdf",
                    "name": "report.pdf",
                    "url_private": "https://files.slack.com/files/F123/report.pdf",
                    "size": 12345
                }]
            }
        });
        let payload: SlackEventPayload = serde_json::from_value(raw).unwrap();
        let event = payload.event.unwrap();
        assert_eq!(event.files.len(), 1);
        let file = &event.files[0];
        assert_eq!(file.id, "F123");
        assert_eq!(file.mimetype.as_deref(), Some("application/pdf"));
        assert_eq!(file.name.as_deref(), Some("report.pdf"));
        assert_eq!(file.size, Some(12345));
    }
}
