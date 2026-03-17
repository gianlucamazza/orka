use std::collections::HashMap;
use std::future::IntoFuture;
use std::sync::Arc;

use async_trait::async_trait;
use axum::{Json, Router, extract::State, routing::post};
use orka_core::traits::ChannelAdapter;
use orka_core::types::{Envelope, MessageSink, OutboundMessage, Payload, SessionId, backoff_delay};
use orka_core::{Error, Result};
use reqwest::Client;
use serde::Deserialize;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

pub struct SlackAdapter {
    bot_token: String,
    client: Client,
    sink: Arc<Mutex<Option<MessageSink>>>,
    shutdown: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    sessions: Arc<Mutex<HashMap<String, SessionId>>>,
    listen_port: u16,
}

impl SlackAdapter {
    pub fn new(bot_token: String, listen_port: u16) -> Self {
        Self {
            bot_token,
            client: Client::new(),
            sink: Arc::new(Mutex::new(None)),
            shutdown: Arc::new(Mutex::new(None)),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            listen_port,
        }
    }
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
}

#[derive(Clone)]
struct AppState {
    sink: Arc<Mutex<Option<MessageSink>>>,
    sessions: Arc<Mutex<HashMap<String, SessionId>>>,
}

async fn handle_event(
    State(state): State<AppState>,
    Json(payload): Json<SlackEventPayload>,
) -> axum::response::Response {
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
            && let (Some(channel), Some(text)) = (event.channel, event.text)
        {
            let session_id = {
                let mut sessions = state.sessions.lock().await;
                sessions
                    .entry(channel.clone())
                    .or_insert_with(SessionId::new)
                    .clone()
            };

            let mut envelope = Envelope::text("slack", session_id, &text);
            envelope
                .metadata
                .insert("slack_channel".to_string(), serde_json::json!(channel));
            if let Some(user) = event.user {
                envelope
                    .metadata
                    .insert("slack_user".to_string(), serde_json::json!(user));
            }

            // "im" = DM, anything else = group
            let chat_type = match event.channel_type.as_deref() {
                Some("im") => "direct",
                _ => "group",
            };
            envelope
                .metadata
                .insert("chat_type".to_string(), serde_json::json!(chat_type));

            let sink = state.sink.lock().await;
            if let Some(ref tx) = *sink
                && tx.send(envelope).await.is_err()
            {
                error!("Slack: sink closed");
            }
        }
    }

    axum::response::IntoResponse::into_response(axum::http::StatusCode::OK)
}

#[async_trait]
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
                _ = async {
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

        let text = match &msg.payload {
            Payload::Text(t) => t.clone(),
            _ => "[unsupported payload type]".into(),
        };

        let body = serde_json::json!({
            "channel": channel,
            "text": text,
        });

        let response = self
            .client
            .post("https://slack.com/api/chat.postMessage")
            .header("Authorization", format!("Bearer {}", self.bot_token))
            .header("Content-Type", "application/json; charset=utf-8")
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Adapter {
                source: Box::new(e),
                context: "Slack chat.postMessage failed".into(),
            })?;

        if !response.status().is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Adapter {
                source: Box::new(std::io::Error::other(body.clone())),
                context: format!("Slack API error: {body}"),
            });
        }

        debug!(channel, "sent message to Slack");
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
mod tests {
    use super::*;
    use orka_core::types::Payload;
    use serde_json::json;

    fn make_adapter() -> SlackAdapter {
        SlackAdapter::new("xoxb-test-token".into(), 3000)
    }

    #[test]
    fn channel_id_returns_slack() {
        let adapter = make_adapter();
        assert_eq!(adapter.channel_id(), "slack");
    }

    #[tokio::test]
    async fn send_errors_when_slack_channel_missing() {
        let adapter = make_adapter();
        let msg = OutboundMessage {
            channel: "slack".into(),
            session_id: SessionId::new(),
            payload: Payload::Text("hello".into()),
            reply_to: None,
            metadata: HashMap::new(),
        };
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
        assert!(payload.challenge.is_none());

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
}
