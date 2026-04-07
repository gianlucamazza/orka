//! Discord Gateway WebSocket adapter for receiving and sending messages.

#![warn(missing_docs)]

mod api;
pub mod config;
mod gateway;
mod types;

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
pub use config::DiscordAdapterConfig;
use orka_core::{
    Error, InteractionSink, Result, SecretStr,
    traits::ChannelAdapter,
    types::{OutboundMessage, SessionId},
};
use reqwest::Client;
use tokio::sync::Mutex;
use tracing::info;
use types::GatewayResponse;

/// Discord Gateway [`ChannelAdapter`] using WebSocket and REST API.
pub struct DiscordAdapter {
    bot_token: Arc<SecretStr>,
    application_id: Option<String>,
    client: Client,
    sink: Arc<Mutex<Option<InteractionSink>>>,
    shutdown: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    sessions: Arc<Mutex<HashMap<String, SessionId>>>,
}

impl DiscordAdapter {
    /// Create an adapter with the given bot token and optional application ID.
    pub fn new(bot_token: SecretStr, application_id: Option<String>) -> Self {
        Self {
            bot_token: Arc::new(bot_token),
            application_id,
            client: Client::new(),
            sink: Arc::new(Mutex::new(None)),
            shutdown: Arc::new(Mutex::new(None)),
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[async_trait]
#[allow(clippy::unnecessary_literal_bound)]
impl ChannelAdapter for DiscordAdapter {
    fn channel_id(&self) -> &str {
        "discord"
    }

    async fn start(&self, sink: InteractionSink) -> Result<()> {
        *self.sink.lock().await = Some(sink.clone());

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        *self.shutdown.lock().await = Some(shutdown_tx);

        let gateway_resp: GatewayResponse = self
            .client
            .get(api::api_url("/gateway/bot"))
            .header("Authorization", format!("Bot {}", self.bot_token.expose()))
            .send()
            .await
            .map_err(|e| Error::Adapter {
                source: Box::new(e),
                context: "failed to get Discord gateway URL".into(),
            })?
            .json()
            .await
            .map_err(|e| Error::Adapter {
                source: Box::new(e),
                context: "failed to parse Discord gateway response".into(),
            })?;

        let initial_ws_url = format!("{}/?v=10&encoding=json", gateway_resp.url);
        let trust_level = self.trust_level();
        tokio::spawn(gateway::run_gateway(
            initial_ws_url,
            Arc::clone(&self.bot_token),
            self.sessions.clone(),
            self.client.clone(),
            shutdown_rx,
            sink,
            trust_level,
        ));

        info!("Discord adapter started (WebSocket gateway)");
        Ok(())
    }

    async fn send(&self, msg: OutboundMessage) -> Result<()> {
        let channel_id = msg
            .chat_id()
            .map_err(|_| Error::Adapter {
                source: Box::new(std::io::Error::other("missing platform_context.chat_id")),
                context: "missing chat_id in platform_context".into(),
            })?
            .to_owned();

        api::send_message(&self.client, &self.bot_token, &channel_id, &msg).await
    }

    async fn register_commands(&self, commands: &[(&str, &str)]) -> Result<()> {
        let Some(app_id) = self.application_id.clone() else {
            tracing::warn!("Discord register_commands: application_id not configured, skipping");
            return Ok(());
        };
        api::register_commands(&self.client, &self.bot_token, &app_id, commands).await
    }

    async fn shutdown(&self) -> Result<()> {
        if let Some(tx) = self.shutdown.lock().await.take() {
            let _ = tx.send(());
        }
        info!("Discord adapter shut down");
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
            Capability::SlashCommands,
            Capability::Threading,
            Capability::RichText,
            Capability::WebsocketBidirectional,
        ]
        .into_iter()
        .collect()
    }

    fn integration_class(&self) -> orka_core::IntegrationClass {
        orka_core::IntegrationClass::MessagingChannel
    }

    fn trust_level(&self) -> orka_core::TrustLevel {
        orka_core::TrustLevel::BotToken
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
    use orka_core::types::{OutboundMessage, SessionId};

    use super::*;
    use crate::types::{GatewayEvent, GatewayResponse};

    #[test]
    fn channel_id_returns_discord() {
        let adapter = DiscordAdapter::new(SecretStr::new("test-token"), None);
        assert_eq!(adapter.channel_id(), "discord");
    }

    #[test]
    fn api_url_constructs_correct_url() {
        let url = api::api_url("/gateway/bot");
        assert_eq!(url, "https://discord.com/api/v10/gateway/bot");
        let url = api::api_url("/channels/123/messages");
        assert_eq!(url, "https://discord.com/api/v10/channels/123/messages");
    }

    #[tokio::test]
    async fn send_errors_when_chat_id_missing() {
        let adapter = DiscordAdapter::new(SecretStr::new("test-token"), None);
        let msg = OutboundMessage::text("discord", SessionId::new(), "hello", None);
        let err = adapter.send(msg).await.unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("platform_context"),
            "expected error about missing platform_context.chat_id, got: {msg}"
        );
    }

    #[test]
    fn deserialize_gateway_response() {
        let json = r#"{"url": "wss://gateway.discord.gg"}"#;
        let resp: GatewayResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.url, "wss://gateway.discord.gg");
    }

    #[test]
    fn deserialize_gateway_event() {
        let json = r#"{"op": 10, "t": "READY", "s": 1, "d": {"heartbeat_interval": 41250}}"#;
        let event: GatewayEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.op, 10);
        assert_eq!(event.t.as_deref(), Some("READY"));
        assert_eq!(event.s, Some(1));
        assert!(event.d.is_some());
    }

    #[test]
    fn deserialize_gateway_event_minimal() {
        let json = r#"{"op": 0}"#;
        let event: GatewayEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.op, 0);
        assert!(event.t.is_none());
        assert!(event.s.is_none());
        assert!(event.d.is_none());
    }
}
