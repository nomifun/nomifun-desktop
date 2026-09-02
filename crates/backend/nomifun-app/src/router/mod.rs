//! HTTP router assembly for the application.

mod agent_platform;
pub(crate) mod agent_role_host;
pub(crate) mod agent_platform_host;
pub(crate) mod agent_wave2_host;
pub(crate) mod agent_wave2_vcs_push;
pub(crate) mod agent_wave4_host;
pub(crate) mod chat_broker_host;
pub(crate) mod fresh_v4_system;
pub(crate) mod legacy_conversation_port;
pub mod instance_token_routes;
pub(crate) mod remote_rest;
pub(crate) mod remote_runtime;
#[cfg(feature = "browser-use")]
pub(crate) mod browser_management;
#[cfg(feature = "browser-use")]
pub(crate) mod browser_login;
mod boot_terminal_proof;
mod computer_permissions;
mod health;
mod knowledge_registration;
mod model_failover;
mod routes;
mod state;
mod trace;

pub use agent_platform::create_agent_platform_router;
pub use routes::{
    create_router, create_router_with_all_state, create_router_with_states, try_create_router,
};
pub use state::{
    ChannelMessageLoopComponents, ModuleStates, build_preset_state, build_conversation_state,
    build_extension_states, build_module_states, build_ws_state,
};
