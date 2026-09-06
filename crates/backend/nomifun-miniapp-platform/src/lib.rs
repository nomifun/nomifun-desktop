//! Phase M1 MiniApp application domain.
//!
//! This crate owns the new Product/Project/Release state machine. It is
//! intentionally independent from the legacy `miniapps` table and service.
//! JavaScript build and service execution are injected ports; this crate never
//! discovers Node or starts a host process on its own.

#![forbid(unsafe_code)]

mod dto;
mod error;
mod model;
mod operation;
mod repository;
mod runtime;
mod service;
#[cfg(test)]
mod tests;

pub use dto::*;
pub use error::*;
pub use model::*;
pub use operation::*;
pub use repository::*;
pub use runtime::*;
pub use service::*;
