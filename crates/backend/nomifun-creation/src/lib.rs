//! `nomifun-creation` — the provider-agnostic media generation engine behind
//! Creative Studio canvas nodes and workflow steps.
//!
//! The engine is **provider-agnostic**: model execution is delegated to the
//! unified invocation layer (`nomifun-model-invoke`), while text nodes use the
//! injected [`CreationTextExecutor`] backed by the Agent Chat engine. The
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
pub use dto::{
    CreationTask, CreativeCreationTask, CreativeCreationTaskOwner, CreativeCreationTaskPage,
    CreativeCreationTaskRetireResult,
};
pub use routes::creation_routes;
pub use service::{
    AssetSink, AssetSource, CreationService, CreationServiceBuilder, CreativeTaskOwner,
    CreationTextExecutor, CreationTextRequest, LoadedAsset, NewCreationTask, PersistAsset,
    TaskArtifactCleanupFailure,
    TaskArtifactIssue, TaskArtifactManifest, TaskArtifactReconcileReport,
};
pub use state::CreationRouterState;
pub use types::{
    CreationError, CreationInput, CreationInputKind, MediaCapability, StandaloneWorkbenchKind,
    TaskStatus,
};
