//! Plugin platform: authoring, immutable artifacts, installation, surfaces,
//! dedicated services, managed storage, and recoverable lifecycle operations.
//!
//! Runtime roles share one platform while retaining their own execution fences.

pub mod application;
pub mod runtime;

mod store;
mod runtime_data;

pub use runtime_data::*;
pub use store::{
    ArtifactImportResult, ArtifactStoreLimits, CancellationFlag, ImportCancellation,
    NeverCancel, PluginArtifactStore, PluginArtifactStoreError, StoredPluginArtifact,
};
