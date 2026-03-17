use std::collections::HashMap;
use std::future::IntoFuture;
use std::sync::Arc;

use async_trait::async_trait;
use axum::{
    Json, Router,
    extract::{Query, State},
    routing::get,
};
use orka_core::traits::ChannelAdapter;
use orka_core::types::{Envelope, MessageSink, OutboundMessage, Payload, SessionId, backoff_delay};
use orka_core::{Error, Result};
use reqwest::Client;
use serde::Deserialize;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

pub struct WhatsAppAdapter {
    access_token: String,
    phone_number_id: String,
    verify_token: String,
    client: Client,
    sink: Arc<Mutex<Option<MessageSink>>>,
    shutdown: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    sessions: Arc<Mutex<HashMap<String, SessionId>>>,
    listen_port: u16,
}

impl WhatsAppAdapter {
    pub fn new(
        access_token: String,
        phone_number_id: String,
        verify_token: String,
        listen_port: u16,
    ) -> Self {
        Self {
            access_token,
            phone_number_id,
            verify_token,
            client: Client::new(),
            sink: Arc::new(Mutex::new(None)),
            shutdown: Arc::new(Mutex::new(None)),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            listen_port,
        }
    }
}

#[derive(Debug, Deserialize)]
struct WebhookVerifyParams {
    #[serde(rename = "hub.mode")]
    mode: Option<String>,
    #[serde(rename = "hub.verify_token")]
    token: Option<String>,
    #[serde(rename = "hub.challenge")]
    challenge: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WebhookPayload {
    entry: Option<Vec<WebhookEntry>>,
}

#[derive(Debug, Deserialize)]
struct WebhookEntry {
    changes: Option<Vec<WebhookChange>>,
}

#[derive(Debug, Deserialize)]
struct WebhookChange {
    value: Option<WebhookValue>,
}

#[derive(Debug, Deserialize)]
struct WebhookValue {
    messages: Option<Vec<WhatsAppMessage>>,
}

#[derive(Debug, Deserialize)]
struct WhatsAppMessage {
    from: String,
    #[serde(rename = "type")]
    msg_type: String,
    text: Option<WhatsAppText>,
}

#[derive(Debug, Deserialize)]
struct WhatsAppText {
    body: String,
}

#[derive(Clone)]
struct AppState {
    verify_token: String,
    sink: Arc<Mutex<Option<MessageSink>>>,
    sessions: Arc<Mutex<HashMap<String, SessionId>>>,
}

async fn webhook_verify(
    State(state): State<AppState>,
    Query(params): Query<WebhookVerifyParams>,
) -> axum::response::Response {
    if params.mode.as_deref() == Some("subscribe")
        && params.token.as_deref() == Some(&state.verify_token)
        && let Some(challenge) = params.challenge
    {
        return axum::response::IntoResponse::into_response(challenge);
    }
    axum::response::IntoResponse::into_response(axum::http::StatusCode::FORBIDDEN)
}

async fn webhook_receive(
    State(state): State<AppState>,
    Json(payload): Json<WebhookPayload>,
) -> axum::http::StatusCode {
    if let Some(entries) = payload.entry {
        for entry in entries {
            if let Some(changes) = entry.changes {
                for change in changes {
                    if let Some(value) = change.value
                        && let Some(messages) = value.messages
                    {
                        for msg in messages {
                            if msg.msg_type != "text" {
                                continue;
                            }
                            if let Some(text) = msg.text {
                                let session_id = {
                                    let mut sessions = state.sessions.lock().await;
                                    sessions
                                        .entry(msg.from.clone())
                                        .or_insert_with(SessionId::new)
                                        .clone()
                                };

                                let mut envelope =
                                    Envelope::text("whatsapp", session_id, &text.body);
                                envelope.metadata.insert(
                                    "whatsapp_from".to_string(),
                                    serde_json::json!(msg.from),
                                );
                                envelope
                                    .metadata
                                    .insert("chat_type".to_string(), serde_json::json!("direct"));

                                let sink = state.sink.lock().await;
                                if let Some(ref tx) = *sink
                                    && tx.send(envelope).await.is_err()
                                {
                                    error!("WhatsApp: sink closed");
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    axum::http::StatusCode::OK
}

#[async_trait]
impl ChannelAdapter for WhatsAppAdapter {
    fn channel_id(&self) -> &str {
        "whatsapp"
    }

    async fn start(&self, sink: MessageSink) -> Result<()> {
        *self.sink.lock().await = Some(sink);

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        *self.shutdown.lock().await = Some(shutdown_tx);

        let state = AppState {
            verify_token: self.verify_token.clone(),
            sink: self.sink.clone(),
            sessions: self.sessions.clone(),
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
                _ = async {
                    let _ = shutdown_rx.await;
                } => {
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
        let to = msg
            .metadata
            .get("whatsapp_from")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Adapter {
                source: Box::new(std::io::Error::other("missing whatsapp_from")),
                context: "missing whatsapp_from in outbound metadata".into(),
            })?;

        let text = match &msg.payload {
            Payload::Text(t) => t.clone(),
            _ => "[unsupported payload type]".into(),
        };

        let url = format!(
            "https://graph.facebook.com/v18.0/{}/messages",
            self.phone_number_id
        );

        let body = serde_json::json!({
            "messaging_product": "whatsapp",
            "to": to,
            "type": "text",
            "text": { "body": text },
        });

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.access_token))
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Adapter {
                source: Box::new(e),
                context: "WhatsApp send message failed".into(),
            })?;

        if !response.status().is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Adapter {
                source: Box::new(std::io::Error::other(body.clone())),
                context: format!("WhatsApp API error: {body}"),
            });
        }

        debug!(to, "sent message via WhatsApp");
        Ok(())
    }

    async fn shutdown(&self) -> Result<()> {
        if let Some(tx) = self.shutdown.lock().await.take() {
            let _ = tx.send(());
        }
        info!("WhatsApp adapter shut down");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orka_core::types::{OutboundMessage, Payload, SessionId};
    use std::collections::HashMap;

    fn make_adapter() -> WhatsAppAdapter {
        WhatsAppAdapter::new(
            "test-token".into(),
            "123456".into(),
            "verify-secret".into(),
            8080,
        )
    }

    #[test]
    fn channel_id_returns_whatsapp() {
        let adapter = make_adapter();
        assert_eq!(adapter.channel_id(), "whatsapp");
    }

    #[tokio::test]
    async fn send_errors_when_whatsapp_from_missing() {
        let adapter = make_adapter();
        let msg = OutboundMessage {
            channel: "whatsapp".into(),
            session_id: SessionId::new(),
            payload: Payload::Text("hello".into()),
            reply_to: None,
            metadata: HashMap::new(),
        };
        let err = adapter.send(msg).await.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("whatsapp_from"),
            "expected error about whatsapp_from, got: {msg}"
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
        let json = r#"{}"#;
        let payload: WebhookPayload = serde_json::from_str(json).unwrap();
        assert!(payload.entry.is_none());
    }

    #[test]
    fn deserialize_webhook_entry() {
        let json = r#"{"changes": []}"#;
        let entry: WebhookEntry = serde_json::from_str(json).unwrap();
        assert!(entry.changes.unwrap().is_empty());
    }

    #[test]
    fn deserialize_webhook_change() {
        let json = r#"{"value": {"messages": []}}"#;
        let change: WebhookChange = serde_json::from_str(json).unwrap();
        assert!(change.value.unwrap().messages.unwrap().is_empty());
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
    fn deserialize_whatsapp_message_without_text() {
        let json = r#"{"from": "15551234567", "type": "image"}"#;
        let msg: WhatsAppMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.from, "15551234567");
        assert_eq!(msg.msg_type, "image");
        assert!(msg.text.is_none());
    }

    #[test]
    fn deserialize_whatsapp_text() {
        let json = r#"{"body": "hello world"}"#;
        let text: WhatsAppText = serde_json::from_str(json).unwrap();
        assert_eq!(text.body, "hello world");
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
    fn deserialize_webhook_verify_params_partial() {
        let json = r#"{"hub.mode": "subscribe"}"#;
        let params: WebhookVerifyParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.mode.as_deref(), Some("subscribe"));
        assert!(params.token.is_none());
        assert!(params.challenge.is_none());
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
