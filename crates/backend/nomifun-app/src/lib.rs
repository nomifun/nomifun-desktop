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

/// Explicitly isolated v3 compatibility graph used by legacy-focused tests.
///
/// Production server, Web, and native Desktop entry points compose
/// [`bootstrap::FreshV4Application`] and do not construct anything exported
/// here. The entire module remains scheduled for C9 physical deletion.
pub mod compatibility {
    pub use crate::router::{
        ChannelMessageLoopComponents, ModuleStates, build_conversation_state,
        build_extension_states, build_module_states, build_preset_state, build_ws_state,
        create_router, create_router_with_all_state, create_router_with_states,
        try_create_router,
    };
    pub use crate::services::AppServices;
}

/// In-process server entry used by embedded hosts (Tauri desktop, `nomifun-web`)
/// and by the `nomicore` bin's default path. Builds environment → data layer →
/// services, then serves until shutdown. For a host that also serves static
/// assets (the web SPA), compose `create_router` + your fallback instead.
pub async fn run_embedded_server(cli: &cli::Cli, merged_path: &str) -> anyhow::Result<std::process::ExitCode> {
    let env = bootstrap::init_environment(cli, merged_path)?;
    let host = env.canonical_host()?;
    let application = host.compose(&env.config).await?;
    commands::run_canonical_server(env, application).await
}
