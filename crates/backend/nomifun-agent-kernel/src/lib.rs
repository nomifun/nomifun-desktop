//! Thin Agent capability Kernel and trusted in-process plugin host.

#![forbid(unsafe_code)]

mod session_capabilities;
mod authority;
mod compiler;
mod error;
mod materialize;
mod plugin;
mod registry;
mod service;
mod state;

pub use session_capabilities::*;
pub use authority::*;
pub use compiler::*;
pub use error::*;
pub use materialize::*;
pub use plugin::*;
pub use registry::*;
pub use service::*;
pub use state::*;

#[cfg(test)]
mod sample_echo;
