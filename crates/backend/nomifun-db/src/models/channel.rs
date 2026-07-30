use nomifun_common::TimestampMs;
use serde::{Deserialize, Serialize};

/// Row mapping for the `channel_plugins` table.
///
/// One row per connected bot. The `config` column holds an encrypted JSON blob
/// containing credentials and options.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ChannelPluginRow {
    pub channel_plugin_id: String,
    /// Platform type (telegram, lark, dingtalk, weixin, slack, discord).
    #[sqlx(rename = "type")]
    pub r#type: String,
    pub name: String,
    pub enabled: bool,
    /// JSON blob: `{ credentials, config }`. Stored encrypted at rest.
    pub config: String,
    pub status: Option<String>,
    pub last_connected: Option<TimestampMs>,
    /// Companion bound to this bot. UNIQUE(type, bot_key) guarantees a bot is
    /// never bound to more than one companion.
    pub companion_id: Option<String>,
    /// Platform-level bot identity (lark app_id, telegram bot id, ...),
    /// extracted from credentials on enable/restore.
    pub bot_key: Option<String>,
    /// Owning domain: `companion` (desktop companion pool, default) or
    /// `customer_service` (customer-service self-managed pool). A
    /// customer-service bot never carries a `companion_id` (DB trigger +
    /// application validation).
    pub owner_domain: String,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

/// Values accepted when inserting a `channel_plugins` row.
///
/// SQLite owns the technical `id`; callers address the row only through the
/// generated `channel_plugin_id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewChannelPluginRow {
    pub r#type: String,
    pub name: String,
    pub enabled: bool,
    pub config: String,
    pub status: Option<String>,
    pub last_connected: Option<TimestampMs>,
    pub companion_id: Option<String>,
    pub bot_key: Option<String>,
    /// Owning domain (`companion` | `customer_service`). Defaults to the
    /// legacy companion pool when omitted on the wire.
    #[serde(default = "default_owner_domain")]
    pub owner_domain: String,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

/// The legacy/default channel bot ownership domain.
pub fn default_owner_domain() -> String {
    CHANNEL_OWNER_DOMAIN_COMPANION.to_owned()
}

/// `channel_plugins.owner_domain` value for desktop-companion bots.
pub const CHANNEL_OWNER_DOMAIN_COMPANION: &str = "companion";
/// `channel_plugins.owner_domain` value for customer-service bots.
pub const CHANNEL_OWNER_DOMAIN_CUSTOMER_SERVICE: &str = "customer_service";

/// Row mapping for the `channel_users` table.
///
/// Represents an IM user authorized to chat with the Agent.
/// UNIQUE constraint on (platform_user_id, platform_type, channel_plugin_id).
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ChannelUserRow {
    pub channel_user_id: String,
    pub platform_user_id: String,
    pub platform_type: String,
    /// Optional logical reference to the `channel_plugins` business identity that owns
    /// this authorization. `None` means the authorization is not plugin-scoped.
    pub channel_plugin_id: Option<String>,
    pub display_name: Option<String>,
    pub authorized_at: TimestampMs,
    pub last_active: Option<TimestampMs>,
}

/// Values accepted when inserting a `channel_users` row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewChannelUserRow {
    pub platform_user_id: String,
    pub platform_type: String,
    pub channel_plugin_id: Option<String>,
    pub display_name: Option<String>,
    pub authorized_at: TimestampMs,
    pub last_active: Option<TimestampMs>,
}

/// Row mapping for the `channel_sessions` table.
///
/// Per-chat session linking an authorized user to a conversation. Relations
/// are application-enforced logical references.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ChannelSessionRow {
    pub channel_session_id: String,
    pub channel_user_id: String,
    pub agent_type: String,
    pub conversation_id: Option<String>,
    pub workspace: Option<String>,
    pub chat_id: Option<String>,
    /// The `channel_plugins` business identity this session arrived through. Two bots
    /// in the same chat get isolated sessions.
    pub channel_plugin_id: Option<String>,
    pub created_at: TimestampMs,
    pub last_activity: TimestampMs,
}

/// Values accepted when inserting a `channel_sessions` row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewChannelSessionRow {
    pub channel_session_id: String,
    pub channel_user_id: String,
    pub agent_type: String,
    pub conversation_id: Option<String>,
    pub workspace: Option<String>,
    pub chat_id: Option<String>,
    pub channel_plugin_id: Option<String>,
    pub created_at: TimestampMs,
    pub last_activity: TimestampMs,
}

/// Durable at-most-once admission record for one provider-owned inbound event.
///
/// The receipt deliberately exists independently of a channel session or
/// Conversation: pairing, `session.new`, requirement creation, and decision
/// submission can all have side effects before either entity exists.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, PartialEq, Eq)]
pub struct ChannelInboundReceiptRow {
    pub operation_key: String,
    pub user_scope_id: String,
    pub user_id: Option<String>,
    pub channel_plugin_scope_id: String,
    pub channel_plugin_id: Option<String>,
    pub platform: String,
    pub chat_id: String,
    pub provider_event_id: String,
    pub payload_hash: String,
    pub status: String,
    pub phase: String,
    pub owner_generation: i64,
    pub conversation_scope_id: Option<String>,
    pub message_scope_id: Option<String>,
    pub conversation_id: Option<String>,
    pub message_id: Option<String>,
    pub outcome_json: Option<String>,
    pub error_text: Option<String>,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
    pub completed_at: Option<TimestampMs>,
}

/// Immutable identity and payload supplied when claiming an inbound receipt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewChannelInboundReceiptRow {
    pub operation_key: String,
    pub user_id: String,
    pub channel_plugin_id: String,
    pub platform: String,
    pub chat_id: String,
    pub provider_event_id: String,
    pub payload_hash: String,
    pub created_at: TimestampMs,
}

/// Row mapping for the `channel_pairing_codes` table.
///
/// 6-digit pairing code with 10-minute expiry. Status transitions:
/// pending → approved | rejected | expired.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ChannelPairingCodeRow {
    pub code: String,
    pub platform_user_id: String,
    pub platform_type: String,
    /// The bot channel this pairing was initiated through.
    pub channel_plugin_id: Option<String>,
    pub display_name: Option<String>,
    pub requested_at: TimestampMs,
    pub expires_at: TimestampMs,
    pub status: String,
}

/// Values accepted when inserting a `channel_pairing_codes` row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewChannelPairingCodeRow {
    pub code: String,
    pub platform_user_id: String,
    pub platform_type: String,
    pub channel_plugin_id: Option<String>,
    pub display_name: Option<String>,
    pub requested_at: TimestampMs,
    pub expires_at: TimestampMs,
    pub status: String,
}

/// Row mapping for the `channel_pending_prompts` table (spec D1).
///
/// One IM prompt that arrived while its bound conversation was busy and is
/// waiting for the queue drain to deliver it FIFO. `state` transitions from
/// `queued` to exactly one absorbing terminal:
/// `delivered | expired | cancelled | failed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct ChannelPendingPromptRow {
    pub prompt_id: String,
    pub channel_plugin_id: String,
    pub chat_id: String,
    pub channel_session_id: String,
    pub conversation_id: String,
    pub text: String,
    /// Idempotency key minted at enqueue time; the drain reuses it so the
    /// eventual delivery rides the same at-most-once receipt the immediate
    /// dispatch path would have used.
    pub idempotency_key: String,
    pub state: String,
    /// Number of automatic retries already spent on this prompt (retryable
    /// delivery failures only, capped by the drain).
    pub attempts: i64,
    pub queued_at: TimestampMs,
    pub settled_at: Option<TimestampMs>,
}

/// Values accepted when enqueueing a `channel_pending_prompts` row. The
/// repository owns `prompt_id`, `state`, `attempts` and `queued_at`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewChannelPendingPromptRow {
    pub channel_plugin_id: String,
    pub chat_id: String,
    pub channel_session_id: String,
    pub conversation_id: String,
    pub text: String,
    pub idempotency_key: String,
}

