//! Retain the complete graph when a source-composed server fails to assemble.
//! Early service-construction cleanup is a different, smaller authority.
use std::{sync::Arc, time::Duration};

use crate::services::AppServices;

struct CleanupAuthority {
    services: AppServices,
    complete: tokio::sync::Mutex<bool>,
}

impl CleanupAuthority {
    async fn cleanup(&self) -> anyhow::Result<()> {
        let mut complete = self.complete.lock().await;
        if *complete {
            return Ok(());
        }
        self.services.shutdown_nomi_core_host().await?;
        *complete = true;
        Ok(())
    }
}

/// Downcastable failure returned when application composition failed and full
/// host cleanup has not yet been proven. Keep the host executor and startup
/// environment alive; a retained worker retries cleanup without rebuilding any
/// engine or re-executing a turn. The error itself also retains the service graph.
pub struct NomiCoreCompositionCleanupError {
    error: anyhow::Error,
    cleanup_error: anyhow::Error,
    authority: Arc<CleanupAuthority>,
}

impl NomiCoreCompositionCleanupError {
    /// Join/retry the same cleanup authority, serialized with automatic retries.
    /// Success proves host resource cleanup, not successful application startup.
    pub async fn retry_cleanup(&self) -> anyhow::Result<()> {
        self.authority.cleanup().await
    }
}

impl std::fmt::Debug for NomiCoreCompositionCleanupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NomiCoreCompositionCleanupError")
            .field("error", &self.error)
            .field("cleanup_error", &self.cleanup_error)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Display for NomiCoreCompositionCleanupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:#}; composed host cleanup remains unproven: {:#}",
            self.error, self.cleanup_error
        )
    }
}

impl std::error::Error for NomiCoreCompositionCleanupError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.error.source()
    }
}

pub(super) async fn cleanup_failed_composition(
    services: AppServices,
    error: anyhow::Error,
) -> anyhow::Error {
    let authority = Arc::new(CleanupAuthority {
        services,
        complete: tokio::sync::Mutex::new(false),
    });
    match authority.cleanup().await {
        Ok(()) => error,
        Err(cleanup_error) => {
            // Compatibility callers may drop an anyhow error immediately.
            // Retain the graph and its boot server-lock authority independently;
            // do not close SQLite while an engine's exit is still unknown.
            let retained = authority.clone();
            tokio::spawn(async move {
                let mut delay = Duration::from_millis(250);
                loop {
                    tokio::time::sleep(delay).await;
                    match retained.cleanup().await {
                        Ok(()) => break,
                        Err(error) => {
                            tracing::warn!(%error, "source-composed host cleanup remains pending")
                        }
                    }
                    delay = (delay * 2).min(Duration::from_secs(10));
                }
            });
            anyhow::Error::new(NomiCoreCompositionCleanupError {
                error,
                cleanup_error,
                authority,
            })
        }
    }
}
