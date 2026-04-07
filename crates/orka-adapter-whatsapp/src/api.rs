//! `WhatsApp` Cloud API REST helpers: media resolution and message sending.

use orka_core::{
    Error, Result, SecretStr,
    types::{OutboundMessage, Payload},
};
use reqwest::Client;
use tracing::{debug, warn};

/// Resolve a `WhatsApp` media object ID to a downloadable URL.
pub(crate) async fn resolve_media_url(
    client: &Client,
    access_token: &str,
    media_id: &str,
    api_version: &str,
) -> Result<String> {
    let resp = client
        .get(format!(
            "https://graph.facebook.com/{api_version}/{media_id}"
        ))
        .header("Authorization", format!("Bearer {access_token}"))
        .send()
        .await
        .map_err(|e| Error::Adapter {
            source: Box::new(e),
            context: format!("WhatsApp resolve media {media_id} failed"),
        })?
        .json::<serde_json::Value>()
        .await
        .map_err(|e| Error::Adapter {
            source: Box::new(e),
            context: "WhatsApp media response parse failed".into(),
        })?;

    resp["url"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| Error::Adapter {
            source: Box::new(std::io::Error::other("missing url in media response")),
            context: format!("WhatsApp media {media_id}: no url field"),
        })
}

/// Send an outbound message via the `WhatsApp` Cloud API, with up to 3 retries.
pub(crate) async fn send_message(
    client: &Client,
    access_token: &SecretStr,
    api_version: &str,
    phone_number_id: &str,
    msg: &OutboundMessage,
) -> Result<()> {
    let to = msg.chat_id().map_err(|_| Error::Adapter {
        source: Box::new(std::io::Error::other("missing platform_context.chat_id")),
        context: "missing chat_id in platform_context".into(),
    })?;

    let url = format!("https://graph.facebook.com/{api_version}/{phone_number_id}/messages");

    let body = match &msg.payload {
        Payload::Text(text) => serde_json::json!({
            "messaging_product": "whatsapp",
            "to": to,
            "type": "text",
            "text": { "body": text },
        }),
        Payload::Media(media) => {
            let (msg_type, media_field) = if media.mime_type.starts_with("image/") {
                ("image", "image")
            } else if media.mime_type.starts_with("video/") {
                ("video", "video")
            } else if media.mime_type.starts_with("audio/") {
                ("audio", "audio")
            } else {
                ("document", "document")
            };

            let mut media_obj = serde_json::json!({ "link": media.url });
            if let Some(ref caption) = media.caption {
                media_obj["caption"] = serde_json::json!(caption);
            }

            serde_json::json!({
                "messaging_product": "whatsapp",
                "to": to,
                "type": msg_type,
                media_field: media_obj,
            })
        }
        _ => {
            warn!("WhatsApp adapter: unsupported payload type, skipping");
            return Ok(());
        }
    };

    let auth = format!("Bearer {}", access_token.expose());
    let mut last_err: Option<Error> = None;
    for attempt in 0..3u32 {
        if attempt > 0 {
            let ms = 500u64.saturating_mul(1u64 << (attempt - 1));
            tokio::time::sleep(std::time::Duration::from_millis(ms.min(10_000))).await;
        }
        match client
            .post(&url)
            .header("Authorization", &auth)
            .json(&body)
            .send()
            .await
        {
            Err(e) if e.is_timeout() || e.is_connect() => {
                last_err = Some(Error::Adapter {
                    source: Box::new(e),
                    context: "WhatsApp send message failed (transient)".into(),
                });
            }
            Err(e) => {
                return Err(Error::Adapter {
                    source: Box::new(e),
                    context: "WhatsApp send message failed".into(),
                });
            }
            Ok(resp) if resp.status() == 429 || resp.status().is_server_error() => {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                last_err = Some(Error::Adapter {
                    source: Box::new(std::io::Error::other(text.clone())),
                    context: format!("WhatsApp API error {status}: {text}"),
                });
            }
            Ok(resp) if !resp.status().is_success() => {
                let text = resp.text().await.unwrap_or_default();
                return Err(Error::Adapter {
                    source: Box::new(std::io::Error::other(text.clone())),
                    context: format!("WhatsApp API error: {text}"),
                });
            }
            Ok(_) => {
                debug!(to, "sent message via WhatsApp");
                return Ok(());
            }
        }
    }
    Err(last_err.unwrap_or_else(|| Error::Adapter {
        source: Box::new(std::io::Error::other("max retries exceeded")),
        context: "WhatsApp send message failed after retries".into(),
    }))
}
