use std::sync::Arc;

use nomifun_api_types::{PairingRequestedPayload, UserAuthorizedPayload, WebSocketMessage};
use nomifun_common::{TimestampMs, now_ms};
use nomifun_db::{IChannelRepository, PairingApprovalOutcome};
use nomifun_db::models::{
    CHANNEL_USER_AUTHORIZATION_APPROVED, CHANNEL_USER_AUTHORIZATION_AUTO_GROUP,
    ChannelPairingCodeRow, ChannelPluginRow, ChannelUserRow, NewChannelPairingCodeRow,
    NewChannelUserRow,
};
use nomifun_realtime::UserEventSink;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::constants::{PAIRING_CLEANUP_INTERVAL, PAIRING_CODE_LENGTH, PAIRING_CODE_TTL};
use crate::error::ChannelError;
use crate::group_policy::GroupPolicyFence;
use crate::types::PairingStatus;

/// Generates a random numeric pairing code of the configured length.
///
/// Uses `getrandom` for cryptographically secure randomness.
/// Returns a zero-padded string (e.g., "003421").
pub fn generate_pairing_code() -> Result<String, ChannelError> {
    let mut bytes = [0u8; 4];
    getrandom::getrandom(&mut bytes).map_err(|e| ChannelError::InvalidConfig(format!("RNG failure: {e}")))?;
    let num = u32::from_le_bytes(bytes) % 10u32.pow(PAIRING_CODE_LENGTH as u32);
    Ok(format!("{num:0>width$}", width = PAIRING_CODE_LENGTH))
}

/// Service for managing pairing authorization flow.
///
/// Handles:
/// - Pairing code generation and creation
/// - Approval / rejection of pairing requests
/// - Periodic cleanup of expired codes
/// - Event broadcasting to WebSocket clients
pub struct PairingService {
    repo: Arc<dyn IChannelRepository>,
    owner_id: Arc<str>,
    user_events: Arc<dyn UserEventSink>,
    pending_group_request_lock: tokio::sync::Mutex<()>,
    group_policy_fence: Arc<GroupPolicyFence>,
}

impl PairingService {
    pub fn new(
        repo: Arc<dyn IChannelRepository>,
        user_events: Arc<dyn UserEventSink>,
        owner_id: impl Into<Arc<str>>,
    ) -> Self {
        Self {
            repo,
            owner_id: owner_id.into(),
            user_events,
            pending_group_request_lock: tokio::sync::Mutex::new(()),
            group_policy_fence: Arc::new(GroupPolicyFence::default()),
        }
    }

    /// Share the manager-owned policy fence with pairing promotion.
    pub fn with_group_policy_fence(mut self, fence: Arc<GroupPolicyFence>) -> Self {
        self.group_policy_fence = fence;
        self
    }

    pub(crate) fn group_policy_fence(&self) -> Arc<GroupPolicyFence> {
        Arc::clone(&self.group_policy_fence)
    }

    /// Returns the bot row whose access policy governs an inbound message.
    pub async fn get_plugin(
        &self,
        channel_plugin_id: &str,
    ) -> Result<Option<ChannelPluginRow>, ChannelError> {
        Ok(self.repo.get_plugin(channel_plugin_id).await?)
    }

    /// Creates a pairing request for an IM user.
    ///
    /// Generates a 6-digit code, stores it with a 10-minute TTL, and
    /// broadcasts a `channel.pairing-requested` event to all WebSocket
    /// clients.
    ///
    /// If the same platform user already has a pending code, that code is
    /// marked as expired before creating the new one.
    pub async fn request_pairing(
        &self,
        platform_user_id: &str,
        platform_type: &str,
        channel_plugin_id: &str,
        display_name: Option<&str>,
    ) -> Result<String, ChannelError> {
        // Expire any existing pending codes for this user on this bot channel
        self.expire_user_pending_codes(platform_user_id, platform_type, channel_plugin_id)
            .await?;

        let code = generate_pairing_code()?;
        let now = now_ms();
        let expires_at = now + PAIRING_CODE_TTL.as_millis() as TimestampMs;

        let row = NewChannelPairingCodeRow {
            code: code.clone(),
            platform_user_id: platform_user_id.to_owned(),
            platform_type: platform_type.to_owned(),
            channel_plugin_id: Some(channel_plugin_id.to_owned()),
            display_name: display_name.map(String::from),
            requested_at: now,
            expires_at,
            status: PairingStatus::Pending.to_string(),
        };

        self.repo.create_pairing(&row).await?;

        info!(
            code = %code,
            platform_user_id = %platform_user_id,
            platform_type = %platform_type,
            channel_plugin_id,
            "pairing code created"
        );

        // Broadcast event
        let payload = PairingRequestedPayload {
            code: code.clone(),
            platform_user_id: platform_user_id.to_owned(),
            platform_type: platform_type.to_owned(),
            channel_plugin_id: Some(channel_plugin_id.to_owned()),
            display_name: display_name.map(String::from),
            requested_at: now,
            expires_at,
        };
        let value = serde_json::to_value(payload)?;
        self.user_events.send_to_user(
            &self.owner_id,
            WebSocketMessage::new("channel.pairing-requested", value),
        );

        Ok(code)
    }

    /// Return the sender's still-valid pending request for this bot, creating
    /// and broadcasting one only when none exists. Group allowlist admission
    /// uses this path so repeated mentions do not rotate a hidden code or spam
    /// the owner's approval UI. Direct-message pairing deliberately continues
    /// to use [`Self::request_pairing`] and retains its refresh semantics.
    pub async fn request_or_reuse_pending(
        &self,
        platform_user_id: &str,
        platform_type: &str,
        channel_plugin_id: &str,
        display_name: Option<&str>,
    ) -> Result<String, ChannelError> {
        let _guard = self.pending_group_request_lock.lock().await;
        let now = now_ms();
        if let Some(existing) = self
            .repo
            .get_pending_pairings()
            .await?
            .into_iter()
            .find(|row| {
                row.platform_user_id == platform_user_id
                    && row.platform_type == platform_type
                    && row.channel_plugin_id.as_deref() == Some(channel_plugin_id)
                    && row.status == PairingStatus::Pending.to_string()
                    && row.expires_at > now
            })
        {
            debug!(
                platform_user_id,
                platform_type,
                channel_plugin_id,
                "reusing pending group allowlist request"
            );
            return Ok(existing.code);
        }

        self.request_pairing(
            platform_user_id,
            platform_type,
            channel_plugin_id,
            display_name,
        )
        .await
    }

    /// Approves a pending pairing code.
    ///
    /// - Validates the code exists and is still pending + not expired
    /// - Creates an `channel_users` record
    /// - Updates the pairing status to `approved`
    /// - Broadcasts a `channel.user-authorized` event
    pub async fn approve_pairing(&self, code: &str) -> Result<(), ChannelError> {
        let row = self.get_valid_pending_pairing(code).await?;
        let channel_plugin_id = row.channel_plugin_id.as_deref().ok_or_else(|| {
            ChannelError::InvalidConfig(format!(
                "pairing code '{code}' is not scoped to a channel plugin"
            ))
        })?;

        // Promotion changes what this identity may execute. Take the same
        // write-side fence as policy mutation so every old guest admission and
        // queued delivery completes or quiesces before the privilege change.
        let promotion_writer = self.group_policy_fence.write(channel_plugin_id).await;
        let now = now_ms();
        let user = match self
            .repo
            .approve_pairing_and_retire_non_direct_sessions(code, now)
            .await?
        {
            PairingApprovalOutcome::Approved(user) => user,
            PairingApprovalOutcome::NotFound => {
                return Err(ChannelError::PairingNotFound(code.to_owned()));
            }
            PairingApprovalOutcome::AlreadyProcessed => {
                return Err(ChannelError::PairingAlreadyProcessed(code.to_owned()));
            }
            PairingApprovalOutcome::Expired => {
                return Err(ChannelError::PairingExpired(code.to_owned()));
            }
        };
        drop(promotion_writer);

        info!(
            code = %code,
            channel_user_id = %user.channel_user_id,
            platform_user_id = %row.platform_user_id,
            "pairing approved, user created"
        );

        // Broadcast event
        let payload = UserAuthorizedPayload {
            channel_user_id: user.channel_user_id,
            platform_user_id: row.platform_user_id,
            platform_type: row.platform_type,
            channel_plugin_id: row.channel_plugin_id,
            display_name: row.display_name,
            authorized_at: now,
        };
        let value = serde_json::to_value(payload)?;
        self.user_events.send_to_user(
            &self.owner_id,
            WebSocketMessage::new("channel.user-authorized", value),
        );

        Ok(())
    }

    /// Revokes one authorized identity behind the same per-plugin fence used
    /// by inbound admission, queue drain, policy changes, and promotion.
    ///
    /// The repository transition cancels every queued prompt belonging to the
    /// user and deletes its sessions/user in one transaction. Keeping the
    /// writer until commit means a successful return is a hard authority
    /// boundary: no admission begun under the old user grant can finish later.
    pub async fn revoke_user(&self, channel_user_id: &str) -> Result<(), ChannelError> {
        let user = self
            .repo
            .get_user(channel_user_id)
            .await?
            .ok_or_else(|| ChannelError::UserNotFound(channel_user_id.to_owned()))?;
        let channel_plugin_id = user.channel_plugin_id.as_deref().ok_or_else(|| {
            ChannelError::InvalidConfig(format!(
                "channel user '{channel_user_id}' is not scoped to a channel plugin"
            ))
        })?;

        let revocation_writer = self.group_policy_fence.write(channel_plugin_id).await;
        self.repo
            .revoke_user_and_cancel_pending(channel_user_id, now_ms())
            .await?;
        drop(revocation_writer);

        info!(channel_user_id, channel_plugin_id, "channel user revoked");
        Ok(())
    }

    /// Rejects a pending pairing code.
    ///
    /// Validates the code exists and is still pending (not expired or
    /// already processed), then marks it as rejected.
    pub async fn reject_pairing(&self, code: &str) -> Result<(), ChannelError> {
        let _row = self.get_valid_pending_pairing(code).await?;

        self.repo
            .update_pairing_status(code, &PairingStatus::Rejected.to_string())
            .await?;

        info!(code = %code, "pairing rejected");
        Ok(())
    }

    /// Returns all pending (not expired) pairing requests.
    pub async fn get_pending_pairings(&self) -> Result<Vec<ChannelPairingCodeRow>, ChannelError> {
        let rows = self.repo.get_pending_pairings().await?;
        let now = now_ms();
        // Filter out expired ones that haven't been cleaned up yet
        let active: Vec<ChannelPairingCodeRow> = rows.into_iter().filter(|r| r.expires_at > now).collect();
        Ok(active)
    }

    /// Checks whether a platform user is already authorized on this bot channel.
    pub async fn is_user_authorized(
        &self,
        platform_user_id: &str,
        platform_type: &str,
        channel_plugin_id: &str,
    ) -> Result<bool, ChannelError> {
        let user = self
            .repo
            .get_user_by_platform(platform_user_id, platform_type, channel_plugin_id)
            .await?;
        Ok(user.is_some_and(|user| {
            user.authorization_kind == CHANNEL_USER_AUTHORIZATION_APPROVED
        }))
    }

    /// Looks up the internal user ID for a platform user on this bot channel.
    ///
    /// Returns `None` if the user is not authorized.
    pub async fn get_internal_user_id(
        &self,
        platform_user_id: &str,
        platform_type: &str,
        channel_plugin_id: &str,
    ) -> Result<Option<String>, ChannelError> {
        let user = self
            .repo
            .get_user_by_platform(platform_user_id, platform_type, channel_plugin_id)
            .await?;
        Ok(user
            .filter(|user| user.authorization_kind == CHANNEL_USER_AUTHORIZATION_APPROVED)
            .map(|user| user.channel_user_id))
    }

    /// Returns the complete platform identity row, including its authorization
    /// kind. Admission owns the decision about which kinds are valid for a
    /// direct, allowlisted-group, or open-group message.
    pub async fn get_channel_user(
        &self,
        platform_user_id: &str,
        platform_type: &str,
        channel_plugin_id: &str,
    ) -> Result<Option<ChannelUserRow>, ChannelError> {
        Ok(self
            .repo
            .get_user_by_platform(platform_user_id, platform_type, channel_plugin_id)
            .await?)
    }

    /// Atomically creates or reuses an automatically admitted guest identity.
    /// Ensures a stable, non-approved guest identity for automatic-admission
    /// paths (open groups and customer-service direct messages). The repository
    /// never downgrades an already-approved user.
    pub async fn ensure_auto_group_user(
        &self,
        platform_user_id: &str,
        platform_type: &str,
        channel_plugin_id: &str,
        display_name: &str,
    ) -> Result<ChannelUserRow, ChannelError> {
        let user = self
            .repo
            .ensure_auto_group_user(&NewChannelUserRow {
                platform_user_id: platform_user_id.to_owned(),
                platform_type: platform_type.to_owned(),
                channel_plugin_id: Some(channel_plugin_id.to_owned()),
                display_name: Some(display_name.to_owned()),
                authorization_kind: CHANNEL_USER_AUTHORIZATION_AUTO_GROUP.to_owned(),
                authorized_at: now_ms(),
                last_active: None,
            })
            .await?;
        Ok(user)
    }

    /// Get or create a stable guest identity for an automatic-admission path.
    ///
    /// This compatibility wrapper never grants pairing approval. The repository
    /// preserves an existing approved identity and otherwise creates/reuses an
    /// `auto_group` guest; new callers should prefer
    /// [`Self::ensure_auto_group_user`] so that distinction is explicit.
    pub async fn ensure_channel_user(
        &self,
        platform_user_id: &str,
        platform_type: &str,
        channel_plugin_id: &str,
        display_name: &str,
    ) -> Result<String, ChannelError> {
        let user = self
            .ensure_auto_group_user(
                platform_user_id,
                platform_type,
                channel_plugin_id,
                display_name,
            )
            .await?;
        info!(
            channel_user_id = %user.channel_user_id,
            platform_user_id = %platform_user_id,
            channel_plugin_id,
            authorization_kind = %user.authorization_kind,
            "automatic-admission channel identity ensured without pairing approval"
        );
        Ok(user.channel_user_id)
    }

    /// Starts a background task that periodically cleans up expired
    /// pairing codes. Returns a `JoinHandle` that can be used to cancel
    /// the task on shutdown.
    pub fn start_cleanup_timer(repo: Arc<dyn IChannelRepository>) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(PAIRING_CLEANUP_INTERVAL);
            loop {
                interval.tick().await;
                let now = now_ms();
                match repo.cleanup_expired_pairings(now).await {
                    Ok(count) if count > 0 => {
                        debug!(count, "cleaned up expired pairing codes");
                    }
                    Ok(_) => {}
                    Err(e) => {
                        warn!(error = %e, "failed to clean up expired pairings");
                    }
                }
            }
        })
    }

    /// Validates that a pairing code exists, is pending, and not expired.
    async fn get_valid_pending_pairing(&self, code: &str) -> Result<ChannelPairingCodeRow, ChannelError> {
        let row = self
            .repo
            .get_pairing_by_code(code)
            .await?
            .ok_or_else(|| ChannelError::PairingNotFound(code.to_owned()))?;

        if row.status != PairingStatus::Pending.to_string() {
            return Err(ChannelError::PairingAlreadyProcessed(code.to_owned()));
        }

        let now = now_ms();
        if row.expires_at <= now {
            // Mark as expired for consistency
            let _ = self
                .repo
                .update_pairing_status(code, &PairingStatus::Expired.to_string())
                .await;
            return Err(ChannelError::PairingExpired(code.to_owned()));
        }

        Ok(row)
    }

    /// Expires any pending codes for the given platform user.
    ///
    /// Called before creating a new code to ensure only one active code
    /// per user at a time.
    async fn expire_user_pending_codes(
        &self,
        platform_user_id: &str,
        platform_type: &str,
        channel_plugin_id: &str,
    ) -> Result<(), ChannelError> {
        let pending = self.repo.get_pending_pairings().await?;
        for row in pending {
            if row.platform_user_id == platform_user_id
                && row.platform_type == platform_type
                && row.channel_plugin_id.as_deref() == Some(channel_plugin_id)
            {
                self.repo
                    .update_pairing_status(&row.code, &PairingStatus::Expired.to_string())
                    .await?;
                debug!(
                    code = %row.code,
                    "expired old pending code for user"
                );
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_db::models::{
        ChannelPairingCodeRow, ChannelPluginRow, ChannelSessionRow, ChannelUserRow,
        NewChannelPairingCodeRow, NewChannelPluginRow, NewChannelSessionRow, NewChannelUserRow,
    };
    use nomifun_db::{DbError, IChannelRepository, UpdatePluginStatusParams};
    use std::sync::Mutex;

    // ── Mock owner-scoped event sink ───────────────────────────────────

    struct MockBroadcaster {
        events: Mutex<Vec<WebSocketMessage<serde_json::Value>>>,
        owners: Mutex<Vec<String>>,
    }

    impl MockBroadcaster {
        fn new() -> Self {
            Self {
                events: Mutex::new(Vec::new()),
                owners: Mutex::new(Vec::new()),
            }
        }

        fn take_events(&self) -> Vec<WebSocketMessage<serde_json::Value>> {
            let mut guard = self.events.lock().unwrap();
            std::mem::take(&mut *guard)
        }

        fn take_owners(&self) -> Vec<String> {
            let mut guard = self.owners.lock().unwrap();
            std::mem::take(&mut *guard)
        }
    }

    impl UserEventSink for MockBroadcaster {
        fn send_to_user(&self, user_id: &str, event: WebSocketMessage<serde_json::Value>) {
            self.owners.lock().unwrap().push(user_id.to_owned());
            self.events.lock().unwrap().push(event);
        }
    }

    // ── Mock IChannelRepository ────────────────────────────────────────

    struct MockRepo {
        pairings: Mutex<Vec<ChannelPairingCodeRow>>,
        users: Mutex<Vec<ChannelUserRow>>,
    }

    impl MockRepo {
        fn new() -> Self {
            Self {
                pairings: Mutex::new(Vec::new()),
                users: Mutex::new(Vec::new()),
            }
        }

        fn get_pairings(&self) -> Vec<ChannelPairingCodeRow> {
            self.pairings.lock().unwrap().clone()
        }

        fn get_users(&self) -> Vec<ChannelUserRow> {
            self.users.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl IChannelRepository for MockRepo {
        // -- Plugin CRUD (unused stubs) --

        async fn get_all_plugins(&self) -> Result<Vec<ChannelPluginRow>, DbError> {
            Ok(vec![])
        }
        async fn get_plugin(&self, _channel_plugin_id: &str) -> Result<Option<ChannelPluginRow>, DbError> {
            Ok(None)
        }
        async fn create_plugin(&self, row: &NewChannelPluginRow) -> Result<ChannelPluginRow, DbError> {
            Ok(ChannelPluginRow {
                channel_plugin_id: nomifun_common::generate_id(),
                r#type: row.r#type.clone(),
                name: row.name.clone(),
                enabled: row.enabled,
                config: row.config.clone(),
                status: row.status.clone(),
                last_connected: row.last_connected,
                companion_id: row.companion_id.clone(),
                bot_key: row.bot_key.clone(),
                owner_domain: row.owner_domain.clone(),
                group_access_mode: row.group_access_mode.clone(),
                created_at: row.created_at,
                updated_at: row.updated_at,
            })
        }
        async fn update_plugin(&self, row: &ChannelPluginRow) -> Result<ChannelPluginRow, DbError> {
            Ok(row.clone())
        }
        async fn update_plugin_status(
            &self,
            _channel_plugin_id: &str,
            _params: &UpdatePluginStatusParams,
        ) -> Result<(), DbError> {
            Ok(())
        }
        async fn update_plugin_companion(
            &self,
            _channel_plugin_id: &str,
            _companion_id: Option<&str>,
        ) -> Result<(), DbError> {
            Ok(())
        }
        async fn update_plugin_bot_key(
            &self,
            _channel_plugin_id: &str,
            _bot_key: &str,
        ) -> Result<(), DbError> {
            Ok(())
        }
        async fn delete_plugin(&self, _channel_plugin_id: &str) -> Result<(), DbError> {
            Ok(())
        }

        // -- User CRUD --

        async fn get_all_users(&self) -> Result<Vec<ChannelUserRow>, DbError> {
            Ok(self.users.lock().unwrap().clone())
        }

        async fn get_user(&self, channel_user_id: &str) -> Result<Option<ChannelUserRow>, DbError> {
            Ok(self
                .users
                .lock()
                .unwrap()
                .iter()
                .find(|user| user.channel_user_id == channel_user_id)
                .cloned())
        }

        async fn get_user_by_platform(
            &self,
            platform_user_id: &str,
            platform_type: &str,
            channel_plugin_id: &str,
        ) -> Result<Option<ChannelUserRow>, DbError> {
            let users = self.users.lock().unwrap();
            Ok(users
                .iter()
                .find(|u| {
                    u.platform_user_id == platform_user_id
                        && u.platform_type == platform_type
                        && u.channel_plugin_id.as_deref() == Some(channel_plugin_id)
                })
                .cloned())
        }

        async fn create_user(&self, row: &NewChannelUserRow) -> Result<ChannelUserRow, DbError> {
            let mut users = self.users.lock().unwrap();
            if let Some(existing) = users.iter_mut().find(|u| {
                u.platform_user_id == row.platform_user_id
                    && u.platform_type == row.platform_type
                    && u.channel_plugin_id == row.channel_plugin_id
            }) {
                if row.authorization_kind == CHANNEL_USER_AUTHORIZATION_APPROVED {
                    existing.authorization_kind = CHANNEL_USER_AUTHORIZATION_APPROVED.to_owned();
                    existing.display_name = row.display_name.clone();
                    existing.authorized_at = row.authorized_at;
                    existing.last_active = row.last_active;
                }
                return Ok(existing.clone());
            }
            let user = ChannelUserRow {
                channel_user_id: nomifun_common::generate_id(),
                platform_user_id: row.platform_user_id.clone(),
                platform_type: row.platform_type.clone(),
                channel_plugin_id: row.channel_plugin_id.clone(),
                display_name: row.display_name.clone(),
                authorization_kind: row.authorization_kind.clone(),
                authorized_at: row.authorized_at,
                last_active: row.last_active,
            };
            users.push(user.clone());
            Ok(user)
        }

        async fn ensure_auto_group_user(
            &self,
            row: &NewChannelUserRow,
        ) -> Result<ChannelUserRow, DbError> {
            let mut users = self.users.lock().unwrap();
            if let Some(existing) = users.iter().find(|u| {
                u.platform_user_id == row.platform_user_id
                    && u.platform_type == row.platform_type
                    && u.channel_plugin_id == row.channel_plugin_id
            }) {
                return Ok(existing.clone());
            }

            let user = ChannelUserRow {
                channel_user_id: nomifun_common::generate_id(),
                platform_user_id: row.platform_user_id.clone(),
                platform_type: row.platform_type.clone(),
                channel_plugin_id: row.channel_plugin_id.clone(),
                display_name: row.display_name.clone(),
                authorization_kind: CHANNEL_USER_AUTHORIZATION_AUTO_GROUP.to_owned(),
                authorized_at: row.authorized_at,
                last_active: row.last_active,
            };
            users.push(user.clone());
            Ok(user)
        }

        async fn update_user_last_active(
            &self,
            channel_user_id: &str,
            last_active: TimestampMs,
        ) -> Result<(), DbError> {
            let mut users = self.users.lock().unwrap();
            if let Some(u) = users
                .iter_mut()
                .find(|u| u.channel_user_id == channel_user_id)
            {
                u.last_active = Some(last_active);
                Ok(())
            } else {
                Err(DbError::NotFound(channel_user_id.to_owned()))
            }
        }

        async fn delete_user(&self, channel_user_id: &str) -> Result<(), DbError> {
            let mut users = self.users.lock().unwrap();
            let len_before = users.len();
            users.retain(|u| u.channel_user_id != channel_user_id);
            if users.len() == len_before {
                Err(DbError::NotFound(channel_user_id.to_owned()))
            } else {
                Ok(())
            }
        }

        async fn revoke_user_and_cancel_pending(
            &self,
            channel_user_id: &str,
            _now: TimestampMs,
        ) -> Result<(), DbError> {
            self.delete_user(channel_user_id).await
        }

        // -- Session CRUD (unused stubs) --

        async fn get_all_sessions(&self) -> Result<Vec<ChannelSessionRow>, DbError> {
            Ok(vec![])
        }
        async fn get_session(&self, _channel_session_id: &str) -> Result<Option<ChannelSessionRow>, DbError> {
            Ok(None)
        }
        async fn get_or_create_session(
            &self,
            _channel_user_id: &str,
            _chat_id: &str,
            _channel_plugin_id: &str,
            new_row: &NewChannelSessionRow,
        ) -> Result<ChannelSessionRow, DbError> {
            Ok(ChannelSessionRow {
                channel_session_id: new_row.channel_session_id.clone(),
                channel_user_id: new_row.channel_user_id.clone(),
                agent_type: new_row.agent_type.clone(),
                conversation_id: new_row.conversation_id.clone(),
                workspace: new_row.workspace.clone(),
                chat_id: new_row.chat_id.clone(),
                channel_plugin_id: new_row.channel_plugin_id.clone(),
                chat_kind: new_row.chat_kind.clone(),
                created_at: new_row.created_at,
                last_activity: new_row.last_activity,
            })
        }
        async fn update_session_activity(&self, _id: &str, _last_activity: TimestampMs) -> Result<(), DbError> {
            Ok(())
        }
        async fn update_session_conversation(&self, _id: &str, _conversation_id: &str) -> Result<(), DbError> {
            Ok(())
        }
        async fn update_session_agent_type(&self, _id: &str, _agent_type: &str) -> Result<(), DbError> {
            Ok(())
        }
        async fn delete_sessions_by_user(&self, _channel_user_id: &str) -> Result<(), DbError> {
            Ok(())
        }
        async fn delete_sessions_by_channel(&self, _channel_plugin_id: &str) -> Result<(), DbError> {
            Ok(())
        }
        async fn delete_session_by_user_chat(
            &self,
            _channel_user_id: &str,
            _chat_id: &str,
            _channel_plugin_id: &str,
        ) -> Result<(), DbError> {
            Ok(())
        }

        // -- Pairing codes --

        async fn create_pairing(&self, row: &NewChannelPairingCodeRow) -> Result<ChannelPairingCodeRow, DbError> {
            let mut pairings = self.pairings.lock().unwrap();
            if pairings.iter().any(|p| p.code == row.code) {
                return Err(DbError::Conflict("duplicate code".into()));
            }
            let pairing = ChannelPairingCodeRow {
                code: row.code.clone(),
                platform_user_id: row.platform_user_id.clone(),
                platform_type: row.platform_type.clone(),
                channel_plugin_id: row.channel_plugin_id.clone(),
                display_name: row.display_name.clone(),
                requested_at: row.requested_at,
                expires_at: row.expires_at,
                status: row.status.clone(),
            };
            pairings.push(pairing.clone());
            Ok(pairing)
        }

        async fn get_pending_pairings(&self) -> Result<Vec<ChannelPairingCodeRow>, DbError> {
            let pairings = self.pairings.lock().unwrap();
            Ok(pairings.iter().filter(|p| p.status == "pending").cloned().collect())
        }

        async fn get_pairing_by_code(&self, code: &str) -> Result<Option<ChannelPairingCodeRow>, DbError> {
            let pairings = self.pairings.lock().unwrap();
            Ok(pairings.iter().find(|p| p.code == code).cloned())
        }

        async fn update_pairing_status(&self, code: &str, status: &str) -> Result<(), DbError> {
            let mut pairings = self.pairings.lock().unwrap();
            if let Some(p) = pairings.iter_mut().find(|p| p.code == code) {
                p.status = status.to_owned();
                Ok(())
            } else {
                Err(DbError::NotFound(code.into()))
            }
        }

        async fn approve_pairing_and_retire_non_direct_sessions(
            &self,
            code: &str,
            now: TimestampMs,
        ) -> Result<PairingApprovalOutcome, DbError> {
            let mut pairings = self.pairings.lock().unwrap();
            let Some(pairing) = pairings.iter_mut().find(|pairing| pairing.code == code) else {
                return Ok(PairingApprovalOutcome::NotFound);
            };
            if pairing.status != "pending" {
                return Ok(PairingApprovalOutcome::AlreadyProcessed);
            }
            if pairing.expires_at <= now {
                pairing.status = "expired".into();
                return Ok(PairingApprovalOutcome::Expired);
            }

            let mut users = self.users.lock().unwrap();
            let user = if let Some(existing) = users.iter_mut().find(|user| {
                user.platform_user_id == pairing.platform_user_id
                    && user.platform_type == pairing.platform_type
                    && user.channel_plugin_id == pairing.channel_plugin_id
            }) {
                if existing.authorization_kind != CHANNEL_USER_AUTHORIZATION_AUTO_GROUP {
                    return Err(DbError::Conflict(format!(
                        "User '{}' on platform '{}' already exists",
                        pairing.platform_user_id, pairing.platform_type
                    )));
                }
                existing.authorization_kind = CHANNEL_USER_AUTHORIZATION_APPROVED.into();
                existing.display_name = pairing
                    .display_name
                    .clone()
                    .or_else(|| existing.display_name.clone());
                existing.authorized_at = now;
                existing.clone()
            } else {
                let user = ChannelUserRow {
                    channel_user_id: nomifun_common::ChannelUserId::new().into_string(),
                    platform_user_id: pairing.platform_user_id.clone(),
                    platform_type: pairing.platform_type.clone(),
                    channel_plugin_id: pairing.channel_plugin_id.clone(),
                    display_name: pairing.display_name.clone(),
                    authorization_kind: CHANNEL_USER_AUTHORIZATION_APPROVED.into(),
                    authorized_at: now,
                    last_active: None,
                };
                users.push(user.clone());
                user
            };
            pairing.status = "approved".into();
            Ok(PairingApprovalOutcome::Approved(user))
        }

        async fn cleanup_expired_pairings(&self, now: TimestampMs) -> Result<u64, DbError> {
            let mut pairings = self.pairings.lock().unwrap();
            let mut count = 0u64;
            for p in pairings.iter_mut() {
                if p.status == "pending" && p.expires_at <= now {
                    p.status = "expired".into();
                    count += 1;
                }
            }
            Ok(count)
        }
    }

    // ── Helpers ────────────────────────────────────────────────────────

    const TEST_CHANNEL_PLUGIN_ID: &str = "0190f5fe-7c00-7a00-8000-000000000001";
    const OTHER_CHANNEL_PLUGIN_ID: &str = "0190f5fe-7c00-7a00-8000-000000000002";

    fn make_service() -> (PairingService, Arc<MockRepo>, Arc<MockBroadcaster>) {
        let repo = Arc::new(MockRepo::new());
        let broadcaster = Arc::new(MockBroadcaster::new());
        let svc = PairingService::new(repo.clone(), broadcaster.clone(), "owner-a");
        (svc, repo, broadcaster)
    }

    // ── generate_pairing_code ──────────────────────────────────────────

    #[test]
    fn code_has_correct_length() {
        let code = generate_pairing_code().unwrap();
        assert_eq!(code.len(), PAIRING_CODE_LENGTH);
    }

    #[test]
    fn code_is_all_digits() {
        let code = generate_pairing_code().unwrap();
        assert!(code.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn code_is_zero_padded() {
        // Generate many codes; at least some should start with '0' statistically,
        // but more importantly verify format consistency.
        for _ in 0..100 {
            let code = generate_pairing_code().unwrap();
            assert_eq!(code.len(), PAIRING_CODE_LENGTH);
            assert!(code.chars().all(|c| c.is_ascii_digit()));
        }
    }

    #[test]
    fn codes_are_not_all_identical() {
        let codes: std::collections::HashSet<String> = (0..50).map(|_| generate_pairing_code().unwrap()).collect();
        // With 6-digit codes, 50 random samples should produce > 1 unique
        assert!(codes.len() > 1);
    }

    // ── request_pairing ────────────────────────────────────────────────

    #[tokio::test]
    async fn request_pairing_creates_code() {
        let (svc, repo, _bc) = make_service();
        let code = svc
            .request_pairing("tg_42", "telegram", TEST_CHANNEL_PLUGIN_ID, Some("Alice"))
            .await
            .unwrap();
        assert_eq!(code.len(), PAIRING_CODE_LENGTH);

        let pairings = repo.get_pairings();
        assert_eq!(pairings.len(), 1);
        assert_eq!(pairings[0].code, code);
        assert_eq!(pairings[0].platform_user_id, "tg_42");
        assert_eq!(pairings[0].platform_type, "telegram");
        assert_eq!(pairings[0].display_name.as_deref(), Some("Alice"));
        assert_eq!(pairings[0].status, "pending");
    }

    #[tokio::test]
    async fn request_pairing_broadcasts_event() {
        let (svc, _repo, bc) = make_service();
        svc.request_pairing("tg_42", "telegram", TEST_CHANNEL_PLUGIN_ID, Some("Alice"))
            .await
            .unwrap();

        let events = bc.take_events();
        assert_eq!(bc.take_owners(), vec!["owner-a"]);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name, "channel.pairing-requested");
        assert_eq!(events[0].data["platform_user_id"], "tg_42");
        assert_eq!(events[0].data["platform_type"], "telegram");
        assert_eq!(events[0].data["display_name"], "Alice");
    }

    #[tokio::test]
    async fn group_pending_request_is_reused_even_when_mentions_race() {
        let (svc, repo, bc) = make_service();
        let (first, second) = tokio::join!(
            svc.request_or_reuse_pending(
                "tg_42",
                "telegram",
                TEST_CHANNEL_PLUGIN_ID,
                Some("Alice"),
            ),
            svc.request_or_reuse_pending(
                "tg_42",
                "telegram",
                TEST_CHANNEL_PLUGIN_ID,
                Some("Alice"),
            ),
        );

        assert_eq!(first.unwrap(), second.unwrap());
        assert_eq!(repo.get_pairings().len(), 1);
        assert_eq!(bc.take_events().len(), 1);
        assert_eq!(bc.take_owners(), vec!["owner-a"]);
    }

    #[tokio::test]
    async fn request_pairing_sets_correct_expiry() {
        let (svc, repo, _bc) = make_service();
        let before = now_ms();
        svc.request_pairing("u1", "lark", TEST_CHANNEL_PLUGIN_ID, None)
            .await
            .unwrap();
        let after = now_ms();

        let p = &repo.get_pairings()[0];
        let expected_ttl = PAIRING_CODE_TTL.as_millis() as TimestampMs;
        assert!(p.expires_at >= before + expected_ttl);
        assert!(p.expires_at <= after + expected_ttl);
    }

    #[tokio::test]
    async fn request_pairing_expires_old_code() {
        let (svc, repo, _bc) = make_service();

        let code1 = svc
            .request_pairing("tg_42", "telegram", TEST_CHANNEL_PLUGIN_ID, Some("Alice"))
            .await
            .unwrap();
        let code2 = svc
            .request_pairing("tg_42", "telegram", TEST_CHANNEL_PLUGIN_ID, Some("Alice"))
            .await
            .unwrap();

        assert_ne!(code1, code2);

        let pairings = repo.get_pairings();
        let old = pairings.iter().find(|p| p.code == code1).unwrap();
        let new = pairings.iter().find(|p| p.code == code2).unwrap();
        assert_eq!(old.status, "expired");
        assert_eq!(new.status, "pending");
    }

    #[tokio::test]
    async fn request_pairing_no_display_name() {
        let (svc, repo, _bc) = make_service();
        svc.request_pairing("u1", "dingtalk", TEST_CHANNEL_PLUGIN_ID, None)
            .await
            .unwrap();

        let pairings = repo.get_pairings();
        assert!(pairings[0].display_name.is_none());
    }

    // ── approve_pairing ────────────────────────────────────────────────

    #[tokio::test]
    async fn approve_creates_user_and_updates_status() {
        let (svc, repo, _bc) = make_service();
        let code = svc
            .request_pairing("tg_42", "telegram", TEST_CHANNEL_PLUGIN_ID, Some("Alice"))
            .await
            .unwrap();

        svc.approve_pairing(&code).await.unwrap();

        // Check pairing status
        let pairings = repo.get_pairings();
        let p = pairings.iter().find(|p| p.code == code).unwrap();
        assert_eq!(p.status, "approved");

        // Check user created
        let users = repo.get_users();
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].platform_user_id, "tg_42");
        assert_eq!(users[0].platform_type, "telegram");
        assert_eq!(users[0].display_name.as_deref(), Some("Alice"));
    }

    #[tokio::test]
    async fn approve_promotes_auto_group_user_without_changing_identity() {
        let (svc, repo, _bc) = make_service();
        let auto_group = svc
            .ensure_auto_group_user(
                "tg_42",
                "telegram",
                TEST_CHANNEL_PLUGIN_ID,
                "Group Alice",
            )
            .await
            .unwrap();
        assert_eq!(
            auto_group.authorization_kind,
            CHANNEL_USER_AUTHORIZATION_AUTO_GROUP
        );

        let code = svc
            .request_pairing(
                "tg_42",
                "telegram",
                TEST_CHANNEL_PLUGIN_ID,
                Some("Alice"),
            )
            .await
            .unwrap();
        svc.approve_pairing(&code).await.unwrap();

        let users = repo.get_users();
        assert_eq!(users.len(), 1, "approval must promote instead of duplicating");
        assert_eq!(users[0].channel_user_id, auto_group.channel_user_id);
        assert_eq!(
            users[0].authorization_kind,
            CHANNEL_USER_AUTHORIZATION_APPROVED
        );
        assert_eq!(users[0].display_name.as_deref(), Some("Alice"));
        assert!(
            svc.is_user_authorized("tg_42", "telegram", TEST_CHANNEL_PLUGIN_ID)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn promotion_waits_for_old_admission_before_changing_authority() {
        let (svc, repo, _bc) = make_service();
        let svc = Arc::new(svc);
        let auto_group = svc
            .ensure_auto_group_user(
                "tg_waiting",
                "telegram",
                TEST_CHANNEL_PLUGIN_ID,
                "Waiting guest",
            )
            .await
            .unwrap();
        let code = svc
            .request_pairing(
                "tg_waiting",
                "telegram",
                TEST_CHANNEL_PLUGIN_ID,
                Some("Approved user"),
            )
            .await
            .unwrap();

        let admission = svc
            .group_policy_fence()
            .read(TEST_CHANNEL_PLUGIN_ID)
            .await;
        let approving = {
            let svc = Arc::clone(&svc);
            let code = code.clone();
            tokio::spawn(async move { svc.approve_pairing(&code).await })
        };
        let mut approving = approving;
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), &mut approving)
                .await
                .is_err(),
            "promotion must wait for an admission holding the old guest authority"
        );
        assert_eq!(
            repo.get_users()[0].authorization_kind,
            CHANNEL_USER_AUTHORIZATION_AUTO_GROUP
        );
        assert_eq!(repo.get_pairings()[0].status, "pending");

        drop(admission);
        tokio::time::timeout(std::time::Duration::from_secs(1), approving)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let users = repo.get_users();
        assert_eq!(users[0].channel_user_id, auto_group.channel_user_id);
        assert_eq!(
            users[0].authorization_kind,
            CHANNEL_USER_AUTHORIZATION_APPROVED
        );
        assert_eq!(repo.get_pairings()[0].status, "approved");
    }

    #[tokio::test]
    async fn revocation_waits_for_old_admission_and_then_removes_user() {
        let (svc, repo, _bc) = make_service();
        let svc = Arc::new(svc);
        let code = svc
            .request_pairing(
                "tg_revoked",
                "telegram",
                TEST_CHANNEL_PLUGIN_ID,
                Some("Revoked user"),
            )
            .await
            .unwrap();
        svc.approve_pairing(&code).await.unwrap();
        let user_id = repo.get_users()[0].channel_user_id.clone();

        let admission = svc
            .group_policy_fence()
            .read(TEST_CHANNEL_PLUGIN_ID)
            .await;
        let revoking = {
            let svc = Arc::clone(&svc);
            let user_id = user_id.clone();
            tokio::spawn(async move { svc.revoke_user(&user_id).await })
        };
        let mut revoking = revoking;
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), &mut revoking)
                .await
                .is_err(),
            "revocation must wait for an admission holding the old user grant"
        );
        assert_eq!(repo.get_users().len(), 1);

        drop(admission);
        tokio::time::timeout(std::time::Duration::from_secs(1), revoking)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(repo.get_users().is_empty());
    }

    #[tokio::test]
    async fn approve_broadcasts_user_authorized() {
        let (svc, _repo, bc) = make_service();
        let code = svc
            .request_pairing("tg_42", "telegram", TEST_CHANNEL_PLUGIN_ID, Some("Alice"))
            .await
            .unwrap();
        bc.take_events(); // clear request event
        bc.take_owners();

        svc.approve_pairing(&code).await.unwrap();

        let events = bc.take_events();
        assert_eq!(bc.take_owners(), vec!["owner-a"]);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name, "channel.user-authorized");
        assert_eq!(events[0].data["platform_user_id"], "tg_42");
        assert_eq!(events[0].data["platform_type"], "telegram");
        assert_eq!(events[0].data["display_name"], "Alice");
        assert!(events[0].data["channel_user_id"].is_string());
    }

    #[tokio::test]
    async fn approve_nonexistent_code_returns_not_found() {
        let (svc, _repo, _bc) = make_service();
        let err = svc.approve_pairing("000000").await.unwrap_err();
        assert!(matches!(err, ChannelError::PairingNotFound(_)));
    }

    #[tokio::test]
    async fn approve_already_approved_returns_already_processed() {
        let (svc, _repo, _bc) = make_service();
        let code = svc
            .request_pairing("tg_42", "telegram", TEST_CHANNEL_PLUGIN_ID, None)
            .await
            .unwrap();
        svc.approve_pairing(&code).await.unwrap();

        let err = svc.approve_pairing(&code).await.unwrap_err();
        assert!(matches!(err, ChannelError::PairingAlreadyProcessed(_)));
    }

    #[tokio::test]
    async fn approve_expired_code_returns_expired() {
        let (svc, repo, _bc) = make_service();
        // Manually insert an already-expired code
        let row = ChannelPairingCodeRow {
            code: "999999".into(),
            platform_user_id: "u1".into(),
            platform_type: "telegram".into(),
            channel_plugin_id: None,
            display_name: None,
            requested_at: 1000,
            expires_at: 1001, // long expired
            status: "pending".into(),
        };
        repo.pairings.lock().unwrap().push(row);

        let err = svc.approve_pairing("999999").await.unwrap_err();
        assert!(matches!(err, ChannelError::PairingExpired(_)));
    }

    // ── reject_pairing ─────────────────────────────────────────────────

    #[tokio::test]
    async fn reject_updates_status() {
        let (svc, repo, _bc) = make_service();
        let code = svc
            .request_pairing("tg_42", "telegram", TEST_CHANNEL_PLUGIN_ID, None)
            .await
            .unwrap();

        svc.reject_pairing(&code).await.unwrap();

        let pairings = repo.get_pairings();
        let p = pairings.iter().find(|p| p.code == code).unwrap();
        assert_eq!(p.status, "rejected");
    }

    #[tokio::test]
    async fn reject_nonexistent_code_returns_not_found() {
        let (svc, _repo, _bc) = make_service();
        let err = svc.reject_pairing("000000").await.unwrap_err();
        assert!(matches!(err, ChannelError::PairingNotFound(_)));
    }

    #[tokio::test]
    async fn reject_already_approved_returns_already_processed() {
        let (svc, _repo, _bc) = make_service();
        let code = svc
            .request_pairing("tg_42", "telegram", TEST_CHANNEL_PLUGIN_ID, None)
            .await
            .unwrap();
        svc.approve_pairing(&code).await.unwrap();

        let err = svc.reject_pairing(&code).await.unwrap_err();
        assert!(matches!(err, ChannelError::PairingAlreadyProcessed(_)));
    }

    // ── get_pending_pairings ───────────────────────────────────────────

    #[tokio::test]
    async fn get_pending_filters_expired() {
        let (svc, repo, _bc) = make_service();

        // Insert valid pending code
        svc.request_pairing("u1", "telegram", TEST_CHANNEL_PLUGIN_ID, None)
            .await
            .unwrap();

        // Insert manually expired code
        let expired_row = ChannelPairingCodeRow {
            code: "000001".into(),
            platform_user_id: "u2".into(),
            platform_type: "lark".into(),
            channel_plugin_id: None,
            display_name: None,
            requested_at: 1000,
            expires_at: 1001,
            status: "pending".into(),
        };
        repo.pairings.lock().unwrap().push(expired_row);

        let pending = svc.get_pending_pairings().await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].platform_user_id, "u1");
    }

    #[tokio::test]
    async fn get_pending_empty_when_none() {
        let (svc, _repo, _bc) = make_service();
        let pending = svc.get_pending_pairings().await.unwrap();
        assert!(pending.is_empty());
    }

    // ── is_user_authorized ─────────────────────────────────────────────

    #[tokio::test]
    async fn unauthorized_user_returns_false() {
        let (svc, _repo, _bc) = make_service();
        let authorized = svc
            .is_user_authorized("tg_42", "telegram", TEST_CHANNEL_PLUGIN_ID)
            .await
            .unwrap();
        assert!(!authorized);
    }

    #[tokio::test]
    async fn auto_group_user_is_idempotent_but_not_directly_authorized() {
        let (svc, repo, _bc) = make_service();
        let first = svc
            .ensure_auto_group_user("tg_42", "telegram", TEST_CHANNEL_PLUGIN_ID, "Alice")
            .await
            .unwrap();
        let second = svc
            .ensure_auto_group_user("tg_42", "telegram", TEST_CHANNEL_PLUGIN_ID, "Alice")
            .await
            .unwrap();

        assert_eq!(first.channel_user_id, second.channel_user_id);
        assert_eq!(repo.get_users().len(), 1);
        assert_eq!(
            first.authorization_kind,
            CHANNEL_USER_AUTHORIZATION_AUTO_GROUP
        );
        assert!(
            !svc.is_user_authorized("tg_42", "telegram", TEST_CHANNEL_PLUGIN_ID)
                .await
                .unwrap()
        );
        assert_eq!(
            svc.get_internal_user_id("tg_42", "telegram", TEST_CHANNEL_PLUGIN_ID)
                .await
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn compatibility_auto_admission_wrapper_never_approves_a_stranger() {
        let (svc, repo, _bc) = make_service();
        let channel_user_id = svc
            .ensure_channel_user("cs_visitor", "lark", TEST_CHANNEL_PLUGIN_ID, "Visitor")
            .await
            .unwrap();

        let users = repo.get_users();
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].channel_user_id, channel_user_id);
        assert_eq!(
            users[0].authorization_kind,
            CHANNEL_USER_AUTHORIZATION_AUTO_GROUP
        );
        assert!(
            !svc.is_user_authorized("cs_visitor", "lark", TEST_CHANNEL_PLUGIN_ID)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn authorized_user_returns_true_after_approval() {
        let (svc, _repo, _bc) = make_service();
        let code = svc
            .request_pairing("tg_42", "telegram", TEST_CHANNEL_PLUGIN_ID, None)
            .await
            .unwrap();
        svc.approve_pairing(&code).await.unwrap();

        let authorized = svc
            .is_user_authorized("tg_42", "telegram", TEST_CHANNEL_PLUGIN_ID)
            .await
            .unwrap();
        assert!(authorized);
    }

    #[tokio::test]
    async fn two_channels_same_user_pair_independently() {
        let (svc, repo, _bc) = make_service();
        let c1 = svc
            .request_pairing("ou_same", "lark", TEST_CHANNEL_PLUGIN_ID, Some("U"))
            .await
            .unwrap();
        let c2 = svc
            .request_pairing("ou_same", "lark", OTHER_CHANNEL_PLUGIN_ID, Some("U"))
            .await
            .unwrap();
        let pend = repo.get_pairings();
        assert_eq!(pend.iter().find(|p| p.code == c1).unwrap().status, "pending");
        assert_eq!(pend.iter().find(|p| p.code == c2).unwrap().status, "pending");
        svc.approve_pairing(&c1).await.unwrap();
        assert!(
            svc.is_user_authorized("ou_same", "lark", TEST_CHANNEL_PLUGIN_ID)
                .await
                .unwrap()
        );
        assert!(
            !svc
                .is_user_authorized("ou_same", "lark", OTHER_CHANNEL_PLUGIN_ID)
                .await
                .unwrap()
        );
    }

    // ── cleanup_expired_pairings (via repo directly) ───────────────────

    #[tokio::test]
    async fn cleanup_marks_expired_as_expired() {
        let (svc, repo, _bc) = make_service();

        // Insert manually expired pending code
        let expired_row = ChannelPairingCodeRow {
            code: "111111".into(),
            platform_user_id: "u1".into(),
            platform_type: "telegram".into(),
            channel_plugin_id: None,
            display_name: None,
            requested_at: 1000,
            expires_at: 2000,
            status: "pending".into(),
        };
        repo.pairings.lock().unwrap().push(expired_row);

        // Insert valid pending code
        svc.request_pairing("u2", "lark", TEST_CHANNEL_PLUGIN_ID, None)
            .await
            .unwrap();

        let count = repo.cleanup_expired_pairings(now_ms()).await.unwrap();
        assert_eq!(count, 1);

        let pairings = repo.get_pairings();
        let expired = pairings.iter().find(|p| p.code == "111111").unwrap();
        assert_eq!(expired.status, "expired");
    }

    // ── start_cleanup_timer ────────────────────────────────────────────

    fn make_expired_row(code: &str) -> ChannelPairingCodeRow {
        ChannelPairingCodeRow {
            code: code.into(),
            platform_user_id: "u1".into(),
            platform_type: "telegram".into(),
            channel_plugin_id: None,
            display_name: None,
            requested_at: 1000,
            expires_at: 2000, // long past
            status: "pending".into(),
        }
    }

    /// Regression: `start_cleanup_timer` existed but had no caller, so the
    /// sweep never ran. This pins the timer behaviour itself — it must keep
    /// invoking `cleanup_expired_pairings` once per `PAIRING_CLEANUP_INTERVAL`
    /// (the assembly in nomifun-app now starts it at boot).
    #[tokio::test(start_paused = true)]
    async fn cleanup_timer_periodically_purges_expired_codes() {
        let repo = Arc::new(MockRepo::new());
        repo.pairings.lock().unwrap().push(make_expired_row("222222"));

        let handle = PairingService::start_cleanup_timer(repo.clone());

        // The paused clock auto-advances while the test sleeps, driving the
        // spawned interval deterministically. The first tick fires
        // immediately and purges the seeded code.
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        assert_eq!(repo.get_pairings()[0].status, "expired");

        // Seed another expired code and cross one full interval to prove
        // the sweep is periodic, not one-shot.
        repo.pairings.lock().unwrap().push(make_expired_row("333333"));
        tokio::time::sleep(PAIRING_CLEANUP_INTERVAL).await;
        let pairings = repo.get_pairings();
        let second = pairings.iter().find(|p| p.code == "333333").unwrap();
        assert_eq!(second.status, "expired");

        handle.abort();
    }
}
