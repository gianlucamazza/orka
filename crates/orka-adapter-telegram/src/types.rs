//! Telegram Bot API types.

use serde::Deserialize;

// Note: These types are used for serde deserialization of Telegram API
// responses. Not all fields are accessed directly in code, but are required for
// complete JSON parsing.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub(crate) struct TelegramResponse<T> {
    pub ok: bool,
    pub result: Option<T>,
    pub description: Option<String>,
    pub parameters: Option<ResponseParameters>,
}

// Note: Used for serde deserialization; retry_after is the primary field
// accessed.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub(crate) struct ResponseParameters {
    pub retry_after: Option<u64>,
}

#[allow(clippy::struct_field_names)]
#[derive(Debug, Deserialize)]
pub(crate) struct Update {
    pub update_id: i64,
    pub message: Option<TelegramMessage>,
    pub edited_message: Option<TelegramMessage>,
    pub callback_query: Option<CallbackQuery>,
}

// Note: Used for serde deserialization; not all fields accessed directly.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub(crate) struct TelegramMessage {
    pub message_id: i64,
    pub chat: Chat,
    pub text: Option<String>,
    pub from: Option<User>,
    #[serde(default)]
    pub photo: Vec<PhotoSize>,
    pub document: Option<Document>,
    pub audio: Option<Audio>,
    pub video: Option<Video>,
    pub voice: Option<Voice>,
    pub video_note: Option<VideoNote>,
    pub sticker: Option<Sticker>,
    pub caption: Option<String>,
    #[serde(default)]
    pub entities: Vec<MessageEntity>,
    pub reply_to_message: Option<Box<TelegramMessage>>,
    pub message_thread_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Chat {
    pub id: i64,
    #[serde(default)]
    pub r#type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct User {
    pub id: i64,
    pub first_name: String,
    pub last_name: Option<String>,
    pub username: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PhotoSize {
    pub file_id: String,
    pub file_size: Option<u64>,
}

// Note: Used for serde deserialization; not all fields accessed directly.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub(crate) struct Document {
    pub file_id: String,
    pub file_name: Option<String>,
    pub mime_type: Option<String>,
    pub file_size: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Audio {
    pub file_id: String,
    pub mime_type: Option<String>,
    pub file_size: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Video {
    pub file_id: String,
    pub mime_type: Option<String>,
    pub file_size: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Voice {
    pub file_id: String,
    pub mime_type: Option<String>,
    pub file_size: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct VideoNote {
    pub file_id: String,
    pub file_size: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Sticker {
    pub file_id: String,
    pub file_size: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
// Note: Used for serde deserialization; not all fields accessed directly.
#[allow(dead_code)]
pub(crate) struct MessageEntity {
    pub r#type: String,
    pub offset: u32,
    pub length: u32,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CallbackQuery {
    pub id: String,
    pub from: User,
    pub message: Option<TelegramMessage>,
    pub data: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TelegramFile {
    pub file_path: Option<String>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn deserialize_update_with_text_message() {
        let data = json!({
            "update_id": 42,
            "message": {
                "message_id": 100,
                "chat": {"id": 7, "type": "private"},
                "text": "ping",
                "from": {"id": 1, "first_name": "Alice"}
            }
        });
        let update: Update = serde_json::from_value(data).unwrap();
        assert_eq!(update.update_id, 42);
        let msg = update.message.unwrap();
        assert_eq!(msg.message_id, 100);
        assert_eq!(msg.chat.id, 7);
        assert_eq!(msg.text.as_deref(), Some("ping"));
        let user = msg.from.unwrap();
        assert_eq!(user.id, 1);
        assert_eq!(user.first_name, "Alice");
    }

    #[test]
    fn deserialize_update_without_message() {
        let data = json!({"update_id": 99});
        let update: Update = serde_json::from_value(data).unwrap();
        assert_eq!(update.update_id, 99);
        assert!(update.message.is_none());
    }

    #[test]
    fn deserialize_telegram_message_minimal() {
        let data = json!({
            "message_id": 5,
            "chat": {"id": 10}
        });
        let msg: TelegramMessage = serde_json::from_value(data).unwrap();
        assert_eq!(msg.message_id, 5);
        assert_eq!(msg.chat.id, 10);
        assert!(msg.text.is_none());
        assert!(msg.from.is_none());
        assert!(msg.chat.r#type.is_none());
    }

    #[test]
    fn deserialize_update_with_edited_message() {
        let data = json!({
            "update_id": 10,
            "edited_message": {
                "message_id": 5,
                "chat": {"id": 3, "type": "private"},
                "text": "edited"
            }
        });
        let update: Update = serde_json::from_value(data).unwrap();
        assert!(update.message.is_none());
        let edited = update.edited_message.unwrap();
        assert_eq!(edited.text.as_deref(), Some("edited"));
    }

    #[test]
    fn deserialize_callback_query() {
        let data = json!({
            "update_id": 20,
            "callback_query": {
                "id": "abc",
                "from": {"id": 9, "first_name": "Bob"},
                "data": "action:1"
            }
        });
        let update: Update = serde_json::from_value(data).unwrap();
        let cq = update.callback_query.unwrap();
        assert_eq!(cq.id, "abc");
        assert_eq!(cq.data.as_deref(), Some("action:1"));
    }

    #[test]
    fn deserialize_message_with_photo() {
        let data = json!({
            "message_id": 1,
            "chat": {"id": 1},
            "photo": [
                {"file_id": "small", "file_size": 1024},
                {"file_id": "large", "file_size": 8192}
            ],
            "caption": "A photo"
        });
        let msg: TelegramMessage = serde_json::from_value(data).unwrap();
        assert_eq!(msg.photo.len(), 2);
        assert_eq!(msg.photo.last().unwrap().file_id, "large");
        assert_eq!(msg.caption.as_deref(), Some("A photo"));
    }

    #[test]
    fn deserialize_message_with_bot_command_entity() {
        let data = json!({
            "message_id": 1,
            "chat": {"id": 1},
            "text": "/start",
            "entities": [{"type": "bot_command", "offset": 0, "length": 6}]
        });
        let msg: TelegramMessage = serde_json::from_value(data).unwrap();
        assert_eq!(msg.entities.len(), 1);
        assert_eq!(msg.entities[0].r#type, "bot_command");
        assert_eq!(msg.entities[0].offset, 0);
    }

    #[test]
    fn deserialize_telegram_response_ok() {
        let data = json!({
            "ok": true,
            "result": [{"update_id": 1}]
        });
        let resp: TelegramResponse<Vec<serde_json::Value>> = serde_json::from_value(data).unwrap();
        assert!(resp.ok);
        assert_eq!(resp.result.unwrap().len(), 1);
    }

    #[test]
    fn deserialize_telegram_response_error() {
        let data = json!({
            "ok": false,
            "description": "Unauthorized"
        });
        let resp: TelegramResponse<Vec<serde_json::Value>> = serde_json::from_value(data).unwrap();
        assert!(!resp.ok);
        assert!(resp.result.is_none());
        assert_eq!(resp.description.unwrap(), "Unauthorized");
    }
}
