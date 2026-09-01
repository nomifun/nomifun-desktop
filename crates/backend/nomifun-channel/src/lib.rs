//! External channel integration: plugin system, pairing handshake, and per-session messaging.
pub mod action;
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
pub use session_port::{
    ChannelSessionPort, ChannelTurnDelivery, conversation_channel_session_port,
};
