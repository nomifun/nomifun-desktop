//! Plugin platform: authoring, immutable artifacts, installation, surfaces,
//! dedicated services, managed storage, and recoverable lifecycle operations.
//!
//! One local Plugin identity owns one Active Artifact and one generation DataRoot.

mod store;
mod data_root;
mod model;
mod repository;
mod install;
mod draft;
mod service_process;
mod backup;
mod service_runtime;
mod bindings;

pub use data_root::*;
pub use model::*;
pub use repository::*;
pub use install::*;
pub use draft::*;
pub use service_process::*;
pub use backup::*;
pub use service_runtime::*;
pub use bindings::*;
pub use store::{
    ArtifactImportResult, ArtifactStoreLimits, CancellationFlag, ImportCancellation,
    NeverCancel, PluginArtifactStore, PluginArtifactStoreError, StoredPluginArtifact,
};
