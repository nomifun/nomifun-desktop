//! Application crate: assembles all domain crates into an Axum server with DI and middleware.
//!
//! This file is a public façade — it only re-exports symbols defined in
//! submodules. All logic lives in the modules below.

mod config;
// Spec D2 delivery-notify observer (public so integration tests can drive
// the full receipt loop without the whole app harness).
pub mod delivery_notify;
#[cfg(feature = "browser-use")]
mod browser_lane_provider;
// Public only for `BUNDLED_CHROME_DIR_ENV`: the desktop shell resolves the
// Tauri resource dir and publishes it through that env seam (F48).
#[cfg(feature = "browser-use")]
pub mod browser_resource;
mod browser_inventory_events;
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
    DesktopKeepAlive, DesktopServer, DesktopStartError, LanRestoreOutcome,
    StartupCleanupDisposition, WebUiAsset, WebUiAssetSource, WebUiStatus,
};
pub use bootstrap::{CanonicalHost, FreshV4Host};
pub use nomifun_auth::AuthPolicy;
pub use router::create_agent_platform_router;

/// Test assembly facade for the in-process Nomi-core graph.
///
/// Product entry points compose [`bootstrap::NomiCoreApplication`] or use the
/// typed desktop startup API. Tests that need individual routers/services use
/// this facade instead of reaching private modules. Fresh-v4 remains isolated
/// in [`bootstrap::FreshV4Application`].
pub mod compatibility {
    pub use crate::router::{
        ChannelMessageLoopComponents, ModuleStates, build_conversation_state,
        build_extension_states, build_module_states, build_ws_state,
        create_router, create_router_with_all_state, create_router_with_states,
        try_create_router,
    };
    pub use crate::services::AppServices;
}

/// In-process server entry used by embedded hosts and by the `nomicore` bin's
/// default path. The current product composition is the original in-process
/// Nomi engine; Fresh-v4/Codex remains an explicit future host.
pub async fn run_embedded_server(cli: &cli::Cli, merged_path: &str) -> anyhow::Result<std::process::ExitCode> {
    let env = bootstrap::init_nomi_core_environment(cli, merged_path)?;
    let application = bootstrap::NomiCoreApplication::compose(&env).await?;
    commands::run_nomi_core_server(env, application).await
}
