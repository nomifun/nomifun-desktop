//! Managed JavaScript/TypeScript authoring source foundation.
//!
//! This crate owns source paths, deterministic scaffolds, canonical source
//! snapshots, exact dependency input contracts, content-addressed dependency
//! cache contracts, and fixed disposable Build Host packaging.

mod canonical;
mod build;
mod dependency;
mod error;
mod manifest;
mod model;
mod npm;
mod path;
mod scaffold;
mod snapshot;
mod store;

pub use dependency::{
    DependencyRequestSet, ExactDependencyLock, LockedNpmPackage, LockedPackagePolicy,
    NpmResolverIdentity,
};
pub use build::{
    FixedPluginPacker, NodeBuildHost, PackedPluginPackage,
    PluginPackageBuildOptions,
};
pub use error::AuthoringError;
pub use manifest::{
    PLUGIN_SOURCE_MANIFEST_FILE, PLUGIN_SOURCE_MANIFEST_VERSION, PluginLanguage,
    PluginSourceManifest,
};
pub use model::{
    CancellationFlag, NeverCancel, OperationCancellation, SourceScope,
    SourceStoreLimits,
};
pub use npm::{
    CachedNpmPackage, ContentAddressedNpmCache, NpmRegistryPort, NpmResolver,
    RegistryPackageRelease,
};
pub use nomifun_agent_contracts::{
    DigestHex, PluginProjectId, UserId, digest_bytes,
};
pub use path::NormalizedSourcePath;
pub use scaffold::PluginScaffoldRequest;
pub use snapshot::{CapturedSource, SourceFileDigest, SourceSnapshot};
pub use store::{ScaffoldedPluginProject, SourceStore, StagedSource, StoredSourceProject};
