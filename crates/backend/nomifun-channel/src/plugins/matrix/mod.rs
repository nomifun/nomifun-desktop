//! Matrix channel plugin (Route B: handwritten, no E2EE, reqwest only).
//!
//! Uses the Matrix Client-Server API v3 directly via `reqwest`.
//! Encrypted rooms (`m.room.encrypted`) are skipped — E2EE is gated on
//! `matrix-sdk` dep compatibility with the workspace (currently blocked
//! by `libsqlite3-sys` link conflict and `reqwest` 0.12 vs 0.13).

pub mod api;
pub mod plugin;
pub mod types;

pub use plugin::MatrixPlugin;
