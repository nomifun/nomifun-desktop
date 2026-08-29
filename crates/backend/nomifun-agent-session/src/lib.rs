//! Canonical AgentSession facts, projections, recovery, and deletion closure.
//!
//! The store attaches to the shared canonical Fresh-v4 SQLite root and owns
//! only the three AgentSession fact tables plus two rebuildable projections.
//! It depends only on the frozen `nomifun-agent-contracts` vocabulary and
//! general-purpose infrastructure crates.

#![forbid(unsafe_code)]

mod checkpoint;
mod error;
mod projector;
mod registry;
mod store;
mod types;

pub use checkpoint::{evaluate_snapshot_compatibility, validate_checkpoint};
pub use error::SessionStoreError;
pub use store::{
    AgentSessionStore, MAX_EVENT_PAGE_SIZE, MAX_INLINE_JSON_BYTES, MAX_SESSION_PAYLOAD_BYTES,
    MAX_SINGLE_PAYLOAD_BYTES,
};
pub use types::*;

#[cfg(test)]
mod tests;
