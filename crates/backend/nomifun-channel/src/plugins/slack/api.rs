//! HTTP client for the Slack Web API.
//!
//! Provides typed methods for `auth.test`, `apps.connections.open`,
//! `chat.postMessage`, and `chat.update`.

use reqwest::Client;
use tracing::debug;

use crate::error::ChannelError;

use super::types::{
    AuthTestResult, ConnectionOpenResult, PostMessageRequest, PostMessageResult,
    SlackResponse, UpdateMessageRequest, UpdateMessageResult,
};

const SLACK_API_BASE: &str = "https://slack.com/api";

/// HTTP client for the Slack Web API.
///
/// Wraps `reqwest::Client` with two tokens:
/// - `bot_token` (xoxb-): used for all regular API calls
/// - `app_token` (xapp-): used only for `apps.connections.open`
pub(crate) struct SlackApi {
    client: Client,
    bot_token: String,
    app_token: String,
}

impl SlackApi {
    /// Create a new API client.
    pub fn new(client: Client, bot_token: &str, app_token: &str) -> Self {
        Self {
            client,
            bot_token: bot_token.to_string(),
            app_token: app_token.to_string(),
        }
    }

    /// `auth.test` -- validate the bot token and return bot identity.
    pub async fn auth_test(&self) -> Result<AuthTestResult, ChannelError> {
        let url = format!("{SLACK_API_BASE}/auth.test");
        let resp: SlackResponse<AuthTestResult> = self
            .client
            .post(&url)
            .bearer_auth(&self.bot_token)
            .send()
            .await
            .map_err(|e| ChannelError::PlatformApi(format!("auth.test request failed: {e}")))?
            .json()
            .await
            .map_err(|e| ChannelError::PlatformApi(format!("auth.test parse failed: {e}")))?;

        if !resp.ok {
            let desc = resp.error.unwrap_or_default();
            return Err(ChannelError::ConnectionFailed(format!(
                "Slack auth.test failed: {desc}"
            )));
        }

        resp.data
            .ok_or_else(|| ChannelError::PlatformApi("auth.test returned no result".into()))
    }

    /// `apps.connections.open` -- obtain a WebSocket URL for Socket Mode.
    pub async fn open_connection(&self) -> Result<String, ChannelError> {
        let url = format!("{SLACK_API_BASE}/apps.connections.open");
        let resp: SlackResponse<ConnectionOpenResult> = self
            .client
            .post(&url)
            .bearer_auth(&self.app_token)
            .send()
            .await
            .map_err(|e| {
                ChannelError::ConnectionFailed(format!(
                    "apps.connections.open request failed: {e}"
                ))
            })?
            .json()
            .await
            .map_err(|e| {
                ChannelError::PlatformApi(format!(
                    "apps.connections.open parse failed: {e}"
                ))
            })?;

        if !resp.ok {
            let desc = resp.error.unwrap_or_default();
            return Err(ChannelError::ConnectionFailed(format!(
                "Slack apps.connections.open failed: {desc}"
            )));
        }

        resp.data
            .and_then(|d| d.url)
            .filter(|u| !u.is_empty())
            .ok_or_else(|| {
                ChannelError::ConnectionFailed(
                    "apps.connections.open returned no URL".into(),
                )
            })
    }

    /// `chat.postMessage` -- send a message to a channel.
    pub async fn post_message(
        &self,
        req: &PostMessageRequest,
    ) -> Result<String, ChannelError> {
        let url = format!("{SLACK_API_BASE}/chat.postMessage");
        debug!(channel = %req.channel, "Sending Slack message");

        let resp: SlackResponse<PostMessageResult> = self
            .client
            .post(&url)
            .bearer_auth(&self.bot_token)
            .json(req)
            .send()
            .await
            .map_err(|e| {
                ChannelError::MessageSendFailed(format!(
                    "chat.postMessage request failed: {e}"
                ))
            })?
            .json()
            .await
            .map_err(|e| {
                ChannelError::MessageSendFailed(format!(
                    "chat.postMessage parse failed: {e}"
                ))
            })?;

        if !resp.ok {
            let desc = resp.error.unwrap_or_default();
            return Err(ChannelError::MessageSendFailed(format!(
                "chat.postMessage failed: {desc}"
            )));
        }

        resp.data
            .and_then(|d| d.ts)
            .ok_or_else(|| {
                ChannelError::MessageSendFailed(
                    "chat.postMessage returned no ts".into(),
                )
            })
    }

    /// `chat.update` -- edit an existing message.
    pub async fn update_message(
        &self,
        req: &UpdateMessageRequest,
    ) -> Result<(), ChannelError> {
        let url = format!("{SLACK_API_BASE}/chat.update");
        debug!(channel = %req.channel, ts = %req.ts, "Editing Slack message");

        let resp: SlackResponse<UpdateMessageResult> = self
            .client
            .post(&url)
            .bearer_auth(&self.bot_token)
            .json(req)
            .send()
            .await
            .map_err(|e| {
                ChannelError::MessageSendFailed(format!(
                    "chat.update request failed: {e}"
                ))
            })?
            .json()
            .await
            .map_err(|e| {
                ChannelError::MessageSendFailed(format!(
                    "chat.update parse failed: {e}"
                ))
            })?;

        if !resp.ok {
            let desc = resp.error.unwrap_or_default();
            return Err(ChannelError::MessageSendFailed(format!(
                "chat.update failed: {desc}"
            )));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_stores_tokens() {
        let client = Client::new();
        let api = SlackApi::new(client, "xoxb-test", "xapp-test");
        assert_eq!(api.bot_token, "xoxb-test");
        assert_eq!(api.app_token, "xapp-test");
    }
}
