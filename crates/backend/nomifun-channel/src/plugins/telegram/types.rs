use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Telegram Bot API response envelope
// ---------------------------------------------------------------------------

/// Generic Telegram Bot API response wrapper.
///
/// All Telegram Bot API methods return JSON objects with `ok` + `result` fields.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct TgResponse<T> {
    pub ok: bool,
    pub result: Option<T>,
    pub description: Option<String>,
}

// ---------------------------------------------------------------------------
// getMe
// ---------------------------------------------------------------------------

/// Response from `getMe` — bot identity.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub(crate) struct TgUser {
    pub id: i64,
    #[serde(default)]
    pub is_bot: bool,
    pub first_name: String,
    pub last_name: Option<String>,
    pub username: Option<String>,
}

// ---------------------------------------------------------------------------
// getUpdates
// ---------------------------------------------------------------------------

/// A single update from `getUpdates`.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct TgUpdate {
    pub update_id: i64,
    pub message: Option<TgMessage>,
    pub callback_query: Option<TgCallbackQuery>,
}

/// Telegram message object (subset of fields we use).
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct TgMessage {
    pub message_id: i64,
    pub from: Option<TgUser>,
    pub chat: TgChat,
    pub date: i64,
    pub text: Option<String>,
    /// Caption for media messages (photo, document, audio, video, voice).
    pub caption: Option<String>,
    pub photo: Option<Vec<TgPhotoSize>>,
    pub document: Option<TgDocument>,
    pub voice: Option<TgVoice>,
    pub audio: Option<TgAudio>,
    pub video: Option<TgVideo>,
    pub sticker: Option<TgSticker>,
    pub reply_to_message: Option<Box<TgMessage>>,
}

/// Telegram chat object.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub(crate) struct TgChat {
    pub id: i64,
    #[serde(rename = "type")]
    pub chat_type: String,
    pub title: Option<String>,
}

/// Telegram photo size (multiple resolutions per photo).
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub(crate) struct TgPhotoSize {
    pub file_id: String,
    pub file_unique_id: String,
    pub width: i32,
    pub height: i32,
    pub file_size: Option<u64>,
}

/// Telegram document attachment.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct TgDocument {
    pub file_id: String,
    pub file_name: Option<String>,
    pub mime_type: Option<String>,
    pub file_size: Option<u64>,
}

/// Telegram voice message.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub(crate) struct TgVoice {
    pub file_id: String,
    pub duration: i32,
    pub mime_type: Option<String>,
    pub file_size: Option<u64>,
}

/// Telegram audio message.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub(crate) struct TgAudio {
    pub file_id: String,
    pub duration: i32,
    pub file_name: Option<String>,
    pub mime_type: Option<String>,
    pub file_size: Option<u64>,
}

/// Telegram video message.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub(crate) struct TgVideo {
    pub file_id: String,
    pub duration: i32,
    pub file_name: Option<String>,
    pub mime_type: Option<String>,
    pub file_size: Option<u64>,
}

/// Telegram sticker.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct TgSticker {
    pub file_id: String,
    pub emoji: Option<String>,
}

/// Telegram callback query from inline keyboard button press.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct TgCallbackQuery {
    pub id: String,
    pub from: TgUser,
    pub message: Option<TgMessage>,
    pub data: Option<String>,
}

// ---------------------------------------------------------------------------
// sendMessage / editMessageText request bodies
// ---------------------------------------------------------------------------

/// Request body for `sendMessage`.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct SendMessageRequest {
    pub chat_id: i64,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to_message_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_markup: Option<ReplyMarkup>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disable_notification: Option<bool>,
}

/// Request body for `editMessageText`.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct EditMessageTextRequest {
    pub chat_id: i64,
    pub message_id: i64,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_markup: Option<ReplyMarkup>,
}

/// Request body for `answerCallbackQuery`.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct AnswerCallbackQueryRequest {
    pub callback_query_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_alert: Option<bool>,
}

// ---------------------------------------------------------------------------
// Keyboard / Inline markup
// ---------------------------------------------------------------------------

/// Union type for reply markup (InlineKeyboard or ReplyKeyboard).
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub(crate) enum ReplyMarkup {
    InlineKeyboard(InlineKeyboardMarkup),
    ReplyKeyboard(ReplyKeyboardMarkup),
}

/// Inline keyboard (buttons inside the message).
#[derive(Debug, Clone, Serialize)]
pub(crate) struct InlineKeyboardMarkup {
    pub inline_keyboard: Vec<Vec<InlineKeyboardButton>>,
}

/// A single inline keyboard button.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct InlineKeyboardButton {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub callback_data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Reply keyboard (persistent buttons at the bottom).
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ReplyKeyboardMarkup {
    pub keyboard: Vec<Vec<KeyboardButton>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resize_keyboard: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub one_time_keyboard: Option<bool>,
}

/// A single reply keyboard button.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct KeyboardButton {
    pub text: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tg_response_ok_parses() {
        let raw = json!({
            "ok": true,
            "result": { "id": 123, "is_bot": true, "first_name": "MyBot", "username": "my_bot" }
        });
        let resp: TgResponse<TgUser> = serde_json::from_value(raw).unwrap();
        assert!(resp.ok);
        let user = resp.result.unwrap();
        assert_eq!(user.id, 123);
        assert!(user.is_bot);
        assert_eq!(user.username.as_deref(), Some("my_bot"));
    }

    #[test]
    fn tg_response_error_parses() {
        let raw = json!({
            "ok": false,
            "description": "Unauthorized"
        });
        let resp: TgResponse<TgUser> = serde_json::from_value(raw).unwrap();
        assert!(!resp.ok);
        assert!(resp.result.is_none());
        assert_eq!(resp.description.as_deref(), Some("Unauthorized"));
    }

    #[test]
    fn tg_update_with_message() {
        let raw = json!({
            "update_id": 100,
            "message": {
                "message_id": 1,
                "from": { "id": 42, "is_bot": false, "first_name": "Alice", "username": "alice" },
                "chat": { "id": -100, "type": "group", "title": "Test Group" },
                "date": 1700000000,
                "text": "Hello"
            }
        });
        let upd: TgUpdate = serde_json::from_value(raw).unwrap();
        assert_eq!(upd.update_id, 100);
        let msg = upd.message.unwrap();
        assert_eq!(msg.text.as_deref(), Some("Hello"));
        assert_eq!(msg.chat.id, -100);
        assert_eq!(msg.chat.chat_type, "group");
    }

    #[test]
    fn tg_update_with_callback_query() {
        let raw = json!({
            "update_id": 101,
            "callback_query": {
                "id": "cb_1",
                "from": { "id": 42, "is_bot": false, "first_name": "Alice" },
                "data": "system:session.new"
            }
        });
        let upd: TgUpdate = serde_json::from_value(raw).unwrap();
        let cb = upd.callback_query.unwrap();
        assert_eq!(cb.id, "cb_1");
        assert_eq!(cb.data.as_deref(), Some("system:session.new"));
    }

    #[test]
    fn tg_message_with_photo() {
        let raw = json!({
            "message_id": 2,
            "chat": { "id": 1, "type": "private" },
            "date": 1700000001,
            "photo": [
                { "file_id": "small", "file_unique_id": "u1", "width": 90, "height": 90 },
                { "file_id": "large", "file_unique_id": "u2", "width": 800, "height": 600, "file_size": 50000 }
            ]
        });
        let msg: TgMessage = serde_json::from_value(raw).unwrap();
        let photos = msg.photo.unwrap();
        assert_eq!(photos.len(), 2);
        assert_eq!(photos[1].file_id, "large");
        assert_eq!(photos[1].file_size, Some(50000));
    }

    #[test]
    fn tg_message_with_document() {
        let raw = json!({
            "message_id": 3,
            "chat": { "id": 1, "type": "private" },
            "date": 1700000002,
            "document": { "file_id": "doc_1", "file_name": "test.pdf", "mime_type": "application/pdf", "file_size": 1024 }
        });
        let msg: TgMessage = serde_json::from_value(raw).unwrap();
        let doc = msg.document.unwrap();
        assert_eq!(doc.file_id, "doc_1");
        assert_eq!(doc.file_name.as_deref(), Some("test.pdf"));
    }

    #[test]
    fn send_message_request_serializes() {
        let req = SendMessageRequest {
            chat_id: 42,
            text: "Hello".into(),
            parse_mode: Some("HTML".into()),
            reply_to_message_id: None,
            reply_markup: None,
            disable_notification: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["chat_id"], 42);
        assert_eq!(json["text"], "Hello");
        assert_eq!(json["parse_mode"], "HTML");
        assert!(json.get("reply_to_message_id").is_none());
    }

    #[test]
    fn inline_keyboard_markup_serializes() {
        let markup = ReplyMarkup::InlineKeyboard(InlineKeyboardMarkup {
            inline_keyboard: vec![vec![
                InlineKeyboardButton {
                    text: "Yes".into(),
                    callback_data: Some("confirm:yes".into()),
                    url: None,
                },
                InlineKeyboardButton {
                    text: "No".into(),
                    callback_data: Some("confirm:no".into()),
                    url: None,
                },
            ]],
        });
        let json = serde_json::to_value(&markup).unwrap();
        assert_eq!(json["inline_keyboard"][0][0]["text"], "Yes");
        assert_eq!(json["inline_keyboard"][0][1]["callback_data"], "confirm:no");
    }

    #[test]
    fn reply_keyboard_markup_serializes() {
        let markup = ReplyMarkup::ReplyKeyboard(ReplyKeyboardMarkup {
            keyboard: vec![vec![KeyboardButton { text: "/start".into() }]],
            resize_keyboard: Some(true),
            one_time_keyboard: None,
        });
        let json = serde_json::to_value(&markup).unwrap();
        assert_eq!(json["keyboard"][0][0]["text"], "/start");
        assert_eq!(json["resize_keyboard"], true);
    }

    #[test]
    fn edit_message_text_request_serializes() {
        let req = EditMessageTextRequest {
            chat_id: 42,
            message_id: 99,
            text: "Updated".into(),
            parse_mode: None,
            reply_markup: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["chat_id"], 42);
        assert_eq!(json["message_id"], 99);
        assert_eq!(json["text"], "Updated");
    }

    #[test]
    fn answer_callback_query_request_serializes() {
        let req = AnswerCallbackQueryRequest {
            callback_query_id: "cb_1".into(),
            text: Some("Done!".into()),
            show_alert: Some(false),
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["callback_query_id"], "cb_1");
        assert_eq!(json["text"], "Done!");
    }

    #[test]
    fn tg_update_with_reply_to_message() {
        let raw = json!({
            "update_id": 102,
            "message": {
                "message_id": 5,
                "from": { "id": 42, "is_bot": false, "first_name": "Alice" },
                "chat": { "id": 1, "type": "private" },
                "date": 1700000003,
                "text": "Reply text",
                "reply_to_message": {
                    "message_id": 3,
                    "chat": { "id": 1, "type": "private" },
                    "date": 1700000002,
                    "text": "Original"
                }
            }
        });
        let upd: TgUpdate = serde_json::from_value(raw).unwrap();
        let msg = upd.message.unwrap();
        let reply = msg.reply_to_message.unwrap();
        assert_eq!(reply.message_id, 3);
        assert_eq!(reply.text.as_deref(), Some("Original"));
    }

    #[test]
    fn tg_message_with_voice() {
        let raw = json!({
            "message_id": 6,
            "chat": { "id": 1, "type": "private" },
            "date": 1700000004,
            "voice": { "file_id": "voice_1", "duration": 5, "mime_type": "audio/ogg", "file_size": 8192 }
        });
        let msg: TgMessage = serde_json::from_value(raw).unwrap();
        let voice = msg.voice.unwrap();
        assert_eq!(voice.file_id, "voice_1");
        assert_eq!(voice.duration, 5);
    }

    #[test]
    fn tg_message_with_sticker() {
        let raw = json!({
            "message_id": 7,
            "chat": { "id": 1, "type": "private" },
            "date": 1700000005,
            "sticker": { "file_id": "sticker_1", "emoji": "😀" }
        });
        let msg: TgMessage = serde_json::from_value(raw).unwrap();
        let sticker = msg.sticker.unwrap();
        assert_eq!(sticker.file_id, "sticker_1");
        assert_eq!(sticker.emoji.as_deref(), Some("😀"));
    }
}
