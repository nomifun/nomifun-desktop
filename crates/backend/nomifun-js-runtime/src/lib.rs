//! Node Runtime discovery, probing, fingerprinting, and global selection.
//!
//! Phase N1 has one JavaScript Runtime provider: Node.js. This crate does not
//! own Plugin packages, the Capability Catalog, or product deployment.

#![forbid(unsafe_code)]

mod error;
mod authority;
mod coordinator;
mod managed;
mod manager;
mod probe;
mod service;

pub use error::*;
pub use authority::*;
pub use coordinator::*;
pub use managed::*;
pub use manager::*;
pub use probe::*;
pub use service::*;
