//! Internal `WhatsApp` webhook payload types.

use std::{collections::HashMap, sync::Arc};

use orka_core::{InteractionSink, SecretStr, types::SessionId};
use reqwest::Client;
use serde::Deserialize;
use tokio::sync::Mutex;

/// HMAC-SHA256 for signature verification.
pub(crate) type HmacSha256 = hmac::Hmac<sha2::Sha256>;

/// Query parameters for the `GET /webhook` verification handshake.
#[derive(Debug, Deserialize)]
pub(crate) struct WebhookVerifyParams {
    #[serde(rename = "hub.mode")]
    pub(crate) mode: Option<String>,
    #[serde(rename = "hub.verify_token")]
    pub(crate) token: Option<String>,
    #[serde(rename = "hub.challenge")]
    pub(crate) challenge: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct WebhookPayload {
    pub(crate) entry: Option<Vec<WebhookEntry>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct WebhookEntry {
    pub(crate) changes: Option<Vec<WebhookChange>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct WebhookChange {
    pub(crate) value: Option<WebhookValue>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct WebhookValue {
    pub(crate) messages: Option<Vec<WhatsAppMessage>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct WhatsAppMessage {
    pub(crate) from: String,
    #[serde(rename = "type")]
    pub(crate) msg_type: String,
    pub(crate) text: Option<WhatsAppText>,
    pub(crate) image: Option<WhatsAppMedia>,
    pub(crate) video: Option<WhatsAppMedia>,
    pub(crate) audio: Option<WhatsAppMedia>,
    pub(crate) document: Option<WhatsAppMedia>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct WhatsAppText {
    pub(crate) body: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct WhatsAppMedia {
    pub(crate) id: String,
    pub(crate) mime_type: Option<String>,
    pub(crate) caption: Option<String>,
}

/// Axum shared state for webhook handlers.
#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) verify_token: Arc<SecretStr>,
    pub(crate) access_token: Arc<SecretStr>,
    pub(crate) api_version: String,
    pub(crate) client: Client,
    pub(crate) sink: Arc<Mutex<Option<InteractionSink>>>,
    pub(crate) sessions: Arc<Mutex<HashMap<String, SessionId>>>,
    pub(crate) app_secret: Option<Arc<SecretStr>>,
    pub(crate) trust_level: orka_contracts::TrustLevel,
}
