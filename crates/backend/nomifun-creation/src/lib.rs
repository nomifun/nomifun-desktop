//! `nomifun-creation` — the 生成引擎 (media generation engine): the async task
//! queue behind the 创意工坊 canvas's generation nodes.
//!
//! The engine is **provider-agnostic**: model execution is delegated to the
//! unified invocation layer (`nomifun-model-invoke`), which owns provider /
//! model / protocol resolution against the model catalog. The
//! [`CreationService`] owns the state machine (`queued → running →
//! succeeded/failed/canceled`), per-provider concurrency + a global cap,
//! cancellation, boot reconciliation, and hands produced bytes to an
//! [`AssetSink`] (implemented by the app over `nomifun-workshop`, so neither
//! domain crate depends on the other — no cycle). Task inputs are read back
//! through the symmetric [`AssetSource`].

mod artifact;
mod dto;
mod types;

pub mod routes;
pub mod service;
pub mod state;

pub use artifact::validate_artifact_payload;
pub use dto::CreationTask;
pub use routes::creation_routes;
pub use service::{
    AssetSink, AssetSource, CreationService, CreationServiceBuilder, LoadedAsset, NewCreationTask,
    PersistAsset, TaskArtifactCleanupFailure, TaskArtifactIssue, TaskArtifactManifest,
    TaskArtifactReconcileReport,
};
pub use state::CreationRouterState;
pub use types::{CreationError, CreationInput, MediaCapability, TaskStatus};
