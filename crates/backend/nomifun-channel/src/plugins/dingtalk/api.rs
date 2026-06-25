use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::Client;
use tokio::sync::RwLock;
use tracing::{debug, warn};

use crate::error::ChannelError;

use super::types::{
    AccessTokenRequest, AccessTokenResponse, CreateCardInstanceRequest, CreateCardInstanceResponse, DeliverCardRequest,
    DeliverCardResponse, RegisterStreamRequest, RegisterStreamResponse, RobotInfoResponse, SendRobotMessageRequest,
    SendRobotMessageResponse, StreamSubscription, StreamingWriteRequest, StreamingWriteResponse, UpdateCardRequest,
    UpdateCardResponse,
};

const DINGTALK_API_BASE: &str = "https://api.dingtalk.com";

/// Token refresh margin — refresh 5 minutes before expiry.
const TOKEN_REFRESH_MARGIN: Duration = Duration::from_secs(5 * 60);

/// Cached access token with expiry tracking.
struct TokenCache {
    token: String,
    acquired_at: Instant,
    expires_in: Duration,
}

impl TokenCache {
    fn is_expired(&self) -> bool {
        self.acquired_at.elapsed() + TOKEN_REFRESH_MARGIN >= self.expires_in
    }
}

/// HTTP client for the DingTalk Open Platform API.
///
/// Manages access token lifecycle (auto-refresh) and provides typed
/// methods for bot info, WebSocket stream registration, AI Card
/// operations, and message sending.
pub(crate) struct DingtalkApi {
    client: Client,
    client_id: String,
    client_secret: String,
    token_cache: Arc<RwLock<Option<TokenCache>>>,
}

impl DingtalkApi {
    /// Create a new DingTalk API client.
    pub fn new(client: Client, client_id: &str, client_secret: &str) -> Self {
        Self {
            client,
            client_id: client_id.to_string(),
            client_secret: client_secret.to_string(),
            token_cache: Arc::new(RwLock::new(None)),
        }
    }

    /// Get a valid access token, refreshing if needed.
    async fn get_token(&self) -> Result<String, ChannelError> {
        // Fast path: check if cached token is still valid
        {
            let cache = self.token_cache.read().await;
            if let Some(ref tc) = *cache
                && !tc.is_expired()
            {
                return Ok(tc.token.clone());
            }
        }

        // Slow path: refresh the token
        self.refresh_token().await
    }

    /// Request a new access token from DingTalk (v2 endpoint).
    async fn refresh_token(&self) -> Result<String, ChannelError> {
        let url = format!("{DINGTALK_API_BASE}/v1.0/oauth2/accessToken");
        let body = AccessTokenRequest {
            app_key: self.client_id.clone(),
            app_secret: self.client_secret.clone(),
        };

        let resp: AccessTokenResponse = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| ChannelError::ConnectionFailed(format!("DingTalk token request failed: {e}")))?
            .json()
            .await
            .map_err(|e| ChannelError::ConnectionFailed(format!("DingTalk token parse failed: {e}")))?;

        if let Some(code) = resp.errcode
            && code != 0
        {
            return Err(ChannelError::ConnectionFailed(format!(
                "DingTalk token error (code={}): {}",
                code,
                resp.errmsg.as_deref().unwrap_or("unknown")
            )));
        }

        let token = resp
            .access_token
            .ok_or_else(|| ChannelError::ConnectionFailed("DingTalk token response missing token".into()))?;

        let expires_in = Duration::from_secs(resp.expire_in.unwrap_or(7200) as u64);

        debug!(expires_in_secs = expires_in.as_secs(), "DingTalk token refreshed");

        let mut cache = self.token_cache.write().await;
        *cache = Some(TokenCache {
            token: token.clone(),
            acquired_at: Instant::now(),
            expires_in,
        });

        Ok(token)
    }

    /// Get bot identity information.
    pub async fn get_bot_info(&self) -> Result<RobotInfoResponse, ChannelError> {
        let token = self.get_token().await?;
        let url = format!("{DINGTALK_API_BASE}/v1.0/im/robot/info");

        let resp: RobotInfoResponse = self
            .client
            .post(&url)
            .header("x-acs-dingtalk-access-token", &token)
            .json(&serde_json::json!({}))
            .send()
            .await
            .map_err(|e| ChannelError::PlatformApi(format!("DingTalk bot info request failed: {e}")))?
            .json()
            .await
            .map_err(|e| ChannelError::PlatformApi(format!("DingTalk bot info parse failed: {e}")))?;

        Ok(resp)
    }

    /// Register for WebSocket Stream and get the connection endpoint.
    ///
    /// Note: This endpoint uses clientId/clientSecret in the request body,
    /// NOT the access token header. The Accept header is required to get
    /// JSON instead of XML.
    pub async fn register_stream(&self) -> Result<RegisterStreamResponse, ChannelError> {
        let url = format!("{DINGTALK_API_BASE}/v1.0/gateway/connections/open");

        let body = RegisterStreamRequest {
            client_id: self.client_id.clone(),
            client_secret: self.client_secret.clone(),
            subscriptions: vec![
                StreamSubscription {
                    sub_type: "EVENT".into(),
                    topic: "*".into(),
                },
                StreamSubscription {
                    sub_type: "CALLBACK".into(),
                    topic: "/v1.0/im/bot/messages/get".into(),
                },
                StreamSubscription {
                    sub_type: "CALLBACK".into(),
                    topic: "/v1.0/card/instances/callback".into(),
                },
            ],
            ua: Some("nomicore".into()),
        };

        let raw_resp = self
            .client
            .post(&url)
            .header("Accept", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ChannelError::ConnectionFailed(format!("DingTalk stream registration failed: {e}")))?;

        let status = raw_resp.status();
        let body_text = raw_resp.text().await.map_err(|e| {
            ChannelError::ConnectionFailed(format!("DingTalk stream registration read body failed: {e}"))
        })?;

        debug!(status = %status, body_len = body_text.len(), "DingTalk stream registration response received");

        let resp: RegisterStreamResponse = serde_json::from_str(&body_text).map_err(|e| {
            ChannelError::ConnectionFailed(format!(
                "DingTalk stream registration parse failed: {e}, body: {}",
                &body_text[..body_text.len().min(200)]
            ))
        })?;

        Ok(resp)
    }

    /// Create an AI Card instance.
    pub async fn create_card_instance(
        &self,
        request: &CreateCardInstanceRequest,
    ) -> Result<CreateCardInstanceResponse, ChannelError> {
        let token = self.get_token().await?;
        let url = format!("{DINGTALK_API_BASE}/v1.0/card/instances");

        let resp: CreateCardInstanceResponse = self
            .client
            .post(&url)
            .header("x-acs-dingtalk-access-token", &token)
            .json(request)
            .send()
            .await
            .map_err(|e| ChannelError::MessageSendFailed(format!("DingTalk create card failed: {e}")))?
            .json()
            .await
            .map_err(|e| ChannelError::MessageSendFailed(format!("DingTalk create card parse failed: {e}")))?;

        if resp.success != Some(true) {
            return Err(ChannelError::MessageSendFailed(
                "DingTalk create card returned success=false".into(),
            ));
        }

        Ok(resp)
    }

    /// Deliver a card instance to a chat.
    pub async fn deliver_card(&self, request: &DeliverCardRequest) -> Result<DeliverCardResponse, ChannelError> {
        let token = self.get_token().await?;
        let url = format!("{DINGTALK_API_BASE}/v1.0/card/instances/deliver");

        let resp: DeliverCardResponse = self
            .client
            .post(&url)
            .header("x-acs-dingtalk-access-token", &token)
            .json(request)
            .send()
            .await
            .map_err(|e| ChannelError::MessageSendFailed(format!("DingTalk deliver card failed: {e}")))?
            .json()
            .await
            .map_err(|e| ChannelError::MessageSendFailed(format!("DingTalk deliver card parse failed: {e}")))?;

        if resp.success != Some(true) {
            warn!("DingTalk deliver card returned success=false");
        }

        Ok(resp)
    }

    /// Stream-write content to an AI Card (append mode).
    pub async fn streaming_write(
        &self,
        request: &StreamingWriteRequest,
    ) -> Result<StreamingWriteResponse, ChannelError> {
        let token = self.get_token().await?;
        let url = format!("{DINGTALK_API_BASE}/v1.0/card/streaming");

        let resp: StreamingWriteResponse = self
            .client
            .put(&url)
            .header("x-acs-dingtalk-access-token", &token)
            .json(request)
            .send()
            .await
            .map_err(|e| ChannelError::MessageSendFailed(format!("DingTalk streaming write failed: {e}")))?
            .json()
            .await
            .map_err(|e| ChannelError::MessageSendFailed(format!("DingTalk streaming write parse failed: {e}")))?;

        Ok(resp)
    }

    /// Update (finalize) an AI Card instance with new data (e.g., buttons).
    pub async fn update_card(&self, request: &UpdateCardRequest) -> Result<UpdateCardResponse, ChannelError> {
        let token = self.get_token().await?;
        let url = format!("{DINGTALK_API_BASE}/v1.0/card/instances");

        let resp: UpdateCardResponse = self
            .client
            .put(&url)
            .header("x-acs-dingtalk-access-token", &token)
            .json(request)
            .send()
            .await
            .map_err(|e| ChannelError::MessageSendFailed(format!("DingTalk update card failed: {e}")))?
            .json()
            .await
            .map_err(|e| ChannelError::MessageSendFailed(format!("DingTalk update card parse failed: {e}")))?;

        Ok(resp)
    }

    /// Send a message via DingTalk Open API (fallback).
    pub async fn send_robot_message(
        &self,
        request: &SendRobotMessageRequest,
    ) -> Result<SendRobotMessageResponse, ChannelError> {
        let token = self.get_token().await?;

        let url = if request.open_conversation_id.is_some() {
            format!("{DINGTALK_API_BASE}/v1.0/robot/groupMessages/send")
        } else {
            format!("{DINGTALK_API_BASE}/v1.0/robot/oToMessages/batchSend")
        };

        let resp: SendRobotMessageResponse = self
            .client
            .post(&url)
            .header("x-acs-dingtalk-access-token", &token)
            .json(request)
            .send()
            .await
            .map_err(|e| ChannelError::MessageSendFailed(format!("DingTalk robot message send failed: {e}")))?
            .json()
            .await
            .map_err(|e| ChannelError::MessageSendFailed(format!("DingTalk robot message parse failed: {e}")))?;

        Ok(resp)
    }

    /// Expose client_id for card delivery.
    pub fn client_id(&self) -> &str {
        &self.client_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_stores_credentials() {
        let client = Client::new();
        let api = DingtalkApi::new(client, "key_123", "secret_456");
        assert_eq!(api.client_id, "key_123");
        assert_eq!(api.client_secret, "secret_456");
    }

    #[test]
    fn token_cache_not_expired() {
        let cache = TokenCache {
            token: "test".into(),
            acquired_at: Instant::now(),
            expires_in: Duration::from_secs(7200),
        };
        assert!(!cache.is_expired());
    }

    #[test]
    fn token_cache_expired() {
        let cache = TokenCache {
            token: "test".into(),
            acquired_at: Instant::now() - Duration::from_secs(7200),
            expires_in: Duration::from_secs(7200),
        };
        assert!(cache.is_expired());
    }

    #[test]
    fn token_cache_near_expiry() {
        // Token acquired 6900s ago with 7200s TTL — within 5min margin
        let cache = TokenCache {
            token: "test".into(),
            acquired_at: Instant::now() - Duration::from_secs(6900),
            expires_in: Duration::from_secs(7200),
        };
        assert!(cache.is_expired());
    }

    #[test]
    fn client_id_accessor() {
        let client = Client::new();
        let api = DingtalkApi::new(client, "my_key", "my_secret");
        assert_eq!(api.client_id(), "my_key");
    }
}
