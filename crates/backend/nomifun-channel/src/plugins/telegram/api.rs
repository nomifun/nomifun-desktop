use reqwest::Client;
use tracing::{debug, warn};

use crate::error::ChannelError;

use super::types::{
    AnswerCallbackQueryRequest, EditMessageTextRequest, SendMessageRequest, TgMessage, TgResponse, TgUpdate, TgUser,
};

const TELEGRAM_API_BASE: &str = "https://api.telegram.org";

/// HTTP client for the Telegram Bot API.
///
/// Wraps `reqwest::Client` and a bot token. Provides typed methods
/// for `getMe`, `getUpdates`, `sendMessage`, `editMessageText`, and
/// `answerCallbackQuery`.
pub(crate) struct TelegramApi {
    client: Client,
    base_url: String,
}

impl TelegramApi {
    /// Create a new API client for the given bot token.
    pub fn new(client: Client, token: &str) -> Self {
        Self {
            client,
            base_url: format!("{TELEGRAM_API_BASE}/bot{token}"),
        }
    }

    /// `getMe` — returns the bot's user identity.
    pub async fn get_me(&self) -> Result<TgUser, ChannelError> {
        let url = format!("{}/getMe", self.base_url);
        let resp: TgResponse<TgUser> = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| ChannelError::PlatformApi(format!("getMe request failed: {e}")))?
            .json()
            .await
            .map_err(|e| ChannelError::PlatformApi(format!("getMe parse failed: {e}")))?;

        if !resp.ok {
            let desc = resp.description.unwrap_or_default();
            return Err(ChannelError::ConnectionFailed(format!("Telegram getMe failed: {desc}")));
        }

        resp.result
            .ok_or_else(|| ChannelError::PlatformApi("getMe returned no result".into()))
    }

    /// `getUpdates` — long-poll for new updates.
    ///
    /// - `offset`: return updates with `update_id >= offset`
    /// - `timeout`: long-polling timeout in seconds (0 = short poll)
    pub async fn get_updates(&self, offset: Option<i64>, timeout: u32) -> Result<Vec<TgUpdate>, ChannelError> {
        let url = format!("{}/getUpdates", self.base_url);

        let mut params = vec![("timeout", timeout.to_string())];
        if let Some(off) = offset {
            params.push(("offset", off.to_string()));
        }

        let resp: TgResponse<Vec<TgUpdate>> = self
            .client
            .get(&url)
            .query(&params)
            .send()
            .await
            .map_err(|e| ChannelError::PlatformApi(format!("getUpdates request failed: {e}")))?
            .json()
            .await
            .map_err(|e| ChannelError::PlatformApi(format!("getUpdates parse failed: {e}")))?;

        if !resp.ok {
            let desc = resp.description.unwrap_or_default();
            warn!("Telegram getUpdates error: {desc}");
            return Err(ChannelError::PlatformApi(format!("getUpdates failed: {desc}")));
        }

        Ok(resp.result.unwrap_or_default())
    }

    /// `sendMessage` — send a text message. Returns the sent message.
    pub async fn send_message(&self, req: &SendMessageRequest) -> Result<TgMessage, ChannelError> {
        let url = format!("{}/sendMessage", self.base_url);
        debug!(chat_id = req.chat_id, "Sending Telegram message");

        let resp: TgResponse<TgMessage> = self
            .client
            .post(&url)
            .json(req)
            .send()
            .await
            .map_err(|e| ChannelError::MessageSendFailed(format!("sendMessage request failed: {e}")))?
            .json()
            .await
            .map_err(|e| ChannelError::MessageSendFailed(format!("sendMessage parse failed: {e}")))?;

        if !resp.ok {
            let desc = resp.description.unwrap_or_default();
            return Err(ChannelError::MessageSendFailed(format!("sendMessage failed: {desc}")));
        }

        resp.result
            .ok_or_else(|| ChannelError::MessageSendFailed("sendMessage returned no result".into()))
    }

    /// `editMessageText` — edit an existing text message.
    pub async fn edit_message_text(&self, req: &EditMessageTextRequest) -> Result<(), ChannelError> {
        let url = format!("{}/editMessageText", self.base_url);
        debug!(
            chat_id = req.chat_id,
            message_id = req.message_id,
            "Editing Telegram message"
        );

        let resp: TgResponse<TgMessage> = self
            .client
            .post(&url)
            .json(req)
            .send()
            .await
            .map_err(|e| ChannelError::MessageSendFailed(format!("editMessageText request failed: {e}")))?
            .json()
            .await
            .map_err(|e| ChannelError::MessageSendFailed(format!("editMessageText parse failed: {e}")))?;

        if !resp.ok {
            let desc = resp.description.unwrap_or_default();
            return Err(ChannelError::MessageSendFailed(format!(
                "editMessageText failed: {desc}"
            )));
        }

        Ok(())
    }

    /// `answerCallbackQuery` — acknowledge a callback query.
    pub async fn answer_callback_query(&self, req: &AnswerCallbackQueryRequest) -> Result<(), ChannelError> {
        let url = format!("{}/answerCallbackQuery", self.base_url);

        let resp: TgResponse<bool> = self
            .client
            .post(&url)
            .json(req)
            .send()
            .await
            .map_err(|e| ChannelError::PlatformApi(format!("answerCallbackQuery request failed: {e}")))?
            .json()
            .await
            .map_err(|e| ChannelError::PlatformApi(format!("answerCallbackQuery parse failed: {e}")))?;

        if !resp.ok {
            let desc = resp.description.unwrap_or_default();
            warn!("answerCallbackQuery error: {desc}");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_constructs_correct_base_url() {
        let client = Client::new();
        let api = TelegramApi::new(client, "123:ABC");
        assert_eq!(api.base_url, "https://api.telegram.org/bot123:ABC");
    }
}
