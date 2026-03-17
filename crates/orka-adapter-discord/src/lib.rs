use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use orka_core::traits::ChannelAdapter;
use orka_core::types::{Envelope, MessageSink, OutboundMessage, Payload, SessionId, backoff_delay};
use orka_core::{Error, Result};
use reqwest::Client;
use serde::Deserialize;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

pub struct DiscordAdapter {
    bot_token: String,
    client: Client,
    sink: Arc<Mutex<Option<MessageSink>>>,
    shutdown: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    sessions: Arc<Mutex<HashMap<String, SessionId>>>,
}

impl DiscordAdapter {
    pub fn new(bot_token: String) -> Self {
        Self {
            bot_token,
            client: Client::new(),
            sink: Arc::new(Mutex::new(None)),
            shutdown: Arc::new(Mutex::new(None)),
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn api_url(path: &str) -> String {
        format!("https://discord.com/api/v10{path}")
    }
}

#[derive(Debug, Deserialize)]
struct GatewayResponse {
    url: String,
}

#[derive(Debug, Deserialize)]
struct GatewayEvent {
    _op: u8,
    t: Option<String>,
    s: Option<u64>,
    d: Option<serde_json::Value>,
}

#[async_trait]
impl ChannelAdapter for DiscordAdapter {
    fn channel_id(&self) -> &str {
        "discord"
    }

    async fn start(&self, sink: MessageSink) -> Result<()> {
        *self.sink.lock().await = Some(sink.clone());

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
        *self.shutdown.lock().await = Some(shutdown_tx);

        // Get gateway URL
        let gateway_resp: GatewayResponse = self
            .client
            .get(Self::api_url("/gateway/bot"))
            .header("Authorization", format!("Bot {}", self.bot_token))
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

        let ws_url = format!("{}/?v=10&encoding=json", gateway_resp.url);
        let bot_token = self.bot_token.clone();
        let sessions = self.sessions.clone();

        tokio::spawn(async move {
            let mut reconnect_count: u32 = 0;

            'reconnect: loop {
                if shutdown_rx.try_recv().is_ok() {
                    info!("Discord adapter shutting down");
                    break;
                }

                if reconnect_count > 0 {
                    let delay = backoff_delay(reconnect_count - 1, 1, 60);
                    warn!(attempt = reconnect_count, ?delay, "Discord reconnecting");
                    tokio::time::sleep(delay).await;
                }

                let ws_stream = match tokio_tungstenite::connect_async(&ws_url).await {
                    Ok((stream, _)) => stream,
                    Err(e) => {
                        error!(%e, "failed to connect to Discord gateway");
                        reconnect_count = reconnect_count.saturating_add(1);
                        continue 'reconnect;
                    }
                };

                let (mut write, mut read) = ws_stream.split();

                // Read Hello event
                let hello = match read.next().await {
                    Some(Ok(msg)) => {
                        let text = msg.to_text().unwrap_or("{}");
                        serde_json::from_str::<GatewayEvent>(text).ok()
                    }
                    _ => None,
                };

                let heartbeat_interval = hello
                    .and_then(|e| e.d.as_ref()?.get("heartbeat_interval")?.as_u64())
                    .unwrap_or(41250);

                // Send Identify
                let identify = serde_json::json!({
                    "op": 2,
                    "d": {
                        "token": bot_token,
                        "intents": 513, // GUILDS + GUILD_MESSAGES
                        "properties": {
                            "os": "linux",
                            "browser": "orka",
                            "device": "orka",
                        }
                    }
                });

                if let Err(e) = write
                    .send(tokio_tungstenite::tungstenite::Message::Text(
                        identify.to_string().into(),
                    ))
                    .await
                {
                    error!(%e, "failed to send Discord Identify");
                    reconnect_count = reconnect_count.saturating_add(1);
                    continue 'reconnect;
                }

                let mut sequence: Option<u64> = None;
                reconnect_count = 0;

                // Heartbeat task
                let write = Arc::new(Mutex::new(write));
                let write_clone = write.clone();
                let heartbeat_handle = tokio::spawn(async move {
                    let mut interval =
                        tokio::time::interval(std::time::Duration::from_millis(heartbeat_interval));
                    loop {
                        interval.tick().await;
                        let heartbeat = serde_json::json!({"op": 1, "d": null});
                        let mut w = write_clone.lock().await;
                        if w.send(tokio_tungstenite::tungstenite::Message::Text(
                            heartbeat.to_string().into(),
                        ))
                        .await
                        .is_err()
                        {
                            break;
                        }
                    }
                });

                loop {
                    tokio::select! {
                        _ = &mut shutdown_rx => {
                            info!("Discord adapter shutting down");
                            heartbeat_handle.abort();
                            break 'reconnect;
                        }
                        msg = read.next() => {
                            match msg {
                                Some(Ok(ws_msg)) => {
                                    let text = ws_msg.to_text().unwrap_or("{}");
                                    if let Ok(event) = serde_json::from_str::<GatewayEvent>(text) {
                                        if let Some(s) = event.s {
                                            sequence = Some(s);
                                        }

                                        // Handle MESSAGE_CREATE
                                        if event.t.as_deref() == Some("MESSAGE_CREATE")
                                            && let Some(ref d) = event.d {
                                                let content = d["content"].as_str().unwrap_or("");
                                                let channel_id = d["channel_id"].as_str().unwrap_or("");
                                                let is_bot = d["author"]["bot"].as_bool().unwrap_or(false);

                                                // Skip bot messages
                                                if is_bot || content.is_empty() {
                                                    continue;
                                                }

                                                let session_id = {
                                                    let mut s = sessions.lock().await;
                                                    s.entry(channel_id.to_string())
                                                        .or_insert_with(SessionId::new)
                                                        .clone()
                                                };

                                                let mut envelope = Envelope::text(
                                                    "discord",
                                                    session_id,
                                                    content,
                                                );
                                                envelope.metadata.insert(
                                                    "discord_channel_id".to_string(),
                                                    serde_json::json!(channel_id),
                                                );

                                                // guild_id present = server (group), absent = DM
                                                let chat_type = if d.get("guild_id").and_then(|v| v.as_str()).is_some() {
                                                    "group"
                                                } else {
                                                    "direct"
                                                };
                                                envelope.metadata.insert(
                                                    "chat_type".to_string(),
                                                    serde_json::json!(chat_type),
                                                );

                                                if sink.send(envelope).await.is_err() {
                                                    debug!("sink closed, stopping Discord listener");
                                                    heartbeat_handle.abort();
                                                    return;
                                                }
                                            }

                                        let _ = sequence; // suppress unused warning
                                    }
                                }
                                Some(Err(e)) => {
                                    error!(%e, "Discord WebSocket error");
                                    heartbeat_handle.abort();
                                    reconnect_count = reconnect_count.saturating_add(1);
                                    continue 'reconnect;
                                }
                                None => {
                                    warn!("Discord WebSocket closed");
                                    heartbeat_handle.abort();
                                    reconnect_count = reconnect_count.saturating_add(1);
                                    continue 'reconnect;
                                }
                            }
                        }
                    }
                }
            }
        });

        info!("Discord adapter started (WebSocket gateway)");
        Ok(())
    }

    async fn send(&self, msg: OutboundMessage) -> Result<()> {
        let channel_id = msg
            .metadata
            .get("discord_channel_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Adapter {
                source: Box::new(std::io::Error::other("missing discord_channel_id")),
                context: "missing discord_channel_id in outbound metadata".into(),
            })?;

        let text = match &msg.payload {
            Payload::Text(t) => t.clone(),
            _ => "[unsupported payload type]".into(),
        };

        let body = serde_json::json!({
            "content": text,
        });

        let response = self
            .client
            .post(Self::api_url(&format!("/channels/{channel_id}/messages")))
            .header("Authorization", format!("Bot {}", self.bot_token))
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Adapter {
                source: Box::new(e),
                context: "Discord send message failed".into(),
            })?;

        if !response.status().is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Adapter {
                source: Box::new(std::io::Error::other(body.clone())),
                context: format!("Discord API error: {body}"),
            });
        }

        debug!(channel_id, "sent message to Discord");
        Ok(())
    }

    async fn shutdown(&self) -> Result<()> {
        if let Some(tx) = self.shutdown.lock().await.take() {
            let _ = tx.send(());
        }
        info!("Discord adapter shut down");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orka_core::types::{OutboundMessage, Payload, SessionId};

    #[test]
    fn channel_id_returns_discord() {
        let adapter = DiscordAdapter::new("test-token".into());
        assert_eq!(adapter.channel_id(), "discord");
    }

    #[test]
    fn api_url_constructs_correct_url() {
        let url = DiscordAdapter::api_url("/gateway/bot");
        assert_eq!(url, "https://discord.com/api/v10/gateway/bot");

        let url = DiscordAdapter::api_url("/channels/123/messages");
        assert_eq!(url, "https://discord.com/api/v10/channels/123/messages");
    }

    #[tokio::test]
    async fn send_errors_when_discord_channel_id_missing() {
        let adapter = DiscordAdapter::new("test-token".into());
        let msg = OutboundMessage {
            channel: "discord".into(),
            session_id: SessionId::new(),
            payload: Payload::Text("hello".into()),
            reply_to: None,
            metadata: HashMap::new(),
        };
        let err = adapter.send(msg).await.unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("discord_channel_id"),
            "expected error about missing discord_channel_id, got: {msg}"
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
        let json = r#"{"_op": 10, "t": "READY", "s": 1, "d": {"heartbeat_interval": 41250}}"#;
        let event: GatewayEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event._op, 10);
        assert_eq!(event.t.as_deref(), Some("READY"));
        assert_eq!(event.s, Some(1));
        assert!(event.d.is_some());
    }

    #[test]
    fn deserialize_gateway_event_minimal() {
        let json = r#"{"_op": 0}"#;
        let event: GatewayEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event._op, 0);
        assert!(event.t.is_none());
        assert!(event.s.is_none());
        assert!(event.d.is_none());
    }
}
