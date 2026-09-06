//! Phase N1 Plugin package ingestion and immutable artifact storage.
//!
//! This crate owns only the filesystem and package-validation boundary. It
//! does not install dependencies, execute JavaScript, mutate Plugin database
//! state, or read the legacy Extension manifest format.

mod store;
mod runtime_data;

pub use runtime_data::*;
pub use store::{
    ArtifactImportResult, ArtifactStoreLimits, CancellationFlag, ImportCancellation,
    NeverCancel, PluginArtifactStore, PluginArtifactStoreError, StoredPluginArtifact,
};
