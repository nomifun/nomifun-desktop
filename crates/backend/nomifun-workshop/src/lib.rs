//! `nomifun-workshop` — the canonical Creative Studio domain. It owns project
//! documents, assets, workflows, archives, and their `/api/creative-studio/*`
//! routes. `nomifun-creation` owns asynchronous model-generation execution.

mod dto;
mod fsio;
mod imagemeta;
mod prompt_catalog;
mod thumbnail;

mod archive;
mod canvas_agent_artifact;
pub mod creative_agent_ops;
pub mod creative_studio;
pub mod routes;
pub mod service;
pub mod state;
pub mod workflow;
pub mod workflow_run;

pub use creative_agent_ops::{
    CreativeAgentOp, CreativeAgentOpResult, MAX_CREATIVE_AGENT_OPS_PER_CALL,
    apply_creative_agent_ops,
};
pub use creative_studio::{
    CREATIVE_STUDIO_SCHEMA, CreativeProjectDocument, CreativeProjectSummary,
    MAX_CREATIVE_PROJECT_DOCUMENT_BYTES,
};
pub use dto::WorkshopAsset;
pub use prompt_catalog::{CreativePromptCatalogItem, CreativePromptCatalogPage};
pub use workflow::{CreativeWorkflowDefinitionV1, MAX_WORKFLOW_DEFINITION_BYTES};
pub use workflow_run::{
    CreativeWorkflowRunAggregateV1, CreativeWorkflowRunCreateRequest, CreativeWorkflowRunStatus,
    MAX_WORKFLOW_RUN_AGGREGATE_BYTES,
};
pub use routes::{workshop_public_routes, workshop_routes};
pub use service::WorkshopService;
pub use state::WorkshopRouterState;

/// Domain root under the backend data dir. Layout:
/// - `{data_dir}/workshop/assets/{id}.{ext}` — asset originals.
/// - `{data_dir}/workshop/assets/thumbs/{id}.jpg` — asset thumbnails (JPEG).
pub const WORKSHOP_REL_DIR: &str = "workshop";

/// Max uploaded asset size (contract §3.2: ≤ 64 MB).
pub const MAX_ASSET_BYTES: usize = 64 * 1024 * 1024;
