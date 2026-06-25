//! Shared MCP stdio bridge dispatch for every host binary.
//!
//! The unified backend is hosted by several binaries — `nomicore`, the
//! `nomifun-web` server, and the `nomifun-desktop` Tauri shell. When an ACP
//! agent CLI (claude/codex/gemini) needs a stdio MCP server it spawns
//! `current_exe() <subcommand>`, which is whichever host binary is running.
//! Every host must therefore honor these subcommands or the injected tools
//! (`requirement_complete`, knowledge, gateway, open/computer/browser) never
//! appear in the session — the "single-binary model". `nomicore` dispatches
//! them via clap; the embedded hosts call [`run_mcp_stdio_subcommand_if_present`]
//! at the very top of `main`.

use std::process::ExitCode;

/// The MCP stdio bridge subcommands a host binary must honor when spawned by an
/// ACP agent CLI. These kebab strings are load-bearing: they mirror the clap
/// `Command` kebab names that external CLIs spawn verbatim.
const MCP_STDIO_SUBCOMMANDS: &[&str] = &[
    "mcp-requirement-stdio",
    "mcp-knowledge-stdio",
    "mcp-gateway-stdio",
    "mcp-open-stdio",
    "mcp-computer-stdio",
    "mcp-browser-stdio",
];

/// The `terminal-hook` subcommand string. Dispatched alongside MCP bridges (same
/// reason: must run before GUI/DB init) but carries an `--event <kind>` arg and
/// is NOT an MCP server — fire-and-forget HTTP POST.
const TERMINAL_HOOK_SUBCOMMAND: &str = "terminal-hook";

/// Classify `argv[1]` as one of the MCP stdio bridge subcommands, if it is one.
fn mcp_stdio_subcommand(argv1: Option<&str>) -> Option<&'static str> {
    let arg = argv1?;
    MCP_STDIO_SUBCOMMANDS.iter().copied().find(|&c| c == arg)
}

/// If this process was spawned by an ACP agent CLI as an MCP stdio bridge
/// (`current_exe() mcp-requirement-stdio`) or as the terminal lifecycle hook
/// relay (`terminal-hook`),
/// run that short-lived helper and return its exit code; otherwise return `None`
/// so the caller proceeds with normal startup.
///
/// Call this FIRST in `main`, before any arg parsing, runtime init, logging, or
/// window creation: the helpers read their config (port/token/conversation id)
/// from env, speak MCP over inherited stdio, and must not touch the database,
/// services, or open a GUI. Mirrors `nomicore`'s clap dispatch so every embedded
/// host (`nomifun-web`, `nomifun-desktop`) honors the same single-binary model.
pub fn run_mcp_stdio_subcommand_if_present() -> Option<ExitCode> {
    let argv1 = std::env::args().nth(1);
    let argv1_str = argv1.as_deref();

    // Check MCP stdio bridges first.
    if let Some(which) = mcp_stdio_subcommand(argv1_str) {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed to build MCP stdio runtime");
        let code = runtime.block_on(async move {
            match which {
                "mcp-requirement-stdio" => super::run_requirement_stdio().await,
                "mcp-knowledge-stdio" => super::run_knowledge_stdio().await,
                "mcp-gateway-stdio" => super::run_gateway_stdio().await,
                "mcp-open-stdio" => super::run_open_stdio().await,
                "mcp-computer-stdio" => super::run_computer_stdio().await,
                "mcp-browser-stdio" => super::run_browser_stdio().await,
                // Unreachable: `which` came from `mcp_stdio_subcommand`.
                other => unreachable!("unclassified mcp stdio subcommand: {other}"),
            }
        });
        return Some(code);
    }

    // Check `terminal-hook` — same reason (pre-GUI/DB), but it carries --event.
    if argv1_str == Some(TERMINAL_HOOK_SUBCOMMAND) {
        let event = parse_terminal_hook_event_arg();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed to build terminal-hook runtime");
        let code = runtime.block_on(async move { super::run_terminal_hook(&event).await });
        return Some(code);
    }

    None
}

/// Parse `--event <value>` from argv for the `terminal-hook` subcommand.
/// Falls back to `"unknown"` if missing (never crash the hook relay — observe-only).
fn parse_terminal_hook_event_arg() -> String {
    let args: Vec<String> = std::env::args().collect();
    // Expect: argv[0]=binary, argv[1]=terminal-hook, argv[2]=--event, argv[3]=<kind>
    for i in 2..args.len() {
        if args[i] == "--event" {
            if let Some(val) = args.get(i + 1) {
                return val.clone();
            }
        }
        // Also support `--event=<value>` form.
        if let Some(val) = args[i].strip_prefix("--event=") {
            return val.to_string();
        }
    }
    "unknown".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Command};
    use clap::Parser;

    #[test]
    fn classifies_the_mcp_stdio_subcommands() {
        assert_eq!(
            mcp_stdio_subcommand(Some("mcp-requirement-stdio")),
            Some("mcp-requirement-stdio")
        );
        assert_eq!(
            mcp_stdio_subcommand(Some("mcp-knowledge-stdio")),
            Some("mcp-knowledge-stdio")
        );
        assert_eq!(
            mcp_stdio_subcommand(Some("mcp-gateway-stdio")),
            Some("mcp-gateway-stdio")
        );
        assert_eq!(
            mcp_stdio_subcommand(Some("mcp-open-stdio")),
            Some("mcp-open-stdio")
        );
        assert_eq!(
            mcp_stdio_subcommand(Some("mcp-computer-stdio")),
            Some("mcp-computer-stdio")
        );
        assert_eq!(
            mcp_stdio_subcommand(Some("mcp-browser-stdio")),
            Some("mcp-browser-stdio")
        );
    }

    #[test]
    fn ignores_flags_absent_argv_and_non_stdio_subcommands() {
        assert_eq!(mcp_stdio_subcommand(None), None);
        assert_eq!(mcp_stdio_subcommand(Some("--port")), None);
        assert_eq!(mcp_stdio_subcommand(Some("")), None);
        // `doctor` is a real subcommand but NOT a stdio bridge — must not match.
        assert_eq!(mcp_stdio_subcommand(Some("doctor")), None);
        // Team MCP is not surfaced in the product, so these hidden bridge names
        // must not be honored by embedded hosts.
        assert_eq!(mcp_stdio_subcommand(Some("mcp-bridge")), None);
        assert_eq!(mcp_stdio_subcommand(Some("mcp-guide-stdio")), None);
        assert_eq!(mcp_stdio_subcommand(Some("mcp-team-stdio")), None);
    }

    #[test]
    fn terminal_hook_is_recognized_by_early_dispatch() {
        // `terminal-hook` is not in the MCP list (it's not an MCP server), but
        // `run_mcp_stdio_subcommand_if_present` must still recognize and dispatch it.
        // Here we verify the constant is correct and that the classifier does NOT
        // claim it (separate code path).
        assert_eq!(TERMINAL_HOOK_SUBCOMMAND, "terminal-hook");
        assert_eq!(mcp_stdio_subcommand(Some("terminal-hook")), None);
    }

    #[test]
    fn terminal_hook_parses_as_valid_cli_command() {
        // Drift guard: `terminal-hook --event turn_end` must parse as the
        // TerminalHook clap variant.
        let cli = Cli::try_parse_from(["nomicore", "terminal-hook", "--event", "turn_end"])
            .expect("terminal-hook must parse as a valid nomicore subcommand");
        assert!(
            matches!(cli.command, Some(Command::TerminalHook { ref event }) if event == "turn_end"),
            "parsed but not to TerminalHook with expected event"
        );
    }

    #[test]
    fn every_listed_subcommand_is_a_real_cli_stdio_command() {
        // Drift guard: each string the hosts dispatch must parse as a valid
        // nomicore subcommand AND be one of the stdio bridge variants. If a clap
        // Command kebab name ever changes, this fails instead of silently
        // regressing the single-binary model.
        for sub in MCP_STDIO_SUBCOMMANDS {
            let cli = Cli::try_parse_from(["nomicore", sub])
                .unwrap_or_else(|e| panic!("'{sub}' must be a valid nomicore subcommand: {e}"));
            assert!(
                matches!(
                    cli.command,
                    Some(
                        Command::McpRequirementStdio
                            | Command::McpKnowledgeStdio
                            | Command::McpGatewayStdio
                            | Command::McpOpenStdio
                            | Command::McpComputerStdio
                            | Command::McpBrowserStdio
                    )
                ),
                "'{sub}' parsed but not to an MCP stdio command"
            );
        }
    }
}
