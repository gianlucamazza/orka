//! Discord REST API helpers: message sending and command registration.

use reqwest::Client;
use tracing::{debug, info, warn};

use orka_core::{
    Error, Result, SecretStr,
    types::{OutboundMessage, Payload},
};

/// Build a Discord API v10 URL from a path segment.
pub(crate) fn api_url(path: &str) -> String {
    format!("https://discord.com/api/v10{path}")
}

/// Send an outbound message to a Discord channel.
///
/// Dispatches to [`send_text`] or [`send_media`] based on payload type.
pub(crate) async fn send_message(
    client: &Client,
    bot_token: &SecretStr,
    channel_id: &str,
    msg: &OutboundMessage,
) -> Result<()> {
    let auth = format!("Bot {}", bot_token.expose());
    let msg_url = api_url(&format!("/channels/{channel_id}/messages"));

    match &msg.payload {
        Payload::Text(text) => send_text(client, &auth, &msg_url, text, channel_id).await,
        Payload::Media(media) => send_media(client, &auth, &msg_url, media, channel_id).await,
        _ => {
            warn!("Discord adapter: unsupported payload type, skipping");
            Ok(())
        }
    }
}

async fn send_text(
    client: &Client,
    auth: &str,
    msg_url: &str,
    text: &str,
    channel_id: &str,
) -> Result<()> {
    let body = serde_json::json!({ "content": text });
    let mut last_err: Option<Error> = None;
    for attempt in 0..3u32 {
        if attempt > 0 {
            let ms = 500u64.saturating_mul(1u64 << (attempt - 1));
            tokio::time::sleep(std::time::Duration::from_millis(ms.min(10_000))).await;
        }
        match client
            .post(msg_url)
            .header("Authorization", auth)
            .json(&body)
            .send()
            .await
        {
            Err(e) if e.is_timeout() || e.is_connect() => {
                last_err = Some(Error::Adapter {
                    source: Box::new(e),
                    context: "Discord send message failed (transient)".into(),
                });
            }
            Err(e) => {
                return Err(Error::Adapter {
                    source: Box::new(e),
                    context: "Discord send message failed".into(),
                });
            }
            Ok(r) if r.status() == 429 || r.status().is_server_error() => {
                let status = r.status();
                let body = r.text().await.unwrap_or_default();
                last_err = Some(Error::Adapter {
                    source: Box::new(std::io::Error::other(body.clone())),
                    context: format!("Discord API error {status}: {body}"),
                });
            }
            Ok(r) if !r.status().is_success() => {
                let body = r.text().await.unwrap_or_default();
                return Err(Error::Adapter {
                    source: Box::new(std::io::Error::other(body.clone())),
                    context: format!("Discord API error: {body}"),
                });
            }
            Ok(_) => {
                debug!(channel_id, "sent text message to Discord");
                last_err = None;
                break;
            }
        }
    }
    if let Some(e) = last_err {
        return Err(e);
    }
    Ok(())
}

async fn send_media(
    client: &Client,
    auth: &str,
    msg_url: &str,
    media: &orka_core::types::MediaPayload,
    channel_id: &str,
) -> Result<()> {
    let raw_bytes: Vec<u8> = if let Some(data) = media.decode_data() {
        data
    } else {
        client
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
            })?
            .to_vec()
    };

    let filename = media.caption.clone().unwrap_or_else(|| "attachment".into());
    let mime = media.mime_type.clone();

    let mut last_err: Option<Error> = None;
    for attempt in 0..3u32 {
        if attempt > 0 {
            let ms = 500u64.saturating_mul(1u64 << (attempt - 1));
            tokio::time::sleep(std::time::Duration::from_millis(ms.min(10_000))).await;
        }
        let part = match reqwest::multipart::Part::bytes(raw_bytes.clone())
            .file_name(filename.clone())
            .mime_str(&mime)
        {
            Ok(p) => p,
            Err(e) => {
                return Err(Error::Adapter {
                    source: Box::new(e),
                    context: "Discord multipart MIME error".into(),
                });
            }
        };
        let form = reqwest::multipart::Form::new().part("files[0]", part);
        match client
            .post(msg_url)
            .header("Authorization", auth)
            .multipart(form)
            .send()
            .await
        {
            Err(e) if e.is_timeout() || e.is_connect() => {
                last_err = Some(Error::Adapter {
                    source: Box::new(e),
                    context: "Discord media send failed (transient)".into(),
                });
            }
            Err(e) => {
                return Err(Error::Adapter {
                    source: Box::new(e),
                    context: "Discord media send failed".into(),
                });
            }
            Ok(r) if r.status() == 429 || r.status().is_server_error() => {
                let status = r.status();
                let body = r.text().await.unwrap_or_default();
                last_err = Some(Error::Adapter {
                    source: Box::new(std::io::Error::other(body.clone())),
                    context: format!("Discord API error {status} (media): {body}"),
                });
            }
            Ok(r) if !r.status().is_success() => {
                let body = r.text().await.unwrap_or_default();
                return Err(Error::Adapter {
                    source: Box::new(std::io::Error::other(body.clone())),
                    context: format!("Discord API error (media): {body}"),
                });
            }
            Ok(_) => {
                debug!(channel_id, "sent media to Discord");
                last_err = None;
                break;
            }
        }
    }
    if let Some(e) = last_err {
        return Err(e);
    }
    Ok(())
}

/// Register global slash commands with the Discord application.
pub(crate) async fn register_commands(
    client: &Client,
    bot_token: &SecretStr,
    app_id: &str,
    commands: &[(&str, &str)],
) -> Result<()> {
    let cmds: Vec<serde_json::Value> = commands
        .iter()
        .map(|(name, description)| {
            serde_json::json!({ "name": name, "description": description, "type": 1 })
        })
        .collect();

    let resp = client
        .put(api_url(&format!("/applications/{app_id}/commands")))
        .header("Authorization", format!("Bot {}", bot_token.expose()))
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

    info!(count = commands.len(), "Discord: registered global slash commands");
    Ok(())
}
