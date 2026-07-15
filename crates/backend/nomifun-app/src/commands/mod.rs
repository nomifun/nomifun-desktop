//! Subcommand implementations for the `nomicore` binary.
//!
//! This file is a façade — module declarations and re-exports only.
//! All logic lives in the submodules.

#[cfg(feature = "browser-use")]
mod browser_stdio;
#[cfg(feature = "browser-use")]
pub(crate) use browser_stdio::bundled_chrome_dir;
#[cfg(feature = "computer-use")]
mod computer_stdio;
mod ctl;
mod backup;
mod doctor;
mod gateway_stdio;
mod knowledge_stdio;
pub(crate) mod mcp_register_template;
mod mcp_stdio;
mod open_stdio;
pub(crate) mod register_knowledge;
pub(crate) mod register_knowledge_global;
mod requirement_stdio;
mod server;
mod stdio_common;
mod terminal_hook;

#[cfg(feature = "browser-use")]
pub use browser_stdio::run_browser_stdio;
#[cfg(feature = "computer-use")]
pub use computer_stdio::run_computer_stdio;
pub use ctl::{run_call, run_tools};
pub use backup::{run_backup, run_restore};
pub use doctor::run_doctor;
pub use gateway_stdio::run_gateway_stdio;
pub use knowledge_stdio::run_knowledge_stdio;
pub use mcp_stdio::run_mcp_stdio_subcommand_if_present;
pub use open_stdio::run_open_stdio;
pub use requirement_stdio::run_requirement_stdio;
pub use server::run_server;
pub use terminal_hook::run_terminal_hook;

/// Stub for builds without the `computer-use` feature: the discrete-tool desktop
/// bridge requires the native screen/input/UI-Automation stack, which web and
/// headless hosts deliberately omit. Keeps the `McpComputerStdio` subcommand and
/// its wiring uniform (no `cfg` sprinkled through cli/main/mcp_stdio) — invoking
/// it on such a build simply reports the capability is absent.
#[cfg(not(feature = "computer-use"))]
pub async fn run_computer_stdio() -> std::process::ExitCode {
    eprintln!(
        "[mcp-computer-stdio] this build lacks the `computer-use` feature; desktop control is \
         unavailable here."
    );
    std::process::ExitCode::from(1)
}

/// Stub for builds without the `browser-use` feature: the discrete-tool browser
/// bridge requires the self-hosted-CDP browser engine + Chromium stack, which web
/// and headless hosts deliberately omit. Keeps the `McpBrowserStdio` subcommand
/// and its wiring uniform (no `cfg` sprinkled through cli/main/mcp_stdio) —
/// invoking it on such a build simply reports the capability is absent.
#[cfg(not(feature = "browser-use"))]
pub async fn run_browser_stdio() -> std::process::ExitCode {
    eprintln!(
        "[mcp-browser-stdio] this build lacks the `browser-use` feature; browser automation is \
         unavailable here."
    );
    std::process::ExitCode::from(1)
}
