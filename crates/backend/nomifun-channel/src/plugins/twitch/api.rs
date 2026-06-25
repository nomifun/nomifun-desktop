//! Twitch OAuth validation API client.

use reqwest::Client;
use tracing::debug;

use crate::error::ChannelError;

use super::types::ValidateResponse;

const TWITCH_VALIDATE_URL: &str = "https://id.twitch.tv/oauth2/validate";

/// HTTP client for Twitch OAuth token validation.
///
/// Only the validate endpoint is needed — all chat I/O goes over IRC-over-WS.
pub(crate) struct TwitchApi {
    client: Client,
}

impl TwitchApi {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    /// Validate the OAuth access token and return the bot's login/user_id.
    ///
    /// `GET https://id.twitch.tv/oauth2/validate`
    /// Header: `Authorization: OAuth <token>`
    pub async fn validate(&self, token: &str) -> Result<ValidateResponse, ChannelError> {
        debug!("Validating Twitch OAuth token");
        let resp = self
            .client
            .get(TWITCH_VALIDATE_URL)
            .header("Authorization", format!("OAuth {token}"))
            .send()
            .await
            .map_err(|e| ChannelError::PlatformApi(format!("Twitch validate request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(ChannelError::ConnectionFailed(format!(
                "Twitch token validation failed: HTTP {status}: {body}"
            )));
        }

        resp.json()
            .await
            .map_err(|e| ChannelError::PlatformApi(format!("Twitch validate parse failed: {e}")))
    }
}
