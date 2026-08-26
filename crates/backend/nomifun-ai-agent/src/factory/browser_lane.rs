//! Trusted Browser Platform capability issuance for native Agent runtimes.
//!
//! This module deliberately owns only the factory-facing abstraction. The
//! application composition root supplies the concrete provider backed by its
//! process-wide `BrowserSessionHub`; model/config JSON never participates in
//! constructing [`TrustedBrowserRuntimeContext`].

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use nomifun_browser_platform::{BrowserErrorCode, BrowserLaneClient, BrowserSurface};
use nomifun_common::AppError;

/// Server-derived ownership facts used to issue one native runtime's browser
/// capability.
///
/// This type is intentionally not serializable or deserializable. Callers must
/// populate it from first-class runtime fields and authoritative persisted
/// execution links, never from the open-ended Agent `extra` object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustedBrowserRuntimeContext {
    pub user_id: String,
    pub conversation_id: Option<String>,
    pub runtime_instance_id: String,
    pub agent_id: Option<String>,
    pub execution_id: Option<String>,
    pub step_id: Option<String>,
    pub attempt_id: Option<String>,
    pub surface: BrowserSurface,
}

/// Revocation seam owned by the host-specific provider.
///
/// `revoke` must be idempotent and non-blocking. It starts or joins the exact
/// owner's retained cleanup flight because runtime `kill` and `Drop` are
/// synchronous lifecycle hooks. [`Self::revoke_and_wait`] starts or joins that
/// same flight and waits for its bounded completion proof; a failed or timed
/// out waiter must not consume the provider's cleanup authority.
#[async_trait::async_trait]
pub trait BrowserOwnerLeaseGuard: Send + Sync {
    fn revoke(&self);

    async fn revoke_and_wait(&self) -> Result<(), AppError>;
}

struct BrowserLaneBindingInner {
    client: BrowserLaneClient,
    lease: Arc<dyn BrowserOwnerLeaseGuard>,
    revocation_requested: AtomicBool,
}

impl BrowserLaneBindingInner {
    fn revoke(&self) {
        self.revocation_requested.store(true, Ordering::Release);
        // Do not suppress later calls here. The concrete lease guard owns the
        // retryable single-flight state, so a first failed Hub cleanup cannot
        // be mistaken for permanent completion at this generic boundary.
        self.lease.revoke();
    }

    async fn revoke_and_wait(&self) -> Result<(), AppError> {
        self.revocation_requested.store(true, Ordering::Release);
        self.lease.revoke_and_wait().await
    }
}

impl Drop for BrowserLaneBindingInner {
    fn drop(&mut self) {
        self.revoke();
    }
}

/// A native runtime's trusted Hub client plus its owner-lease lifecycle guard.
///
/// Clones share one revocation bit. Dropping the final clone is a construction
/// failure/backstop cleanup path; normal runtime teardown calls [`Self::revoke`]
/// explicitly.
#[derive(Clone)]
pub struct BrowserLaneBinding {
    inner: Arc<BrowserLaneBindingInner>,
}

impl BrowserLaneBinding {
    pub fn new(
        client: BrowserLaneClient,
        lease: Arc<dyn BrowserOwnerLeaseGuard>,
    ) -> Self {
        Self {
            inner: Arc::new(BrowserLaneBindingInner {
                client,
                lease,
                revocation_requested: AtomicBool::new(false),
            }),
        }
    }

    pub fn client(&self) -> BrowserLaneClient {
        self.inner.client.clone()
    }

    pub fn revoke(&self) {
        self.inner.revoke();
    }

    /// Close every ordinary Lane created by the current Agent turn without
    /// revoking the runtime's owner lease. Explicitly keep-alive Lanes (for
    /// example a user-requested media playback) remain available for the next
    /// turn; runtime teardown still revokes the owner and closes all Lanes.
    ///
    /// Native Nomi runtimes are intentionally reusable across turns.  A turn
    /// boundary must release its Chromium resources immediately, while the
    /// same trusted client remains valid so the next turn can lazily open a
    /// fresh default Lane.  This operation therefore uses the Hub's
    /// owner-scoped `close_all` semantics rather than the lease-revocation
    /// path used by runtime teardown.
    pub async fn close_turn_lanes(&self) -> Result<(), AppError> {
        match self
            .inner
            .client
            .close_turn_lanes()
            .await
        {
            Ok(_) => Ok(()),
            Err(error) => {
                // Runtime `kill()` revokes the owner lease before an unwinding
                // turn reaches this boundary, so `close_all` can fail with an
                // expired-lease refusal even though the Hub already treats
                // this owner's Lanes as its own cleanup obligation
                // (`close_owner_lease` closes by lease id without requiring a
                // live record, and the sweep covers naturally expired leases).
                // Only that exact refusal maps to satisfied-by-revocation; any
                // other failure — even after a revocation was requested — is a
                // real cleanup failure and must surface so the terminal event
                // is not published over a live Chromium Lane. The
                // result-bearing `revoke_and_wait` remains the teardown proof.
                if error.code == BrowserErrorCode::OwnerLeaseExpired {
                    // An expired lease proves that revocation started, not
                    // that its Chromium cleanup finished. Join the concrete
                    // guard's exact-owner flight and propagate any failure or
                    // timeout so no terminal event can overtake cleanup.
                    return self.inner.revoke_and_wait().await;
                }
                Err(AppError::Internal(format!(
                    "failed to close Native Nomi browser lanes at the turn boundary: {error}"
                )))
            }
        }
    }

    /// Revoke this exact owner and wait for the provider's bounded completion
    /// proof. Concurrent callers join one cleanup flight. A timeout leaves the
    /// flight and Hub-owned pending cleanup available to later callers.
    pub async fn revoke_and_wait(&self) -> Result<(), AppError> {
        self.inner.revoke_and_wait().await
    }

    /// Lifecycle-friendly alias used by result-bearing runtime shutdown paths.
    pub async fn shutdown(&self) -> Result<(), AppError> {
        self.revoke_and_wait().await
    }
}

impl fmt::Debug for BrowserLaneBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BrowserLaneBinding")
            .field("client", &"<redacted trusted capability>")
            .field(
                "revocation_requested",
                &self.inner.revocation_requested.load(Ordering::Acquire),
            )
            .finish()
    }
}

/// Host-provided issuer for native Browser Platform clients.
#[async_trait::async_trait]
pub trait BrowserLaneClientProvider: Send + Sync {
    async fn issue(
        &self,
        context: TrustedBrowserRuntimeContext,
    ) -> Result<BrowserLaneBinding, AppError>;
}

/// One-shot late-wire slot used by the application composition root.
///
/// `AppServices` must construct the Agent factory before it has finished
/// constructing the concrete Browser Host/Hub. The factory receives a clone of
/// this slot, then the composition root installs exactly one provider once the
/// process-wide Hub exists. A configured-but-empty slot is a fail-closed state,
/// not permission to launch a private browser.
#[derive(Clone, Default)]
pub struct BrowserLaneClientProviderSlot {
    provider: Arc<OnceLock<Arc<dyn BrowserLaneClientProvider>>>,
}

impl BrowserLaneClientProviderSlot {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn install(
        &self,
        provider: Arc<dyn BrowserLaneClientProvider>,
    ) -> Result<(), AppError> {
        self.provider.set(provider).map_err(|_| {
            AppError::Conflict(
                "the native browser lane provider is already installed".to_owned(),
            )
        })
    }

    pub fn get(&self) -> Option<Arc<dyn BrowserLaneClientProvider>> {
        self.provider.get().cloned()
    }
}

impl fmt::Debug for BrowserLaneClientProviderSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BrowserLaneClientProviderSlot")
            .field("installed", &self.provider.get().is_some())
            .finish()
    }
}
