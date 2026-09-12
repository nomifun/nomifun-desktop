//! HTTP router assembly for the application.

mod agent_platform;
pub(crate) mod agent_role_host;
pub(crate) mod agent_platform_host;
pub(crate) mod agent_wave1_companion_host;
pub(crate) mod agent_wave1_memory_receipts;
pub(crate) mod agent_wave2_host;
pub(crate) mod agent_wave2_mcp;
pub(crate) mod agent_wave2_vcs_push;
pub(crate) mod agent_wave3_creation_host;
pub(crate) mod agent_wave3_host;
pub(crate) mod agent_wave3_miniapp_host;
pub(crate) mod agent_wave3_template_runner;
pub(crate) mod agent_wave3_workshop_host;
pub(crate) mod agent_wave4_host;
pub(crate) mod nomi_core_wave4;
pub(crate) mod chat_broker_host;
pub(crate) mod fresh_v4_system;
pub(crate) mod legacy_conversation_port;
pub mod instance_token_routes;
pub(crate) mod remote_rest;
pub(crate) mod remote_runtime;
pub(crate) mod nomi_core_agent_projection;
pub(crate) mod nomi_core_builtins;
pub(crate) mod nomi_core_chat_route;
pub(crate) mod nomi_core_control_plane;
pub(crate) mod nomi_core_remote_mcp;
pub(crate) mod nomi_core_robot;
pub(crate) mod nomi_core_resource_bindings;
pub(crate) mod nomi_core_session;
pub(crate) mod nomi_core_wave2;
pub(crate) mod plugin_platform;
mod plugin_runtime_host;
#[cfg(feature = "browser-use")]
pub(crate) mod browser_management;
#[cfg(feature = "browser-use")]
pub(crate) mod browser_login;
mod boot_terminal_proof;
mod computer_permissions;
mod health;
mod javascript_runtime;
mod knowledge_registration;
mod miniapp_m1;
mod model_failover;
mod routes;
mod state;
mod trace;

pub use agent_platform::create_agent_platform_router;
pub use routes::{
    create_router, create_router_with_all_state, create_router_with_states, try_create_router,
};
pub use state::{
    ChannelMessageLoopComponents, ModuleStates, build_conversation_state,
    build_module_states, build_skill_state, build_ws_state,
};
