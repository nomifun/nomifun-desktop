//! AgentPreset, Capability Catalog, binding, preview, and editor-test control plane.

#![forbid(unsafe_code)]

mod catalog;
mod compiler;
mod continuation;
mod error;
mod impact;
mod routes;
mod service;
mod store;
mod wire;

pub use catalog::{
    CatalogProvider, CatalogSnapshot, OfficialTemplateCatalog, StaticCatalogProvider,
};
pub use compiler::{
    CanonicalRegistryProvider, CompilerReleaseInputs, PresetPreviewCompiler, PreviewCompilation,
};
pub use continuation::{
    AGENT_SESSION_CREATE_PATH, continuation_view, editor_test_plan,
    installation_token_state, remote_credential_continuation, revoked_installation_token,
    rotated_installation_token,
};
pub use error::ControlPlaneError;
pub use impact::{
    ControlPlaneRevisionImpactCatalogProvider, RevisionImpactCatalogProvider,
    StaticRevisionImpactCatalogProvider,
};
pub use routes::{
    AuthenticatedOwner, control_plane_router, control_plane_router_without_legacy_skills,
};
pub use service::{AgentControlPlane, DefaultChatRouteResolver};
pub use store::{
    AgentBindingTarget, ControlPlaneStore, InMemoryControlPlaneStore, StoredAgentBinding,
    StoredPreset,
};
