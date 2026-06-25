//! Twitch channel plugin: IRC-over-WebSocket (fully outbound).
//!
//! Connects to Twitch chat via `wss://irc-ws.chat.twitch.tv:443` using the
//! standard IRC-over-WebSocket protocol with TMI capabilities.

mod api;
mod plugin;
pub mod types;

pub use plugin::TwitchPlugin;
