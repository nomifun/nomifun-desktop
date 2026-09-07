//! Managed JavaScript/TypeScript authoring source foundation.
//!
//! This crate owns source paths, deterministic scaffolds, canonical source
//! snapshots, exact dependency input contracts, and disposable build staging.
//! It does not resolve npm packages, maintain an npm cache, pack runtime
//! artifacts, execute Node.js, or implement a Build Host.

mod canonical;
mod dependency;
mod error;
mod model;
mod path;
mod scaffold;
mod snapshot;
mod store;

pub use dependency::{
    DependencyRequestSet, ExactDependencyLock, LockedNpmPackage, LockedPackagePolicy,
    NpmResolverIdentity,
};
pub use error::AuthoringError;
pub use model::{
    CancellationFlag, NeverCancel, OperationCancellation, SourceScope,
    SourceStoreLimits,
};
pub use nomifun_agent_contracts::{
    DigestHex, PluginProjectId, UserId, digest_bytes,
};
pub use path::NormalizedSourcePath;
pub use scaffold::{PLUGIN_SOURCE_MANIFEST_VERSION, PluginLanguage, PluginScaffoldRequest};
pub use snapshot::{CapturedSource, SourceFileDigest, SourceSnapshot};
pub use store::{ScaffoldedPluginProject, SourceStore, StagedSource, StoredSourceProject};
