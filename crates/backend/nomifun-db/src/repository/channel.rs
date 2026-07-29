use nomifun_common::TimestampMs;

use crate::error::DbError;
use crate::models::{
    ChannelInboundReceiptRow, ChannelPairingCodeRow, ChannelPendingPromptRow, ChannelPluginRow,
    ChannelSessionRow, ChannelUserRow, NewChannelInboundReceiptRow, NewChannelPairingCodeRow,
    NewChannelPendingPromptRow, NewChannelPluginRow, NewChannelSessionRow, NewChannelUserRow,
};

/// Result of atomically claiming a provider-owned inbound event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelInboundClaim {
    /// This caller owns the exact generation and may cross the effects fence.
    Owner(ChannelInboundReceiptRow),
    /// The same immutable event was already admitted. No side effect may run.
    Replay(ChannelInboundReceiptRow),
}

/// Outcome of enqueueing one busy-time channel prompt (spec D1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingPromptEnqueue {
    /// The prompt was persisted; `position` is its 1-based place in the
    /// conversation's queued FIFO (so the channel can answer "第 N 位").
    Queued {
        row: ChannelPendingPromptRow,
        position: i64,
    },
    /// The conversation already has the maximum number of queued prompts.
    QueueFull,
}

impl ChannelInboundClaim {
    pub fn receipt(&self) -> &ChannelInboundReceiptRow {
        match self {
            Self::Owner(receipt) | Self::Replay(receipt) => receipt,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SettleChannelInboundReceiptParams {
    pub conversation_id: Option<String>,
    pub message_id: Option<String>,
    pub outcome_json: Option<String>,
    pub error_text: Option<String>,
}

/// Data access abstraction for channel integration tables.
///
/// Covers channel plugins/users/sessions/pairing plus durable inbound receipts.
///
/// Object-safe via `async_trait` to support `Arc<dyn IChannelRepository>`.
#[async_trait::async_trait]
pub trait IChannelRepository: Send + Sync {
    // ── Plugin CRUD ──────────────────────────────────────────────────

    /// Returns all registered plugins.
    async fn get_all_plugins(&self) -> Result<Vec<ChannelPluginRow>, DbError>;

    /// Returns a single plugin by business id, or `None` if not found.
    async fn get_plugin(&self, channel_plugin_id: &str) -> Result<Option<ChannelPluginRow>, DbError>;

    /// Inserts a plugin and returns the persisted row with its generated UUIDv7.
    async fn create_plugin(&self, row: &NewChannelPluginRow) -> Result<ChannelPluginRow, DbError>;

    /// Updates an existing plugin by its business id.
    async fn update_plugin(&self, row: &ChannelPluginRow) -> Result<ChannelPluginRow, DbError>;

    /// Updates only the `status` and `last_connected` of a plugin.
    async fn update_plugin_status(
        &self,
        channel_plugin_id: &str,
        params: &UpdatePluginStatusParams,
    ) -> Result<(), DbError>;

    /// Updates the companion binding of a plugin row (`None` clears it).
    async fn update_plugin_companion(
        &self,
        channel_plugin_id: &str,
        companion_id: Option<&str>,
    ) -> Result<(), DbError>;

    /// Backfills or rotates the stable platform bot identity for a plugin.
    async fn update_plugin_bot_key(
        &self,
        channel_plugin_id: &str,
        bot_key: &str,
    ) -> Result<(), DbError>;

    /// Deletes a plugin by business id. Returns `DbError::NotFound` if absent.
    async fn delete_plugin(&self, channel_plugin_id: &str) -> Result<(), DbError>;

    // ── User CRUD ────────────────────────────────────────────────────

    /// Returns all authorized users.
    async fn get_all_users(&self) -> Result<Vec<ChannelUserRow>, DbError>;

    /// Finds a user by platform identity scoped to one bot channel.
    async fn get_user_by_platform(
        &self,
        platform_user_id: &str,
        platform_type: &str,
        channel_plugin_id: &str,
    ) -> Result<Option<ChannelUserRow>, DbError>;

    /// Creates an authorized user and returns its generated UUIDv7.
    async fn create_user(&self, row: &NewChannelUserRow) -> Result<ChannelUserRow, DbError>;

    /// Updates `last_active` timestamp for a user.
    async fn update_user_last_active(
        &self,
        channel_user_id: &str,
        last_active: TimestampMs,
    ) -> Result<(), DbError>;

    /// Deletes a user and its associated sessions transactionally by business
    /// id. Returns `DbError::NotFound` if absent.
    async fn delete_user(&self, channel_user_id: &str) -> Result<(), DbError>;

    // ── Session CRUD ─────────────────────────────────────────────────

    /// Returns all sessions.
    async fn get_all_sessions(&self) -> Result<Vec<ChannelSessionRow>, DbError>;

    /// Returns a single session by id.
    async fn get_session(&self, channel_session_id: &str) -> Result<Option<ChannelSessionRow>, DbError>;

    /// Finds an existing session by channel + user + chat, or creates a new
    /// one. If found, updates `last_activity` and returns the existing row.
    /// If not found, inserts `new_row` and returns it.
    async fn get_or_create_session(
        &self,
        channel_user_id: &str,
        chat_id: &str,
        channel_plugin_id: &str,
        new_row: &NewChannelSessionRow,
    ) -> Result<ChannelSessionRow, DbError>;

    /// Updates `last_activity` timestamp for a session.
    async fn update_session_activity(&self, channel_session_id: &str, last_activity: TimestampMs) -> Result<(), DbError>;

    /// Updates the `conversation_id` of a session.
    async fn update_session_conversation(&self, channel_session_id: &str, conversation_id: &str) -> Result<(), DbError>;

    /// Updates the `agent_type` of a session.
    async fn update_session_agent_type(&self, channel_session_id: &str, agent_type: &str) -> Result<(), DbError>;

    /// Deletes all sessions belonging to a user.
    async fn delete_sessions_by_user(&self, channel_user_id: &str) -> Result<(), DbError>;

    /// Deletes all sessions that arrived through a channel row.
    async fn delete_sessions_by_channel(&self, channel_plugin_id: &str) -> Result<(), DbError>;

    /// Deletes the session for a specific channel + user + chat triple.
    async fn delete_session_by_user_chat(
        &self,
        channel_user_id: &str,
        chat_id: &str,
        channel_plugin_id: &str,
    ) -> Result<(), DbError>;

    // ── Pairing Codes ────────────────────────────────────────────────

    /// Claim one immutable provider event before *any* channel side effect.
    ///
    /// A same-key/same-payload replay returns [`ChannelInboundClaim::Replay`].
    /// A key reused with a different identity or payload returns Conflict.
    /// Every existing row is absorbing, including one still in `claimed`.
    /// There is deliberately no wall-clock takeover: a suspended or slow
    /// process must never be mistaken for a dead owner.
    async fn claim_inbound_receipt(
        &self,
        _row: &NewChannelInboundReceiptRow,
    ) -> Result<ChannelInboundClaim, DbError> {
        Err(DbError::Conflict(
            "durable channel inbound receipts are not supported by this repository".to_owned(),
        ))
    }

    /// Cross the durable point-of-no-return for the exact owner generation.
    ///
    /// `true` means this call performed `claimed -> effects_started`; `false`
    /// means the fence was already crossed or ownership was lost, so the caller
    /// must stop without executing anything.
    async fn begin_inbound_effects(
        &self,
        _operation_key: &str,
        _payload_hash: &str,
        _owner_generation: i64,
        _now: TimestampMs,
    ) -> Result<bool, DbError> {
        Err(DbError::Conflict(
            "durable channel inbound receipts are not supported by this repository".to_owned(),
        ))
    }

    /// Settle an effects-started receipt. Both `completed` and `failed` are
    /// absorbing: failure may have happened after an external side effect.
    async fn settle_inbound_receipt(
        &self,
        _operation_key: &str,
        _payload_hash: &str,
        _owner_generation: i64,
        _status: &str,
        _params: &SettleChannelInboundReceiptParams,
        _now: TimestampMs,
    ) -> Result<ChannelInboundReceiptRow, DbError> {
        Err(DbError::Conflict(
            "durable channel inbound receipts are not supported by this repository".to_owned(),
        ))
    }

    /// Creates a pairing code and returns its SQLite-assigned id.
    async fn create_pairing(&self, row: &NewChannelPairingCodeRow) -> Result<ChannelPairingCodeRow, DbError>;

    // ── Busy-time pending prompt queue (spec D1) ─────────────────────
    //
    // Default implementations fail closed so repositories/mocks that never
    // queue prompts (tests, alternative stores) need no changes.

    /// Persist one busy-time prompt. Returns [`PendingPromptEnqueue::QueueFull`]
    /// when the conversation already has [`PENDING_PROMPT_QUEUE_LIMIT`] queued
    /// rows; otherwise the inserted row plus its 1-based FIFO position.
    async fn enqueue_pending_prompt(
        &self,
        _row: &NewChannelPendingPromptRow,
        _now: TimestampMs,
    ) -> Result<PendingPromptEnqueue, DbError> {
        Err(DbError::Conflict(
            "channel pending prompts are not supported by this repository".to_owned(),
        ))
    }

    /// The oldest still-queued prompt of a conversation (FIFO head), if any.
    async fn peek_next_queued(
        &self,
        _conversation_id: &str,
    ) -> Result<Option<ChannelPendingPromptRow>, DbError> {
        Err(DbError::Conflict(
            "channel pending prompts are not supported by this repository".to_owned(),
        ))
    }

    /// Settle a queued prompt into one absorbing terminal state
    /// (`delivered | expired | cancelled | failed`). Settling an already
    /// settled prompt is a Conflict; the terminal state never changes.
    async fn settle_prompt(
        &self,
        _prompt_id: &str,
        _state: &str,
        _now: TimestampMs,
    ) -> Result<(), DbError> {
        Err(DbError::Conflict(
            "channel pending prompts are not supported by this repository".to_owned(),
        ))
    }

    /// Increment the retry counter of a still-queued prompt and return the new
    /// value. Used by the drain's bounded retryable-failure retries.
    async fn increment_prompt_attempts(&self, _prompt_id: &str) -> Result<i64, DbError> {
        Err(DbError::Conflict(
            "channel pending prompts are not supported by this repository".to_owned(),
        ))
    }

    /// Mark every queued prompt older than `before_ms` as `expired` and return
    /// the expired rows so the caller can notify their chats.
    async fn expire_stale(
        &self,
        _before_ms: TimestampMs,
        _now: TimestampMs,
    ) -> Result<Vec<ChannelPendingPromptRow>, DbError> {
        Err(DbError::Conflict(
            "channel pending prompts are not supported by this repository".to_owned(),
        ))
    }

    /// Cancel every queued prompt of one `(plugin, chat)` scope (the IM
    /// 「取消排队」 command). Returns how many prompts were cancelled.
    async fn cancel_chat_queue(
        &self,
        _channel_plugin_id: &str,
        _chat_id: &str,
        _now: TimestampMs,
    ) -> Result<u64, DbError> {
        Err(DbError::Conflict(
            "channel pending prompts are not supported by this repository".to_owned(),
        ))
    }

    /// Conversations that still have at least one queued prompt (startup
    /// recovery sweep).
    async fn list_queued_conversations(&self) -> Result<Vec<String>, DbError> {
        Err(DbError::Conflict(
            "channel pending prompts are not supported by this repository".to_owned(),
        ))
    }

    /// Returns all pairing codes with status = 'pending'.
    async fn get_pending_pairings(&self) -> Result<Vec<ChannelPairingCodeRow>, DbError>;

    /// Retrieves a single pairing code, or `None` if not found.
    async fn get_pairing_by_code(&self, code: &str) -> Result<Option<ChannelPairingCodeRow>, DbError>;

    /// Updates the status of a pairing code.
    /// Returns `DbError::NotFound` if the code doesn't exist.
    async fn update_pairing_status(&self, code: &str, status: &str) -> Result<(), DbError>;

    /// Marks all expired-but-still-pending pairing codes as 'expired'.
    /// `now` is the current timestamp in milliseconds.
    async fn cleanup_expired_pairings(&self, now: TimestampMs) -> Result<u64, DbError>;
}

/// Parameters for updating plugin runtime status.
#[derive(Debug, Clone, Default)]
pub struct UpdatePluginStatusParams {
    pub status: Option<String>,
    pub last_connected: Option<TimestampMs>,
    pub enabled: Option<bool>,
}

/// Maximum number of `queued` prompts one conversation may hold (spec D1:
/// 每 chat 队列上限 10 — enforced per conversation, which upper-bounds every
/// chat riding it).
pub const PENDING_PROMPT_QUEUE_LIMIT: i64 = 10;

/// How long a queued prompt may wait before the drain expires it (spec D1).
pub const PENDING_PROMPT_EXPIRY_MS: i64 = 30 * 60 * 1000;
