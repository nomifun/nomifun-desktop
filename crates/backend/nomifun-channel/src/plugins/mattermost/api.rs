//! HTTP client for the Mattermost API v4.
//!
//! Wraps `reqwest::Client`, a base server URL, and a bearer token.  Provides
//! typed methods for the three endpoints the plugin requires:
//!
//! - `GET  /api/v4/users/me`        — validate token, fetch bot identity
//! - `POST /api/v4/posts`           — create a post (send message)
//! - `PUT  /api/v4/posts/{post_id}` — update a post (edit/stream message)

use reqwest::Client;
use tracing::debug;

use crate::error::ChannelError;

use super::types::{CreatePostRequest, CreatePostResponse, MmUser, UpdatePostRequest};

/// REST client for the Mattermost API v4.
pub(crate) struct MattermostApi {
    client: Client,
    /// Base URL without trailing slash, e.g. `https://mm.example.com`.
    base_url: String,
    /// Bot access token (sent as `Authorization: Bearer <token>`).
    token: String,
}

impl MattermostApi {
    pub fn new(client: Client, base_url: &str, token: &str) -> Self {
        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_owned(),
            token: token.to_owned(),
        }
    }

    /// `GET /api/v4/users/me` — fetch bot identity.
    pub async fn get_me(&self) -> Result<MmUser, ChannelError> {
        let url = format!("{}/api/v4/users/me", self.base_url);
        let resp = self
            .client
            .get(&url)
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| ChannelError::PlatformApi(format!("Mattermost get_me request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(ChannelError::ConnectionFailed(format!(
                "Mattermost get_me failed ({status}): {body}"
            )));
        }

        resp.json::<MmUser>()
            .await
            .map_err(|e| ChannelError::PlatformApi(format!("Mattermost get_me parse failed: {e}")))
    }

    /// `POST /api/v4/posts` — create a post. Returns the post id.
    pub async fn create_post(&self, req: &CreatePostRequest) -> Result<CreatePostResponse, ChannelError> {
        let url = format!("{}/api/v4/posts", self.base_url);
        debug!(channel_id = %req.channel_id, "Mattermost creating post");

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.token)
            .json(req)
            .send()
            .await
            .map_err(|e| ChannelError::MessageSendFailed(format!("Mattermost create_post request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(ChannelError::MessageSendFailed(format!(
                "Mattermost create_post failed ({status}): {body}"
            )));
        }

        resp.json::<CreatePostResponse>()
            .await
            .map_err(|e| ChannelError::MessageSendFailed(format!("Mattermost create_post parse failed: {e}")))
    }

    /// `PUT /api/v4/posts/{post_id}` — update an existing post.
    pub async fn update_post(&self, req: &UpdatePostRequest) -> Result<(), ChannelError> {
        let url = format!("{}/api/v4/posts/{}", self.base_url, req.id);
        debug!(post_id = %req.id, "Mattermost updating post");

        let resp = self
            .client
            .put(&url)
            .bearer_auth(&self.token)
            .json(req)
            .send()
            .await
            .map_err(|e| ChannelError::MessageSendFailed(format!("Mattermost update_post request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(ChannelError::MessageSendFailed(format!(
                "Mattermost update_post failed ({status}): {body}"
            )));
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
        let api = MattermostApi::new(client, "https://mm.example.com/", "my-token");
        assert_eq!(api.base_url, "https://mm.example.com");
        assert_eq!(api.token, "my-token");
    }

    #[test]
    fn api_strips_trailing_slash() {
        let client = Client::new();
        let api = MattermostApi::new(client, "https://mm.example.com///", "tok");
        assert_eq!(api.base_url, "https://mm.example.com");
    }
}
