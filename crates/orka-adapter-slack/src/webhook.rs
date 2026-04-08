//! Slack Events API webhook handlers: signature verification and event
//! dispatch.

use axum::{Json, body::Bytes, extract::State, http::HeaderMap};
use hmac::Mac;
use orka_contracts::{
    InboundInteraction, InteractionContent, MediaAttachment, PlatformContext, SenderInfo,
    TraceContext, TrustLevel,
};
use orka_core::{MessageId, types::SessionId};
use tracing::{error, warn};

use crate::types::{AppState, HmacSha256, SlackEvent, SlackEventPayload};

/// Verify `X-Slack-Signature` using HMAC-SHA256.
///
/// Slack signs each request as `v0={hex(HMAC-SHA256(signing_secret,
/// "v0:{timestamp}:{body}")))}` and includes the timestamp in
/// `X-Slack-Request-Timestamp`.  Requests older than 5 minutes are rejected to
/// prevent replay attacks.
pub(crate) fn verify_slack_signature(
    headers: &HeaderMap,
    body: &[u8],
    signing_secret: &str,
) -> bool {
    let timestamp = if let Some(ts) = headers
        .get("X-Slack-Request-Timestamp")
        .and_then(|v| v.to_str().ok())
    {
        ts.to_owned()
    } else {
        warn!("Slack webhook: missing X-Slack-Request-Timestamp");
        return false;
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

    let provided_sig = if let Some(s) = headers
        .get("X-Slack-Signature")
        .and_then(|v| v.to_str().ok())
    {
        s.to_owned()
    } else {
        warn!("Slack webhook: missing X-Slack-Signature");
        return false;
    };

    let base_string = format!("v0:{}:{}", timestamp, String::from_utf8_lossy(body));
    let Ok(mut mac) = HmacSha256::new_from_slice(signing_secret.as_bytes()) else {
        return false;
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

fn make_slack_interaction(
    session_id: SessionId,
    channel: &str,
    chat_type: &str,
    user: Option<String>,
    content: InteractionContent,
    trust_level: TrustLevel,
) -> InboundInteraction {
    InboundInteraction {
        id: MessageId::new().as_uuid(),
        source_channel: "slack".into(),
        session_id: session_id.as_uuid(),
        timestamp: chrono::Utc::now(),
        content,
        context: PlatformContext {
            sender: SenderInfo {
                platform_user_id: user,
                display_name: None,
                user_id: None,
            },
            chat_id: Some(channel.to_owned()),
            interaction_kind: Some(chat_type.to_owned()),
            trust_level: Some(trust_level),
            ..Default::default()
        },
        trace: TraceContext::default(),
    }
}

async fn send_to_sink(
    sink: &std::sync::Arc<tokio::sync::Mutex<Option<orka_core::InteractionSink>>>,
    interaction: InboundInteraction,
) {
    let guard = sink.lock().await;
    if let Some(ref tx) = *guard
        && tx.send(interaction).await.is_err()
    {
        error!("Slack: sink closed");
    }
}

pub(crate) async fn process_message_event(event: SlackEvent, state: &AppState) {
    let Some(channel) = event.channel else { return };
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
    for file in &event.files {
        let Some(url) = file.url_private.clone() else {
            continue;
        };
        let mime = file
            .mimetype
            .clone()
            .unwrap_or_else(|| "application/octet-stream".into());
        let interaction = make_slack_interaction(
            session_id,
            &channel,
            chat_type,
            event.user.clone(),
            InteractionContent::Media(MediaAttachment {
                mime_type: mime,
                url,
                filename: file.name.clone(),
                caption: None,
                size_bytes: file.size,
                data_base64: None,
            }),
            state.trust_level,
        );
        send_to_sink(&state.sink, interaction).await;
    }
    if let Some(text) = event.text {
        let interaction = make_slack_interaction(
            session_id,
            &channel,
            chat_type,
            event.user.clone(),
            InteractionContent::Text(text),
            state.trust_level,
        );
        send_to_sink(&state.sink, interaction).await;
    }
}

/// `POST /slack/events` — receive and dispatch Slack Events API payloads.
pub(crate) async fn handle_event(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    if let Some(ref secret) = state.signing_secret
        && !verify_slack_signature(&headers, &body, (**secret).expose())
    {
        warn!("Slack webhook: signature verification failed, rejecting request");
        return axum::response::IntoResponse::into_response(axum::http::StatusCode::UNAUTHORIZED);
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

    if payload.event_type == "url_verification"
        && let Some(challenge) = payload.challenge
    {
        return axum::response::IntoResponse::into_response(Json(
            serde_json::json!({ "challenge": challenge }),
        ));
    }

    if payload.event_type == "event_callback"
        && let Some(event) = payload.event
        && event.bot_id.is_none()
        && event.event_type == "message"
    {
        process_message_event(event, &state).await;
    }

    axum::response::IntoResponse::into_response(axum::http::StatusCode::OK)
}
