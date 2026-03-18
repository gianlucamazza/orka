//! Helpers for resolving and routing Telegram media payloads.

use orka_core::types::MediaPayload;

use crate::api::TelegramApi;
use crate::types::TelegramMessage;

/// How to send an outbound media file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SendMethod {
    Photo,
    Audio,
    Video,
    Document,
}

/// Determine the Telegram send method from a MIME type.
pub(crate) fn select_send_method(mime_type: &str) -> SendMethod {
    if mime_type.starts_with("image/") {
        SendMethod::Photo
    } else if mime_type.starts_with("audio/") {
        SendMethod::Audio
    } else if mime_type.starts_with("video/") {
        SendMethod::Video
    } else {
        SendMethod::Document
    }
}

/// Resolve inbound Telegram media to a `MediaPayload`, or `None` if the message
/// contains no media.
pub(crate) async fn resolve_inbound_media(
    api: &TelegramApi,
    msg: &TelegramMessage,
) -> Option<MediaPayload> {
    // Photo: pick the largest size (last element).
    if !msg.photo.is_empty() {
        let largest = msg.photo.last().unwrap();
        let url = api.get_file_url(&largest.file_id).await.ok()?;
        let mut mp = MediaPayload::new("image/jpeg", url);
        mp.caption = msg.caption.clone();
        mp.size_bytes = largest.file_size;
        return Some(mp);
    }

    if let Some(doc) = &msg.document {
        let url = api.get_file_url(&doc.file_id).await.ok()?;
        let mime = doc
            .mime_type
            .clone()
            .unwrap_or_else(|| "application/octet-stream".into());
        let mut mp = MediaPayload::new(mime, url);
        mp.caption = msg.caption.clone();
        mp.size_bytes = doc.file_size;
        return Some(mp);
    }

    if let Some(audio) = &msg.audio {
        let url = api.get_file_url(&audio.file_id).await.ok()?;
        let mime = audio
            .mime_type
            .clone()
            .unwrap_or_else(|| "audio/mpeg".into());
        let mut mp = MediaPayload::new(mime, url);
        mp.caption = msg.caption.clone();
        mp.size_bytes = audio.file_size;
        return Some(mp);
    }

    if let Some(video) = &msg.video {
        let url = api.get_file_url(&video.file_id).await.ok()?;
        let mime = video
            .mime_type
            .clone()
            .unwrap_or_else(|| "video/mp4".into());
        let mut mp = MediaPayload::new(mime, url);
        mp.caption = msg.caption.clone();
        mp.size_bytes = video.file_size;
        return Some(mp);
    }

    if let Some(voice) = &msg.voice {
        let url = api.get_file_url(&voice.file_id).await.ok()?;
        let mime = voice
            .mime_type
            .clone()
            .unwrap_or_else(|| "audio/ogg".into());
        let mut mp = MediaPayload::new(mime, url);
        mp.size_bytes = voice.file_size;
        return Some(mp);
    }

    if let Some(vn) = &msg.video_note {
        let url = api.get_file_url(&vn.file_id).await.ok()?;
        let mut mp = MediaPayload::new("video/mp4", url);
        mp.size_bytes = vn.file_size;
        return Some(mp);
    }

    if let Some(sticker) = &msg.sticker {
        let url = api.get_file_url(&sticker.file_id).await.ok()?;
        let mut mp = MediaPayload::new("image/webp", url);
        mp.size_bytes = sticker.file_size;
        return Some(mp);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_send_method_image() {
        assert_eq!(select_send_method("image/jpeg"), SendMethod::Photo);
        assert_eq!(select_send_method("image/png"), SendMethod::Photo);
        assert_eq!(select_send_method("image/webp"), SendMethod::Photo);
    }

    #[test]
    fn select_send_method_audio() {
        assert_eq!(select_send_method("audio/mpeg"), SendMethod::Audio);
        assert_eq!(select_send_method("audio/ogg"), SendMethod::Audio);
    }

    #[test]
    fn select_send_method_video() {
        assert_eq!(select_send_method("video/mp4"), SendMethod::Video);
    }

    #[test]
    fn select_send_method_document() {
        assert_eq!(select_send_method("application/pdf"), SendMethod::Document);
        assert_eq!(select_send_method("text/plain"), SendMethod::Document);
        assert_eq!(
            select_send_method("application/octet-stream"),
            SendMethod::Document
        );
    }
}
