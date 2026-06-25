//! `nomicore` (no subcommand): the main HTTP server.

use std::process::ExitCode;
use std::time::Instant;

use anyhow::{Context, Result};
use tracing::{info, warn};

use crate::{AppServices, create_router};

use crate::bootstrap::ServerEnvironment;

/// Start the HTTP server with fully constructed services.
pub async fn run_server(env: ServerEnvironment, services: AppServices) -> Result<ExitCode> {
    let boot = Instant::now();

    let has_users = services.user_repo.has_users().await?;
    if !has_users {
        info!("No configured users detected — initial setup required via /api/auth/status");
    }

    let router = create_router(&services).await;
    info!(
        elapsed_ms = boot.elapsed().as_millis(),
        "startup: router ready for socket bind"
    );
    // Resolve the bind IP up front so a bad host fails fast and clearly.
    let ip: std::net::IpAddr = env.config.host.parse().with_context(|| {
        format!(
            "invalid host '{}': expected an IP literal like 127.0.0.1 or 0.0.0.0",
            env.config.host
        )
    })?;
    info!(
        elapsed_ms = boot.elapsed().as_millis(),
        host = %ip,
        preferred_port = env.config.port,
        "startup: socket bind started"
    );
    // Port failover (shared with desktop LAN + nomifun-web): bind a bounded-scan
    // neighbour or an ephemeral port instead of hard-failing if the preferred
    // port is taken, then announce the actually-bound port via port.json/stdout.
    let (actual_port, listener) =
        crate::bootstrap::bind_with_fallback(ip, env.config.port).await?;
    if actual_port != env.config.port {
        warn!(
            requested = env.config.port,
            actual = actual_port,
            "preferred port was busy; bound a fallback port"
        );
    }
    crate::bootstrap::announce_bound_port(&env.config.data_dir, &env.config.host, actual_port);
    info!(
        elapsed_ms = boot.elapsed().as_millis(),
        host = %env.config.host,
        port = actual_port,
        "startup: socket bind completed"
    );
    info!(
        elapsed_ms = boot.elapsed().as_millis(),
        "Server listening on {}:{}", env.config.host, actual_port
    );

    // Kick off the idle-ACP-agent reaper. `start_idle_scanner` returns
    // immediately with a `JoinHandle`; the scanner task polls every 60 s
    // and kills ACP agents whose `status == Finished` + last_activity
    // exceeds the default 5-minute idle threshold. The watch channel
    // propagates graceful-shutdown so the scanner exits on SIGINT/SIGTERM.
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let idle_scanner_handle =
        nomifun_ai_agent::start_idle_scanner(services.worker_task_manager.clone(), shutdown_rx, None, None);

    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            let _ = shutdown_tx.send(true);
        })
        .await?;

    // Wait for the scanner to observe the shutdown watch value and
    // return; at worst this blocks for the current 60 s tick.
    if let Err(e) = idle_scanner_handle.await {
        warn!(error = %e, "idle scanner join failed");
    }

    services.database.close().await;
    info!("Server shut down gracefully");

    // Prevent the log guard from being dropped before final log flush.
    drop(env);

    Ok(ExitCode::SUCCESS)
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {
            info!("Received SIGINT, shutting down...");
        }
        () = terminate => {
            info!("Received SIGTERM, shutting down...");
        }
    }
}
