//! FullAuto JSON stream protocol for host-agent communication.
//!
//! The protocol contains agent events, client commands, and JSON-lines I/O.
//! Tool execution is always governed by the host's fixed FullAuto policy; this
//! crate intentionally has no interactive execution-policy state machine.

pub mod commands;
pub mod events;
pub mod reader;
pub mod writer;
