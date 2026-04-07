//! Slack REST API helpers: message sending, file upload, and retried POST.

use orka_core::{Error, Result, types::MediaPayload};
use reqwest::Client;
use tracing::debug;

/// POST JSON to `url` with up to 3 attempts (retry on 429 or 5xx).
/// Returns the successful `reqwest::Response` or the last error encountered.
pub(crate) async fn post_json_retried(
    client: &Client,
    url: &str,
    auth: &str,
    body: &serde_json::Value,
    context: &str,
) -> Result<reqwest::Response> {
    let mut last_err: Option<Error> = None;
    for attempt in 0..3u32 {
        if attempt > 0 {
            let ms = 500u64.saturating_mul(1u64 << (attempt - 1));
            tokio::time::sleep(std::time::Duration::from_millis(ms.min(10_000))).await;
        }
        match client
            .post(url)
            .header("Authorization", auth)
            .header("Content-Type", "application/json; charset=utf-8")
            .json(body)
            .send()
            .await
        {
            Err(e) if e.is_timeout() || e.is_connect() => {
                last_err = Some(Error::Adapter {
                    source: Box::new(e),
                    context: format!("{context} (transient)"),
                });
            }
            Err(e) => {
                return Err(Error::Adapter {
                    source: Box::new(e),
                    context: context.to_string(),
                });
            }
            Ok(r) if r.status() == 429 || r.status().is_server_error() => {
                let status = r.status();
                let text = r.text().await.unwrap_or_default();
                last_err = Some(Error::Adapter {
                    source: Box::new(std::io::Error::other(text.clone())),
                    context: format!("{context}: HTTP {status}: {text}"),
                });
            }
            Ok(r) => return Ok(r),
        }
    }
    Err(last_err.unwrap_or_else(|| Error::Adapter {
        source: Box::new(std::io::Error::other("max retries exceeded")),
        context: format!("{context}: max retries exceeded"),
    }))
}

/// Send a plain-text message to a Slack channel via `chat.postMessage`.
pub(crate) async fn send_text_message(
    client: &Client,
    auth: &str,
    channel: &str,
    text: &str,
) -> Result<()> {
    let body = serde_json::json!({ "channel": channel, "text": text });
    let response = post_json_retried(
        client,
        "https://slack.com/api/chat.postMessage",
        auth,
        &body,
        "Slack chat.postMessage failed",
    )
    .await?;
    if !response.status().is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(Error::Adapter {
            source: Box::new(std::io::Error::other(body.clone())),
            context: format!("Slack API error: {body}"),
        });
    }
    debug!(channel, "sent text message to Slack");
    Ok(())
}

/// Send a Block Kit image block to a Slack channel (URL-based images only).
pub(crate) async fn send_image_block(
    client: &Client,
    auth: &str,
    channel: &str,
    media: &MediaPayload,
) -> Result<()> {
    let blocks = serde_json::json!([{
        "type": "image",
        "image_url": media.url,
        "alt_text": media.caption.as_deref().unwrap_or("image"),
    }]);
    let body = serde_json::json!({ "channel": channel, "blocks": blocks });
    let response = post_json_retried(
        client,
        "https://slack.com/api/chat.postMessage",
        auth,
        &body,
        "Slack image block send failed",
    )
    .await?;
    if !response.status().is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(Error::Adapter {
            source: Box::new(std::io::Error::other(body.clone())),
            context: format!("Slack API error (image): {body}"),
        });
    }
    debug!(channel, "sent image block to Slack");
    Ok(())
}

/// Request an external upload URL from `files.getUploadURLExternal`.
/// Returns `(upload_url, file_id)`.
pub(crate) async fn get_upload_url(
    client: &Client,
    auth: &str,
    filename: &str,
    size: usize,
) -> Result<(String, String)> {
    let mut last_err: Option<Error> = None;
    for attempt in 0..3u32 {
        if attempt > 0 {
            let ms = 500u64.saturating_mul(1u64 << (attempt - 1));
            tokio::time::sleep(std::time::Duration::from_millis(ms.min(10_000))).await;
        }
        match client
            .get("https://slack.com/api/files.getUploadURLExternal")
            .header("Authorization", auth)
            .query(&[("filename", filename), ("length", &size.to_string())])
            .send()
            .await
        {
            Err(e) if e.is_timeout() || e.is_connect() => {
                last_err = Some(Error::Adapter {
                    source: Box::new(e),
                    context: "Slack getUploadURLExternal failed (transient)".into(),
                });
            }
            Err(e) => {
                return Err(Error::Adapter {
                    source: Box::new(e),
                    context: "Slack getUploadURLExternal failed".into(),
                });
            }
            Ok(r) if r.status() == 429 || r.status().is_server_error() => {
                let status = r.status();
                let text = r.text().await.unwrap_or_default();
                last_err = Some(Error::Adapter {
                    source: Box::new(std::io::Error::other(text.clone())),
                    context: format!("Slack getUploadURLExternal HTTP {status}: {text}"),
                });
            }
            Ok(r) => {
                let v: serde_json::Value = r.json().await.map_err(|e| Error::Adapter {
                    source: Box::new(e),
                    context: "Slack getUploadURLExternal parse failed".into(),
                })?;
                if v["ok"].as_bool() != Some(true) {
                    return Err(Error::Adapter {
                        source: Box::new(std::io::Error::other(v.to_string())),
                        context: "Slack getUploadURLExternal returned ok=false".into(),
                    });
                }
                let upload_url = v["upload_url"].as_str().unwrap_or("").to_string();
                let file_id = v["file_id"].as_str().unwrap_or("").to_string();
                return Ok((upload_url, file_id));
            }
        }
    }
    Err(last_err.unwrap_or_else(|| Error::Adapter {
        source: Box::new(std::io::Error::other("max retries exceeded")),
        context: "Slack getUploadURLExternal max retries exceeded".into(),
    }))
}

/// Upload raw file bytes to the pre-signed `upload_url`.
pub(crate) async fn upload_file_bytes(
    client: &Client,
    upload_url: &str,
    bytes: Vec<u8>,
) -> Result<()> {
    let mut last_err: Option<Error> = None;
    for attempt in 0..3u32 {
        if attempt > 0 {
            let ms = 500u64.saturating_mul(1u64 << (attempt - 1));
            tokio::time::sleep(std::time::Duration::from_millis(ms.min(10_000))).await;
        }
        match client.post(upload_url).body(bytes.clone()).send().await {
            Err(e) if e.is_timeout() || e.is_connect() => {
                last_err = Some(Error::Adapter {
                    source: Box::new(e),
                    context: "Slack file upload failed (transient)".into(),
                });
            }
            Err(e) => {
                return Err(Error::Adapter {
                    source: Box::new(e),
                    context: "Slack file upload failed".into(),
                });
            }
            Ok(r) if r.status() == 429 || r.status().is_server_error() => {
                let status = r.status();
                let text = r.text().await.unwrap_or_default();
                last_err = Some(Error::Adapter {
                    source: Box::new(std::io::Error::other(text.clone())),
                    context: format!("Slack file upload HTTP {status}: {text}"),
                });
            }
            Ok(_) => return Ok(()),
        }
    }
    Err(last_err.unwrap_or_else(|| Error::Adapter {
        source: Box::new(std::io::Error::other("max retries exceeded")),
        context: "Slack file upload max retries exceeded".into(),
    }))
}

/// Complete a file upload via `files.completeUploadExternal`.
pub(crate) async fn complete_file_upload(
    client: &Client,
    auth: &str,
    file_id: &str,
    channel: &str,
) -> Result<()> {
    let body = serde_json::json!({
        "files": [{ "id": file_id }],
        "channel_id": channel,
    });
    let resp: serde_json::Value = post_json_retried(
        client,
        "https://slack.com/api/files.completeUploadExternal",
        auth,
        &body,
        "Slack completeUploadExternal failed",
    )
    .await?
    .json()
    .await
    .map_err(|e| Error::Adapter {
        source: Box::new(e),
        context: "Slack completeUploadExternal parse failed".into(),
    })?;
    if resp["ok"].as_bool() != Some(true) {
        return Err(Error::Adapter {
            source: Box::new(std::io::Error::other(resp.to_string())),
            context: "Slack completeUploadExternal returned ok=false".into(),
        });
    }
    Ok(())
}

/// Upload a media file to Slack using the 3-step external upload flow.
pub(crate) async fn send_file_upload(
    client: &Client,
    auth: &str,
    channel: &str,
    media: &MediaPayload,
) -> Result<()> {
    let filename = media.caption.clone().unwrap_or_else(|| "attachment".into());
    let file_bytes: Vec<u8> = if let Some(data) = media.decode_data() {
        data
    } else {
        client
            .get(&media.url)
            .send()
            .await
            .map_err(|e| Error::Adapter {
                source: Box::new(e),
                context: "Slack media download failed".into(),
            })?
            .bytes()
            .await
            .map_err(|e| Error::Adapter {
                source: Box::new(e),
                context: "Slack media read failed".into(),
            })?
            .to_vec()
    };
    let (upload_url, file_id) = get_upload_url(client, auth, &filename, file_bytes.len()).await?;
    upload_file_bytes(client, &upload_url, file_bytes).await?;
    complete_file_upload(client, auth, &file_id, channel).await?;
    debug!(channel, "uploaded file to Slack");
    Ok(())
}
