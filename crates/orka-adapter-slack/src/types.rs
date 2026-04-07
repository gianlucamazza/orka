//! Internal Slack Events API payload types and shared axum state.

use std::{collections::HashMap, sync::Arc};

use orka_core::{types::SessionId, InteractionSink, SecretStr};
use serde::Deserialize;
use tokio::sync::Mutex;

/// HMAC-SHA256 for Slack signature verification.
pub(crate) type HmacSha256 = hmac::Hmac<sha2::Sha256>;

#[derive(Debug, Deserialize)]
pub(crate) struct SlackEventPayload {
    #[serde(rename = "type")]
    pub(crate) event_type: String,
    pub(crate) challenge: Option<String>,
    pub(crate) event: Option<SlackEvent>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SlackEvent {
    #[serde(rename = "type")]
    pub(crate) event_type: String,
    pub(crate) channel: Option<String>,
    pub(crate) text: Option<String>,
    pub(crate) user: Option<String>,
    #[serde(default)]
    pub(crate) bot_id: Option<String>,
    #[serde(default)]
    pub(crate) channel_type: Option<String>,
    #[serde(default)]
    pub(crate) files: Vec<SlackFile>,
}

#[derive(Debug, Deserialize, Clone)]
pub(crate) struct SlackFile {
    /// File ID from Slack API (used for completeness in deserialization).
    /// Note: File download uses `url_private` directly; upload uses
    /// `files.getUploadURLExternal`/`completeUploadExternal` flow.
    // Note: ID is required by Slack API schema for deserialization completeness.
    #[allow(dead_code)]
    pub(crate) id: String,
    pub(crate) mimetype: Option<String>,
    pub(crate) name: Option<String>,
    pub(crate) url_private: Option<String>,
    pub(crate) size: Option<u64>,
}

/// Axum shared state for Slack webhook handlers.
#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) sink: Arc<Mutex<Option<InteractionSink>>>,
    pub(crate) sessions: Arc<Mutex<HashMap<String, SessionId>>>,
    pub(crate) signing_secret: Option<Arc<SecretStr>>,
    pub(crate) trust_level: orka_contracts::TrustLevel,
}
