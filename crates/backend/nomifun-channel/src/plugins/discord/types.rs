//! Discord Gateway + REST wire types (v10, JSON encoding).

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Gateway opcodes & intents
// ---------------------------------------------------------------------------

pub const OP_DISPATCH: u8 = 0;
pub const OP_HEARTBEAT: u8 = 1;
pub const OP_IDENTIFY: u8 = 2;
pub const OP_RECONNECT: u8 = 7;
pub const OP_INVALID_SESSION: u8 = 9;
pub const OP_HELLO: u8 = 10;
pub const OP_HEARTBEAT_ACK: u8 = 11;

// Gateway intents (bitfield). We need guild + DM message events plus the
// privileged MESSAGE_CONTENT intent (must be enabled in the Dev Portal) to
// actually receive message text.
pub const INTENT_GUILDS: u64 = 1 << 0;
pub const INTENT_GUILD_MESSAGES: u64 = 1 << 9;
pub const INTENT_DIRECT_MESSAGES: u64 = 1 << 12;
pub const INTENT_MESSAGE_CONTENT: u64 = 1 << 15;

/// Combined intents the bot identifies with.
pub const GATEWAY_INTENTS: u64 = INTENT_GUILDS | INTENT_GUILD_MESSAGES | INTENT_DIRECT_MESSAGES | INTENT_MESSAGE_CONTENT;

/// Interaction type for a message component (button) click.
pub const INTERACTION_TYPE_MESSAGE_COMPONENT: u8 = 3;

/// Interaction callback type: acknowledge a component interaction without
/// changing the message (DEFERRED_UPDATE_MESSAGE).
pub const INTERACTION_CALLBACK_DEFERRED_UPDATE: u8 = 6;

/// Component types.
pub const COMPONENT_ACTION_ROW: u8 = 1;
pub const COMPONENT_BUTTON: u8 = 2;

/// Button style: secondary (grey).
pub const BUTTON_STYLE_SECONDARY: u8 = 2;

// ---------------------------------------------------------------------------
// Inbound gateway envelope
// ---------------------------------------------------------------------------

/// Top-level gateway frame. `d` is opcode-specific; `s`/`t` only on dispatch.
#[derive(Debug, Deserialize)]
pub struct GatewayPayload {
    pub op: u8,
    #[serde(default)]
    pub d: serde_json::Value,
    #[serde(default)]
    pub s: Option<u64>,
    #[serde(default)]
    pub t: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct HelloData {
    pub heartbeat_interval: u64,
}

#[derive(Debug, Deserialize)]
pub struct ReadyData {
    pub user: DiscordUser,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DiscordUser {
    pub id: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub global_name: Option<String>,
    #[serde(default)]
    pub bot: bool,
}

impl DiscordUser {
    /// Best human-readable name: global display name if set, else username.
    pub fn display(&self) -> String {
        self.global_name.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| self.username.clone())
    }
}

/// MESSAGE_CREATE dispatch payload (subset we use).
#[derive(Debug, Deserialize)]
pub struct MessageCreate {
    pub id: String,
    pub channel_id: String,
    #[serde(default)]
    pub guild_id: Option<String>,
    pub author: DiscordUser,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub attachments: Vec<DiscordAttachment>,
    #[serde(default)]
    pub mentions: Vec<DiscordUser>,
    #[serde(default)]
    pub message_reference: Option<MessageReference>,
}

#[derive(Debug, Deserialize)]
pub struct MessageReference {
    #[serde(default)]
    pub message_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DiscordAttachment {
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub content_type: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub url: String,
}

/// INTERACTION_CREATE dispatch payload (subset for component/button clicks).
#[derive(Debug, Deserialize)]
pub struct InteractionCreate {
    pub id: String,
    pub token: String,
    #[serde(rename = "type")]
    pub interaction_type: u8,
    #[serde(default)]
    pub data: Option<InteractionData>,
    #[serde(default)]
    pub channel_id: Option<String>,
    #[serde(default)]
    pub member: Option<InteractionMember>,
    #[serde(default)]
    pub user: Option<DiscordUser>,
    #[serde(default)]
    pub message: Option<InteractionMessage>,
}

impl InteractionCreate {
    /// Resolve the acting user from either `member.user` (guild) or `user` (DM).
    pub fn acting_user(&self) -> Option<&DiscordUser> {
        self.member.as_ref().map(|m| &m.user).or(self.user.as_ref())
    }
}

#[derive(Debug, Deserialize)]
pub struct InteractionData {
    #[serde(default)]
    pub custom_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct InteractionMember {
    pub user: DiscordUser,
}

#[derive(Debug, Deserialize)]
pub struct InteractionMessage {
    pub id: String,
}

// ---------------------------------------------------------------------------
// Outbound gateway frames
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct OutgoingFrame<T: Serialize> {
    pub op: u8,
    pub d: T,
}

#[derive(Debug, Serialize)]
pub struct IdentifyData {
    pub token: String,
    pub intents: u64,
    pub properties: IdentifyProperties,
}

#[derive(Debug, Serialize)]
pub struct IdentifyProperties {
    pub os: String,
    pub browser: String,
    pub device: String,
}

// ---------------------------------------------------------------------------
// REST request/response
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Default)]
pub struct CreateMessageRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_reference: Option<RestMessageReference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub components: Option<Vec<ActionRow>>,
}

#[derive(Debug, Serialize)]
pub struct RestMessageReference {
    pub message_id: String,
    pub fail_if_not_exists: bool,
}

#[derive(Debug, Serialize)]
pub struct ActionRow {
    #[serde(rename = "type")]
    pub component_type: u8,
    pub components: Vec<ButtonComponent>,
}

#[derive(Debug, Serialize)]
pub struct ButtonComponent {
    #[serde(rename = "type")]
    pub component_type: u8,
    pub style: u8,
    pub label: String,
    pub custom_id: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateMessageResponse {
    pub id: String,
}

#[derive(Debug, Deserialize)]
pub struct GetMeResponse {
    pub id: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub global_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct InteractionCallbackBody {
    #[serde(rename = "type")]
    pub callback_type: u8,
}
