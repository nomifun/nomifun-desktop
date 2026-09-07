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

    let has_users = match services.user_repo.has_users().await {
        Ok(has_users) => has_users,
        Err(error) => {
            return Err(services
                .cleanup_after_startup_failure(anyhow::Error::new(error))
                .await);
        }
    };
    if !has_users {
        info!("No configured users detected — initial setup required via /api/auth/status");
    }

    let router = create_router(&services).await;
    info!(
        elapsed_ms = boot.elapsed().as_millis(),
        "startup: router ready for socket bind"
    );
    // Resolve the bind IP up front so a bad host fails fast and clearly.
    let ip: std::net::IpAddr = match env.config.host.parse().with_context(|| {
        format!(
            "invalid host '{}': expected an IP literal like 127.0.0.1 or 0.0.0.0",
            env.config.host
        )
    }) {
        Ok(ip) => ip,
        Err(error) => {
            return Err(services.cleanup_after_startup_failure(error).await);
        }
    };
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
        match crate::bootstrap::bind_with_fallback(ip, env.config.port).await {
            Ok(bound) => bound,
            Err(error) => {
                return Err(services.cleanup_after_startup_failure(error).await);
            }
        };
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

    let (shutdown_tx, _shutdown_rx) = tokio::sync::watch::channel(false);

    let signal_shutdown_tx = shutdown_tx.clone();
    let serve_result = axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            let _ = signal_shutdown_tx.send(true);
        })
        .await;

    // `axum::serve` can fail before the graceful-shutdown future receives a
    // process signal. Always broadcast shutdown explicitly in that path.
    let _ = shutdown_tx.send(true);

    services.shutdown_computer_history().await;
    let browser_shutdown_result = services.shutdown_browser_platform().await;
    match &browser_shutdown_result {
        Ok(()) => info!("managed browser platform shut down"),
        Err(error) => warn!(%error, "managed browser platform shutdown failed"),
    }

    close_database_after_successful_browser_cleanup(&browser_shutdown_result, || {
        services.database.close()
    })
    .await;

    let result = merge_server_and_browser_shutdown_results(serve_result, browser_shutdown_result);
    if result.is_ok() {
        info!("Server shut down gracefully");
    }

    // Prevent the log guard from being dropped before final log flush.
    drop(env);

    result
}

fn merge_server_and_browser_shutdown_results(
    serve_result: std::io::Result<()>,
    browser_shutdown_result: anyhow::Result<()>,
) -> Result<ExitCode> {
    match (serve_result, browser_shutdown_result) {
        (Ok(()), Ok(())) => Ok(ExitCode::SUCCESS),
        (Err(serve_error), Ok(())) => Err(anyhow::Error::new(serve_error)),
        (Ok(()), Err(cleanup_error)) => Err(cleanup_error),
        (Err(serve_error), Err(cleanup_error)) => Err(anyhow::anyhow!(
            "{serve_error}; managed browser platform cleanup after server failure also failed: {cleanup_error:#}"
        )),
    }
}

async fn close_database_after_successful_browser_cleanup<F, Fut>(
    browser_cleanup_result: &anyhow::Result<()>,
    close_database: F,
)
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    if browser_cleanup_result.is_ok() {
        close_database().await;
    }
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::{
        close_database_after_successful_browser_cleanup,
        merge_server_and_browser_shutdown_results,
    };

    #[tokio::test]
    async fn failed_browser_cleanup_keeps_database_close_barrier_closed() {
        let close_calls = AtomicUsize::new(0);
        let browser_cleanup_result = Err(anyhow::anyhow!("browser cleanup failed"));

        close_database_after_successful_browser_cleanup(&browser_cleanup_result, || async {
            close_calls.fetch_add(1, Ordering::AcqRel);
        })
        .await;

        assert_eq!(
            close_calls.load(Ordering::Acquire),
            0,
            "database close must wait for confirmed browser cleanup"
        );
    }

    #[tokio::test]
    async fn successful_browser_cleanup_opens_database_close_barrier_once() {
        let close_calls = AtomicUsize::new(0);
        let browser_cleanup_result = Ok(());

        close_database_after_successful_browser_cleanup(&browser_cleanup_result, || async {
            close_calls.fetch_add(1, Ordering::AcqRel);
        })
        .await;

        assert_eq!(close_calls.load(Ordering::Acquire), 1);
    }

    #[test]
    fn successful_serve_and_browser_cleanup_return_success() {
        let result = merge_server_and_browser_shutdown_results(Ok(()), Ok(()));

        assert!(result.is_ok());
    }

    #[test]
    fn failed_serve_is_preserved_when_browser_cleanup_succeeds() {
        let error = merge_server_and_browser_shutdown_results(
            Err(std::io::Error::other("serve failed first")),
            Ok(()),
        )
        .expect_err("serve failure must be returned");

        assert_eq!(error.to_string(), "serve failed first");
    }

    #[test]
    fn browser_cleanup_failure_is_returned_after_successful_serve() {
        let error = merge_server_and_browser_shutdown_results(
            Ok(()),
            Err(anyhow::anyhow!("browser cleanup failed second")),
        )
        .expect_err("browser cleanup failure must be returned");

        assert_eq!(error.to_string(), "browser cleanup failed second");
    }

    #[test]
    fn failed_serve_stays_primary_when_browser_cleanup_also_fails() {
        let error = merge_server_and_browser_shutdown_results(
            Err(std::io::Error::other("serve failed first")),
            Err(anyhow::anyhow!("browser cleanup failed second")),
        )
        .expect_err("both failures must be returned");
        let display = error.to_string();

        assert!(
            display.starts_with("serve failed first"),
            "serve error must be displayed first: {display}"
        );
        assert!(
            display.contains("browser cleanup failed second"),
            "cleanup error must remain visible: {display}"
        );
        assert!(
            display.find("serve failed first") < display.find("browser cleanup failed second"),
            "serve error must precede cleanup error: {display}"
        );
    }
}
