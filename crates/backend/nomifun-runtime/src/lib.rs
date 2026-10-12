//! Manifest-approved product tool supply and platform command discovery.
//! Managed tools have one verified absolute path and version for this process.

mod cache;
mod download;
mod embed;
mod extract;
mod managed;
mod notices;
mod progress;
mod resolver;
mod shell_env;

pub use cache::{init, runtime_root};
pub use managed::{
    ToolError, ToolId, VerifiedTool, begin_tool_preparation, cancel_tool_preparation,
    component_inventory, prepare_tool, resolve_tool, subscribe_tool_preparation,
    tool_preparation_snapshot,
};
pub use progress::{ToolPreparationFailure, ToolPreparationPhase, ToolPreparationSnapshot};
pub use resolver::{
    PreparedCommand, ResolveError, bun_bin_dir, managed_command_tool, prepare_managed_command,
    resolve_bun, resolve_cached_managed_command, resolve_command_in, resolve_command_path,
};
pub use shell_env::enhance_process_path;
