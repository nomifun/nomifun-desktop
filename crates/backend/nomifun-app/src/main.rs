use std::process::ExitCode;

use anyhow::Result;
use clap::Parser;

// bootstrap/cli/commands now live in the library so embedded hosts can reuse
// them; the bin consumes them from there.
use nomifun_app::cli::{Cli, Command};
use nomifun_app::{bootstrap, commands};

fn main() -> Result<ExitCode> {
    let mut cli = Cli::parse();

    // mcp-* subcommands route into short-lived stdio helpers that live entirely
    // outside the main HTTP server. They share the global flags so clap can
    // parse a uniform CLI, but bypass `nomifun_runtime::init` (which would
    // anchor the bun cache under --data-dir) — these helpers don't host agents.
    //
    // `doctor`, in contrast, is meant to mirror the real server's CLI
    // detection path exactly. It must hit the same `nomifun_runtime::init`
    // (so the bundled `bun` resolves through the same cache the server
    // uses) before falling through to PATH probing.
    //
    // Server-shaped commands additionally resolve the effective data root:
    // known self-export/default locations map onto the channel default and
    // the one-shot legacy layout migration runs (`NomiFun/Nomi<suffix>` →
    // `NomiFun<suffix>`). MCP helpers keep their inherited value verbatim —
    // it is the parent backend's authoritative export, not a boot decision.
    let owns_data_root = matches!(
        cli.command,
        None | Some(Command::Doctor) | Some(Command::Backup { .. })
    );
    if owns_data_root {
        cli.data_dir =
            bootstrap::resolve_startup_data_root(cli.data_dir.clone());
    }
    let needs_runtime = matches!(cli.command, None | Some(Command::Doctor));
    if needs_runtime {
        nomifun_runtime::init(&cli.data_dir);
    }

    // SAFETY: called before any worker thread exists (including the tokio
    // runtime constructed below). Rust 2024 requires `unsafe` for
    // `std::env::set_var` invoked inside `enhance_process_path`.
    let merged_path = unsafe { nomifun_runtime::enhance_process_path() };

    let runtime = if commands::is_mcp_stdio_cli_command(cli.command.as_ref()) {
        commands::build_mcp_stdio_runtime()?
    } else {
        // The long-lived application remains hardware-adaptive; only the
        // short-lived stdio proxy children use the fixed runtime profile.
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?
    };
    runtime.block_on(async_main(merged_path, cli))
}

async fn async_main(merged_path: String, cli: Cli) -> Result<ExitCode> {
    // MCP stdio helpers must not touch the database, logging setup, or `AppServices`.
    match &cli.command {
        Some(Command::McpRequirementStdio) => Ok(commands::run_requirement_stdio().await),
        Some(Command::McpKnowledgeStdio) => Ok(commands::run_knowledge_stdio().await),
        Some(Command::McpGatewayStdio) => Ok(commands::run_gateway_stdio().await),
        Some(Command::McpOpenStdio) => Ok(commands::run_open_stdio().await),
        Some(Command::TerminalHook { event }) => Ok(commands::run_terminal_hook(event).await),
        Some(Command::Doctor) => commands::run_doctor(&cli, &merged_path).await,
        Some(Command::Remote { operation }) => Ok(commands::run_remote(operation).await),
        Some(Command::Backup { output }) => commands::run_backup(&cli, output.clone()).await,
        Some(Command::Restore {
            bundle,
            destination_data_dir,
        }) => commands::run_restore(bundle.clone(), destination_data_dir.clone()).await,
        None => nomifun_app::run_embedded_server(&cli, &merged_path).await,
    }
}
