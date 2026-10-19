//! `nomifun-companion` — the desktop-companion domain: a roster of companions sharing one
//! memory hub (opt-in event collection + per-companion scheduled LLM learning
//! that distills those events into memories that companion owns), per-companion
//! persona companion chats over the real agent engine, and the companion
//! config/status API surface.
//!
//! Layering: `profile` is the per-companion/shared config split; `registry` is the
//! companion roster;
//! `store` owns the shared sqlite db under `{data_dir}/companion/shared/`;
//! `collector` taps the global event bus and appends JSONL event files;
//! `learner` is the periodic LLM distillation loop; `companion` is the
//! per-companion companion chat; `service` bundles them; `routes`/`state` are the
//! API surface. v3 starts from a fresh companion dataset; non-v3 layouts are
//! rejected and must be removed by the app-level dataset reset before this crate opens.

pub mod collector;
pub mod agent_capability;
mod agent_memory_projection;
pub mod companion;
pub mod archiver;
pub mod archive_port;
pub mod config;
pub mod events;
pub mod evolution;
pub mod export;
pub mod figure;
pub mod figures;
mod fsio;
pub mod gamify;
pub mod learner;
pub mod matting_model;
pub mod memory_search;
pub mod memory_context;
pub mod learning_commit;
pub mod profile;
pub mod prompt;
pub mod registry;
pub mod routes;
mod session_port;
pub mod service;
pub mod state;
pub mod store;
mod skill_io;

pub use events::CompanionEventEmitter;
pub use agent_memory_projection::{AgentMemoryProjection, AgentMemorySearchHit};
pub use companion::CompanionLaunchConfiguration;
pub use agent_capability::{
    COMPANION_AGENT_ACTION_IDS, COMPANION_EVOLVE_ACTION_ID,
    COMPANION_LEARN_ACTION_ID, COMPANION_MEMORY_ACTION_IDS,
    COMPANION_MEMORY_MODULE_ID, COMPANION_MEMORY_RECALL_ACTION_ID,
    COMPANION_MEMORY_WRITE_ACTION_ID, COMPANION_MODULE_ID,
    CompanionAgentCapabilityOwner, CompanionPersonaContext,
    companion_action_input_schema, companion_action_resource_requirement,
};
pub use archive_port::CompanionArchiveSessionPort;
pub use figures::FigureMeta;
pub use memory_search::{
    CompanionMemoryRow, MemorySearchHit, MemorySearchQuery, MemoryStatusFilter,
};
pub use memory_context::CompanionMemoryContext;
pub use learning_commit::{LearnedMemoryCandidate, LearnedMemoryReinforcement};
pub use profile::{
    CompanionEvolveConfig, CompanionLearnConfig, CompanionMood, CompanionProfileConfig, CompanionSkillConfig,
    CompanionWindowConfig, CustomFigureMeta, HeadBox, SharedArchiveConfig, SharedCompanionConfig,
};
pub use registry::CompanionRegistry;
pub use routes::{companion_public_routes, companion_routes};
pub use session_port::{
    companion_ports_from_typed_host, CompanionHostPorts, CompanionRole,
    CompanionSessionCreateContext, CompanionSessionPort,
    CompanionDeletionGuard,
    CompanionHistoryRecallQuery, CompanionHistoryRecallPage, CompanionHistoryFragment,
    CompanionHistoryRecallCursor,
};
pub use service::CompanionService;
pub use state::CompanionRouterState;
pub use store::CompanionStore;

/// Shared multi-companion artifacts (under the backend data dir): shared
/// `config.json`, `events/*.jsonl`, `memory.db`.
pub const COMPANION_SHARED_REL_DIR: &str = "companion/shared";

/// Per-companion profile roots (under the backend data dir): one
/// `{COMPANION_COMPANIONS_REL_DIR}/{companion_id}/config.json` per companion.
pub const COMPANION_COMPANIONS_REL_DIR: &str = "companion/companions";

/// Backend-owned persistent ML assets shared across companions. MODNet bytes
/// are checked against the shared pinned manifest before serving; missing or
/// corrupt cache entries are reacquired (see [`matting_model`]).
pub const COMPANION_MODELS_REL_DIR: &str = "companion/models";

/// Shared custom-figure library (under the backend data dir): reusable figures
/// decoupled from any single companion — `{id}.webp` + `index.json` (see
/// [`figures`]).
pub const COMPANION_FIGURES_REL_DIR: &str = "companion/figures";
