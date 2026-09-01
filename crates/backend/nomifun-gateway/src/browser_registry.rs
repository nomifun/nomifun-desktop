//! Gateway adapter for the main-process browser session hub.
//!
//! The gateway is deliberately not a browser owner. It never constructs a
//! `BrowserTool`, browser engine, Chromium process, profile, or operation mutex.
//! Instead, the application injects the one shared [`BrowserSessionHub`] and a
//! trusted identity resolver. The hub owns lane allocation, per-lane
//! serialization, resource admission, and browser lifecycle.
//!
//! A lane is keyed by `(CallerIdentity.runtime_instance_id, lane_name)`. A
//! companion is attribution context only; it is never an ownership or lane key.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::Arc;

use futures::{StreamExt, stream};
use nomi_browser::{ManagedBrowserFacade, TRUSTED_OWNER_INPUT_FIELDS};
use nomi_types::tool::ToolResult;
use nomifun_browser_platform::{
    BrowserErrorCode, BrowserIdentityMode, BrowserLaneClient, BrowserLaneId,
    BrowserLaneSnapshot,
    BrowserOperation, BrowserOperationKind, BrowserOperationResult,
    BrowserPlatformError, BrowserSessionHub, BrowserSurface, CallerIdentity,
    CloseResult, LaneKey, OpenLaneOutcome, OwnerLeaseId, normalize_lane_name,
};
use serde_json::{Value, json};
use tokio::sync::Mutex as AsyncMutex;

use crate::deps::CallerCtx;

/// Complete server-defined browser operation scope.
///
/// Callers may pass a narrower set to [`BrowserRegistry::attach_trusted_identity_scoped`];
/// browser tool JSON never contributes to this authority.
pub fn all_browser_operations() -> BTreeSet<BrowserOperationKind> {
    BTreeSet::from([
        BrowserOperationKind::Navigate,
        BrowserOperationKind::Observe,
        BrowserOperationKind::Act,
        BrowserOperationKind::Crawl,
        BrowserOperationKind::Screenshot,
        BrowserOperationKind::Tabs,
        BrowserOperationKind::Download,
        BrowserOperationKind::Debug,
        BrowserOperationKind::Manage,
    ])
}

#[derive(Clone, Debug)]
struct CachedBrowserIdentity {
    identity: CallerIdentity,
    /// A failed Hub cleanup leaves this record authoritative and retryable.
    revocation_pending: bool,
    /// Exact cleanup debt for this attachment's current owner generation.
    ///
    /// This is deliberately an `Option`, not a collection: an expired owner
    /// must be cleaned before a replacement can be issued, so one runtime can
    /// never accumulate generations when cleanup keeps failing. The marker is
    /// published only after the Hub retained that owner's exact cleanup
    /// authority; a successful retry clears it before replacement admission.
    pending_owner_cleanup: Option<OwnerLeaseId>,
}

/// Safe shutdown postcondition for one attachment authority.
///
/// An attachment remains authoritative until every exact owner lease associated
/// with it has been handed back to the Hub. The Hub may retain lower-level Lane
/// or Host cleanup after that point; its own shutdown authority is responsible
/// for those retained resources.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BrowserCleanupStatus {
    pub pending_attachments: usize,
    pub pending_owner_leases: usize,
    pub revocation_pending_attachments: usize,
}

impl BrowserCleanupStatus {
    pub fn is_empty(self) -> bool {
        self.pending_attachments == 0
            && self.pending_owner_leases == 0
            && self.revocation_pending_attachments == 0
    }
}

/// One runtime-scoped lifecycle slot.
///
/// Attach/revoke transitions for the same trusted runtime must be atomic, but
/// unrelated runtimes must not wait behind a slow Hub cleanup. The slot removes
/// itself once the final holder finishes, so long-lived gateways do not retain
/// one mutex per runtime forever.
struct RuntimeLifecycleSlot {
    runtime_instance_id: String,
    slots: Arc<
        std::sync::Mutex<
            HashMap<String, Arc<AsyncMutex<()>>>,
        >,
    >,
    gate: Arc<AsyncMutex<()>>,
}

impl Drop for RuntimeLifecycleSlot {
    fn drop(&mut self) {
        let mut slots = self
            .slots
            .lock()
            .expect("gateway browser lifecycle slot store poisoned");
        let can_remove = slots
            .get(&self.runtime_instance_id)
            .is_some_and(|current| {
                Arc::ptr_eq(current, &self.gate)
                    && Arc::strong_count(&self.gate) == 2
            });
        if can_remove {
            slots.remove(&self.runtime_instance_id);
        }
    }
}

/// Resolves the browser capability that the main process attached to a
/// validated Gateway caller.
///
/// Implementations must never derive identity from model arguments. The normal
/// implementation reads [`CallerCtx::browser_identity`], which is populated
/// only by the authenticated app ingress.
pub trait TrustedBrowserIdentityResolver: Send + Sync {
    fn resolve(
        &self,
        caller: &CallerCtx,
    ) -> Result<CallerIdentity, BrowserPlatformError>;
}

impl<F> TrustedBrowserIdentityResolver for F
where
    F: Fn(&CallerCtx) -> Result<CallerIdentity, BrowserPlatformError>
        + Send
        + Sync,
{
    fn resolve(
        &self,
        caller: &CallerCtx,
    ) -> Result<CallerIdentity, BrowserPlatformError> {
        self(caller)
    }
}

/// Default resolver for the authenticated application path.
#[derive(Clone, Copy, Debug, Default)]
pub struct CallerCtxBrowserIdentityResolver;

impl TrustedBrowserIdentityResolver for CallerCtxBrowserIdentityResolver {
    fn resolve(
        &self,
        caller: &CallerCtx,
    ) -> Result<CallerIdentity, BrowserPlatformError> {
        caller.browser_identity.clone().ok_or_else(missing_identity_error)
    }
}

/// One gateway browser call used by [`BrowserRegistry::execute_parallel`].
#[derive(Clone, Debug)]
pub struct GatewayBrowserCall {
    pub caller: CallerCtx,
    pub lane_name: String,
    pub input: Value,
}

/// Upper bound on retained revoked-runtime tombstones. Lease/session ids are
/// never legitimately reused, so the tombstone only has to outlive the short
/// attach-vs-close race window; the oldest entries are evicted in insertion
/// order once this many distinct runtimes have been revoked.
const REVOKED_RUNTIME_TOMBSTONE_CAPACITY: usize = 4096;
/// Final-drain and retry sweeps may cover many legitimate active runtimes, but
/// they must never poll one cleanup future per runtime simultaneously.
const MAX_CONCURRENT_GATEWAY_OWNER_CLEANUPS: usize = 16;
const MODEL_IDENTITY_INPUT_FIELDS: &[&str] = &[
    "identity",
    "identity_mode",
    "authenticated",
    "auth_identity",
    "profile",
    "account",
];
// Trusted-owner caller fields are NOT gateway-local: every managed browser
// surface rejects/strips the ONE shared
// [`nomi_browser::TRUSTED_OWNER_INPUT_FIELDS`] list (F23), so identical
// requests behave identically across surfaces.

/// Bounded revoked-runtime tombstones (F62).
///
/// Ids are tracked in insertion order; once the capacity is reached the oldest
/// tombstone is evicted, keeping the anti-resurrection authority for recent
/// revocations without growing per revoked session forever.
#[derive(Default)]
struct RevokedRuntimeTombstones {
    entries: HashSet<String>,
    insertion_order: VecDeque<String>,
}

impl RevokedRuntimeTombstones {
    fn insert(&mut self, runtime_instance_id: &str) {
        if self.entries.insert(runtime_instance_id.to_owned()) {
            self.insertion_order.push_back(runtime_instance_id.to_owned());
            while self.insertion_order.len() > REVOKED_RUNTIME_TOMBSTONE_CAPACITY {
                let Some(evicted) = self.insertion_order.pop_front() else {
                    break;
                };
                self.entries.remove(&evicted);
            }
        }
    }

    fn contains(&self, runtime_instance_id: &str) -> bool {
        self.entries.contains(runtime_instance_id)
    }
}

/// Clone-cheap bridge to the application-owned browser hub.
#[derive(Clone)]
pub struct BrowserRegistry {
    hub: Option<BrowserSessionHub>,
    identity_resolver: Arc<dyn TrustedBrowserIdentityResolver>,
    /// Stable owner capability per server-validated runtime attachment.
    identities: Arc<std::sync::Mutex<HashMap<String, CachedBrowserIdentity>>>,
    /// Runtime ids are never reusable after an authoritative revoke. Keeping a
    /// bounded tombstone prevents an attach racing with close from resurrecting
    /// a revoked Browser owner.
    revoked_runtime_ids: Arc<std::sync::Mutex<RevokedRuntimeTombstones>>,
    /// Runtime-scoped attachment/revocation gates. Same-runtime transitions
    /// serialize; unrelated runtimes can attach and clean up concurrently.
    runtime_lifecycle_slots: Arc<
        std::sync::Mutex<
            HashMap<String, Arc<AsyncMutex<()>>>,
        >,
    >,
}

impl BrowserRegistry {
    /// Inject the shared main-process hub and trusted identity resolver.
    pub fn new(
        hub: BrowserSessionHub,
        identity_resolver: Arc<dyn TrustedBrowserIdentityResolver>,
    ) -> Self {
        Self {
            hub: Some(hub),
            identity_resolver,
            identities: Arc::new(std::sync::Mutex::new(HashMap::new())),
            revoked_runtime_ids: Arc::new(std::sync::Mutex::new(
                RevokedRuntimeTombstones::default(),
            )),
            runtime_lifecycle_slots: Arc::new(std::sync::Mutex::new(
                HashMap::new(),
            )),
        }
    }

    /// Inject a hub and use the app-populated identity on [`CallerCtx`].
    pub fn from_hub(hub: BrowserSessionHub) -> Self {
        Self::new(hub, Arc::new(CallerCtxBrowserIdentityResolver))
    }

    /// Attach a browser identity after the Gateway server validates signed
    /// child capability claims.
    ///
    /// `runtime_instance_id` must come from the signed child lease, never from
    /// tool arguments. Access-token renewals reuse one owner lease; a new
    /// child/attempt lease receives a distinct runtime and therefore a distinct
    /// default lane.
    pub async fn attach_trusted_identity(
        &self,
        caller: &mut CallerCtx,
        runtime_instance_id: &str,
        attempt_id: Option<&str>,
        capability_expires_at_ms: u64,
    ) -> Result<(), BrowserPlatformError> {
        self.attach_trusted_identity_scoped(
            caller,
            runtime_instance_id,
            attempt_id,
            capability_expires_at_ms,
            all_browser_operations(),
        )
        .await
    }

    /// Attach a browser identity with a server-derived operation scope.
    pub async fn attach_trusted_identity_scoped(
        &self,
        caller: &mut CallerCtx,
        runtime_instance_id: &str,
        attempt_id: Option<&str>,
        capability_expires_at_ms: u64,
        allowed_operations: BTreeSet<BrowserOperationKind>,
    ) -> Result<(), BrowserPlatformError> {
        self.attach_trusted_identity_scoped_locked(
            caller,
            runtime_instance_id,
            attempt_id,
            capability_expires_at_ms,
            allowed_operations,
        )
        .await
    }

    async fn attach_trusted_identity_scoped_locked(
        &self,
        caller: &mut CallerCtx,
        runtime_instance_id: &str,
        attempt_id: Option<&str>,
        capability_expires_at_ms: u64,
        allowed_operations: BTreeSet<BrowserOperationKind>,
    ) -> Result<(), BrowserPlatformError> {
        let lifecycle = self.runtime_lifecycle_slot(runtime_instance_id);
        let _lifecycle_guard = lifecycle.gate.lock().await;
        let hub = self.hub.clone().ok_or_else(|| {
            BrowserPlatformError::new(
                BrowserErrorCode::BrowserUnavailable,
                "The browser hub was not injected into the gateway.",
                true,
                "Start browser support in the main application and retry.",
            )
        })?;
        if runtime_instance_id.is_empty()
            || runtime_instance_id.trim() != runtime_instance_id
            || runtime_instance_id.len() > 128
        {
            return Err(missing_identity_error());
        }
        if allowed_operations.is_empty() {
            return Err(BrowserPlatformError::new(
                BrowserErrorCode::OperationNotAllowed,
                "The server-derived browser capability scope is empty.",
                false,
                "Request an authenticated browser capability that includes browser operations.",
            ));
        }

        if self
            .revoked_runtime_ids
            .lock()
            .expect("gateway browser revoked-runtime store poisoned")
            .contains(runtime_instance_id)
        {
            return Err(revoked_runtime_error());
        }

        let existing = self
            .identities
            .lock()
            .expect("gateway browser identity cache poisoned")
            .get(runtime_instance_id)
            .cloned();

        let identity = if let Some(existing) = existing {
            if existing.revocation_pending {
                return Err(revoked_runtime_error());
            }
            validate_identity_binding(caller, &existing.identity)?;

            // A logical attachment may be re-presented by more than one
            // request while its Hub owner is renewed or replaced. Keep the
            // cached, server-established surface and make operation scope
            // monotonic: a later request may narrow it, but it may never
            // broaden the replacement owner.
            let effective_allowed_operations = existing
                .identity
                .allowed_operations
                .intersection(&allowed_operations)
                .copied()
                .collect::<BTreeSet<_>>();
            if effective_allowed_operations.is_empty() {
                return Err(BrowserPlatformError::new(
                    BrowserErrorCode::OperationNotAllowed,
                    "The renewed browser capability has no operations in common with the established owner policy.",
                    false,
                    "Request a fresh browser runtime for a different capability scope.",
                ));
            }

            // A prior failed replacement attempt keeps exactly one owner
            // cleanup marker on the cached generation. Settle it before even
            // considering a new lease; failure is a hard admission fence.
            if existing.pending_owner_cleanup.is_some() {
                self.retry_owner_cleanup(
                    runtime_instance_id,
                    &existing.identity.owner_lease_id,
                    existing.pending_owner_cleanup.clone(),
                    &hub,
                )
                .await?;
            }

            let lease = match hub.renew_owner_lease(
                &existing.identity.owner_lease_id,
            ) {
                Ok(lease) => lease,
                Err(error)
                    if error.code == BrowserErrorCode::OwnerLeaseExpired =>
                {
                    // Revoke/clean the exact expired generation before minting
                    // its successor. The runtime lifecycle gate is held across
                    // both operations, so concurrent attach calls cannot race
                    // a second replacement into existence.
                    let old_owner_lease_id =
                        existing.identity.owner_lease_id.clone();
                    if let Err(error) =
                        hub.revoke_owner_lease(&old_owner_lease_id).await
                    {
                        let mut identities = self
                            .identities
                            .lock()
                            .expect("gateway browser identity cache poisoned");
                        if let Some(cached) = identities.get_mut(runtime_instance_id)
                            && !cached.revocation_pending
                            && cached.identity.owner_lease_id == old_owner_lease_id
                        {
                            // Replace, rather than append: one runtime owns at
                            // most one blocked generation regardless of TTL
                            // cycles or retry count.
                            cached.pending_owner_cleanup =
                                Some(old_owner_lease_id);
                        }
                        return Err(error);
                    }
                    hub.issue_owner_lease(
                        existing.identity.user_id.clone(),
                        existing.identity.conversation_id.clone(),
                        existing.identity.runtime_instance_id.clone(),
                    )?
                }
                Err(error) => return Err(error),
            };

            let mut identity = existing.identity;
            identity.owner_lease_id = lease.lease_id;
            identity.capability_expires_at_ms =
                capability_expires_at_ms.min(lease.expires_at_ms);
            identity.attempt_id = attempt_id.map(str::to_owned);
            identity.allowed_operations = effective_allowed_operations;
            self.identities
                .lock()
                .expect("gateway browser identity cache poisoned")
                .insert(
                    runtime_instance_id.to_owned(),
                    CachedBrowserIdentity {
                        identity: identity.clone(),
                        revocation_pending: false,
                        pending_owner_cleanup: None,
                    },
                );
            identity
        } else {
            let conversation_id = caller
                .conversation_id
                .as_ref()
                .map(|id| id.as_str().to_owned());
            let owner = hub.issue_owner_lease(
                caller.user_id.as_str(),
                conversation_id.clone(),
                runtime_instance_id,
            )?;
            let identity = CallerIdentity {
                user_id: caller.user_id.as_str().to_owned(),
                conversation_id,
                runtime_instance_id: runtime_instance_id.to_owned(),
                agent_id: None,
                companion_id: caller
                    .companion_id
                    .as_ref()
                    .map(|id| id.as_str().to_owned()),
                execution_id: None,
                step_id: None,
                attempt_id: attempt_id.map(str::to_owned),
                remote_connection_id: None,
                surface: BrowserSurface::Gateway,
                owner_lease_id: owner.lease_id,
                capability_expires_at_ms: capability_expires_at_ms
                    .min(owner.expires_at_ms),
                allowed_operations,
            };
            self.identities
                .lock()
                .expect("gateway browser identity cache poisoned")
                .insert(
                    runtime_instance_id.to_owned(),
                    CachedBrowserIdentity {
                        identity: identity.clone(),
                        revocation_pending: false,
                        pending_owner_cleanup: None,
                    },
                );
            identity
        };

        caller.browser_identity = Some(identity);
        Ok(())
    }

    async fn revoke_identity(
        &self,
        runtime_instance_id: &str,
    ) -> Result<CloseResult, BrowserPlatformError> {
        let lifecycle = self.runtime_lifecycle_slot(runtime_instance_id);
        let _lifecycle_guard = lifecycle.gate.lock().await;
        self.revoke_identity_locked(runtime_instance_id).await
    }

    /// Revoke one cached identity while the attachment lifecycle gate is held.
    ///
    /// The current owner lease and its optional exact cleanup marker are
    /// cleaned independently. A failed cleanup keeps the cached authority
    /// marked pending so the next sweep can retry it rather than silently
    /// losing the old lease.
    async fn revoke_identity_locked(
        &self,
        runtime_instance_id: &str,
    ) -> Result<CloseResult, BrowserPlatformError> {
        let (owner_lease_id, effective_runtime_id, pending_owner_cleanup) = {
            let mut identities = self
                .identities
                .lock()
                .expect("gateway browser identity cache poisoned");
            let Some(cached) = identities.get_mut(runtime_instance_id) else {
                self.revoked_runtime_ids
                    .lock()
                    .expect("gateway browser revoked-runtime store poisoned")
                    .insert(runtime_instance_id);
                return Ok(CloseResult {
                    closed: 0,
                    already_closed: true,
                    ..Default::default()
                });
            };
            cached.revocation_pending = true;
            (
                cached.identity.owner_lease_id.clone(),
                cached.identity.runtime_instance_id.clone(),
                cached.pending_owner_cleanup.clone(),
            )
        };

        self.revoked_runtime_ids
            .lock()
            .expect("gateway browser revoked-runtime store poisoned")
            .insert(&effective_runtime_id);

        let hub = self.hub.clone().ok_or_else(|| {
            BrowserPlatformError::new(
                BrowserErrorCode::BrowserUnavailable,
                "The browser hub was not injected into the gateway.",
                true,
                "Start browser support in the main application and retry.",
            )
        })?;
        let mut closed = 0usize;
        let mut already_closed = true;
        let mut first_error = None;
        let mut remaining_pending = None;

        // Always attempt the current lease first, then its one exact marker.
        // A later retry repeats successful revocations idempotently, which is
        // important when one of the independent lane cleanups fails.
        let mut lease_ids = Vec::with_capacity(2);
        lease_ids.push((true, owner_lease_id.clone()));
        lease_ids.extend(
            pending_owner_cleanup
                .into_iter()
                .filter(|lease_id| lease_id != &owner_lease_id)
                .map(|lease_id| (false, lease_id)),
        );
        for (is_current, lease_id) in lease_ids {
            match hub.revoke_owner_lease(&lease_id).await {
                Ok(result) => {
                    closed = closed.saturating_add(result.closed);
                    already_closed &= result.already_closed;
                }
                Err(error) => {
                    if !is_current {
                        remaining_pending = Some(lease_id);
                    }
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
        }

        if let Some(error) = first_error {
            let mut identities = self
                .identities
                .lock()
                .expect("gateway browser identity cache poisoned");
            if let Some(cached) = identities.get_mut(runtime_instance_id)
                && cached.revocation_pending
                && cached.identity.owner_lease_id == owner_lease_id
            {
                // The current lease remains in the identity field even when
                // its cleanup failed; at most one distinct exact marker can
                // remain beside it. The next retry attempts both idempotently.
                cached.pending_owner_cleanup = remaining_pending;
            }
            return Err(error);
        }

        let mut identities = self
            .identities
            .lock()
            .expect("gateway browser identity cache poisoned");
        if identities
            .get(runtime_instance_id)
            .is_some_and(|cached| {
                cached.revocation_pending
                    && cached.identity.owner_lease_id == owner_lease_id
            })
        {
            identities.remove(runtime_instance_id);
        }
        Ok(CloseResult {
            closed,
            already_closed: already_closed && closed == 0,
            ..Default::default()
        })
    }

    async fn retry_owner_cleanup(
        &self,
        runtime_instance_id: &str,
        current_owner_lease_id: &OwnerLeaseId,
        pending_owner_cleanup: Option<OwnerLeaseId>,
        hub: &BrowserSessionHub,
    ) -> Result<(), BrowserPlatformError> {
        let mut remaining = None;
        let mut cleanup_error = None;
        if let Some(owner_lease_id) = pending_owner_cleanup {
            match hub.revoke_owner_lease(&owner_lease_id).await {
                Ok(_) => {}
                Err(error) => {
                    remaining = Some(owner_lease_id);
                    cleanup_error = Some(error);
                }
            }
        }
        let mut identities = self
            .identities
            .lock()
            .expect("gateway browser identity cache poisoned");
        if let Some(cached) = identities.get_mut(runtime_instance_id)
            && cached.identity.owner_lease_id == *current_owner_lease_id
        {
            cached.pending_owner_cleanup = remaining;
        }
        if let Some(error) = cleanup_error {
            return Err(error);
        }
        Ok(())
    }

    fn runtime_lifecycle_slot(
        &self,
        runtime_instance_id: &str,
    ) -> RuntimeLifecycleSlot {
        let gate = {
            let mut slots = self
                .runtime_lifecycle_slots
                .lock()
                .expect("gateway browser lifecycle slot store poisoned");
            Arc::clone(
                slots
                    .entry(runtime_instance_id.to_owned())
                    .or_insert_with(|| Arc::new(AsyncMutex::new(()))),
            )
        };
        RuntimeLifecycleSlot {
            runtime_instance_id: runtime_instance_id.to_owned(),
            slots: Arc::clone(&self.runtime_lifecycle_slots),
            gate,
        }
    }

    async fn retry_owner_cleanup_serialized(
        &self,
        runtime_instance_id: &str,
        current_owner_lease_id: &OwnerLeaseId,
        pending_owner_cleanup: Option<OwnerLeaseId>,
        hub: &BrowserSessionHub,
    ) -> Result<(), BrowserPlatformError> {
        let lifecycle = self.runtime_lifecycle_slot(runtime_instance_id);
        let _lifecycle_guard = lifecycle.gate.lock().await;
        self.retry_owner_cleanup(
            runtime_instance_id,
            current_owner_lease_id,
            pending_owner_cleanup,
            hub,
        )
        .await
    }

    /// Retry all browser attachment cleanups that previously failed.
    ///
    /// It is called from the Gateway lifecycle sweep because a child process can
    /// disappear without sending an explicit HTTP revoke request.
    pub async fn retry_pending_browser_cleanups(&self) {
        let Some(hub) = self.hub.clone() else {
            return;
        };

        let pending = self
            .identities
            .lock()
            .expect("gateway browser identity cache poisoned")
            .iter()
            .filter_map(|(runtime_id, cached)| {
                if cached.revocation_pending || cached.pending_owner_cleanup.is_some() {
                    Some((
                        runtime_id.clone(),
                        cached.revocation_pending,
                        cached.identity.owner_lease_id.clone(),
                        cached.pending_owner_cleanup.clone(),
                    ))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        stream::iter(pending)
            .for_each_concurrent(
                MAX_CONCURRENT_GATEWAY_OWNER_CLEANUPS,
                |(
                runtime_id,
                revocation_pending,
                current_owner,
                pending_owner_cleanup,
                )| {
                    let hub = hub.clone();
                    async move {
                    if revocation_pending {
                        if let Err(error) = self
                            .revoke_identity(&runtime_id)
                            .await
                        {
                            tracing::warn!(
                                runtime_id = %runtime_id,
                                code = ?error.code,
                                "Gateway browser attachment cleanup retry failed"
                            );
                        }
                    } else if let Err(error) = self
                        .retry_owner_cleanup_serialized(
                            &runtime_id,
                            &current_owner,
                            pending_owner_cleanup,
                            &hub,
                        )
                        .await
                    {
                        tracing::warn!(
                            runtime_id = %runtime_id,
                            code = ?error.code,
                            "Gateway browser superseded-owner cleanup retry failed"
                        );
                    }
                    }
                },
            )
            .await;
    }

    /// Return the exact-owner cleanup still attributed to signed Gateway child
    /// runtimes.
    ///
    /// Every remaining signed-child attachment counts as pending during final
    /// Gateway shutdown, even if its first revoke has not started yet. This
    /// makes the status a postcondition rather than an observation of only the
    /// last failed call.
    pub fn signed_child_cleanup_status(&self) -> BrowserCleanupStatus {
        self.identities
            .lock()
            .expect("gateway browser identity cache poisoned")
            .values()
            .fold(BrowserCleanupStatus::default(), |mut status, cached| {
                status.pending_attachments =
                    status.pending_attachments.saturating_add(1);
                let distinct_marker = usize::from(
                    cached
                        .pending_owner_cleanup
                        .as_ref()
                        .is_some_and(|owner| owner != &cached.identity.owner_lease_id),
                );
                status.pending_owner_leases = status
                    .pending_owner_leases
                    .saturating_add(1 + distinct_marker);
                if cached.revocation_pending {
                    status.revocation_pending_attachments = status
                        .revocation_pending_attachments
                        .saturating_add(1);
                }
                status
            })
    }

    /// Run one authoritative final-drain attempt for every signed Gateway
    /// child and verify the exact-owner postcondition.
    ///
    /// Failed records remain in `identities` with their owner lease ids intact,
    /// so callers may retry this method without reconstructing authority. A
    /// successful return means no signed-child attachment can later publish a
    /// forgotten owner cleanup.
    pub async fn drain_signed_child_browser_owners_once(
        &self,
    ) -> Result<(), BrowserPlatformError> {
        let runtime_ids = self
            .identities
            .lock()
            .expect("gateway browser identity cache poisoned")
            .iter()
            .map(|(runtime_id, _)| runtime_id.clone())
            .collect::<Vec<_>>();

        let mut first_retryable_error = None;
        let mut first_terminal_error = None;
        let mut outcomes = stream::iter(runtime_ids)
            .map(|runtime_id| async move {
                self.revoke_signed_child_lease(&runtime_id).await
            })
            .buffer_unordered(MAX_CONCURRENT_GATEWAY_OWNER_CLEANUPS);
        while let Some(outcome) = outcomes.next().await {
            if let Err(error) = outcome {
                if error.retryable {
                    if first_retryable_error.is_none() {
                        first_retryable_error = Some(error);
                    }
                } else if first_terminal_error.is_none() {
                    first_terminal_error = Some(error);
                }
            }
        }

        let status = self.signed_child_cleanup_status();
        if status.is_empty() {
            return Ok(());
        }

        let mut error = first_terminal_error
            .or(first_retryable_error)
            .unwrap_or_else(|| {
                BrowserPlatformError::new(
                    BrowserErrorCode::BrowserUnavailable,
                    "Gateway browser owner cleanup remains pending.",
                    true,
                    "Retry the authoritative Gateway shutdown barrier.",
                )
            });
        error.metadata = json!({
            "pending_attachments": status.pending_attachments,
            "pending_owner_leases": status.pending_owner_leases,
            "revocation_pending_attachments": status.revocation_pending_attachments,
        });
        Err(error)
    }

    /// Revoke the Hub owner capability associated with one successfully
    /// revoked, signed Gateway child lease.
    ///
    /// Cached identity and Hub-owned Lane state are removed before this
    /// returns. A repeated revoke is a successful no-op and never affects
    /// another child runtime.
    pub async fn revoke_signed_child_lease(
        &self,
        signed_child_lease_id: &str,
    ) -> Result<CloseResult, BrowserPlatformError> {
        self.revoke_identity(signed_child_lease_id).await
    }

    /// Reconcile cached browser owners with the process-local signed
    /// capability registry.
    ///
    /// The final in-process `LoopbackCapabilityLease` guard can revoke an
    /// issuer lease without an HTTP revoke request (for example when a signed
    /// child process crashes or its runtime is dropped). The Gateway server calls
    /// this periodically so those owners and their Lane state do not wait for the
    /// longer Hub owner-lease TTL.
    pub async fn cleanup_inactive_signed_child_leases(
        &self,
        mut is_active: impl FnMut(&str) -> bool,
    ) {
        let inactive = self
            .identities
            .lock()
            .expect("gateway browser identity cache poisoned")
            .iter()
            .filter(|(runtime_id, _)| !is_active(runtime_id))
            .map(|(runtime_id, _)| runtime_id.clone())
            .collect::<Vec<_>>();
        for lease_id in inactive {
            if let Err(error) = self.revoke_signed_child_lease(&lease_id).await {
                tracing::warn!(
                    lease_id,
                    code = ?error.code,
                    "Gateway browser owner cleanup for an inactive signed capability failed"
                );
            }
        }
    }

    /// Open (or recover) a lane. Repeating this call with the same trusted
    /// runtime and lane name returns the same lane; another attempt/runtime gets
    /// a distinct lane even if the companion and conversation are identical.
    pub async fn open(
        &self,
        caller: &CallerCtx,
        lane_name: Option<&str>,
    ) -> Result<BrowserLaneSnapshot, BrowserPlatformError> {
        let resolved = self.resolve(caller, lane_name)?;
        let outcome = resolved
            .client
            .open(
                Some(&resolved.lane_key.lane_name),
                BrowserIdentityMode::Primary,
                None,
            )
            .await?;
        match outcome {
            OpenLaneOutcome::Running { lane } => Ok(lane),
            OpenLaneOutcome::Queued { lane } => {
                let queue = lane
                    .queue
                    .as_ref()
                    .and_then(|queue| serde_json::to_value(queue).ok())
                    .unwrap_or(Value::Null);
                Err(BrowserPlatformError::new(
                    BrowserErrorCode::BrowserCapacityQueued,
                    "The browser lane is queued for capacity.",
                    true,
                    "Retry after the reported queue delay.",
                )
                .for_lane(lane.lane_id)
                .with_metadata(json!({ "queue": queue })))
            }
        }
    }

    /// Execute through the shared hub. Same-lane serialization and cross-lane
    /// concurrency are enforced by the hub, not by a gateway-global mutex.
    pub async fn execute(
        &self,
        caller: &CallerCtx,
        lane_name: Option<&str>,
        input: Value,
    ) -> Result<BrowserOperationResult, BrowserPlatformError> {
        reject_untrusted_caller_fields(&input)?;
        let lane = self.open(caller, lane_name).await?;
        let resolved = self.resolve(caller, Some(&lane.lane_key.lane_name))?;
        let operation = operation_from_input(&input)?;
        resolved.client.execute(&lane.lane_id, operation).await
    }

    /// Validate the semantic browser selectors without attaching or renewing
    /// any browser capability.
    ///
    /// Transport adapters must call this after their typed serde preflight and
    /// before [`Self::attach_trusted_identity_scoped`]. A lane selector is
    /// model-visible input, while the caller identity and owner lease remain
    /// trusted ingress state.
    pub async fn validate_managed_request(
        &self,
        caller: &CallerCtx,
        tool_name: &str,
        input: &Value,
    ) -> Result<(), BrowserPlatformError> {
        reject_untrusted_caller_fields(input)?;

        let Some(object) = input.as_object() else {
            return Err(BrowserPlatformError::new(
                BrowserErrorCode::OperationNotAllowed,
                format!("Browser tool `{tool_name}` arguments must be an object."),
                false,
                "Send the browser tool arguments as a JSON object.",
            ));
        };

        let lane = selector_string(object, "lane")?;
        let lane_name = selector_string(object, "lane_name")?;
        let lane_id = selector_string(object, "lane_id")?;

        let lane = lane
            .as_deref()
            .map(normalize_lane_name)
            .transpose()?;
        let lane_name = lane_name
            .as_deref()
            .map(normalize_lane_name)
            .transpose()?;
        let lane_id = lane_id.map(BrowserLaneId::parse).transpose()?;

        if lane.is_some() && lane_name.is_some() {
            return Err(lane_selector_conflict_error(
                "Use either `lane` or `lane_name`, not both.",
            ));
        }
        if (lane.is_some() || lane_name.is_some()) && lane_id.is_some() {
            return Err(lane_selector_conflict_error(
                "Use either a logical lane name or `lane_id`, not both.",
            ));
        }

        let Some(lane_id) = lane_id else {
            return Ok(());
        };

        // Both transports run this semantic preflight on a per-request caller
        // whose trusted identity has not been attached yet. Defer the owner
        // check instead of failing closed so the owner-scoped handle returned
        // by browser_fork stays usable: the handle never influences which
        // owner gets attached, and an unowned handle still fails at the
        // attached re-validation and at the bound Hub authorization check.
        if caller.browser_identity.is_none() {
            return Ok(());
        }

        let resolved = self.resolve(caller, None)?;
        resolved.client.status(&lane_id).await.map(|_| ())
    }

    /// Dispatch the shared managed Browser contract without constructing a
    /// `BrowserTool` or browser engine. `lane_id` is an owner-scoped selector;
    /// a legacy logical `lane` remains supported by resolving it through this
    /// caller's trusted runtime before dispatch.
    pub async fn dispatch_managed(
        &self,
        caller: &CallerCtx,
        legacy_lane_name: Option<&str>,
        mut input: Value,
    ) -> Result<ToolResult, BrowserPlatformError> {
        reject_untrusted_caller_fields(&input)?;
        if legacy_lane_name.is_some() && input.get("lane_id").is_some_and(|v| !v.is_null()) {
            return Err(BrowserPlatformError::new(
                BrowserErrorCode::OperationNotAllowed,
                "Use either legacy `lane` or `lane_id`, not both.",
                false,
                "Keep lane_id and remove the legacy lane name.",
            ));
        }
        if let Some(lane_name) = legacy_lane_name {
            let lane = self.open(caller, Some(lane_name)).await?;
            input["lane_id"] = Value::String(lane.lane_id.to_string());
        }
        let action = input
            .get("action")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|action| !action.is_empty())
            .ok_or_else(|| {
                BrowserPlatformError::new(
                    BrowserErrorCode::OperationNotAllowed,
                    "The browser action is missing.",
                    false,
                    "Provide a registered Browser action.",
                )
            })?
            .to_owned();
        let resolved = self.resolve(caller, None)?;
        let result = ManagedBrowserFacade::new(resolved.client, None)
            .execute(&action, &input)
            .await;
        Ok(result)
    }

    /// Run multiple calls concurrently while preserving input order. The hub
    /// serializes calls that resolve to one lane and permits different lanes to
    /// overlap subject to its global resource semaphore.
    pub async fn execute_parallel(
        &self,
        calls: Vec<GatewayBrowserCall>,
    ) -> Vec<Result<BrowserOperationResult, BrowserPlatformError>> {
        let futures = calls.into_iter().map(|call| async move {
            self.execute(
                &call.caller,
                Some(&call.lane_name),
                call.input,
            )
            .await
        });
        futures::future::join_all(futures).await
    }

    fn resolve(
        &self,
        caller: &CallerCtx,
        lane_name: Option<&str>,
    ) -> Result<ResolvedBrowserCaller, BrowserPlatformError> {
        let hub = self.hub.as_ref().ok_or_else(|| {
            BrowserPlatformError::new(
                BrowserErrorCode::BrowserUnavailable,
                "The browser hub was not injected into the gateway.",
                true,
                "Start browser support in the main application and retry.",
            )
        })?;
        let identity = self.identity_resolver.resolve(caller)?;
        validate_identity_binding(caller, &identity)?;
        let lane_key = LaneKey::new(identity.runtime_instance_id.clone(), lane_name)?;
        let client = hub.bind(identity)?;
        Ok(ResolvedBrowserCaller { client, lane_key })
    }
}

struct ResolvedBrowserCaller {
    client: BrowserLaneClient,
    lane_key: LaneKey,
}

fn validate_identity_binding(
    caller: &CallerCtx,
    identity: &CallerIdentity,
) -> Result<(), BrowserPlatformError> {
    let conversation_matches = match (
        caller.conversation_id.as_ref(),
        identity.conversation_id.as_deref(),
    ) {
        (Some(expected), Some(actual)) => expected.as_str() == actual,
        (None, _) => true,
        (Some(_), None) => false,
    };
    let companion_matches = match (
        caller.companion_id.as_ref(),
        identity.companion_id.as_deref(),
    ) {
        (Some(expected), Some(actual)) => expected.as_str() == actual,
        (None, _) => true,
        (Some(_), None) => false,
    };
    if identity.user_id != caller.user_id.as_str()
        || !conversation_matches
        || !companion_matches
    {
        return Err(BrowserPlatformError::new(
            BrowserErrorCode::InvalidCallerIdentity,
            "The trusted browser identity does not match the Gateway caller.",
            false,
            "Request a fresh authenticated browser capability.",
        ));
    }
    Ok(())
}

fn missing_identity_error() -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::InvalidCallerIdentity,
        "The Gateway caller has no trusted browser identity.",
        false,
        "Request a fresh authenticated browser capability.",
    )
}

fn revoked_runtime_error() -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::OwnerLeaseExpired,
        "The browser runtime owner has been revoked.",
        false,
        "Request a fresh authenticated browser runtime.",
    )
}

fn selector_string(
    object: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<Option<String>, BrowserPlatformError> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(BrowserPlatformError::new(
            BrowserErrorCode::OperationNotAllowed,
            format!("Browser selector `{field}` must be a string or null."),
            false,
            format!("Remove `{field}` or provide a valid string value."),
        )),
    }
}

fn lane_selector_conflict_error(message: &'static str) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::OperationNotAllowed,
        message,
        false,
        "Keep exactly one browser lane selector.",
    )
}

fn reject_untrusted_caller_fields(
    input: &Value,
) -> Result<(), BrowserPlatformError> {
    let Some(object) = input.as_object() else {
        return Ok(());
    };
    if let Some(field) = TRUSTED_OWNER_INPUT_FIELDS
        .iter()
        .find(|field| object.contains_key(**field))
    {
        return Err(BrowserPlatformError::new(
            BrowserErrorCode::InvalidCallerIdentity,
            format!(
                "Browser caller field `{field}` is main-process managed."
            ),
            false,
            "Remove caller identity fields from browser tool arguments.",
        ));
    }
    if let Some(field) = MODEL_IDENTITY_INPUT_FIELDS
        .iter()
        .find(|field| object.contains_key(**field))
    {
        return Err(BrowserPlatformError::new(
            BrowserErrorCode::InvalidCallerIdentity,
            format!(
                "Browser identity field `{field}` is selected by trusted host policy."
            ),
            false,
            "Remove identity-selection fields from browser tool arguments.",
        ));
    }
    Ok(())
}

fn operation_from_input(
    input: &Value,
) -> Result<BrowserOperation, BrowserPlatformError> {
    reject_untrusted_caller_fields(input)?;
    let action = input
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|action| !action.is_empty())
        .ok_or_else(|| {
            BrowserPlatformError::new(
                BrowserErrorCode::OperationNotAllowed,
                "The browser action is missing.",
                false,
                "Provide a valid browser action.",
            )
        })?
        .to_owned();
    if action == "bring_to_front" {
        return Err(BrowserPlatformError::new(
            BrowserErrorCode::OperationNotAllowed,
            "Foregrounding the managed browser is not an Agent browser operation.",
            false,
            "Use the authenticated Browser management page to open a running Primary lane in the foreground.",
        ));
    }
    let kind = operation_kind(&action);
    let mut sanitized = input.as_object().cloned().unwrap_or_default();
    sanitized.remove("lane_id");
    sanitized.remove("lane");
    sanitized.remove("lane_name");
    sanitized.remove("keep_alive");
    sanitized.remove("keepAlive");
    sanitized.remove("pinned");
    sanitized.remove("persistent_media");
    sanitized.remove("persistentMedia");
    sanitized.remove("expected_browser_epoch");
    for field in TRUSTED_OWNER_INPUT_FIELDS {
        sanitized.remove(*field);
    }
    Ok(BrowserOperation {
        kind,
        action: action.clone(),
        input: Value::Object(sanitized),
        expected_browser_epoch: input
            .get("expected_browser_epoch")
            .or_else(|| input.get("browser_epoch"))
            .and_then(Value::as_u64),
        target_id: input
            .get("target_id")
            .or_else(|| input.get("tab_id"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        frame_id: input
            .get("frame_id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        ref_generation: input
            .get("ref_generation")
            .and_then(Value::as_u64),
        may_modify_identity: may_modify_identity(&action, input),
    })
}

fn operation_kind(action: &str) -> BrowserOperationKind {
    match action {
        "navigate" | "back" | "forward" | "reload" => BrowserOperationKind::Navigate,
        "observe" => BrowserOperationKind::Observe,
        "screenshot" => BrowserOperationKind::Screenshot,
        "tabs" | "switch_tab" | "close_tab" | "open_link_new_tab" => {
            BrowserOperationKind::Tabs
        }
        "download" | "save_as_pdf" => BrowserOperationKind::Download,
        "get_console_logs" | "get_page_errors" | "get_network_log"
        | "rendered_html" => BrowserOperationKind::Debug,
        "capabilities" | "device_pixel_ratio" => {
            BrowserOperationKind::Manage
        }
        "crawl" | "crawl_many" => BrowserOperationKind::Crawl,
        _ => BrowserOperationKind::Act,
    }
}

fn may_modify_identity(action: &str, input: &Value) -> bool {
    match action {
        "navigate" | "back" | "forward" | "reload" | "open_link_new_tab"
        | "crawl" | "crawl_many" => input_declares_stateful_request(input),
        "click"
        | "type"
        | "set_value"
        | "select_option"
        | "press_key"
        | "upload_file"
        | "evaluate"
        | "clear_cookies"
        | "set_cookie"
        | "clear_storage"
        | "login"
        | "logout"
        | "switch_account"
        | "account_switch"
        | "submit"
        | "submit_form" => true,
        "observe"
        | "screenshot"
        | "tabs"
        | "switch_tab"
        | "close_tab"
        | "get_console_logs"
        | "get_page_errors"
        | "get_network_log"
        | "rendered_html"
        | "capabilities"
        | "device_pixel_ratio"
        | "hover"
        | "scroll"
        | "scroll_to_text"
        | "wait"
        | "wait_for"
        | "extract"
        | "switch_frame"
        | "download"
        | "save_as_pdf" => false,
        // Gateway accepts a compatibility action string before the driver
        // validates it. Unknown future actions therefore fail closed.
        _ => true,
    }
}

fn input_declares_stateful_request(input: &Value) -> bool {
    input
        .get("method")
        .and_then(Value::as_str)
        .is_some_and(|method| {
            !method.eq_ignore_ascii_case("get") && !method.eq_ignore_ascii_case("head")
        })
        || input
            .get("submits_form")
            .and_then(Value::as_bool)
            .unwrap_or(false)
}

/// Render a platform operation result into the Gateway's established envelope.
pub fn browser_result_to_value(
    result: Result<BrowserOperationResult, BrowserPlatformError>,
) -> Value {
    match result {
        Ok(result) => {
            let mut payload = if result.output.is_string() {
                json!({ "text": result.output })
            } else if result.output.is_null() {
                json!({})
            } else {
                result.output
            };
            if let Some(object) = payload.as_object_mut() {
                if !object.contains_key("text") {
                    let text = object
                        .get("yaml")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .or_else(|| {
                            object
                                .get("message")
                                .and_then(Value::as_str)
                                .map(str::to_owned)
                        })
                        .or_else(|| {
                            object
                                .get("final_url")
                                .and_then(Value::as_str)
                                .map(|url| format!("Navigated to {url}"))
                        })
                        .or_else(|| {
                            object
                                .get("media_type")
                                .and_then(Value::as_str)
                                .map(|_| "Screenshot captured.".to_owned())
                        });
                    if let Some(text) = text {
                        object.insert("text".to_owned(), Value::String(text));
                    }
                }
                if let (Some(media_type), Some(data)) = (
                    object
                        .get("media_type")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    object
                        .get("data")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                ) {
                    object.insert(
                        "images".to_owned(),
                        json!([{ "media_type": media_type, "data": data }]),
                    );
                }
            }
            json!({ "result": payload })
        }
        Err(error) => platform_error_to_value(error),
    }
}

/// Stable error envelope used by capability handlers.
pub fn platform_error_to_value(error: BrowserPlatformError) -> Value {
    let code = serde_json::to_value(error.code)
        .unwrap_or_else(|_| Value::String("browser_unavailable".to_owned()));
    json!({
        "error": error.message,
        "code": code,
        "retryable": error.retryable,
        "next_action": error.next_action,
        "lane_id": error.lane_id,
        "metadata": error.metadata,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::atomic::{
        AtomicBool, AtomicUsize, Ordering,
    };
    use std::time::Duration;

    use async_trait::async_trait;
    use nomifun_browser_platform::{
        BrowserHostDriver, BrowserHostFactory, BrowserHostId, BrowserProfileFootprint,
        BrowserLaneDriver, BrowserLaneId, DriverOperationContext, HostLaunchRequest,
        HostLifecycleState, HubConfig, LaneLaunchRequest,
    };
    use tokio::sync::{Notify, Semaphore};

    use super::*;

    struct Probe {
        active: AtomicUsize,
        maximum: AtomicUsize,
        entered: AtomicUsize,
        lane_closes: AtomicUsize,
        lane_close_failures_remaining: AtomicUsize,
        block_lane_close: AtomicBool,
        lane_close_notify: Notify,
        lane_close_releases: Semaphore,
        notify: Notify,
        releases: Semaphore,
    }

    impl Probe {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                active: AtomicUsize::new(0),
                maximum: AtomicUsize::new(0),
                entered: AtomicUsize::new(0),
                lane_closes: AtomicUsize::new(0),
                lane_close_failures_remaining: AtomicUsize::new(0),
                block_lane_close: AtomicBool::new(false),
                lane_close_notify: Notify::new(),
                lane_close_releases: Semaphore::new(0),
                notify: Notify::new(),
                releases: Semaphore::new(0),
            })
        }

        async fn wait_for_active(&self, expected: usize) {
            loop {
                if self.active.load(Ordering::Acquire) >= expected {
                    return;
                }
                self.notify.notified().await;
            }
        }

        async fn wait_for_lane_closes(&self, expected: usize) {
            loop {
                if self.lane_closes.load(Ordering::Acquire) >= expected {
                    return;
                }
                self.lane_close_notify.notified().await;
            }
        }
    }

    struct FakeLane {
        lane_id: BrowserLaneId,
        probe: Arc<Probe>,
    }

    #[async_trait]
    impl BrowserLaneDriver for FakeLane {
        async fn execute(
            &self,
            operation: BrowserOperation,
            _context: DriverOperationContext,
        ) -> Result<BrowserOperationResult, BrowserPlatformError> {
            let active = self.probe.active.fetch_add(1, Ordering::AcqRel) + 1;
            self.probe.maximum.fetch_max(active, Ordering::AcqRel);
            self.probe.entered.fetch_add(1, Ordering::AcqRel);
            self.probe.notify.notify_waiters();
            if operation
                .input
                .get("block")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                self.probe
                    .releases
                    .acquire()
                    .await
                    .expect("test release semaphore closed")
                    .forget();
            }
            self.probe.active.fetch_sub(1, Ordering::AcqRel);
            let text = if operation.action == "observe" {
                format!(
                    "{}:{}\n- button \"Pay now\" [ref=f0e7]\n- link \"Docs\" [ref=f0a1]",
                    self.lane_id, operation.action
                )
            } else {
                format!("{}:{}", self.lane_id, operation.action)
            };
            Ok(BrowserOperationResult {
                output: json!({ "text": text }),
                ..Default::default()
            })
        }

        async fn close(&self) -> Result<(), BrowserPlatformError> {
            self.probe.lane_closes.fetch_add(1, Ordering::AcqRel);
            self.probe.lane_close_notify.notify_waiters();
            if self.probe.block_lane_close.load(Ordering::Acquire) {
                self.probe
                    .lane_close_releases
                    .acquire()
                    .await
                    .expect("test lane-close release semaphore closed")
                    .forget();
            }
            if self
                .probe
                .lane_close_failures_remaining
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                    if remaining == 0 {
                        None
                    } else {
                        Some(remaining.saturating_sub(1))
                    }
                })
                .is_ok()
            {
                return Err(BrowserPlatformError::new(
                    BrowserErrorCode::BrowserUnavailable,
                    "Synthetic lane cleanup failure.",
                    true,
                    "Retry the authoritative cleanup.",
                ));
            }
            Ok(())
        }
    }

    struct FakeHost {
        id: BrowserHostId,
        probe: Arc<Probe>,
    }

    #[async_trait]
    impl BrowserHostDriver for FakeHost {
        fn host_id(&self) -> BrowserHostId {
            self.id.clone()
        }

        fn epoch(&self) -> u64 {
            1
        }

        // This fake manages no on-disk profile, so report a completed
        // zero measurement. Inheriting the trait default would instead
        // mean "could not measure", which fences Primary fail-closed.
        async fn profile_footprint(
            &self,
            _stop_after_bytes: u64,
            _stop_after_entries: u64,
        ) -> Result<Option<BrowserProfileFootprint>, BrowserPlatformError> {
            Ok(Some(BrowserProfileFootprint::EMPTY))
        }

        fn state(&self) -> HostLifecycleState {
            HostLifecycleState::Running
        }

        async fn open_lane(
            &self,
            request: LaneLaunchRequest,
        ) -> Result<Arc<dyn BrowserLaneDriver>, BrowserPlatformError> {
            Ok(Arc::new(FakeLane {
                lane_id: request.lane_id,
                probe: Arc::clone(&self.probe),
            }))
        }

        async fn shutdown(&self) -> Result<(), BrowserPlatformError> {
            Ok(())
        }
    }

    struct FakeFactory {
        launches: AtomicUsize,
        probe: Arc<Probe>,
    }

    #[async_trait]
    impl BrowserHostFactory for FakeFactory {
        async fn launch(
            &self,
            request: HostLaunchRequest,
        ) -> Result<Arc<dyn BrowserHostDriver>, BrowserPlatformError> {
            self.launches.fetch_add(1, Ordering::AcqRel);
            Ok(Arc::new(FakeHost {
                id: request.host_id,
                probe: Arc::clone(&self.probe),
            }))
        }
    }

    struct Harness {
        hub: BrowserSessionHub,
        registry: BrowserRegistry,
        factory: Arc<FakeFactory>,
        probe: Arc<Probe>,
    }

    fn harness() -> Harness {
        harness_with_owner_ttl(HubConfig::default().owner_lease_ttl_ms)
    }

    fn harness_with_owner_ttl(owner_lease_ttl_ms: u64) -> Harness {
        let mut config = HubConfig::default();
        config.owner_lease_ttl_ms = owner_lease_ttl_ms;
        harness_with_config(config)
    }

    fn harness_with_config(config: HubConfig) -> Harness {
        let probe = Probe::new();
        let factory = Arc::new(FakeFactory {
            launches: AtomicUsize::new(0),
            probe: Arc::clone(&probe),
        });
        let hub = BrowserSessionHub::new(factory.clone(), config);
        let registry = BrowserRegistry::from_hub(hub.clone());
        Harness {
            hub,
            registry,
            factory,
            probe,
        }
    }

    fn caller(
        hub: &BrowserSessionHub,
        runtime: &str,
        attempt: &str,
    ) -> CallerCtx {
        caller_for_conversation(
            hub,
            runtime,
            attempt,
            "0190f5fe-7c00-7a00-8abc-012345678901",
        )
    }

    fn caller_for_conversation(
        hub: &BrowserSessionHub,
        runtime: &str,
        attempt: &str,
        conversation: &str,
    ) -> CallerCtx {
        let user_id =
            nomifun_common::UserId::parse("0190f5fe-7c00-7a00-8000-000000000001")
                .unwrap();
        let conversation_id =
            nomifun_common::ConversationId::parse(conversation).unwrap();
        let companion_id = nomifun_common::CompanionId::parse(
            "0190f5fe-7c00-7a00-8abc-012345678902",
        )
        .unwrap();
        let lease = hub
            .issue_owner_lease(
                user_id.as_str(),
                Some(conversation_id.as_str().to_owned()),
                runtime,
            )
            .unwrap();
        CallerCtx {
            conversation_id: Some(conversation_id.clone()),
            user_id: user_id.clone(),
            companion_id: Some(companion_id.clone()),
            browser_identity: Some(CallerIdentity {
                user_id: user_id.as_str().to_owned(),
                conversation_id: Some(
                    conversation_id.as_str().to_owned(),
                ),
                runtime_instance_id: runtime.to_owned(),
                agent_id: Some("agent-1".to_owned()),
                companion_id: Some(companion_id.as_str().to_owned()),
                execution_id: Some("execution-1".to_owned()),
                step_id: Some("step-1".to_owned()),
                attempt_id: Some(attempt.to_owned()),
                remote_connection_id: None,
                surface: nomifun_browser_platform::BrowserSurface::Gateway,
                owner_lease_id: lease.lease_id,
                capability_expires_at_ms: u64::MAX,
                allowed_operations: BTreeSet::from([
                    BrowserOperationKind::Manage,
                    BrowserOperationKind::Navigate,
                    BrowserOperationKind::Observe,
                    BrowserOperationKind::Act,
                ]),
            }),
            ..Default::default()
        }
    }

    fn gateway_caller_without_browser_identity() -> CallerCtx {
        CallerCtx {
            conversation_id: Some(
                nomifun_common::ConversationId::parse(
                    "0190f5fe-7c00-7a00-8abc-012345678901",
                )
                .unwrap(),
            ),
            user_id: nomifun_common::UserId::parse(
                "0190f5fe-7c00-7a00-8000-000000000001",
            )
            .unwrap(),
            companion_id: Some(
                nomifun_common::CompanionId::parse(
                    "0190f5fe-7c00-7a00-8abc-012345678902",
                )
                .unwrap(),
            ),
            ..Default::default()
        }
    }

    /// Spread a cleanup-concurrency fixture across real task families while
    /// retaining several sibling runtimes in every family.  These tests target
    /// the Gateway's global cleanup worker window, not the independent
    /// per-task Lane ceiling, so one shared conversation would correctly stop
    /// admission before the cleanup window could be exercised.
    fn assign_cleanup_stress_task_family(
        caller: &mut CallerCtx,
        runtime_index: usize,
        family_width: usize,
    ) {
        let family_index = runtime_index / family_width.max(1);
        caller.conversation_id = Some(
            nomifun_common::ConversationId::parse(&format!(
                "0190f5fe-7c00-7a00-8abc-{family_index:012x}"
            ))
            .expect("synthetic cleanup-stress conversation id"),
        );
    }

    #[tokio::test]
    async fn different_attempt_runtimes_get_distinct_lanes_even_for_same_companion() {
        let harness = harness();
        let first = caller(&harness.hub, "runtime-attempt-1", "attempt-1");
        let second = caller(&harness.hub, "runtime-attempt-2", "attempt-2");
        let lane_a = harness.registry.open(&first, None).await.unwrap();
        let lane_b = harness.registry.open(&second, None).await.unwrap();
        assert_ne!(lane_a.lane_id, lane_b.lane_id);
        assert_ne!(lane_a.lane_key, lane_b.lane_key);
        assert_eq!(
            lane_a.caller.companion_id,
            lane_b.caller.companion_id,
            "companion is attribution, not the lane key"
        );
    }

    #[tokio::test]
    async fn same_runtime_default_lane_is_stable() {
        let harness = harness();
        let caller = caller(&harness.hub, "runtime-stable", "attempt-1");
        let first = harness.registry.open(&caller, None).await.unwrap();
        let second = harness
            .registry
            .open(&caller, Some("default"))
            .await
            .unwrap();
        assert_eq!(first.lane_id, second.lane_id);
        assert_eq!(first.lane_key.lane_name, "default");
        assert_eq!(harness.factory.launches.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn managed_contract_accepts_owned_lane_id_and_rejects_sibling_handle() {
        let harness = harness();
        let first = caller(&harness.hub, "runtime-contract-a", "attempt-a");
        let sibling = caller(&harness.hub, "runtime-contract-b", "attempt-b");
        let opened = harness
            .registry
            .dispatch_managed(
                &first,
                None,
                json!({
                    "action": "browser_open",
                    "lane_name": "research",
                }),
            )
            .await
            .unwrap();
        assert!(!opened.is_error, "{}", opened.content);
        let opened: Value = serde_json::from_str(&opened.content).unwrap();
        let lane_id = opened
            .pointer("/lane/lane_id")
            .and_then(Value::as_str)
            .unwrap()
            .to_owned();

        let navigated = harness
            .registry
            .dispatch_managed(
                &first,
                None,
                json!({
                    "action": "navigate",
                    "url": "https://example.test/research",
                    "lane_id": lane_id,
                }),
            )
            .await
            .unwrap();
        assert!(!navigated.is_error, "{}", navigated.content);
        let navigated: Value = serde_json::from_str(&navigated.content).unwrap();
        assert_eq!(
            navigated.get("lane_id").and_then(Value::as_str),
            Some(lane_id.as_str())
        );
        assert!(
            navigated.get("text").and_then(Value::as_str).is_some(),
            "legacy action output remains available at the established top level"
        );

        let crossed = harness
            .registry
            .dispatch_managed(
                &sibling,
                None,
                json!({"action": "browser_status", "lane_id": lane_id}),
            )
            .await
            .unwrap();
        assert!(crossed.is_error, "an unowned Lane handle must fail closed");
        assert_eq!(harness.hub.list_lanes().await.len(), 1);
    }

    #[tokio::test]
    async fn managed_crawl_many_preserves_url_order_and_cleans_hub_lanes() {
        let harness = harness();
        let caller = caller(&harness.hub, "runtime-crawl", "attempt-crawl");
        let crawled = harness
            .registry
            .dispatch_managed(
                &caller,
                None,
                json!({
                    "action": "browser_crawl_many",
                    "urls": [
                        "https://example.test/a",
                        "https://example.test/b",
                        "https://example.test/c"
                    ],
                    "concurrency": 2,
                }),
            )
            .await
            .unwrap();
        assert!(!crawled.is_error, "{}", crawled.content);
        let crawled: Value = serde_json::from_str(&crawled.content).unwrap();
        let results = crawled["results"].as_array().unwrap();
        assert_eq!(results.len(), 3);
        for (index, suffix) in ["a", "b", "c"].into_iter().enumerate() {
            assert_eq!(
                results[index]["url"],
                format!("https://example.test/{suffix}")
            );
            for field in [
                "lane_id",
                "lifecycle_state",
                "identity_mode",
                "browser_epoch",
                "recommended_concurrency",
                "capacity_or_recovery_hint",
            ] {
                assert!(
                    results[index].get(field).is_some(),
                    "crawl result {index} is missing {field}: {}",
                    results[index]
                );
            }
        }
        assert!(
            harness.hub.list_lanes().await.is_empty(),
            "crawl worker Lanes must be closed through the Hub"
        );
    }

    #[tokio::test]
    async fn signed_runtime_attachment_reuses_one_lease_and_separates_new_attempts() {
        let harness = harness();
        let mut first = gateway_caller_without_browser_identity();
        harness
            .registry
            .attach_trusted_identity(
                &mut first,
                "signed-child-lease-a",
                Some("attempt-a"),
                u64::MAX,
            )
            .await
            .unwrap();
        let first_identity = first.browser_identity.clone().unwrap();

        let mut renewed = gateway_caller_without_browser_identity();
        harness
            .registry
            .attach_trusted_identity(
                &mut renewed,
                "signed-child-lease-a",
                Some("attempt-a"),
                u64::MAX,
            )
            .await
            .unwrap();
        let renewed_identity = renewed.browser_identity.unwrap();
        assert_eq!(
            first_identity.owner_lease_id,
            renewed_identity.owner_lease_id
        );
        assert_eq!(
            first_identity.runtime_instance_id,
            renewed_identity.runtime_instance_id
        );

        let mut next_attempt = gateway_caller_without_browser_identity();
        harness
            .registry
            .attach_trusted_identity(
                &mut next_attempt,
                "signed-child-lease-b",
                Some("attempt-b"),
                u64::MAX,
            )
            .await
            .unwrap();
        let next_identity = next_attempt.browser_identity.unwrap();
        assert_ne!(
            first_identity.owner_lease_id,
            next_identity.owner_lease_id
        );
        assert_ne!(
            first_identity.runtime_instance_id,
            next_identity.runtime_instance_id
        );
    }

    #[tokio::test]
    async fn final_signed_child_drain_reports_pending_exact_owner_cleanup() {
        // A terminal lane-cleanup failure on a host with no surviving lanes is
        // resolved by authoritative host retirement, so the drain
        // postcondition is already met on the first attempt.
        let harness = harness();
        let mut signed = gateway_caller_without_browser_identity();
        harness
            .registry
            .attach_trusted_identity(
                &mut signed,
                "signed-child-final-retired",
                Some("attempt-final-retired"),
                u64::MAX,
            )
            .await
            .unwrap();
        harness.registry.open(&signed, None).await.unwrap();
        harness
            .probe
            .lane_close_failures_remaining
            .store(1, Ordering::Release);
        harness
            .registry
            .drain_signed_child_browser_owners_once()
            .await
            .expect("host retirement resolves the terminal lane-cleanup failure");
        assert!(harness.registry.signed_child_cleanup_status().is_empty());
        assert!(harness.hub.list_lanes().await.is_empty());
        // Retirement resolved the failure through the Host process shutdown;
        // no second lane close was needed.
        assert_eq!(harness.probe.lane_closes.load(Ordering::Acquire), 1);

        // A sibling lane on the shared Primary host makes retirement
        // impossible, so the retained exact owner stays pending until retry.
        let harness = self::harness();
        // Keep the sibling outside the Registry identity cache. The final drain
        // intentionally detaches every cached owner before host retirement, so
        // a sibling cached there would disappear from the Hub's live-lane view
        // and would not exercise the shared-host postcondition.
        let sibling = caller(
            &harness.hub,
            "signed-child-final-sibling",
            "attempt-final-sibling",
        );
        let sibling_identity = sibling
            .browser_identity
            .clone()
            .expect("sibling fixture has a trusted browser identity");
        let sibling_client = harness
            .hub
            .bind(sibling_identity.clone())
            .expect("bind sibling browser owner");
        let sibling_lane = match sibling_client
            .open(None, BrowserIdentityMode::Primary, None)
            .await
            .expect("open sibling lane")
        {
            OpenLaneOutcome::Running { lane } => lane,
            OpenLaneOutcome::Queued { .. } => {
                panic!("sibling fixture must not be queued")
            }
        };
        let mut signed = gateway_caller_without_browser_identity();
        harness
            .registry
            .attach_trusted_identity(
                &mut signed,
                "signed-child-final-pending",
                Some("attempt-final-pending"),
                u64::MAX,
            )
            .await
            .unwrap();
        harness.registry.open(&signed, None).await.unwrap();
        harness
            .probe
            .lane_close_failures_remaining
            .store(1, Ordering::Release);

        let error = harness
            .registry
            .drain_signed_child_browser_owners_once()
            .await
            .expect_err("a retained exact owner may not be reported as drained");
        assert_eq!(error.code, BrowserErrorCode::BrowserUnavailable);
        assert_eq!(
            error
                .metadata
                .get("pending_attachments")
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            error
                .metadata
                .get("pending_owner_leases")
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            harness.registry.signed_child_cleanup_status(),
            BrowserCleanupStatus {
                pending_attachments: 1,
                pending_owner_leases: 1,
                revocation_pending_attachments: 1,
            }
        );

        harness
            .registry
            .drain_signed_child_browser_owners_once()
            .await
            .expect("retry must consume the retained exact-owner authority");
        assert!(harness.registry.signed_child_cleanup_status().is_empty());
        let lanes = harness.hub.list_lanes().await;
        assert_eq!(lanes.len(), 1);
        assert_eq!(lanes[0].lane_id, sibling_lane.lane_id);
        harness
            .hub
            .close_owner_lease(&sibling_identity.owner_lease_id)
            .await
            .expect("close the out-of-band sibling fixture");
    }

    #[tokio::test]
    async fn final_signed_child_drain_polls_only_a_fixed_cleanup_window() {
        let total = MAX_CONCURRENT_GATEWAY_OWNER_CLEANUPS + 4;
        let mut config = HubConfig::default();
        config.resource_policy.max_open_lanes = total;
        let family_width = config.resource_policy.max_task_open_lanes.max(1);
        let harness = harness_with_config(config);
        let mut task_family_counts = HashMap::<String, usize>::new();

        for index in 0..total {
            let mut signed = gateway_caller_without_browser_identity();
            assign_cleanup_stress_task_family(&mut signed, index, family_width);
            harness
                .registry
                .attach_trusted_identity(
                    &mut signed,
                    &format!("signed-child-bounded-drain-{index}"),
                    Some(&format!("attempt-{index}")),
                    u64::MAX,
                )
                .await
                .unwrap();
            let family_key = signed
                .browser_identity
                .as_ref()
                .expect("attached signed-child browser identity")
                .task_resource_family_key()
                .into_string();
            *task_family_counts.entry(family_key).or_default() += 1;
            harness.registry.open(&signed, None).await.unwrap();
        }
        assert_eq!(
            task_family_counts.len(),
            (total + family_width - 1) / family_width,
            "the global cleanup fixture must span multiple task families"
        );
        assert!(
            task_family_counts
                .values()
                .all(|count| *count <= family_width),
            "no synthetic task family may bypass its Lane ceiling"
        );
        assert!(
            task_family_counts.values().any(|count| *count > 1),
            "the fixture must retain sibling runtimes sharing one task family"
        );

        harness
            .probe
            .block_lane_close
            .store(true, Ordering::Release);
        let registry = harness.registry.clone();
        let draining = tokio::spawn(async move {
            registry.drain_signed_child_browser_owners_once().await
        });

        tokio::time::timeout(
            Duration::from_secs(3),
            harness
                .probe
                .wait_for_lane_closes(MAX_CONCURRENT_GATEWAY_OWNER_CLEANUPS),
        )
        .await
        .expect("the fixed final-drain window did not start");
        for _ in 0..32 {
            tokio::task::yield_now().await;
        }
        assert_eq!(
            harness.probe.lane_closes.load(Ordering::Acquire),
            MAX_CONCURRENT_GATEWAY_OWNER_CLEANUPS,
            "the drain must not poll cleanup for every runtime at once"
        );

        harness.probe.lane_close_releases.add_permits(total);
        tokio::time::timeout(Duration::from_secs(5), draining)
            .await
            .expect("bounded final drain timed out")
            .expect("bounded final drain task panicked")
            .expect("bounded final drain failed");
        harness
            .probe
            .block_lane_close
            .store(false, Ordering::Release);
        assert_eq!(harness.probe.lane_closes.load(Ordering::Acquire), total);
        assert!(harness.registry.signed_child_cleanup_status().is_empty());
    }

    #[tokio::test]
    async fn signed_child_revoke_is_exact_and_idempotent() {
        let harness = harness();
        let mut signed = gateway_caller_without_browser_identity();
        harness
            .registry
            .attach_trusted_identity(
                &mut signed,
                "signed-child-close",
                None,
                u64::MAX,
            )
            .await
            .unwrap();
        harness.registry.open(&signed, None).await.unwrap();

        let first = harness
            .registry
            .revoke_signed_child_lease("signed-child-close")
            .await
            .unwrap();
        assert_eq!(first.closed, 1);
        assert!(!first.already_closed);
        assert!(harness.hub.list_lanes().await.is_empty());

        let repeated = harness
            .registry
            .revoke_signed_child_lease("signed-child-close")
            .await
            .unwrap();
        assert_eq!(repeated.closed, 0);
        assert!(repeated.already_closed);
    }

    #[tokio::test]
    async fn failed_signed_child_revoke_remains_authoritative_until_retry() {
        // A terminal lane-cleanup failure on a host with no surviving lanes is
        // resolved by authoritative host retirement: the revoke succeeds on
        // its first attempt and no retained authority survives.
        let harness = harness();
        let mut signed = gateway_caller_without_browser_identity();
        harness
            .registry
            .attach_trusted_identity(
                &mut signed,
                "signed-child-retired",
                None,
                u64::MAX,
            )
            .await
            .unwrap();
        harness.registry.open(&signed, None).await.unwrap();
        harness
            .probe
            .lane_close_failures_remaining
            .store(1, Ordering::Release);
        let result = harness
            .registry
            .revoke_signed_child_lease("signed-child-retired")
            .await
            .expect("host retirement resolves the terminal lane-cleanup failure");
        assert_eq!(result.closed, 1);
        assert!(harness.hub.list_lanes().await.is_empty());
        assert!(
            !harness
                .registry
                .identities
                .lock()
                .expect("gateway browser identity cache poisoned")
                .contains_key("remote-session-retired")
        );
        assert_eq!(harness.probe.lane_closes.load(Ordering::Acquire), 1);

        // A sibling lane on the shared Primary host makes retirement
        // impossible, so the failed cleanup retains its authority until retry.
        let harness = self::harness();
        let mut signed = gateway_caller_without_browser_identity();
        harness
            .registry
            .attach_trusted_identity(
                &mut signed,
                "signed-child-retry",
                None,
                u64::MAX,
            )
            .await
            .unwrap();
        harness.registry.open(&signed, None).await.unwrap();
        let mut sibling = gateway_caller_without_browser_identity();
        harness
            .registry
            .attach_trusted_identity(
                &mut sibling,
                "signed-child-retry-sibling",
                None,
                u64::MAX,
            )
            .await
            .unwrap();
        let sibling_lane = harness.registry.open(&sibling, None).await.unwrap();
        harness
            .probe
            .lane_close_failures_remaining
            .store(1, Ordering::Release);

        let error = harness
            .registry
            .revoke_signed_child_lease("signed-child-retry")
            .await
            .unwrap_err();
        assert_eq!(error.code, BrowserErrorCode::BrowserUnavailable);
        {
            let identities = harness
                .registry
                .identities
                .lock()
                .expect("gateway browser identity cache poisoned");
            let cached = identities
                .get("signed-child-retry")
                .expect("failed cleanup must retain its authority");
            assert!(cached.revocation_pending);
        }

        harness.registry.retry_pending_browser_cleanups().await;
        assert!(
            !harness
                .registry
                .identities
                .lock()
                .expect("gateway browser identity cache poisoned")
                .contains_key("signed-child-retry")
        );
        assert_eq!(harness.probe.lane_closes.load(Ordering::Acquire), 2);

        // The sibling attachment and its lane survive the scoped retry, and no
        // lower-level cleanup remains for the lifecycle sweep.
        harness.hub.sweep().await.unwrap();
        assert_eq!(harness.probe.lane_closes.load(Ordering::Acquire), 2);
        let lanes = harness.hub.list_lanes().await;
        assert_eq!(lanes.len(), 1);
        assert_eq!(lanes[0].lane_id, sibling_lane.lane_id);
    }

    #[tokio::test]
    async fn pending_cleanup_retry_polls_only_a_fixed_runtime_window() {
        let total = MAX_CONCURRENT_GATEWAY_OWNER_CLEANUPS + 4;
        let mut config = HubConfig::default();
        config.resource_policy.max_open_lanes = total;
        let family_width = config.resource_policy.max_task_open_lanes.max(1);
        let harness = harness_with_config(config);
        let runtime_ids = (0..total)
            .map(|index| format!("signed-child-bounded-retry-{index}"))
            .collect::<Vec<_>>();
        let mut owner_lease_ids = Vec::with_capacity(total);
        let mut task_family_counts = HashMap::<String, usize>::new();

        for (index, runtime_id) in runtime_ids.iter().enumerate() {
            let mut signed = gateway_caller_without_browser_identity();
            assign_cleanup_stress_task_family(&mut signed, index, family_width);
            harness
                .registry
                .attach_trusted_identity(
                    &mut signed,
                    runtime_id,
                    None,
                    u64::MAX,
                )
                .await
                .unwrap();
            owner_lease_ids.push(
                signed
                    .browser_identity
                    .as_ref()
                    .expect("attached browser identity")
                    .owner_lease_id
                    .clone(),
            );
            let family_key = signed
                .browser_identity
                .as_ref()
                .expect("attached signed-child browser identity")
                .task_resource_family_key()
                .into_string();
            *task_family_counts.entry(family_key).or_default() += 1;
            harness.registry.open(&signed, None).await.unwrap();
        }
        assert_eq!(
            task_family_counts.len(),
            (total + family_width - 1) / family_width,
            "the global cleanup fixture must span multiple task families"
        );
        assert!(
            task_family_counts
                .values()
                .all(|count| *count <= family_width),
            "no synthetic task family may bypass its Lane ceiling"
        );
        assert!(
            task_family_counts.values().any(|count| *count > 1),
            "the fixture must retain sibling runtimes sharing one task family"
        );

        // Model the exact Gateway state at the start of a retry sweep. Every
        // record still owns a live Hub Lane (and therefore an exact cleanup
        // token); publishing the marker directly avoids racing this Gateway
        // concurrency test against the Hub's independent retry supervisor.
        {
            let mut identities = harness
                .registry
                .identities
                .lock()
                .expect("gateway browser identity cache poisoned");
            for runtime_id in &runtime_ids {
                identities
                    .get_mut(runtime_id)
                    .expect("attached runtime identity")
                    .revocation_pending = true;
            }
        }
        let closes_before_retry = harness.probe.lane_closes.load(Ordering::Acquire);
        assert_eq!(closes_before_retry, 0);

        harness
            .probe
            .block_lane_close
            .store(true, Ordering::Release);
        let registry = harness.registry.clone();
        let retrying = tokio::spawn(async move {
            registry.retry_pending_browser_cleanups().await;
        });
        tokio::time::timeout(
            Duration::from_secs(3),
            harness.probe.wait_for_lane_closes(
                closes_before_retry + MAX_CONCURRENT_GATEWAY_OWNER_CLEANUPS,
            ),
        )
        .await
        .expect("the fixed retry window did not start");
        for _ in 0..32 {
            tokio::task::yield_now().await;
        }
        assert_eq!(
            harness
                .registry
                .runtime_lifecycle_slots
                .lock()
                .expect("gateway browser lifecycle slot store poisoned")
                .len(),
            MAX_CONCURRENT_GATEWAY_OWNER_CLEANUPS,
            "the retry sweep must poll only its fixed window; the remaining runtimes must stay lazy"
        );

        harness.probe.lane_close_releases.add_permits(total);
        tokio::time::timeout(Duration::from_secs(5), retrying)
            .await
            .expect("bounded cleanup retry timed out")
            .expect("bounded cleanup retry task panicked");
        harness
            .probe
            .block_lane_close
            .store(false, Ordering::Release);
        assert!(
            harness
                .registry
                .identities
                .lock()
                .expect("gateway browser identity cache poisoned")
                .is_empty(),
            "every Gateway identity and its pending-cleanup marker must converge"
        );
        assert!(
            harness
                .registry
                .runtime_lifecycle_slots
                .lock()
                .expect("gateway browser lifecycle slot store poisoned")
                .is_empty(),
            "completed retries must release every per-runtime lifecycle gate"
        );
        assert!(harness.hub.list_lanes().await.is_empty());
        let overview = harness.hub.overview().await;
        assert_eq!(overview.total_lanes, 0);
        assert_eq!(overview.managed_host_count, 0);
        assert_eq!(overview.pending_cleanup_count, 0);
        assert!(overview.hosts.is_empty());
        for owner_lease_id in owner_lease_ids {
            assert_eq!(
                harness
                    .hub
                    .renew_owner_lease(&owner_lease_id)
                    .expect_err("cleanup must revoke the exact owner authority")
                    .code,
                BrowserErrorCode::OwnerLeaseExpired
            );
        }
    }

    #[tokio::test]
    async fn expired_owner_replacement_cleans_old_generation_before_publish() {
        // A terminal lane-cleanup failure on a host with no surviving lanes is
        // resolved by authoritative host retirement, so the replacement attach
        // consumes the superseded owner immediately.
        let harness = harness_with_owner_ttl(10);
        let mut first = gateway_caller_without_browser_identity();
        harness
            .registry
            .attach_trusted_identity(
                &mut first,
                "signed-child-retired-owner",
                None,
                u64::MAX,
            )
            .await
            .unwrap();
        let old_owner = first
            .browser_identity
            .as_ref()
            .unwrap()
            .owner_lease_id
            .clone();
        harness.registry.open(&first, None).await.unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        harness
            .probe
            .lane_close_failures_remaining
            .store(1, Ordering::Release);
        let mut replacement = gateway_caller_without_browser_identity();
        harness
            .registry
            .attach_trusted_identity(
                &mut replacement,
                "signed-child-retired-owner",
                None,
                u64::MAX,
            )
            .await
            .unwrap();
        {
            let identities = harness
                .registry
                .identities
                .lock()
                .expect("gateway browser identity cache poisoned");
            let cached = identities
                .get("signed-child-retired-owner")
                .expect("replacement authority must be published");
            assert!(
                cached.pending_owner_cleanup.is_none(),
                "host retirement resolves the superseded-owner cleanup failure"
            );
            assert_ne!(cached.identity.owner_lease_id, old_owner);
        }
        assert!(harness.hub.list_lanes().await.is_empty());
        assert_eq!(harness.probe.lane_closes.load(Ordering::Acquire), 1);

        // A sibling Lane on the shared Primary Host makes retirement
        // impossible. The expired generation remains the only published
        // owner and replacement admission fails closed until exact cleanup
        // succeeds.
        let harness = harness_with_owner_ttl(10);
        let mut first = gateway_caller_without_browser_identity();
        harness
            .registry
            .attach_trusted_identity(
                &mut first,
                "signed-child-superseded",
                None,
                u64::MAX,
            )
            .await
            .unwrap();
        let old_owner = first
            .browser_identity
            .as_ref()
            .unwrap()
            .owner_lease_id
            .clone();
        harness.registry.open(&first, None).await.unwrap();

        tokio::time::sleep(Duration::from_millis(20)).await;
        let mut sibling = gateway_caller_without_browser_identity();
        harness
            .registry
            .attach_trusted_identity(
                &mut sibling,
                "signed-child-superseded-sibling",
                None,
                u64::MAX,
            )
            .await
            .unwrap();
        let sibling_lane = harness.registry.open(&sibling, None).await.unwrap();
        harness
            .probe
            .lane_close_failures_remaining
            .store(1, Ordering::Release);

        let mut replacement = gateway_caller_without_browser_identity();
        let error = harness
            .registry
            .attach_trusted_identity(
                &mut replacement,
                "signed-child-superseded",
                None,
                u64::MAX,
            )
            .await
            .expect_err("cleanup failure must fence replacement publication");
        assert_eq!(error.code, BrowserErrorCode::BrowserUnavailable);
        assert!(replacement.browser_identity.is_none());

        {
            let identities = harness
                .registry
                .identities
                .lock()
                .expect("gateway browser identity cache poisoned");
            let cached = identities
                .get("signed-child-superseded")
                .expect("expired owner cleanup authority must remain cached");
            assert_eq!(cached.pending_owner_cleanup, Some(old_owner.clone()));
            assert_eq!(cached.identity.owner_lease_id, old_owner);
        }

        harness
            .probe
            .lane_close_failures_remaining
            .store(0, Ordering::Release);
        harness
            .registry
            .attach_trusted_identity(
                &mut replacement,
                "signed-child-superseded",
                None,
                u64::MAX,
            )
            .await
            .expect("replacement may publish after exact old-owner cleanup");
        let replacement_identity = replacement.browser_identity.clone().unwrap();
        assert_ne!(replacement_identity.owner_lease_id, old_owner);
        assert!(
            harness
                .registry
                .identities
                .lock()
                .expect("gateway browser identity cache poisoned")
                ["signed-child-superseded"]
                .pending_owner_cleanup
                .is_none()
        );
        let replacement_lane = harness.registry.open(&replacement, None).await.unwrap();

        let result = harness
            .registry
            .revoke_signed_child_lease("signed-child-superseded")
            .await
            .unwrap();
        assert_eq!(result.closed, 1);
        assert!(
            !harness
                .registry
                .identities
                .lock()
                .expect("gateway browser identity cache poisoned")
                .contains_key("signed-child-superseded")
        );

        // Old-owner cleanup, replacement cleanup, and the sibling remain
        // exact; no unpublished owner generation was able to own a Lane.
        assert_eq!(harness.probe.lane_closes.load(Ordering::Acquire), 3);
        let lanes = harness.hub.list_lanes().await;
        assert_eq!(lanes.len(), 1);
        assert_eq!(lanes[0].lane_id, sibling_lane.lane_id);
        assert!(
            lanes
                .iter()
                .all(|lane| lane.lane_id != replacement_lane.lane_id)
        );
    }

    #[tokio::test]
    async fn permanent_expired_owner_cleanup_is_constant_and_concurrent_recovery_mints_once() {
        let harness = harness_with_owner_ttl(10);
        let runtime_id = "signed-child-generation-fence";
        let mut first = gateway_caller_without_browser_identity();
        harness
            .registry
            .attach_trusted_identity(
                &mut first,
                runtime_id,
                None,
                u64::MAX,
            )
            .await
            .unwrap();
        let old_owner = first
            .browser_identity
            .as_ref()
            .unwrap()
            .owner_lease_id
            .clone();
        harness.registry.open(&first, None).await.unwrap();

        // Keep a sibling target on the shared Primary Host so a failed Lane
        // close cannot be discharged by retiring the whole Host.
        let mut sibling = gateway_caller_without_browser_identity();
        harness
            .registry
            .attach_trusted_identity(
                &mut sibling,
                "signed-child-generation-fence-sibling",
                None,
                u64::MAX,
            )
            .await
            .unwrap();
        harness.registry.open(&sibling, None).await.unwrap();

        tokio::time::sleep(Duration::from_millis(20)).await;
        harness
            .probe
            .lane_close_failures_remaining
            .store(usize::MAX, Ordering::Release);

        for attempt in 0..8 {
            // Cross many owner TTL windows. No failed attempt may append a
            // generation or publish a capability to its caller.
            tokio::time::sleep(Duration::from_millis(20)).await;
            let mut blocked = gateway_caller_without_browser_identity();
            let error = harness
                .registry
                .attach_trusted_identity(
                    &mut blocked,
                    runtime_id,
                    Some(&format!("blocked-{attempt}")),
                    u64::MAX,
                )
                .await
                .expect_err("permanent old-owner cleanup failure must fence attach");
            assert_eq!(error.code, BrowserErrorCode::BrowserUnavailable);
            assert!(blocked.browser_identity.is_none());

            let identities = harness
                .registry
                .identities
                .lock()
                .expect("gateway browser identity cache poisoned");
            let cached = identities
                .get(runtime_id)
                .expect("the exact failed owner must remain authoritative");
            assert_eq!(cached.identity.owner_lease_id, old_owner);
            assert_eq!(cached.pending_owner_cleanup, Some(old_owner.clone()));
            assert_eq!(
                identities
                    .keys()
                    .filter(|runtime| runtime.as_str() == runtime_id)
                    .count(),
                1,
                "one runtime key may retain only one owner generation"
            );
        }

        let closes_before_recovery =
            harness.probe.lane_closes.load(Ordering::Acquire);
        harness
            .probe
            .block_lane_close
            .store(true, Ordering::Release);
        harness
            .probe
            .lane_close_failures_remaining
            .store(0, Ordering::Release);

        let attempts = (0..32)
            .map(|attempt| {
                let registry = harness.registry.clone();
                tokio::spawn(async move {
                    let mut caller = gateway_caller_without_browser_identity();
                    registry
                        .attach_trusted_identity(
                            &mut caller,
                            runtime_id,
                            Some(&format!("recovery-{attempt}")),
                            u64::MAX,
                        )
                        .await?;
                    Ok::<_, BrowserPlatformError>(
                        caller.browser_identity.expect("successful attach identity"),
                    )
                })
            })
            .collect::<Vec<_>>();

        tokio::time::timeout(
            Duration::from_secs(3),
            harness
                .probe
                .wait_for_lane_closes(closes_before_recovery + 1),
        )
        .await
        .expect("the exact old-owner cleanup did not start");
        for _ in 0..32 {
            tokio::task::yield_now().await;
        }
        assert_eq!(
            harness.probe.lane_closes.load(Ordering::Acquire),
            closes_before_recovery + 1,
            "same-runtime attach contenders must wait behind one cleanup flight"
        );
        harness.probe.lane_close_releases.add_permits(1);

        let mut replacement_owners = HashSet::new();
        let mut recovered_callers = Vec::new();
        for attempt in attempts {
            let identity = attempt
                .await
                .expect("recovery attach task panicked")
                .expect("cleanup recovery must permit attach");
            replacement_owners.insert(identity.owner_lease_id.clone());
            recovered_callers.push(identity);
        }
        harness
            .probe
            .block_lane_close
            .store(false, Ordering::Release);

        assert_eq!(replacement_owners.len(), 1);
        assert!(!replacement_owners.contains(&old_owner));
        assert_eq!(
            harness.probe.lane_closes.load(Ordering::Acquire),
            closes_before_recovery + 1,
            "only the exact blocked generation should require cleanup"
        );
        let identities = harness
            .registry
            .identities
            .lock()
            .expect("gateway browser identity cache poisoned");
        let cached = &identities[runtime_id];
        assert_eq!(
            replacement_owners,
            HashSet::from([cached.identity.owner_lease_id.clone()])
        );
        assert!(cached.pending_owner_cleanup.is_none());
        assert_eq!(recovered_callers.len(), 32);
    }

    #[tokio::test]
    async fn unrelated_runtime_revokes_do_not_share_a_lifecycle_gate() {
        let harness = harness();
        let mut first = gateway_caller_without_browser_identity();
        harness
            .registry
            .attach_trusted_identity(
                &mut first,
                "runtime-slow-cleanup",
                Some("attempt-slow"),
                u64::MAX,
            )
            .await
            .unwrap();
        let mut second = gateway_caller_without_browser_identity();
        harness
            .registry
            .attach_trusted_identity(
                &mut second,
                "runtime-fast-cleanup",
                Some("attempt-fast"),
                u64::MAX,
            )
            .await
            .unwrap();
        harness.registry.open(&first, None).await.unwrap();
        harness.registry.open(&second, None).await.unwrap();

        harness
            .probe
            .block_lane_close
            .store(true, Ordering::Release);
        let slow_registry = harness.registry.clone();
        let slow = tokio::spawn(async move {
            slow_registry
                .revoke_signed_child_lease("runtime-slow-cleanup")
                .await
        });
        tokio::time::timeout(
            Duration::from_secs(2),
            harness.probe.wait_for_lane_closes(1),
        )
        .await
        .expect("slow runtime cleanup should reach its Lane close");

        // Keep the first runtime blocked while allowing the second runtime's
        // close to finish. A gateway-global lifecycle gate would make this
        // timeout instead of progressing independently.
        harness
            .probe
            .block_lane_close
            .store(false, Ordering::Release);
        let fast = tokio::time::timeout(
            Duration::from_secs(1),
            harness
                .registry
                .revoke_signed_child_lease("runtime-fast-cleanup"),
        )
        .await
        .expect("an unrelated runtime must not wait for slow cleanup")
        .unwrap();
        assert_eq!(fast.closed, 1);

        harness.probe.lane_close_releases.add_permits(1);
        let slow = slow.await.unwrap().unwrap();
        assert_eq!(slow.closed, 1);
    }

    #[tokio::test]
    async fn server_derived_operation_scope_cannot_be_widened_by_tool_input() {
        let harness = harness();
        let mut caller = gateway_caller_without_browser_identity();
        harness
            .registry
            .attach_trusted_identity_scoped(
                &mut caller,
                "remote-session-observe-only",
                None,
                u64::MAX,
                BTreeSet::from([BrowserOperationKind::Observe]),
            )
            .await
            .unwrap();
        let identity = caller.browser_identity.unwrap();
        assert_eq!(
            identity.allowed_operations,
            BTreeSet::from([BrowserOperationKind::Observe])
        );
        assert!(
            reject_untrusted_caller_fields(&json!({
                "action": "navigate",
                "url": "https://example.test",
                "allowed_operations": ["navigate", "act"],
            }))
            .is_err(),
            "model input cannot widen the server-derived operation scope"
        );
    }

    #[tokio::test]
    async fn explicit_child_revoke_closes_only_its_lanes_and_is_idempotent() {
        let harness = harness();
        let mut first = gateway_caller_without_browser_identity();
        harness
            .registry
            .attach_trusted_identity(
                &mut first,
                "signed-child-lease-a",
                Some("attempt-a"),
                u64::MAX,
            )
            .await
            .unwrap();
        let mut second = gateway_caller_without_browser_identity();
        harness
            .registry
            .attach_trusted_identity(
                &mut second,
                "signed-child-lease-b",
                Some("attempt-b"),
                u64::MAX,
            )
            .await
            .unwrap();

        harness.registry.open(&first, None).await.unwrap();
        harness
            .registry
            .open(&first, Some("secondary"))
            .await
            .unwrap();
        let surviving_lane =
            harness.registry.open(&second, None).await.unwrap();
        harness
            .registry
            .execute(&first, None, json!({ "action": "observe" }))
            .await
            .unwrap();
        harness
            .registry
            .execute(&second, None, json!({ "action": "observe" }))
            .await
            .unwrap();

        let revoked = harness
            .registry
            .revoke_signed_child_lease("signed-child-lease-a")
            .await
            .unwrap();
        assert_eq!(revoked.closed, 2);
        assert!(!revoked.already_closed);

        let lanes = harness.hub.list_lanes().await;
        assert_eq!(lanes.len(), 1);
        assert_eq!(lanes[0].lane_id, surviving_lane.lane_id);
        assert_eq!(
            lanes[0].caller.runtime_instance_id,
            "signed-child-lease-b"
        );
        let identities = harness
            .registry
            .identities
            .lock()
            .expect("gateway browser identity cache poisoned");
        assert!(!identities.contains_key("signed-child-lease-a"));
        assert!(identities.contains_key("signed-child-lease-b"));
        drop(identities);
        let repeated = harness
            .registry
            .revoke_signed_child_lease("signed-child-lease-a")
            .await
            .unwrap();
        assert_eq!(repeated.closed, 0);
        assert!(repeated.already_closed);
        let lanes = harness.hub.list_lanes().await;
        assert_eq!(lanes.len(), 1);
        assert_eq!(lanes[0].lane_id, surviving_lane.lane_id);
    }

    #[tokio::test]
    async fn inactive_child_reconciliation_closes_only_revoked_owner() {
        let harness = harness();
        let mut first = gateway_caller_without_browser_identity();
        harness
            .registry
            .attach_trusted_identity(
                &mut first,
                "signed-child-inactive",
                Some("attempt-inactive"),
                u64::MAX,
            )
            .await
            .unwrap();
        let mut second = gateway_caller_without_browser_identity();
        harness
            .registry
            .attach_trusted_identity(
                &mut second,
                "signed-child-active",
                Some("attempt-active"),
                u64::MAX,
            )
            .await
            .unwrap();

        let inactive_lane = harness.registry.open(&first, None).await.unwrap();
        let active_lane = harness.registry.open(&second, None).await.unwrap();
        harness
            .registry
            .cleanup_inactive_signed_child_leases(|lease_id| {
                lease_id == "signed-child-active"
            })
            .await;

        let lanes = harness.hub.list_lanes().await;
        assert_eq!(lanes.len(), 1);
        assert_eq!(lanes[0].lane_id, active_lane.lane_id);
        assert_ne!(lanes[0].lane_id, inactive_lane.lane_id);
        let identities = harness
            .registry
            .identities
            .lock()
            .expect("gateway browser identity cache poisoned");
        assert!(!identities.contains_key("signed-child-inactive"));
        assert!(identities.contains_key("signed-child-active"));
    }

    #[tokio::test]
    async fn different_lanes_are_not_serialized_by_the_gateway() {
        let harness = harness();
        let caller = caller(&harness.hub, "runtime-parallel", "attempt-1");
        harness
            .registry
            .open(&caller, Some("one"))
            .await
            .unwrap();
        harness
            .registry
            .open(&caller, Some("two"))
            .await
            .unwrap();

        let first_registry = harness.registry.clone();
        let first_caller = caller.clone();
        let first = tokio::spawn(async move {
            first_registry
                .execute(
                    &first_caller,
                    Some("one"),
                    json!({
                        "action": "navigate",
                        "url": "https://example.test/one",
                        "block": true,
                    }),
                )
                .await
        });
        harness.probe.wait_for_active(1).await;

        let second_registry = harness.registry.clone();
        let second = tokio::spawn(async move {
            second_registry
                .execute(
                    &caller,
                    Some("two"),
                    json!({
                        "action": "navigate",
                        "url": "https://example.test/two",
                        "block": true,
                    }),
                )
                .await
        });
        tokio::time::timeout(
            Duration::from_secs(1),
            harness.probe.wait_for_active(2),
        )
        .await
        .expect("different lanes were globally serialized");
        assert_eq!(harness.probe.maximum.load(Ordering::Acquire), 2);
        harness.probe.releases.add_permits(2);
        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn missing_trusted_identity_fails_closed_before_host_launch() {
        let harness = harness();
        let error = harness
            .registry
            .open(&CallerCtx::default(), None)
            .await
            .unwrap_err();
        assert_eq!(error.code, BrowserErrorCode::InvalidCallerIdentity);
        assert_eq!(harness.factory.launches.load(Ordering::Acquire), 0);
    }

    #[tokio::test]
    async fn managed_identity_fields_fail_closed_before_facade_or_host_launch() {
        let harness = harness();
        let caller = caller(&harness.hub, "runtime-identity-policy", "attempt-a");
        for field in MODEL_IDENTITY_INPUT_FIELDS {
            let error = harness
                .registry
                .dispatch_managed(
                    &caller,
                    None,
                    json!({
                        "action": "browser_open",
                        "lane_name": "research",
                        (*field): "model-controlled",
                    }),
                )
                .await
                .unwrap_err();
            assert_eq!(
                error.code,
                BrowserErrorCode::InvalidCallerIdentity,
                "{field} must fail at the BrowserRegistry boundary"
            );
        }
        assert_eq!(harness.factory.launches.load(Ordering::Acquire), 0);
        assert!(
            harness.hub.list_lanes().await.is_empty(),
            "identity-policy input must not reach facade dispatch or Host launch"
        );
    }

    #[tokio::test]
    async fn semantic_preflight_rejects_invalid_first_request_without_attachment() {
        let harness = harness();
        let caller = gateway_caller_without_browser_identity();

        let error = harness
            .registry
            .validate_managed_request(
                &caller,
                "nomi_browser_act",
                &json!({
                    "action": "click",
                    "runtime_instance_id": "model-controlled",
                }),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, BrowserErrorCode::InvalidCallerIdentity);
        assert_eq!(
            harness.registry.signed_child_cleanup_status(),
            BrowserCleanupStatus::default(),
            "semantic rejection must not create an owner attachment"
        );

        let error = harness
            .registry
            .validate_managed_request(
                &caller,
                "nomi_browser_open",
                &json!({
                    "lane": "default",
                    "lane_name": "research",
                }),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, BrowserErrorCode::OperationNotAllowed);
        assert_eq!(
            harness.registry.signed_child_cleanup_status(),
            BrowserCleanupStatus::default(),
            "selector conflict must not create an owner attachment"
        );
    }

    #[tokio::test]
    async fn semantic_preflight_rejects_invalid_renewal_without_renewing_owner() {
        let harness = harness();
        let mut caller = gateway_caller_without_browser_identity();
        harness
            .registry
            .attach_trusted_identity(
                &mut caller,
                "semantic-renewal",
                None,
                u64::MAX,
            )
            .await
            .unwrap();
        let before = harness
            .registry
            .identities
            .lock()
            .expect("identity cache")
            .get("semantic-renewal")
            .expect("cached attachment")
            .identity
            .clone();

        let error = harness
            .registry
            .validate_managed_request(
                &caller,
                "nomi_browser_act",
                &json!({
                    "action": "click",
                    "identity_mode": "isolated",
                }),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, BrowserErrorCode::InvalidCallerIdentity);
        let after = harness
            .registry
            .identities
            .lock()
            .expect("identity cache")
            .get("semantic-renewal")
            .expect("cached attachment")
            .identity
            .clone();
        assert_eq!(
            after, before,
            "semantic rejection must not mutate or renew the cached owner"
        );
    }

    #[tokio::test]
    async fn semantic_preflight_defers_lane_id_owner_check_until_identity_attachment() {
        let harness = harness();
        let mut owner = gateway_caller_without_browser_identity();
        harness
            .registry
            .attach_trusted_identity(
                &mut owner,
                "signed-lane-id-owner",
                None,
                u64::MAX,
            )
            .await
            .unwrap();
        let forked = harness
            .registry
            .dispatch_managed(
                &owner,
                None,
                json!({ "action": "browser_fork", "lane_name": "research" }),
            )
            .await
            .unwrap();
        assert!(!forked.is_error, "{}", forked.content);
        let forked: Value = serde_json::from_str(&forked.content).unwrap();
        let lane_id = forked
            .pointer("/lane/lane_id")
            .and_then(Value::as_str)
            .unwrap()
            .to_owned();

        // Both transports validate on a per-request caller that has no
        // browser identity yet; an owner-scoped lane_id must not be rejected
        // before the trusted identity is attached.
        let unattached = gateway_caller_without_browser_identity();
        harness
            .registry
            .validate_managed_request(
                &unattached,
                "nomi_browser_status",
                &json!({ "lane_id": lane_id }),
            )
            .await
            .expect(
                "a lane_id carried by a not-yet-attached request must defer \
                 the owner check instead of failing closed",
            );

        // Re-validation after attachment performs the definitive owner check.
        harness
            .registry
            .validate_managed_request(
                &owner,
                "nomi_browser_status",
                &json!({ "lane_id": lane_id }),
            )
            .await
            .expect("the owning identity must pass the lane_id owner check");

        let mut sibling = gateway_caller_without_browser_identity();
        harness
            .registry
            .attach_trusted_identity(
                &mut sibling,
                "signed-lane-id-sibling",
                None,
                u64::MAX,
            )
            .await
            .unwrap();
        let error = harness
            .registry
            .validate_managed_request(
                &sibling,
                "nomi_browser_status",
                &json!({ "lane_id": lane_id }),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, BrowserErrorCode::OperationNotAllowed);
    }

    #[tokio::test]
    async fn revoked_runtime_tombstones_are_bounded_with_insertion_order_eviction() {
        let harness = harness();
        for index in 0..=REVOKED_RUNTIME_TOMBSTONE_CAPACITY {
            harness
                .registry
                .revoke_signed_child_lease(&format!("remote-tombstone-{index}"))
                .await
                .unwrap();
        }

        // Recent revocations keep their anti-resurrection authority.
        let mut newest = gateway_caller_without_browser_identity();
        let error = harness
            .registry
            .attach_trusted_identity(
                &mut newest,
                &format!(
                    "remote-tombstone-{REVOKED_RUNTIME_TOMBSTONE_CAPACITY}"
                ),
                None,
                u64::MAX,
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, BrowserErrorCode::OwnerLeaseExpired);

        // The oldest tombstone is evicted instead of retained forever.
        let mut oldest = gateway_caller_without_browser_identity();
        harness
            .registry
            .attach_trusted_identity(
                &mut oldest,
                "remote-tombstone-0",
                None,
                u64::MAX,
            )
            .await
            .expect("evicted tombstones must not grow the store unbounded");
    }

    #[tokio::test]
    async fn semantic_preflight_rejects_cross_owner_lane_without_renewing() {
        let harness = harness();
        let mut first = gateway_caller_without_browser_identity();
        harness
            .registry
            .attach_trusted_identity(
                &mut first,
                "semantic-owner-a",
                None,
                u64::MAX,
            )
            .await
            .unwrap();
        let lane = harness.registry.open(&first, None).await.unwrap();

        let mut second = gateway_caller_without_browser_identity();
        harness
            .registry
            .attach_trusted_identity(
                &mut second,
                "semantic-owner-b",
                None,
                u64::MAX,
            )
            .await
            .unwrap();
        let before = harness
            .registry
            .identities
            .lock()
            .expect("identity cache")
            .get("semantic-owner-b")
            .expect("second cached attachment")
            .identity
            .clone();

        let error = harness
            .registry
            .validate_managed_request(
                &second,
                "nomi_browser_status",
                &json!({
                    "lane_id": lane.lane_id,
                }),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, BrowserErrorCode::OperationNotAllowed);
        let after = harness
            .registry
            .identities
            .lock()
            .expect("identity cache")
            .get("semantic-owner-b")
            .expect("second cached attachment")
            .identity
            .clone();
        assert_eq!(
            after, before,
            "cross-owner selector rejection must not mutate or renew the caller owner"
        );
    }

    #[test]
    fn model_input_cannot_supply_trusted_caller_fields() {
        let error = reject_untrusted_caller_fields(&json!({
            "action": "navigate",
            "url": "https://example.test",
            "runtime_instance_id": "model-chosen-runtime",
        }))
        .unwrap_err();
        assert_eq!(error.code, BrowserErrorCode::InvalidCallerIdentity);
        assert!(reject_untrusted_caller_fields(&json!({
            "action": "navigate",
            "url": "https://example.test",
        }))
        .is_ok());
        for field in ["surface", "target_id", "browser_epoch", "lane_key"] {
            let error = reject_untrusted_caller_fields(&json!({
                "action": "navigate",
                (field): "model-controlled",
            }))
            .unwrap_err();
            assert_eq!(
                error.code,
                BrowserErrorCode::InvalidCallerIdentity,
                "{field} must remain main-process managed"
            );
        }
        for field in MODEL_IDENTITY_INPUT_FIELDS {
            let error = reject_untrusted_caller_fields(&json!({
                "action": "browser_open",
                (*field): "model-controlled",
            }))
            .unwrap_err();
            assert_eq!(
                error.code,
                BrowserErrorCode::InvalidCallerIdentity,
                "{field} must remain trusted host policy"
            );
        }
        assert!(
            reject_untrusted_caller_fields(&json!({
                "action": "observe",
                "lane_id": "owner-scoped-handle",
            }))
            .is_ok(),
            "lane_id is an owner-scoped selector authorized by the bound client"
        );
    }

    #[test]
    fn gateway_rejects_every_shared_trusted_owner_field() {
        // F23: the gateway must enforce the ONE shared trusted-owner field
        // list (`nomi_browser::TRUSTED_OWNER_INPUT_FIELDS`). A divergent
        // gateway-local list would make identical requests behave differently
        // across supposedly-equivalent managed browser surfaces.
        assert!(
            nomi_browser::TRUSTED_OWNER_INPUT_FIELDS.contains(&"runtime_cleanup_key"),
            "the exact runtime cleanup key must remain host-owned"
        );
        for field in nomi_browser::TRUSTED_OWNER_INPUT_FIELDS {
            let error = reject_untrusted_caller_fields(&json!({
                "action": "navigate",
                (*field): "model-controlled",
            }))
            .unwrap_err();
            assert_eq!(
                error.code,
                BrowserErrorCode::InvalidCallerIdentity,
                "shared trusted-owner field `{field}` must be rejected by the gateway"
            );
        }
    }

    #[test]
    fn model_cannot_downgrade_identity_modifying_actions() {
        for action in [
            "evaluate",
            "click",
            "type",
            "set_value",
            "select_option",
            "press_key",
            "upload_file",
            "clear_cookies",
            "set_cookie",
            "clear_storage",
            "login",
            "logout",
            "switch_account",
            "submit_form",
        ] {
            let operation = operation_from_input(&json!({
                "action": action,
                "may_modify_identity": false,
            }))
            .unwrap();
            assert!(
                operation.may_modify_identity,
                "{action} must remain identity-modifying despite a model-supplied false"
            );
        }

        for action in [
            "navigate",
            "back",
            "forward",
            "reload",
            "crawl",
            "crawl_many",
            "close_tab",
            "open_link_new_tab",
            "observe",
            "screenshot",
            "tabs",
            "switch_tab",
            "get_console_logs",
            "get_page_errors",
            "get_network_log",
            "rendered_html",
            "capabilities",
            "device_pixel_ratio",
            "hover",
            "scroll",
            "wait",
            "extract",
        ] {
            let benign = operation_from_input(&json!({
                "action": action,
                "may_modify_identity": true,
            }))
            .unwrap();
            assert!(
                !benign.may_modify_identity,
                "{action} must use the server classifier rather than model input"
            );
        }
        assert!(
            operation_from_input(&json!({
                "action": "navigate",
                "method": "POST",
                "may_modify_identity": false,
            }))
            .unwrap()
            .may_modify_identity
        );
        assert!(
            operation_from_input(&json!({
                "action": "future_gateway_action",
                "may_modify_identity": false,
            }))
            .unwrap()
            .may_modify_identity
        );
    }

    #[test]
    fn model_cannot_foreground_the_managed_browser_through_gateway_json() {
        let error = operation_from_input(&json!({
            "action": "bring_to_front",
        }))
        .unwrap_err();

        assert_eq!(error.code, BrowserErrorCode::OperationNotAllowed);
        assert!(!error.retryable);
        assert_eq!(operation_kind("bring_to_front"), BrowserOperationKind::Act);
    }
}
