//! AgentPreset, Capability Catalog, binding, and revision control plane.

#![forbid(unsafe_code)]

mod catalog;
mod compiler;
mod continuation;
mod error;
mod impact;
mod kernel_catalog;
mod routes;
mod role_defaults;
pub use role_defaults::InstallationRoleBindingStore;
mod ui_bindings;
pub use ui_bindings::AgentUiBindingStore;
mod service;
mod store;
mod wire;

pub use catalog::{
    CatalogProvider, CatalogSnapshot, PluginProductCatalogPublicationSource,
    OfficialTemplateCatalog, SharedPluginProductCatalogPublications, StaticCatalogProvider,
};
pub use compiler::{CanonicalRegistryProvider, PresetRevisionCompiler};
pub use continuation::{
    continuation_view, installation_token_state, remote_credential_continuation,
    revoked_installation_token, rotated_installation_token,
};
pub use error::ControlPlaneError;
pub use kernel_catalog::{
    KernelCatalogProvider, materialize_capability_catalog_entries,
    materialize_catalog_snapshot, materialize_catalog_snapshot_with_plugin_products,
};
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
