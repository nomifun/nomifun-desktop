//! A separate composition root, not a runtime loader or third official engine.
mod driver;
mod history;
mod model;

use clap::Parser;
use nomifun_app::{bootstrap, cli::Cli, commands};

fn main() -> anyhow::Result<std::process::ExitCode> {
    // Built-in Nomi in this composition still uses self-executable helpers.
    // These must never open the application database or register engines.
    if let Some(code) = commands::run_mcp_stdio_subcommand_if_present() {
        return Ok(code);
    }
    let mut cli = Cli::parse();
    anyhow::ensure!(
        cli.command.is_none(),
        "This reference executable only hosts the server; use nomicore for administrative commands"
    );
    cli.data_dir = bootstrap::resolve_nomi_core_data_root(cli.data_dir.clone());
    nomifun_runtime::init(&cli.data_dir);
    // Same startup ordering as nomicore: no worker threads exist yet.
    let path = unsafe { nomifun_runtime::enhance_process_path() };
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async move {
            let environment = bootstrap::init_nomi_core_environment(&cli, &path)?;
            let application = bootstrap::NomiCoreApplication::compose_with_runtime_engines(
                &environment,
                driver::register,
            )
            .await?;
            commands::run_nomi_core_server(environment, application).await
        })
}
