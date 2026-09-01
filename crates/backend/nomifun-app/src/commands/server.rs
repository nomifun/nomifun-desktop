//! `nomicore` (no subcommand): the canonical Fresh-v4 HTTP server.

use std::process::ExitCode;
use std::time::Instant;

use anyhow::{Context, Result};
use tracing::{info, warn};

use crate::bootstrap::{FreshV4Application, ServerEnvironment};

/// Start the canonical Fresh-v4 HTTP server.
///
/// This path deliberately owns only the v4 application composition. It never
/// opens or closes the legacy `nomifun-backend.db`, and it has no dependency on
/// the compatibility service graph.
pub async fn run_canonical_server(
    env: ServerEnvironment,
    application: FreshV4Application,
) -> Result<ExitCode> {
    let boot = Instant::now();
    env.require_fresh_v4("Fresh-v4 HTTP server startup")?;

    let ip: std::net::IpAddr = match env.config.host.parse().with_context(|| {
        format!(
            "invalid host '{}': expected an IP literal like 127.0.0.1 or 0.0.0.0",
            env.config.host
        )
    }) {
        Ok(ip) => ip,
        Err(error) => {
            return Err(merge_canonical_cleanup_error(
                error,
                application.close().await,
            ));
        }
    };

    let (actual_port, listener) =
        match crate::bootstrap::bind_with_fallback(ip, env.config.port).await {
            Ok(bound) => bound,
            Err(error) => {
                return Err(merge_canonical_cleanup_error(
                    error,
                    application.close().await,
                ));
            }
        };
    if actual_port != env.config.port {
        warn!(
            requested = env.config.port,
            actual = actual_port,
            "preferred port was busy; bound a fallback port"
        );
    }
    crate::bootstrap::announce_bound_port(
        &env.config.data_dir,
        &env.config.host,
        actual_port,
    );
    info!(
        elapsed_ms = boot.elapsed().as_millis(),
        host = %env.config.host,
        port = actual_port,
        "Fresh-v4 server listening"
    );

    let (shutdown_tx, _shutdown_rx) = tokio::sync::watch::channel(false);
    let signal_shutdown_tx = shutdown_tx.clone();
    let serve_result = axum::serve(listener, application.router())
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            let _ = signal_shutdown_tx.send(true);
        })
        .await;
    let _ = shutdown_tx.send(true);
    let cleanup_result = application.close().await;
    drop(env);

    match (serve_result, cleanup_result) {
        (Ok(()), Ok(())) => {
            info!("Fresh-v4 server shut down gracefully");
            Ok(ExitCode::SUCCESS)
        }
        (Err(error), Ok(())) => Err(anyhow::Error::new(error)),
        (Ok(()), Err(cleanup_error)) => Err(cleanup_error),
        (Err(error), Err(cleanup_error)) => Err(anyhow::anyhow!(
            "{error}; Fresh-v4 runtime cleanup also failed: {cleanup_error:#}"
        )),
    }
}

fn merge_canonical_cleanup_error(
    error: anyhow::Error,
    cleanup: anyhow::Result<()>,
) -> anyhow::Error {
    match cleanup {
        Ok(()) => error,
        Err(cleanup_error) => anyhow::anyhow!(
            "{error:#}; Fresh-v4 runtime cleanup also failed: {cleanup_error:#}"
        ),
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
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
