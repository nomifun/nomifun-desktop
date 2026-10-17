//! Manifest-approved product tool supply and platform command discovery.
//! Existing user runtimes take priority; missing required runtimes are prepared on demand.

mod cache;
mod download;
mod embed;
mod environment;
mod extract;
mod managed;
mod notices;
mod node_distribution;
#[path = "../node_distribution_contract.rs"]
mod node_distribution_contract;
mod progress;
mod resolver;
mod shell_env;
#[cfg(unix)]
mod login_shell;

pub use cache::{init, runtime_root};
pub use environment::{PreparedProcessLaunch, ProcessEnvironmentTarget, prepare_process_launch, process_environment, process_environment_for, user_cli_root};
pub use managed::{
    PreparedComponentLaunch, ToolError, ToolId, ToolSource, VerifiedTool,
    begin_tool_preparation, cancel_tool_preparation, component_inventory,
    prepare_component_launch, prepare_tool, resolve_existing_tool, resolve_tool, subscribe_tool_preparation,
    tool_preparation_snapshot,
};
pub use progress::{ToolPreparationFailure, ToolPreparationPhase, ToolPreparationSnapshot};
pub use resolver::{
    PreparedCommand, ResolveError, bun_bin_dir, managed_command_tool, prepare_managed_command,
    prepare_managed_command_with_env,
    resolve_bun, resolve_cached_managed_command, resolve_cached_managed_command_with_env,
    resolve_command_in, resolve_command_path,
};
pub use shell_env::enhance_process_path;
#[cfg(unix)]
pub use login_shell::resolve_login_shell;

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod node_acceptance;
#[cfg(all(test, windows))]
mod node_windows_acceptance;
