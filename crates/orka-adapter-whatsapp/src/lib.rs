//! `WhatsApp` Cloud API adapter for receiving and sending messages.

#![warn(missing_docs)]

mod api;
pub mod config;
mod types;
mod webhook;

use std::{collections::HashMap, future::IntoFuture, sync::Arc};

use async_trait::async_trait;
use axum::{Router, routing::get};
pub use config::WhatsAppAdapterConfig;
use orka_core::{
    Error, InteractionSink, Result, SecretStr,
    traits::ChannelAdapter,
    types::{OutboundMessage, SessionId, backoff_delay},
};
use reqwest::Client;
use tokio::sync::Mutex;
use tracing::{error, info, warn};
use types::AppState;
use webhook::{webhook_receive, webhook_verify};

/// Default Graph API version used when not overridden by the caller.
const DEFAULT_API_VERSION: &str = "v21.0";

/// `WhatsApp` Cloud API [`ChannelAdapter`] using webhook verification.
pub struct WhatsAppAdapter {
    access_token: Arc<SecretStr>,
    phone_number_id: String,
    verify_token: Arc<SecretStr>,
    /// App secret used to verify `X-Hub-Signature-256` on incoming webhooks.
    /// When `None`, signature verification is skipped and a warning is logged.
    app_secret: Option<Arc<SecretStr>>,
    api_version: String,
    client: Client,
    sink: Arc<Mutex<Option<InteractionSink>>>,
    shutdown: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    sessions: Arc<Mutex<HashMap<String, SessionId>>>,
    listen_port: u16,
}

impl WhatsAppAdapter {
    /// Create an adapter with the given Cloud API credentials and webhook port.
    pub fn new(
        access_token: SecretStr,
        phone_number_id: String,
        verify_token: SecretStr,
        app_secret: Option<SecretStr>,
        listen_port: u16,
    ) -> Self {
        if app_secret.is_none() {
            warn!(
                "WhatsApp app_secret not configured — incoming webhooks will not be authenticated"
            );
        }
        Self {
            access_token: Arc::new(access_token),
            phone_number_id,
            verify_token: Arc::new(verify_token),
            app_secret: app_secret.map(Arc::new),
            api_version: DEFAULT_API_VERSION.to_string(),
            client: Client::new(),
            sink: Arc::new(Mutex::new(None)),
            shutdown: Arc::new(Mutex::new(None)),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            listen_port,
        }
    }

    /// Override the Graph API version (default: `v21.0`).
    #[must_use]
    pub fn with_api_version(mut self, version: impl Into<String>) -> Self {
        self.api_version = version.into();
        self
    }
}

#[async_trait]
impl ChannelAdapter for WhatsAppAdapter {
    fn channel_id(&self) -> &'static str {
        "whatsapp"
    }

    async fn start(&self, sink: InteractionSink) -> Result<()> {
        *self.sink.lock().await = Some(sink);

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        *self.shutdown.lock().await = Some(shutdown_tx);

        let state = AppState {
            verify_token: self.verify_token.clone(),
            access_token: self.access_token.clone(),
            api_version: self.api_version.clone(),
            client: self.client.clone(),
            sink: self.sink.clone(),
            sessions: self.sessions.clone(),
            app_secret: self.app_secret.clone(),
            trust_level: self.trust_level(),
        };

        let state_for_restart = state.clone();
        let app = Router::new()
            .route("/webhook", get(webhook_verify).post(webhook_receive))
            .with_state(state);

        let addr = format!("0.0.0.0:{}", self.listen_port);
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .map_err(|e| Error::Adapter {
                source: Box::new(e),
                context: format!("failed to bind WhatsApp webhook on {addr}"),
            })?;

        let listen_port = self.listen_port;
        tokio::spawn(async move {
            let mut reconnect_count: u32 = 0;
            let server = axum::serve(listener, app);
            tokio::select! {
                result = server.into_future() => {
                    if let Err(e) = result {
                        error!(%e, "WhatsApp webhook server error, attempting restart");
                        loop {
                            let delay = backoff_delay(reconnect_count, 1, 60);
                            warn!(attempt = reconnect_count + 1, ?delay, "WhatsApp server reconnecting");
                            tokio::time::sleep(delay).await;
                            reconnect_count = reconnect_count.saturating_add(1);
                            match tokio::net::TcpListener::bind(format!("0.0.0.0:{listen_port}")).await {
                                Ok(new_listener) => {
                                    let new_state = state_for_restart.clone();
                                    let new_app = Router::new()
                                        .route("/webhook", get(webhook_verify).post(webhook_receive))
                                        .with_state(new_state);
                                    info!("WhatsApp server restarted");
                                    let _ = axum::serve(new_listener, new_app).into_future().await;
                                    break;
                                }
                                Err(e) => {
                                    error!(%e, "WhatsApp rebind failed");
                                }
                            }
                        }
                    }
                }
                () = async { let _ = shutdown_rx.await; } => {
                    info!("WhatsApp adapter shutting down");
                }
            }
        });

        info!(
            port = self.listen_port,
            "WhatsApp adapter started (Cloud API webhook)"
        );
        Ok(())
    }

    async fn send(&self, msg: OutboundMessage) -> Result<()> {
        api::send_message(
            &self.client,
            &self.access_token,
            &self.api_version,
            &self.phone_number_id,
            &msg,
        )
        .await
    }

    async fn shutdown(&self) -> Result<()> {
        if let Some(tx) = self.shutdown.lock().await.take() {
            let _ = tx.send(());
        }
        info!("WhatsApp adapter shut down");
        Ok(())
    }

    fn capabilities(&self) -> orka_core::CapabilitySet {
        use orka_core::Capability;
        [
            Capability::TextInbound,
            Capability::TextOutbound,
            Capability::MediaInbound,
            Capability::MediaOutbound,
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
    clippy::stable_sort_primitive,
    clippy::map_unwrap_or,
    clippy::unnecessary_literal_bound,
    clippy::redundant_closure_for_method_calls
)]
mod tests {
    use orka_core::types::{OutboundMessage, SessionId};

    use super::*;
    use crate::types::{WebhookPayload, WebhookValue, WebhookVerifyParams, WhatsAppMessage};

    fn make_adapter() -> WhatsAppAdapter {
        WhatsAppAdapter::new(
            SecretStr::new("test-token"),
            "123456".into(),
            SecretStr::new("verify-secret"),
            None,
            8080,
        )
    }

    #[test]
    fn channel_id_returns_whatsapp() {
        let adapter = make_adapter();
        assert_eq!(adapter.channel_id(), "whatsapp");
    }

    #[tokio::test]
    async fn send_errors_when_chat_id_missing() {
        let adapter = make_adapter();
        let msg = OutboundMessage::text("whatsapp", SessionId::new(), "hello", None);
        let err = adapter.send(msg).await.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("platform_context"),
            "expected error about missing platform_context.chat_id, got: {msg}"
        );
    }

    #[test]
    fn deserialize_webhook_payload_empty_entry() {
        let json = r#"{"entry": []}"#;
        let payload: WebhookPayload = serde_json::from_str(json).unwrap();
        assert!(payload.entry.unwrap().is_empty());
    }

    #[test]
    fn deserialize_webhook_payload_none_entry() {
        let json = r"{}";
        let payload: WebhookPayload = serde_json::from_str(json).unwrap();
        assert!(payload.entry.is_none());
    }

    #[test]
    fn deserialize_webhook_value_with_messages() {
        let json =
            r#"{"messages": [{"from": "15551234567", "type": "text", "text": {"body": "hi"}}]}"#;
        let value: WebhookValue = serde_json::from_str(json).unwrap();
        let msgs = value.messages.unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].from, "15551234567");
        assert_eq!(msgs[0].msg_type, "text");
        assert_eq!(msgs[0].text.as_ref().unwrap().body, "hi");
    }

    #[test]
    fn deserialize_whatsapp_message_image() {
        let json = r#"{"from": "15551234567", "type": "image", "image": {"id": "media123", "mime_type": "image/jpeg", "caption": "hello"}}"#;
        let msg: WhatsAppMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.msg_type, "image");
        let img = msg.image.unwrap();
        assert_eq!(img.id, "media123");
        assert_eq!(img.mime_type.as_deref(), Some("image/jpeg"));
        assert_eq!(img.caption.as_deref(), Some("hello"));
    }

    #[test]
    fn deserialize_whatsapp_message_without_text() {
        let json = r#"{"from": "15551234567", "type": "image"}"#;
        let msg: WhatsAppMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.from, "15551234567");
        assert_eq!(msg.msg_type, "image");
        assert!(msg.text.is_none());
    }

    #[test]
    fn deserialize_webhook_verify_params() {
        let json = r#"{"hub.mode": "subscribe", "hub.verify_token": "secret", "hub.challenge": "challenge123"}"#;
        let params: WebhookVerifyParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.mode.as_deref(), Some("subscribe"));
        assert_eq!(params.token.as_deref(), Some("secret"));
        assert_eq!(params.challenge.as_deref(), Some("challenge123"));
    }

    #[test]
    fn deserialize_full_webhook_payload() {
        let json = r#"{
            "entry": [{
                "changes": [{
                    "value": {
                        "messages": [{
                            "from": "15551234567",
                            "type": "text",
                            "text": {"body": "Hello from WhatsApp"}
                        }]
                    }
                }]
            }]
        }"#;
        let payload: WebhookPayload = serde_json::from_str(json).unwrap();
        let entry = &payload.entry.unwrap()[0];
        let change = &entry.changes.as_ref().unwrap()[0];
        let value = change.value.as_ref().unwrap();
        let msg = &value.messages.as_ref().unwrap()[0];
        assert_eq!(msg.from, "15551234567");
        assert_eq!(msg.text.as_ref().unwrap().body, "Hello from WhatsApp");
    }
}
