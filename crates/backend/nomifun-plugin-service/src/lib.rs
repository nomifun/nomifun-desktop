//! Product application service for the Phase N1 Plugin Platform.
//!
//! This crate owns orchestration and product-facing projection only. It does
//! not create a second registry and it does not read legacy Extension data.
//! Database, artifact storage, and Kernel/Host execution are injected behind
//! small adapters so App composition can wire the existing implementations.

#![forbid(unsafe_code)]

mod error;
mod repository;
mod service;
mod state;
mod types;

pub use error::*;
pub use repository::*;
pub use service::*;
pub use state::*;
pub use types::*;
