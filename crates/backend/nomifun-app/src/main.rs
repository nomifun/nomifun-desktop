use std::process::ExitCode;

use anyhow::Result;
use clap::Parser;

// bootstrap/cli/commands now live in the library so embedded hosts can reuse
// them; the bin consumes them from there.
use nomifun_app::cli::{Cli, Command};
use nomifun_app::{bootstrap, commands};

fn main() -> Result<ExitCode> {
    #[cfg(windows)]
    if let Some(code) = nomifun_app::windows_system_execution_entry() { return Ok(code); }
    let mut cli = Cli::parse();

    // mcp-* subcommands route into short-lived stdio helpers that live entirely
    // outside the main HTTP server. They share the global flags so clap can
    // parse a uniform CLI, but don't initialize host component preparation.
    //
    // `doctor`, in contrast, is meant to mirror the real server's CLI
    // detection path exactly. It initializes the same managed component root
    // as the server; component suppliers never fall back to a same-named PATH
    // executable. Only explicit verification begins preparation.
    //
    // SAFETY: discover/apply PATH before storage relocation can create
    // Tokio/SQLite workers, as well as before this host's server runtime.
    let merged_path = if matches!(cli.command, Some(Command::ToolDoctor { .. })) {
        String::new()
    } else {
        unsafe { nomifun_runtime::enhance_process_path() }
    };

    // Server-shaped commands resolve the one canonical data root.
    let owns_data_root = matches!(
        cli.command,
        None | Some(Command::Doctor) | Some(Command::Backup { .. })
    );
    if owns_data_root {
        cli.data_dir = if cli.command.is_none() {
            bootstrap::prepare_startup_storage_root(cli.data_dir.clone(), cli.work_dir.as_deref())?
        } else { bootstrap::resolve_startup_data_root(cli.data_dir.clone())? };
    }
    let needs_runtime = matches!(cli.command, None | Some(Command::Doctor) | Some(Command::ToolDoctor { verify: true }));
    if needs_runtime {
        nomifun_runtime::init(&cli.data_dir);
    }

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
        Some(Command::ToolDoctor { verify }) => commands::run_tool_doctor(*verify).await,
        Some(Command::Remote { operation }) => Ok(commands::run_remote(operation).await),
        Some(Command::Backup { output }) => commands::run_backup(&cli, output.clone()).await,
        Some(Command::Restore {
            bundle,
            destination_data_dir,
        }) => commands::run_restore(bundle.clone(), destination_data_dir.clone()).await,
        None => nomifun_app::run_embedded_server(&cli, &merged_path).await,
    }
}
