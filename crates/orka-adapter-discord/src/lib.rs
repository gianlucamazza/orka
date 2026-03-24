//! Discord Gateway WebSocket adapter for receiving and sending messages.

#![warn(missing_docs)]

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use orka_core::{
    Error, Result,
    traits::ChannelAdapter,
    types::{
        CommandPayload, Envelope, MediaPayload, MessageSink, OutboundMessage, Payload, SessionId,
        backoff_delay,
    },
};
use reqwest::Client;
use serde::Deserialize;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

/// Discord Gateway [`ChannelAdapter`] using WebSocket and REST API.
pub struct DiscordAdapter {
    bot_token: String,
    application_id: Option<String>,
    client: Client,
    sink: Arc<Mutex<Option<MessageSink>>>,
    shutdown: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    sessions: Arc<Mutex<HashMap<String, SessionId>>>,
}

impl DiscordAdapter {
    /// Create an adapter with the given bot token and optional application ID.
    pub fn new(bot_token: String, application_id: Option<String>) -> Self {
        Self {
            bot_token,
            application_id,
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
    op: u8,
    t: Option<String>,
    s: Option<u64>,
    d: Option<serde_json::Value>,
}

/// Session resumption state tracked across reconnects.
struct ResumeState {
    session_id: Option<String>,
    resume_gateway_url: Option<String>,
    sequence: Option<u64>,
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

        let initial_ws_url = format!("{}/?v=10&encoding=json", gateway_resp.url);
        let bot_token = self.bot_token.clone();
        let sessions = self.sessions.clone();

        tokio::spawn(async move {
            let mut reconnect_count: u32 = 0;
            let mut resume = ResumeState {
                session_id: None,
                resume_gateway_url: None,
                sequence: None,
            };

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

                let ws_url = resume
                    .resume_gateway_url
                    .as_ref()
                    .map(|u| format!("{u}/?v=10&encoding=json"))
                    .unwrap_or_else(|| initial_ws_url.clone());

                let ws_stream = match tokio_tungstenite::connect_async(&ws_url).await {
                    Ok((stream, _)) => stream,
                    Err(e) => {
                        error!(%e, "failed to connect to Discord gateway");
                        reconnect_count = reconnect_count.saturating_add(1);
                        continue 'reconnect;
                    }
                };

                let (mut write, mut read) = ws_stream.split();

                // Read Hello (op 10)
                let heartbeat_interval = match read.next().await {
                    Some(Ok(msg)) => {
                        serde_json::from_str::<GatewayEvent>(msg.to_text().unwrap_or("{}"))
                            .ok()
                            .and_then(|e| e.d?.get("heartbeat_interval")?.as_u64())
                            .unwrap_or(41250)
                    }
                    _ => 41250,
                };

                // Resume (op 6) or Identify (op 2)
                let handshake = if let (Some(sid), Some(seq)) =
                    (resume.session_id.as_deref(), resume.sequence)
                {
                    serde_json::json!({
                        "op": 6,
                        "d": { "token": bot_token, "session_id": sid, "seq": seq }
                    })
                } else {
                    serde_json::json!({
                        "op": 2,
                        "d": {
                            "token": bot_token,
                            "intents": 33280, // GUILDS | GUILD_MESSAGES | MESSAGE_CONTENT
                            "properties": { "os": "linux", "browser": "orka", "device": "orka" }
                        }
                    })
                };

                if let Err(e) = write
                    .send(tokio_tungstenite::tungstenite::Message::Text(
                        handshake.to_string().into(),
                    ))
                    .await
                {
                    error!(%e, "failed to send Discord handshake");
                    reconnect_count = reconnect_count.saturating_add(1);
                    continue 'reconnect;
                }

                reconnect_count = 0;

                // Heartbeat task — includes last sequence number
                let write = Arc::new(Mutex::new(write));
                let write_hb = write.clone();
                let heartbeat_handle = tokio::spawn(async move {
                    let mut interval =
                        tokio::time::interval(std::time::Duration::from_millis(heartbeat_interval));
                    loop {
                        interval.tick().await;
                        let hb = serde_json::json!({"op": 1, "d": serde_json::Value::Null});
                        let mut w = write_hb.lock().await;
                        if w.send(tokio_tungstenite::tungstenite::Message::Text(
                            hb.to_string().into(),
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
                                    let Ok(event) = serde_json::from_str::<GatewayEvent>(text) else { continue };

                                    if let Some(s) = event.s {
                                        resume.sequence = Some(s);
                                    }

                                    match event.op {
                                        0 => {
                                            // Dispatch
                                            let Some(ref d) = event.d else { continue };
                                            match event.t.as_deref() {
                                                Some("READY") => {
                                                    resume.session_id = d["session_id"].as_str().map(String::from);
                                                    resume.resume_gateway_url = d["resume_gateway_url"].as_str().map(String::from);
                                                    info!("Discord READY");
                                                }
                                                Some("MESSAGE_CREATE") => {
                                                    let is_bot = d["author"]["bot"].as_bool().unwrap_or(false);
                                                    if is_bot { continue; }

                                                    let channel_id = d["channel_id"].as_str().unwrap_or("");
                                                    let session_id = {
                                                        let mut s = sessions.lock().await;
                                                        *s.entry(channel_id.to_string()).or_insert_with(SessionId::new)
                                                    };
                                                    let chat_type = if d.get("guild_id").and_then(|v| v.as_str()).is_some() { "group" } else { "direct" };

                                                    // Media attachments
                                                    if let Some(atts) = d["attachments"].as_array() {
                                                        for att in atts {
                                                            let url = att["url"].as_str().unwrap_or("").to_string();
                                                            let mime = att["content_type"].as_str().unwrap_or("application/octet-stream").to_string();
                                                            let size = att["size"].as_u64();
                                                            let filename = att["filename"].as_str().map(String::from);

                                                            let mut media = MediaPayload::new(mime, url);
                                                            media.caption = filename;
                                                            media.size_bytes = size;

                                                            let mut env = Envelope::text("discord", session_id, "");
                                                            env.payload = Payload::Media(media);
                                                            env.metadata.insert("discord_channel_id".into(), serde_json::json!(channel_id));
                                                            env.metadata.insert("chat_type".into(), serde_json::json!(chat_type));
                                                            if sink.send(env).await.is_err() {
                                                                heartbeat_handle.abort();
                                                                return;
                                                            }
                                                        }
                                                    }

                                                    let content = d["content"].as_str().unwrap_or("");
                                                    if content.is_empty() { continue; }

                                                    let mut envelope = Envelope::text("discord", session_id, content);
                                                    envelope.metadata.insert("discord_channel_id".into(), serde_json::json!(channel_id));
                                                    envelope.metadata.insert("chat_type".into(), serde_json::json!(chat_type));
                                                    if sink.send(envelope).await.is_err() {
                                                        debug!("sink closed, stopping Discord listener");
                                                        heartbeat_handle.abort();
                                                        return;
                                                    }
                                                }
                                                Some("INTERACTION_CREATE") => {
                                                    // APPLICATION_COMMAND (type 2)
                                                    if d["type"].as_u64() != Some(2) { continue; }

                                                    let channel_id = d["channel_id"].as_str().unwrap_or("");
                                                    let cmd_name = d["data"]["name"].as_str().unwrap_or("").to_string();
                                                    let mut args = HashMap::new();
                                                    if let Some(opts) = d["data"]["options"].as_array() {
                                                        for opt in opts {
                                                            if let Some(name) = opt["name"].as_str() {
                                                                args.insert(name.to_string(), opt["value"].clone());
                                                            }
                                                        }
                                                    }

                                                    let session_id = {
                                                        let mut s = sessions.lock().await;
                                                        *s.entry(channel_id.to_string()).or_insert_with(SessionId::new)
                                                    };

                                                    let mut envelope = Envelope::text("discord", session_id, "");
                                                    envelope.payload = Payload::Command(CommandPayload::new(cmd_name, args));
                                                    envelope.metadata.insert("discord_channel_id".into(), serde_json::json!(channel_id));
                                                    envelope.metadata.insert("discord_interaction_id".into(), d["id"].clone());
                                                    envelope.metadata.insert("discord_interaction_token".into(), d["token"].clone());

                                                    if sink.send(envelope).await.is_err() {
                                                        heartbeat_handle.abort();
                                                        return;
                                                    }
                                                }
                                                _ => {}
                                            }
                                        }
                                        7 => {
                                            // Reconnect
                                            warn!("Discord requested reconnect (op 7)");
                                            heartbeat_handle.abort();
                                            reconnect_count = reconnect_count.saturating_add(1);
                                            continue 'reconnect;
                                        }
                                        9 => {
                                            // Invalid Session
                                            let resumable = event.d.as_ref().and_then(|d| d.as_bool()).unwrap_or(false);
                                            warn!(resumable, "Discord Invalid Session (op 9)");
                                            if !resumable {
                                                resume.session_id = None;
                                                resume.resume_gateway_url = None;
                                                resume.sequence = None;
                                            }
                                            heartbeat_handle.abort();
                                            reconnect_count = reconnect_count.saturating_add(1);
                                            continue 'reconnect;
                                        }
                                        11 => {} // Heartbeat ACK
                                        _ => {}
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

        match &msg.payload {
            Payload::Text(text) => {
                let body = serde_json::json!({ "content": text });
                let resp = self
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
                if !resp.status().is_success() {
                    let body = resp.text().await.unwrap_or_default();
                    return Err(Error::Adapter {
                        source: Box::new(std::io::Error::other(body.clone())),
                        context: format!("Discord API error: {body}"),
                    });
                }
                debug!(channel_id, "sent text message to Discord");
            }
            Payload::Media(media) => {
                let bytes = self
                    .client
                    .get(&media.url)
                    .send()
                    .await
                    .map_err(|e| Error::Adapter {
                        source: Box::new(e),
                        context: "Discord media download failed".into(),
                    })?
                    .bytes()
                    .await
                    .map_err(|e| Error::Adapter {
                        source: Box::new(e),
                        context: "Discord media read failed".into(),
                    })?;

                let filename = media.caption.clone().unwrap_or_else(|| "attachment".into());
                let part = reqwest::multipart::Part::bytes(bytes.to_vec())
                    .file_name(filename)
                    .mime_str(&media.mime_type)
                    .map_err(|e| Error::Adapter {
                        source: Box::new(e),
                        context: "Discord multipart MIME error".into(),
                    })?;

                let form = reqwest::multipart::Form::new().part("files[0]", part);
                let resp = self
                    .client
                    .post(Self::api_url(&format!("/channels/{channel_id}/messages")))
                    .header("Authorization", format!("Bot {}", self.bot_token))
                    .multipart(form)
                    .send()
                    .await
                    .map_err(|e| Error::Adapter {
                        source: Box::new(e),
                        context: "Discord media send failed".into(),
                    })?;
                if !resp.status().is_success() {
                    let body = resp.text().await.unwrap_or_default();
                    return Err(Error::Adapter {
                        source: Box::new(std::io::Error::other(body.clone())),
                        context: format!("Discord API error (media): {body}"),
                    });
                }
                debug!(channel_id, "sent media to Discord");
            }
            _ => {
                warn!("Discord adapter: unsupported payload type, skipping");
            }
        }

        Ok(())
    }

    async fn register_commands(&self, commands: &[(&str, &str)]) -> Result<()> {
        let app_id = match &self.application_id {
            Some(id) => id.clone(),
            None => {
                warn!("Discord register_commands: application_id not configured, skipping");
                return Ok(());
            }
        };

        let cmds: Vec<serde_json::Value> = commands
            .iter()
            .map(|(name, description)| {
                serde_json::json!({ "name": name, "description": description, "type": 1 })
            })
            .collect();

        let resp = self
            .client
            .put(Self::api_url(&format!("/applications/{app_id}/commands")))
            .header("Authorization", format!("Bot {}", self.bot_token))
            .json(&cmds)
            .send()
            .await
            .map_err(|e| Error::Adapter {
                source: Box::new(e),
                context: "Discord register_commands failed".into(),
            })?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Adapter {
                source: Box::new(std::io::Error::other(body.clone())),
                context: format!("Discord register_commands API error: {body}"),
            });
        }

        info!(
            count = commands.len(),
            "Discord: registered global slash commands"
        );
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
    use orka_core::types::{OutboundMessage, SessionId};

    use super::*;

    #[test]
    fn channel_id_returns_discord() {
        let adapter = DiscordAdapter::new("test-token".into(), None);
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
        let adapter = DiscordAdapter::new("test-token".into(), None);
        let msg = OutboundMessage::text("discord", SessionId::new(), "hello", None);
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
