//! QQ Bot channel plugin: Gateway WebSocket (inbound) + REST API (outbound).
//!
//! QQ Bot uses an outbound WebSocket gateway (like Discord) with OAuth2
//! client-credentials token flow. Inbound events are dispatched as
//! C2C/group/channel/DM messages; outbound uses per-target REST endpoints
//! with passive-reply windowing (max 5 replies per inbound msg, 1h TTL).

mod api;
pub(crate) mod gateway;
mod plugin;
pub mod types;

pub use plugin::QqbotPlugin;
