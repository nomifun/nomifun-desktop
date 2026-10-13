//! Manifest-approved product tool supply and platform command discovery.
//! Existing user runtimes take priority; managed downloads start only by explicit request.

mod cache;
mod download;
mod embed;
mod extract;
mod managed;
mod notices;
mod progress;
mod resolver;
mod shell_env;
#[cfg(unix)]
mod login_shell;

pub use cache::{init, runtime_root};
pub use managed::{
    ToolError, ToolId, ToolSource, VerifiedTool, begin_tool_preparation, cancel_tool_preparation,
    component_inventory, prepare_tool, resolve_existing_tool, resolve_tool, subscribe_tool_preparation,
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
