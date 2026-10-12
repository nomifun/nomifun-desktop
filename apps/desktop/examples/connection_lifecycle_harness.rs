//! Stdio JSON RPC over the production DesktopServer and RelayPairingManager.
//! The parent acceptance runner must capture stdout privately: `boot` returns
//! the test owner's trust capability so it can exercise the real local API.
#[path = "../src/relay_pairing.rs"]
mod relay_pairing;

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use serde_json::{Value, json};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    data_dir: PathBuf,
    #[arg(long)]
    spa_dir: Option<PathBuf>,
    #[arg(long)]
    frontend_url: Option<String>,
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<()> {
    let args = Args::parse();
    // Reserve stdout exclusively for RPC before DesktopServer installs its
    // default console subscriber. Never hold a stdout lock across an await:
    // backend request logging must not block the response the RPC awaits.
    tracing_subscriber::fmt().with_writer(std::io::stderr).with_ansi(false)
        .with_max_level(tracing::Level::WARN).try_init()
        .map_err(|_| anyhow::anyhow!("could not install private harness diagnostic channel"))?;
    anyhow::ensure!(args.data_dir.is_absolute(), "acceptance data root must be absolute");
    let marker = args.data_dir.join(".nomifun-connection-acceptance");
    anyhow::ensure!(std::fs::read_to_string(marker).ok().as_deref() == Some("connection-lifecycle-v1"),
        "refusing a data root without the isolated acceptance marker");
    // An acceptance result must exercise the shipped SHA-pinned runtime.
    for key in ["NOMIFUN_NFAGENT_PATH", "NOMIFUN_NFAGENT_DOWNLOAD_URL", "NOMIFUN_NFAGENT_SHA256"] {
        anyhow::ensure!(std::env::var_os(key).is_none(), "runtime overrides are prohibited in acceptance");
    }
    let work = args.data_dir.join("work");
    std::fs::create_dir_all(&work)?;
    let cli = nomifun_app::cli::Cli::parse_from([
        "connection-lifecycle-acceptance".to_owned(), "--data-dir".to_owned(), args.data_dir.display().to_string(),
        "--work-dir".to_owned(), work.display().to_string(),
    ]);
    let (server, _keep) = nomifun_app::DesktopServer::start_nomi_core(
        &cli, &std::env::var("PATH").unwrap_or_default(), args.spa_dir, args.frontend_url, None,
    ).await.context("isolated Desktop backend startup failed")?;
    let manager = Arc::new(relay_pairing::RelayPairingManager::new(args.data_dir)?);
    let relay_shutdown_owner = manager.clone();
    server.register_relay_shutdown(move || {
        let owner = relay_shutdown_owner.clone();
        async move { owner.shutdown().await }
    })?;
    let restoring = manager.clone();
    let restore_server = server.clone();
    tokio::spawn(async move { restoring.restore(restore_server).await; });
    let input = std::io::stdin();
    for line in input.lock().lines() {
        let line = line?;
        let request: Value = serde_json::from_str(&line).context("invalid RPC request")?;
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let method = request.get("method").and_then(Value::as_str).unwrap_or("");
        let stage = rpc_stage(method);
        eprintln!("[native-rpc] begin {stage}");
        let result: Result<Value> = async {
            match method {
                "boot" => Ok(json!({"backendUrl":format!("http://127.0.0.1:{}",server.loopback_port()),
                    "nativeTrust":server.local_trust_secret(),"relayPort":25811})),
                // This explicit acceptance RPC authorizes the pinned download;
                // automatic restore and ordinary actions remain local-only.
                "prepare_runtime" => Ok(json!({"runtimePath":manager.download_nfagent().await?.display().to_string(),
                    "runtimeVersion":"0.1.0"})),
                "bootstrap" => Ok(serde_json::to_value(manager.bootstrap(server.clone(),
                    request.get("envelope").and_then(Value::as_str).context("bootstrap envelope is required")?,
                    request.get("allowRuntimeDownload").and_then(Value::as_bool).unwrap_or(false)).await?)?),
                "status" => Ok(serde_json::to_value(manager.status(server.clone()).await?)?),
                "pause" => Ok(serde_json::to_value(manager.stop(server.clone()).await?)?),
                "restart" => Ok(serde_json::to_value(manager.restart(server.clone(),
                    request.get("allowRuntimeDownload").and_then(Value::as_bool).unwrap_or(false)).await?)?),
                "disconnect" => Ok(serde_json::to_value(manager.disconnect(server.clone()).await?)?),
                "forget_local" => Ok(serde_json::to_value(manager.forget_local(server.clone()).await?)?),
                "refresh_qr" => Ok(serde_json::to_value(manager.refresh_qr(server.clone()).await?)?),
                "lan_start" => Ok(serde_json::to_value(server.start_lan().await)?),
                "lan_stop" => Ok(serde_json::to_value(server.stop_lan().await)?),
                "lan_status" => Ok(serde_json::to_value(server.status_snapshot().await)?),
                "shutdown" => { server.shutdown_all().await?; Ok(json!({"stopped":true})) },
                _ => anyhow::bail!("unknown RPC method"),
            }
        }.await;
        let response = match result {
            Ok(value) => json!({"id":id,"ok":true,"result":value}),
            Err(error) => json!({"id":id,"ok":false,"error":format!("{error:#}")}),
        };
        eprintln!("[native-rpc] finish {stage} ok={}", response["ok"] == true);
        {
            let mut output = std::io::stdout().lock();
            writeln!(output, "{response}")?;
            output.flush()?;
        }
        if method == "shutdown" && response["ok"] == true { return Ok(()); }
    }
    server.shutdown_all().await?;
    Ok(())
}

fn rpc_stage(method: &str) -> &'static str {
    match method {
        "boot" => "boot", "prepare_runtime" => "prepare_runtime", "bootstrap" => "bootstrap",
        "status" => "status", "pause" => "pause", "restart" => "restart", "disconnect" => "disconnect",
        "forget_local" => "forget_local",
        "refresh_qr" => "refresh_qr", "lan_start" => "lan_start", "lan_stop" => "lan_stop",
        "lan_status" => "lan_status", "shutdown" => "shutdown", _ => "unknown",
    }
}
