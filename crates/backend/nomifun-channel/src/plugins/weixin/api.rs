use std::time::Duration;

use base64::Engine;
use reqwest::Client;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tracing::{debug, warn};
use uuid::Uuid;

use crate::constants::{WEIXIN_API_TIMEOUT, WEIXIN_POLL_TIMEOUT};
use crate::error::ChannelError;

use super::types::{
    GetUpdatesRequest, GetUpdatesResponse, ILinkResponse, ITEM_TYPE_TEXT, QrCodeData, QrCodeStatusData,
    SendMessageItem, SendMessageMsg, SendMessageRequest, SendTextItem,
};

/// HTTP client for the WeChat iLink Bot API.
pub(crate) struct WeixinApi {
    client: Client,
    base_url: String,
    bot_token: String,
    wechat_uin: String,
}

impl WeixinApi {
    pub fn new(client: Client, base_url: &str, bot_token: &str) -> Self {
        let base = base_url.trim_end_matches('/');

        let mut uin_bytes = [0u8; 4];
        getrandom::getrandom(&mut uin_bytes).expect("RNG failure");
        let wechat_uin = base64::engine::general_purpose::STANDARD.encode(uin_bytes);

        Self {
            client,
            base_url: base.to_string(),
            bot_token: bot_token.to_string(),
            wechat_uin,
        }
    }

    #[cfg(test)]
    pub fn bot_token(&self) -> &str {
        &self.bot_token
    }

    #[cfg(test)]
    pub fn wechat_uin(&self) -> &str {
        &self.wechat_uin
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    async fn authenticated_post<T: DeserializeOwned>(
        &self,
        endpoint: &str,
        body: &impl Serialize,
        timeout: Duration,
    ) -> Result<T, ChannelError> {
        let url = format!("{}/{}", self.base_url, endpoint);

        let resp = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("AuthorizationType", "ilink_bot_token")
            .header("Authorization", format!("Bearer {}", self.bot_token))
            .header("X-WECHAT-UIN", &self.wechat_uin)
            .timeout(timeout)
            .json(body)
            .send()
            .await
            .map_err(|e| ChannelError::PlatformApi(format!("{endpoint} request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(ChannelError::PlatformApi(format!("{endpoint} HTTP {status}: {text}")));
        }

        resp.json()
            .await
            .map_err(|e| ChannelError::PlatformApi(format!("{endpoint} parse failed: {e}")))
    }

    async fn ilink_get<T: DeserializeOwned>(&self, endpoint: &str, query: &[(&str, &str)]) -> Result<T, ChannelError> {
        let url = format!("{}/{}", self.base_url, endpoint);

        let resp = self
            .client
            .get(&url)
            .header("iLink-App-ClientVersion", "1")
            .query(query)
            .send()
            .await
            .map_err(|e| ChannelError::PlatformApi(format!("{endpoint} request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(ChannelError::PlatformApi(format!("{endpoint} HTTP {status}: {text}")));
        }

        resp.json()
            .await
            .map_err(|e| ChannelError::PlatformApi(format!("{endpoint} parse failed: {e}")))
    }

    // -----------------------------------------------------------------------
    // QR code login
    // -----------------------------------------------------------------------

    /// Fetch a QR code for bot login.
    ///
    /// `GET /ilink/bot/get_bot_qrcode?bot_type=3`
    pub async fn get_bot_qrcode(&self) -> Result<QrCodeData, ChannelError> {
        debug!("Fetching WeChat QR code");

        // Try direct response first, then wrapped
        let result: Result<QrCodeData, _> = self.ilink_get("ilink/bot/get_bot_qrcode", &[("bot_type", "3")]).await;

        match result {
            Ok(data) if data.qrcode.is_some() => Ok(data),
            _ => {
                let wrapped: ILinkResponse<QrCodeData> =
                    self.ilink_get("ilink/bot/get_bot_qrcode", &[("bot_type", "3")]).await?;
                wrapped
                    .data
                    .ok_or_else(|| ChannelError::PlatformApi("get_bot_qrcode returned no data".into()))
            }
        }
    }

    /// Check the status of a QR code scan.
    ///
    /// `GET /ilink/bot/get_qrcode_status?qrcode=<ticket>`
    pub async fn get_qrcode_status(&self, qrcode: &str) -> Result<QrCodeStatusData, ChannelError> {
        // Try direct response first, then wrapped
        let result: Result<QrCodeStatusData, _> = self
            .ilink_get("ilink/bot/get_qrcode_status", &[("qrcode", qrcode)])
            .await;

        match result {
            Ok(data) if data.status.is_some() => Ok(data),
            _ => {
                let wrapped: ILinkResponse<QrCodeStatusData> = self
                    .ilink_get("ilink/bot/get_qrcode_status", &[("qrcode", qrcode)])
                    .await?;
                wrapped
                    .data
                    .ok_or_else(|| ChannelError::PlatformApi("get_qrcode_status returned no data".into()))
            }
        }
    }

    // -----------------------------------------------------------------------
    // Long-polling
    // -----------------------------------------------------------------------

    /// Long-poll for new updates using buffer-based protocol.
    ///
    /// `POST /ilink/bot/getupdates`
    pub async fn get_updates(&self, buf: &str) -> Result<GetUpdatesResponse, ChannelError> {
        let body = GetUpdatesRequest {
            get_updates_buf: buf.to_string(),
            base_info: serde_json::json!({}),
        };

        let timeout = WEIXIN_POLL_TIMEOUT + Duration::from_secs(10);

        self.authenticated_post("ilink/bot/getupdates", &body, timeout).await
    }

    // -----------------------------------------------------------------------
    // Send message
    // -----------------------------------------------------------------------

    /// Send a text message.
    ///
    /// `POST /ilink/bot/sendmessage`
    pub async fn send_message(
        &self,
        to_user_id: &str,
        text: &str,
        context_token: Option<&str>,
    ) -> Result<(), ChannelError> {
        debug!(to_user_id, "Sending WeChat message");

        let body = SendMessageRequest {
            msg: SendMessageMsg {
                to_user_id: to_user_id.to_string(),
                client_id: Uuid::new_v4().to_string(),
                message_type: 2,
                message_state: 2,
                item_list: vec![SendMessageItem {
                    item_type: ITEM_TYPE_TEXT,
                    text_item: Some(SendTextItem { text: text.to_string() }),
                }],
                context_token: context_token.map(String::from),
            },
            base_info: serde_json::json!({}),
        };

        let _resp: serde_json::Value = self
            .authenticated_post("ilink/bot/sendmessage", &body, WEIXIN_API_TIMEOUT)
            .await
            .map_err(|e| {
                warn!(to_user_id, error = %e, "sendmessage failed");
                ChannelError::MessageSendFailed(format!("sendmessage failed: {e}"))
            })?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_stores_credentials() {
        let client = Client::new();
        let api = WeixinApi::new(client, "https://ilinkai.weixin.qq.com/", "tok_abc");
        assert_eq!(api.base_url, "https://ilinkai.weixin.qq.com");
        assert_eq!(api.bot_token(), "tok_abc");
    }

    #[test]
    fn api_normalizes_trailing_slash() {
        let client = Client::new();
        let api = WeixinApi::new(client, "https://ilinkai.weixin.qq.com///", "tok");
        assert!(api.base_url.ends_with("com"));
    }

    #[test]
    fn api_generates_wechat_uin() {
        let client = Client::new();
        let api = WeixinApi::new(client, "https://example.com", "tok");
        // base64 of 4 bytes should be 8 chars (with padding)
        assert_eq!(api.wechat_uin().len(), 8);
        // Should be valid base64
        let decoded = base64::engine::general_purpose::STANDARD.decode(api.wechat_uin());
        assert!(decoded.is_ok());
        assert_eq!(decoded.unwrap().len(), 4);
    }

    #[test]
    fn api_generates_different_uin_each_time() {
        let client1 = Client::new();
        let api1 = WeixinApi::new(client1, "https://example.com", "tok");
        let client2 = Client::new();
        let api2 = WeixinApi::new(client2, "https://example.com", "tok");
        // Extremely unlikely to collide (2^32 space)
        assert_ne!(api1.wechat_uin(), api2.wechat_uin());
    }
}
