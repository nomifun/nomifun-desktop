//! External channel integration: plugin system, pairing handshake, and per-session messaging.
pub mod action;
pub mod agent_capability;
pub mod channel_settings;
pub mod constants;
pub mod error;
pub mod formatter;
pub mod group_policy;
pub mod manager;
pub mod media_refs;
pub mod message_service;
pub mod message_loop;
pub mod pairing;
pub mod pending_decision;
pub mod plugin;
pub mod plugins;
pub mod queue_drain;
pub mod routes;
mod session_port;
pub mod session;
pub mod stream_relay;
pub mod think_filter;
pub mod types;

pub use routes::{ChannelRouterState, channel_routes};
pub use agent_capability::{
    CHANNEL_MESSAGING_ACTION_IDS, CHANNEL_MESSAGING_MODULE_ID,
    CHANNEL_REPLY_ACTION_ID, CHANNEL_SEND_ACTION_ID,
    ChannelAgentCapabilityOwner, ChannelCustomerBindingAuthority,
    ChannelSceneContext, ChannelSceneIngressPort, ChannelSceneTarget,
    channel_action_input_schema, channel_action_resource_operation,
};
pub use session_port::{
    ChannelCompletedTurnReceipt, ChannelSessionPort, ChannelTurnDelivery,
    ChannelTurnDeliveryReceipt, ChannelTurnReceiptState,
};
