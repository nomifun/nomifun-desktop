//! Plugin releases, optional pages, dedicated services and managed data.
//!
//! Release cutover, session fences and storage recovery belong to this runtime
//! role. The platform composition injects build and execution authorities.

#![forbid(unsafe_code)]

mod dto;
mod manifest;
mod error;
mod bridge;
mod backup;
mod model;
mod operation;
mod repository;
mod runtime;
mod m1_build;
mod m1_application;
mod service;
mod service_host;
mod service_process;
mod service_module_registry;
mod service_runtime;
mod surface;
mod storage;
mod managed_storage;
mod source;
mod release;
mod share;
#[cfg(test)]
mod foundation_tests;
#[cfg(test)]
mod tests;

pub use dto::*;
pub use manifest::*;
pub use error::*;
pub use bridge::*;
pub use backup::*;
pub use model::*;
pub use operation::*;
pub use repository::*;
pub use runtime::*;
pub use m1_build::*;
pub use m1_application::*;
pub use service::*;
pub use service_host::*;
pub use service_process::*;
pub use service_module_registry::*;
pub use service_runtime::*;
pub use surface::*;
pub use storage::*;
pub use managed_storage::*;
pub use source::*;
pub use release::*;
pub use share::*;
