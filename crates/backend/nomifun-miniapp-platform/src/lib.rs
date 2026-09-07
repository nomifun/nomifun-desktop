//! Phase M1 MiniApp application domain.
//!
//! This crate owns the new Product/Project/Release state machine. It is
//! intentionally independent from the legacy `miniapps` table and service.
//! JavaScript build and service execution are injected ports; this crate never
//! discovers Node or starts a host process on its own.

#![forbid(unsafe_code)]

mod dto;
mod error;
mod bridge;
mod model;
mod operation;
mod repository;
mod runtime;
mod m1_application;
mod service;
mod service_host;
mod storage;
#[cfg(test)]
mod foundation_tests;
#[cfg(test)]
mod tests;

pub use dto::*;
pub use error::*;
pub use bridge::*;
pub use model::*;
pub use operation::*;
pub use repository::*;
pub use runtime::*;
pub use m1_application::*;
pub use service::*;
pub use service_host::*;
pub use storage::*;
