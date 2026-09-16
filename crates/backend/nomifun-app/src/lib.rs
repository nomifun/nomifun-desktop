//! Application crate: assembles all domain crates into an Axum server with DI and middleware.
//!
//! This file is a public façade — it only re-exports symbols defined in
//! submodules. All logic lives in the modules below.

mod config;
// Spec D2 delivery-notify observer (public so integration tests can drive
// the full receipt loop without the whole app harness).
pub mod delivery_notify;
#[cfg(feature = "browser-use")]
mod browser_workspace_provider;
#[cfg(feature = "browser-use")]
pub mod headless_render;
#[cfg(feature = "browser-use")]
pub mod system_browser;
#[cfg(feature = "browser-use")]
mod system_browser_owner;
mod provider_deletion;
mod robot_wiring;
mod router;
mod services;
mod workshop_bridge;
mod channel_asset_resolver;

// Promoted from the `nomicore` bin so in-process hosts (Tauri desktop, web)
// can boot the backend as a library — no spawned binary.
pub mod bootstrap;
pub mod channel;
pub mod cli;
pub mod commands;
pub mod desktop;
// Public because a non-desktop host (`nomifun-web`) has to publish its own
// LAN-reachable address to the robot endpoint advertiser.
pub mod lan_endpoint;

pub use config::{AppConfig, derive_encryption_key, load_or_create_data_encryption_key};
pub use desktop::{
    DesktopHostServices, DesktopKeepAlive, DesktopServer, DesktopStartError, LanRestoreOutcome,
    StartupCleanupDisposition, WebUiAsset, WebUiAssetSource, WebUiStatus,
};
pub use nomifun_auth::AuthPolicy;
pub use router::runtime_engines::{RuntimeEngineHost, SessionEngineDriverFactory};
pub use router::engine_session_host::{AdmittedEngineSession, EngineSessionHost, EngineTurnReceipt};
pub use router::engine_journal::{EngineJournalWrite, EngineTurnJournal};
pub use router::engine_history::{EngineHistoryRecord, EngineHistoryTurn, EngineHistoryWindow, EngineHistoryMessage, EngineMessageHistoryWindow};
pub use router::engine_model_facts::{EngineModelLimits, EngineRouteCandidateFacts, EngineRouteModelFacts};
pub use router::engine_kernel_session::EngineKernelSession;
pub use router::engine_skills::SelectedSkills as SelectedEngineSkills;
pub use router::engine_tool_host::{EngineToolHost, EngineToolObservationPolicy, BoundedEngineToolObservation, EngineToolDispatchRecord, bounded_engine_tool_result};

/// Test assembly facade for the in-process Nomi-core graph.
///
/// Product entry points compose [`bootstrap::NomiCoreApplication`] or use the
/// typed desktop startup API. Tests that need individual routers/services use
/// this facade instead of reaching private modules.
pub mod compatibility {
    pub use crate::router::{
        ChannelMessageLoopComponents, ModuleStates, build_conversation_state,
        build_module_states, build_skill_state, build_ws_state,
        create_router, create_router_with_all_state, create_router_with_states,
        try_create_router,
    };
    pub use crate::services::AppServices;
}

/// In-process server entry used by embedded hosts and by the `nomicore` bin's
/// default path. The product uses one Conversation owner with source-registered
/// Engines (official Nomi and Coding), selected by immutable Agent revisions.
pub async fn run_embedded_server(cli: &cli::Cli, merged_path: &str) -> anyhow::Result<std::process::ExitCode> {
    let env = bootstrap::init_nomi_core_environment(cli, merged_path)?;
    let application = bootstrap::NomiCoreApplication::compose(&env).await?;
    commands::run_nomi_core_server(env, application).await
}
