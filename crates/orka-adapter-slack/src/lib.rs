//! Slack Events API adapter for receiving and sending messages.

#![warn(missing_docs)]

mod api;
pub mod config;
mod types;
mod webhook;

use std::{collections::HashMap, future::IntoFuture, sync::Arc};

use async_trait::async_trait;
use axum::{Router, routing::post};
pub use config::SlackAdapterConfig;
use orka_core::{
    Error, InteractionSink, Result, SecretStr,
    traits::ChannelAdapter,
    types::{OutboundMessage, Payload, SessionId, backoff_delay},
};
use reqwest::Client;
use tokio::sync::Mutex;
use tracing::{error, info, warn};
use types::AppState;
use webhook::handle_event;

/// Slack Events API [`ChannelAdapter`] using HTTP webhooks.
pub struct SlackAdapter {
    bot_token: Arc<SecretStr>,
    /// Slack signing secret used to verify `X-Slack-Signature` on incoming
    /// events.  When `None`, signature verification is skipped and a warning
    /// is logged at startup.
    signing_secret: Option<Arc<SecretStr>>,
    client: Client,
    sink: Arc<Mutex<Option<InteractionSink>>>,
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

#[async_trait]
#[allow(clippy::unnecessary_literal_bound)]
impl ChannelAdapter for SlackAdapter {
    fn channel_id(&self) -> &str {
        "slack"
    }

    async fn start(&self, sink: InteractionSink) -> Result<()> {
        *self.sink.lock().await = Some(sink);

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        *self.shutdown.lock().await = Some(shutdown_tx);

        let state = AppState {
            sink: self.sink.clone(),
            sessions: self.sessions.clone(),
            signing_secret: self.signing_secret.clone(),
            trust_level: self.trust_level(),
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
                () = async { let _ = shutdown_rx.await; } => {
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
        let channel = msg.chat_id().map_err(|_| Error::Adapter {
            source: Box::new(std::io::Error::other("missing platform_context.chat_id")),
            context: "missing chat_id in platform_context".into(),
        })?;

        let auth = format!("Bearer {}", self.bot_token.expose());

        match &msg.payload {
            Payload::Text(text) => {
                api::send_text_message(&self.client, &auth, channel, text).await?;
            }
            Payload::Media(media) => {
                // Inline data or non-image URL → 3-step file upload.
                // URL-based images → Block Kit image block (lighter path).
                if media.mime_type.starts_with("image/") && media.data_base64.is_none() {
                    api::send_image_block(&self.client, &auth, channel, media).await?;
                } else {
                    api::send_file_upload(&self.client, &auth, channel, media).await?;
                }
            }
            _ => warn!("Slack adapter: unsupported payload type, skipping"),
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

    fn capabilities(&self) -> orka_core::CapabilitySet {
        use orka_core::Capability;
        [
            Capability::TextInbound,
            Capability::TextOutbound,
            Capability::StreamingDeltas,
            Capability::MediaInbound,
            Capability::MediaOutbound,
            Capability::Threading,
            Capability::RichText,
            Capability::FileUpload,
            Capability::WebhookPush,
        ]
        .into_iter()
        .collect()
    }

    fn integration_class(&self) -> orka_core::IntegrationClass {
        orka_core::IntegrationClass::MessagingChannel
    }

    fn trust_level(&self) -> orka_core::TrustLevel {
        orka_core::TrustLevel::VerifiedWebhook
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
    use crate::types::SlackEventPayload;

    fn make_adapter() -> SlackAdapter {
        SlackAdapter::new(SecretStr::new("xoxb-test-token"), None, 3000)
    }

    #[test]
    fn channel_id_returns_slack() {
        let adapter = make_adapter();
        assert_eq!(adapter.channel_id(), "slack");
    }

    #[tokio::test]
    async fn send_errors_when_chat_id_missing() {
        let adapter = make_adapter();
        let msg = orka_core::types::OutboundMessage::text("slack", SessionId::new(), "hello", None);
        let err = adapter.send(msg).await.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("platform_context"),
            "expected error about missing platform_context.chat_id, got: {msg}"
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
