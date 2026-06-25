//! Discord channel plugin: Gateway WebSocket (inbound) + REST API (outbound).
//!
//! Mirrors the structure of the other channels — `plugin.rs` implements the
//! [`crate::plugin::ChannelPlugin`] trait, `gateway.rs` runs the long-lived
//! Gateway connection loop (HELLO/IDENTIFY/HEARTBEAT/dispatch with backoff
//! reconnect, like `lark`), and `api.rs` is the REST client (like `telegram`).

mod api;
mod gateway;
mod plugin;
pub mod types;

pub use plugin::DiscordPlugin;
