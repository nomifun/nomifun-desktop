//! HTTP router assembly for the application.

pub(crate) mod agent_role_host;
pub(crate) mod agent_wave1_host;
pub(crate) mod agent_wave1_companion_host;
pub(crate) mod agent_memory_authority;
pub(crate) mod agent_wave1_memory_receipts;
pub(crate) mod agent_wave2_host;
pub(crate) mod agent_wave2_mcp;
pub(crate) mod agent_wave2_vcs_push;
pub(crate) mod agent_wave3_creation_host;
pub(crate) mod agent_wave3_host;
pub(crate) mod agent_wave3_plugin_host;
pub(crate) mod agent_wave3_template_runner;
pub(crate) mod agent_wave3_workshop_host;
pub(crate) mod agent_wave5_host;
pub(crate) mod nomi_core_wave4;
pub(crate) mod chat_broker_host;
pub(crate) mod legacy_conversation_port;
pub mod instance_token_routes;
pub(crate) mod remote_runtime;
pub(crate) mod nomi_core_agent_projection;
pub(crate) mod nomi_core_builtins;
pub(crate) mod nomi_core_tool_discovery;
pub(crate) mod nomi_core_chat_route;
pub(crate) mod nomi_core_control_plane;
pub(crate) mod nomi_core_role_defaults;
pub(crate) mod nomi_core_remote_mcp;
pub(crate) mod nomi_core_robot;
pub(crate) mod nomi_core_resource_bindings;
pub(crate) mod nomi_core_session;
pub mod runtime_engines;
pub mod engine_session_host;
pub mod engine_journal;
pub mod engine_history;
pub mod engine_model_facts;
pub mod engine_tool_host;
pub mod engine_kernel_session;
mod engine_git_lifecycle;
mod engine_plugin_product_tools;
mod engine_robot_tools;
mod workspace_file_read;
mod engine_workspace_media;
mod engine_mcp_media;
pub(crate) mod coding_runtime_host;
mod coding_runtime_history;
mod coding_patch_recovery;
mod engine_process_host;
mod engine_process_recovery;
mod coding_event_buffer;
mod coding_tool_surface;
mod coding_attachments;
mod coding_skills;
pub mod engine_skills;
pub(crate) mod nomi_core_wave2;
#[cfg(feature = "browser-use")]
pub(crate) mod knowledge_browser;
mod nomi_core_mcp;
mod nomi_core_mcp_resources;
mod mcp_effect_receipts;
mod hosted_effect_receipts;
mod nomi_core_mcp_catalog;
pub(crate) mod plugin_platform;
mod plugin_runtime_host;
#[cfg(feature = "browser-use")]
pub(crate) mod browser_workspace;
mod boot_terminal_proof;
mod computer_permissions;
mod health;
mod javascript_runtime;
mod knowledge_registration;
mod plugin_runtime;
mod plugin_product;
mod model_failover;
mod routes;
mod state;
mod skill_publication;
mod trace;

pub use routes::{
    create_router, create_router_with_all_state, create_router_with_states, try_create_router,
};
pub use state::{
    ChannelMessageLoopComponents, ModuleStates, build_conversation_state,
    build_module_states, build_skill_state, build_ws_state,
};
