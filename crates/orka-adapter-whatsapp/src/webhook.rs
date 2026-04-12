//! `WhatsApp` webhook handlers: signature verification, challenge response, and
//! inbound message dispatch.

use axum::{
    body::Bytes,
    extract::{Query, State},
    http::HeaderMap,
};
use hmac::Mac;
use orka_core::{
    InboundInteraction, InteractionContent, MediaAttachment, MessageId, PlatformContext,
    SenderInfo, TraceContext, types::SessionId,
};
use tracing::{error, warn};

use crate::{
    api::resolve_media_url,
    types::{AppState, HmacSha256, WebhookPayload, WebhookVerifyParams, WhatsAppMessage},
};

/// Verify `X-Hub-Signature-256` using HMAC-SHA256 with the app secret.
///
/// Meta sends `sha256={hex(HMAC-SHA256(app_secret, raw_body))}` in the header.
pub(crate) fn verify_whatsapp_signature(
    headers: &HeaderMap,
    body: &[u8],
    app_secret: &str,
) -> bool {
    let sig_header = if let Some(s) = headers
        .get("X-Hub-Signature-256")
        .and_then(|v| v.to_str().ok())
    {
        s.to_owned()
    } else {
        warn!("WhatsApp webhook: missing X-Hub-Signature-256 header");
        return false;
    };

    let provided_hex = sig_header.strip_prefix("sha256=").unwrap_or(&sig_header);

    let Ok(mut mac) = HmacSha256::new_from_slice(app_secret.as_bytes()) else {
        return false;
    };
    mac.update(body);
    let expected_hex = hex::encode(mac.finalize().into_bytes());

    let expected_b = expected_hex.as_bytes();
    let provided_b = provided_hex.as_bytes();
    if expected_b.len() != provided_b.len() {
        return false;
    }
    // Constant-time comparison to prevent timing attacks.
    expected_b
        .iter()
        .zip(provided_b.iter())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

/// `GET /webhook` — Meta webhook challenge/verification handshake.
pub(crate) async fn webhook_verify(
    State(state): State<AppState>,
    Query(params): Query<WebhookVerifyParams>,
) -> axum::response::Response {
    if params.mode.as_deref() == Some("subscribe")
        && params.token.as_deref() == Some((*state.verify_token).expose())
        && let Some(challenge) = params.challenge
    {
        return axum::response::IntoResponse::into_response(challenge);
    }
    axum::response::IntoResponse::into_response(axum::http::StatusCode::FORBIDDEN)
}

/// `POST /webhook` — receive and dispatch inbound `WhatsApp` messages.
pub(crate) async fn webhook_receive(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> axum::http::StatusCode {
    if let Some(ref secret) = state.app_secret
        && !verify_whatsapp_signature(&headers, &body, (*secret).expose())
    {
        warn!("WhatsApp webhook: X-Hub-Signature-256 verification failed, rejecting request");
        return axum::http::StatusCode::UNAUTHORIZED;
    }

    let payload: WebhookPayload = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(e) => {
            warn!(%e, "WhatsApp webhook: failed to parse payload");
            return axum::http::StatusCode::BAD_REQUEST;
        }
    };

    for entry in payload.entry.into_iter().flatten() {
        for change in entry.changes.into_iter().flatten() {
            if let Some(value) = change.value {
                for msg in value.messages.into_iter().flatten() {
                    handle_whatsapp_message(msg, &state).await;
                }
            }
        }
    }

    axum::http::StatusCode::OK
}

/// Dispatch a single inbound `WhatsAppMessage` to the sink.
async fn handle_whatsapp_message(msg: WhatsAppMessage, state: &AppState) {
    let session_id = {
        let mut sessions = state.sessions.lock().await;
        *sessions
            .entry(msg.from.clone())
            .or_insert_with(SessionId::new)
    };

    let maybe_media: Option<(&crate::types::WhatsAppMedia, &str)> = match msg.msg_type.as_str() {
        "image" => msg.image.as_ref().map(|m| (m, "image/jpeg")),
        "video" => msg.video.as_ref().map(|m| (m, "video/mp4")),
        "audio" => msg.audio.as_ref().map(|m| (m, "audio/ogg")),
        "document" => msg
            .document
            .as_ref()
            .map(|m| (m, "application/octet-stream")),
        _ => None,
    };

    if let Some((media_obj, default_mime)) = maybe_media {
        let url = match resolve_media_url(
            &state.client,
            state.access_token.expose(),
            &media_obj.id,
            &state.api_version,
        )
        .await
        {
            Ok(u) => u,
            Err(e) => {
                error!(%e, "WhatsApp: failed to resolve media URL");
                return;
            }
        };
        let interaction = InboundInteraction {
            id: MessageId::new().as_uuid(),
            source_channel: "whatsapp".into(),
            session_id: session_id.as_uuid(),
            timestamp: chrono::Utc::now(),
            content: InteractionContent::Media(MediaAttachment {
                mime_type: media_obj
                    .mime_type
                    .clone()
                    .unwrap_or_else(|| default_mime.into()),
                url,
                filename: None,
                caption: media_obj.caption.clone(),
                size_bytes: None,
                data_base64: None,
            }),
            context: PlatformContext {
                sender: SenderInfo {
                    platform_user_id: Some(msg.from.clone()),
                    display_name: None,
                    user_id: None,
                },
                chat_id: Some(msg.from.clone()),
                interaction_kind: Some("direct".into()),
                trust_level: Some(state.trust_level),
                ..Default::default()
            },
            trace: TraceContext::default(),
        };
        let sink = state.sink.lock().await;
        if let Some(ref tx) = *sink
            && tx.send(interaction).await.is_err()
        {
            error!("WhatsApp: sink closed");
        }
        return;
    }

    if msg.msg_type != "text" {
        return;
    }
    if let Some(text) = msg.text {
        let interaction = InboundInteraction {
            id: MessageId::new().as_uuid(),
            source_channel: "whatsapp".into(),
            session_id: session_id.as_uuid(),
            timestamp: chrono::Utc::now(),
            content: InteractionContent::Text(text.body),
            context: PlatformContext {
                sender: SenderInfo {
                    platform_user_id: Some(msg.from.clone()),
                    display_name: None,
                    user_id: None,
                },
                chat_id: Some(msg.from.clone()),
                interaction_kind: Some("direct".into()),
                trust_level: Some(state.trust_level),
                ..Default::default()
            },
            trace: TraceContext::default(),
        };
        let sink = state.sink.lock().await;
        if let Some(ref tx) = *sink
            && tx.send(interaction).await.is_err()
        {
            error!("WhatsApp: sink closed");
        }
    }
}
