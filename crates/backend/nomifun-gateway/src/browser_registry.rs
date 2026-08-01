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

use nomi_browser::{
    ActionContext, ApprovalTier, ManagedBrowserFacade, TRUSTED_OWNER_INPUT_FIELDS,
    classify_action,
};
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

/// Server-side authority that created one cached Gateway browser attachment.
///
/// Signed child attachments are reconciled against the process-local loopback
/// issuer. Remote MCP attachments instead live and die with rmcp's logical
/// session manager and must never be interpreted as signed-child lease ids.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum BrowserAttachmentAuthority {
    SignedChild,
    RemoteMcpSession,
}

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
    authority: BrowserAttachmentAuthority,
    identity: CallerIdentity,
    /// A failed Hub cleanup leaves this record authoritative and retryable.
    revocation_pending: bool,
    /// Owner leases superseded after an expired renewal remain here until
    /// their lane cleanup succeeds.
    pending_owner_cleanup: Vec<OwnerLeaseId>,
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

/// The immutable ownership key stored with a pending out-of-band approval.
#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingOwner {
    user_id: String,
    lane_key: LaneKey,
    owner_lease_id: OwnerLeaseId,
}

/// A browser action held awaiting out-of-band approval.
#[derive(Clone, Debug)]
pub struct PendingBrowserAction {
    pub input: Value,
    pub lane_name: String,
    runtime_instance_id: String,
    owner_lease_id: OwnerLeaseId,
    user_id: String,
}

impl PendingBrowserAction {
    fn owner(&self) -> PendingOwner {
        PendingOwner {
            user_id: self.user_id.clone(),
            lane_key: LaneKey {
                runtime_instance_id: self.runtime_instance_id.clone(),
                lane_name: self.lane_name.clone(),
            },
            owner_lease_id: self.owner_lease_id.clone(),
        }
    }
}

/// One gateway browser call used by [`BrowserRegistry::execute_parallel`].
#[derive(Clone, Debug)]
pub struct GatewayBrowserCall {
    pub caller: CallerCtx,
    pub lane_name: String,
    pub input: Value,
}

const MAX_PENDING: usize = 64;
/// Upper bound on retained revoked-runtime tombstones. Lease/session ids are
/// never legitimately reused, so the tombstone only has to outlive the short
/// attach-vs-close race window; the oldest entries are evicted in insertion
/// order once this many distinct runtimes have been revoked.
const REVOKED_RUNTIME_TOMBSTONE_CAPACITY: usize = 4096;
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

/// Bounded authority-aware revoked-runtime tombstones (F62).
///
/// Ids are tracked in insertion order; once the capacity is reached the oldest
/// tombstone is evicted, keeping the anti-resurrection authority for recent
/// revocations without growing per revoked session forever.
#[derive(Default)]
struct RevokedRuntimeTombstones {
    entries: HashMap<String, HashSet<BrowserAttachmentAuthority>>,
    insertion_order: VecDeque<String>,
}

impl RevokedRuntimeTombstones {
    fn insert(
        &mut self,
        runtime_instance_id: &str,
        authority: BrowserAttachmentAuthority,
    ) {
        match self.entries.get_mut(runtime_instance_id) {
            Some(authorities) => {
                authorities.insert(authority);
            }
            None => {
                self.entries.insert(
                    runtime_instance_id.to_owned(),
                    HashSet::from([authority]),
                );
                self.insertion_order.push_back(runtime_instance_id.to_owned());
                while self.insertion_order.len() > REVOKED_RUNTIME_TOMBSTONE_CAPACITY {
                    let Some(evicted) = self.insertion_order.pop_front() else {
                        break;
                    };
                    self.entries.remove(&evicted);
                }
            }
        }
    }

    fn contains(
        &self,
        runtime_instance_id: &str,
        authority: BrowserAttachmentAuthority,
    ) -> bool {
        self.entries
            .get(runtime_instance_id)
            .is_some_and(|authorities| authorities.contains(&authority))
    }
}

/// Clone-cheap bridge to the application-owned browser hub.
#[derive(Clone)]
pub struct BrowserRegistry {
    hub: Option<BrowserSessionHub>,
    identity_resolver: Arc<dyn TrustedBrowserIdentityResolver>,
    pending: Arc<std::sync::Mutex<HashMap<String, PendingBrowserAction>>>,
    /// Stable owner capability per server-validated runtime attachment.
    identities: Arc<std::sync::Mutex<HashMap<String, CachedBrowserIdentity>>>,
    /// Runtime ids are never reusable after an authoritative revoke. Keeping
    /// an authority-aware tombstone prevents an attach racing with session
    /// close from resurrecting a revoked Browser owner while ensuring an
    /// unrelated authority cannot tombstone the runtime.
    revoked_runtime_ids: Arc<std::sync::Mutex<RevokedRuntimeTombstones>>,
    /// Runtime-scoped attachment/revocation gates. Same-runtime transitions
    /// serialize; unrelated runtimes can attach and clean up concurrently.
    runtime_lifecycle_slots: Arc<
        std::sync::Mutex<
            HashMap<String, Arc<AsyncMutex<()>>>,
        >,
    >,
    /// Last observation text per lane. This keeps the existing GW2 semantic-name
    /// check without owning a BrowserTool. Browser execution state remains in
    /// the hub/driver.
    observations: Arc<std::sync::Mutex<HashMap<LaneKey, String>>>,
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
            pending: Arc::new(std::sync::Mutex::new(HashMap::new())),
            identities: Arc::new(std::sync::Mutex::new(HashMap::new())),
            revoked_runtime_ids: Arc::new(std::sync::Mutex::new(
                RevokedRuntimeTombstones::default(),
            )),
            runtime_lifecycle_slots: Arc::new(std::sync::Mutex::new(
                HashMap::new(),
            )),
            observations: Arc::new(std::sync::Mutex::new(HashMap::new())),
        }
    }

    /// Inject a hub and use the app-populated identity on [`CallerCtx`].
    pub fn from_hub(hub: BrowserSessionHub) -> Self {
        Self::new(hub, Arc::new(CallerCtxBrowserIdentityResolver))
    }

    /// Test-only fail-closed constructor: no hub injected, launches nothing.
    /// Application wiring must use [`Self::from_hub`] or [`Self::new`].
    #[cfg(test)]
    pub(crate) fn default_for_browser_use() -> Self {
        Self {
            hub: None,
            identity_resolver: Arc::new(CallerCtxBrowserIdentityResolver),
            pending: Arc::new(std::sync::Mutex::new(HashMap::new())),
            identities: Arc::new(std::sync::Mutex::new(HashMap::new())),
            revoked_runtime_ids: Arc::new(std::sync::Mutex::new(
                RevokedRuntimeTombstones::default(),
            )),
            runtime_lifecycle_slots: Arc::new(std::sync::Mutex::new(
                HashMap::new(),
            )),
            observations: Arc::new(std::sync::Mutex::new(HashMap::new())),
        }
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
        self.attach_trusted_identity_with_authority(
            caller,
            runtime_instance_id,
            attempt_id,
            capability_expires_at_ms,
            BrowserAttachmentAuthority::SignedChild,
        )
        .await
    }

    /// Attach a browser identity whose runtime and lifecycle authority were
    /// derived by a trusted server ingress.
    ///
    /// `allowed_operations` is supplied by that ingress after it resolves the
    /// advertised tool scope. It is never read from browser tool arguments.
    pub async fn attach_trusted_identity_with_authority(
        &self,
        caller: &mut CallerCtx,
        runtime_instance_id: &str,
        attempt_id: Option<&str>,
        capability_expires_at_ms: u64,
        authority: BrowserAttachmentAuthority,
    ) -> Result<(), BrowserPlatformError> {
        self.attach_trusted_identity_scoped(
            caller,
            runtime_instance_id,
            attempt_id,
            capability_expires_at_ms,
            authority,
            all_browser_operations(),
        )
        .await
    }

    /// Scoped form of [`Self::attach_trusted_identity_with_authority`].
    pub async fn attach_trusted_identity_scoped(
        &self,
        caller: &mut CallerCtx,
        runtime_instance_id: &str,
        attempt_id: Option<&str>,
        capability_expires_at_ms: u64,
        authority: BrowserAttachmentAuthority,
        allowed_operations: BTreeSet<BrowserOperationKind>,
    ) -> Result<(), BrowserPlatformError> {
        let lifecycle = self.runtime_lifecycle_slot(runtime_instance_id);
        let _lifecycle_guard = lifecycle.gate.lock().await;
        self.attach_trusted_identity_scoped_locked(
            caller,
            runtime_instance_id,
            attempt_id,
            capability_expires_at_ms,
            authority,
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
        authority: BrowserAttachmentAuthority,
        allowed_operations: BTreeSet<BrowserOperationKind>,
    ) -> Result<(), BrowserPlatformError> {
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
                "Request a Remote MCP profile that includes browser operations.",
            ));
        }

        if self
            .revoked_runtime_ids
            .lock()
            .expect("gateway browser revoked-runtime store poisoned")
            .contains(runtime_instance_id, authority)
        {
            return Err(revoked_runtime_error());
        }

        let (identity, pending_owner_cleanup, replaced_owner_lease_id) = {
            let mut identities = self
                .identities
                .lock()
                .expect("gateway browser identity cache poisoned");
            if let Some(existing) = identities.get(runtime_instance_id) {
                if existing.revocation_pending {
                    return Err(revoked_runtime_error());
                }
                if existing.authority != authority {
                    return Err(BrowserPlatformError::new(
                        BrowserErrorCode::InvalidCallerIdentity,
                        "The browser runtime is already bound to another authority.",
                        false,
                        "Request a fresh authenticated browser runtime.",
                    ));
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

                let mut pending_owner_cleanup =
                    existing.pending_owner_cleanup.clone();
                let (lease, owner_replaced) = match hub
                    .renew_owner_lease(&existing.identity.owner_lease_id)
                {
                    Ok(lease) => (lease, false),
                    Err(error)
                        if error.code == BrowserErrorCode::OwnerLeaseExpired =>
                    {
                        // A live logical runtime may outlast the Hub's
                        // renewable owner TTL. Replace only this exact stale
                        // lease and retain it for authoritative cleanup.
                        let old_owner_lease_id =
                            existing.identity.owner_lease_id.clone();
                        if !pending_owner_cleanup
                            .iter()
                            .any(|id| id == &old_owner_lease_id)
                        {
                            pending_owner_cleanup.push(old_owner_lease_id);
                        }
                        hub.issue_owner_lease(
                            existing.identity.user_id.clone(),
                            existing.identity.conversation_id.clone(),
                            existing.identity.runtime_instance_id.clone(),
                        )
                        .map(|lease| (lease, true))?
                    }
                    Err(error) => return Err(error),
                };
                let mut identity = existing.identity.clone();
                let replaced_owner_lease_id =
                    owner_replaced.then(|| existing.identity.owner_lease_id.clone());
                identity.owner_lease_id = lease.lease_id;
                identity.capability_expires_at_ms =
                    capability_expires_at_ms.min(lease.expires_at_ms);
                identity.attempt_id = attempt_id.map(str::to_owned);
                identity.allowed_operations = effective_allowed_operations;
                identities.insert(
                    runtime_instance_id.to_owned(),
                    CachedBrowserIdentity {
                        authority,
                        identity: identity.clone(),
                        revocation_pending: false,
                        pending_owner_cleanup: pending_owner_cleanup.clone(),
                    },
                );
                (identity, pending_owner_cleanup, replaced_owner_lease_id)
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
                    surface: if caller.remote {
                        BrowserSurface::Remote
                    } else {
                        BrowserSurface::Gateway
                    },
                    owner_lease_id: owner.lease_id,
                    capability_expires_at_ms: capability_expires_at_ms
                        .min(owner.expires_at_ms),
                    allowed_operations,
                };
                identities.insert(
                    runtime_instance_id.to_owned(),
                    CachedBrowserIdentity {
                        authority,
                        identity: identity.clone(),
                        revocation_pending: false,
                        pending_owner_cleanup: Vec::new(),
                    },
                );
                (identity, Vec::new(), None)
            }
        };

        if let Some(replaced_owner_lease_id) = replaced_owner_lease_id {
            self.clear_owner_lease_caches(&replaced_owner_lease_id);
        }
        if let Err(error) = self
            .retry_owner_cleanup(
                runtime_instance_id,
                &identity.owner_lease_id,
                pending_owner_cleanup,
                &hub,
            )
            .await
        {
            tracing::warn!(
                runtime_id = %runtime_instance_id,
                code = ?error.code,
                "Gateway browser replacement published with superseded-owner cleanup pending"
            );
        }
        caller.browser_identity = Some(identity);
        Ok(())
    }

    /// Revoke one exact trusted attachment regardless of its ingress source.
    ///
    /// The runtime id comes from signed claims or rmcp's server-generated
    /// session id. Repeated lifecycle callbacks are successful no-ops.
    pub async fn revoke_trusted_identity(
        &self,
        runtime_instance_id: &str,
    ) -> Result<CloseResult, BrowserPlatformError> {
        self.revoke_identity_for_authority(
            runtime_instance_id,
            BrowserAttachmentAuthority::RemoteMcpSession,
        )
        .await
    }

    async fn revoke_identity_for_authority(
        &self,
        runtime_instance_id: &str,
        expected_authority: BrowserAttachmentAuthority,
    ) -> Result<CloseResult, BrowserPlatformError> {
        let lifecycle = self.runtime_lifecycle_slot(runtime_instance_id);
        let _lifecycle_guard = lifecycle.gate.lock().await;
        self.revoke_identity_for_authority_locked(
            runtime_instance_id,
            expected_authority,
        )
        .await
    }

    /// Revoke one cached identity while the attachment lifecycle gate is held.
    ///
    /// The current owner lease and every lease retained after an expired
    /// renewal are cleaned independently. A failed cleanup keeps the cached
    /// authority marked pending so the next sweep can retry it rather than
    /// silently losing the old lease.
    async fn revoke_identity_for_authority_locked(
        &self,
        runtime_instance_id: &str,
        expected_authority: BrowserAttachmentAuthority,
    ) -> Result<CloseResult, BrowserPlatformError> {
        let (owner_lease_id, effective_runtime_id, authority, pending_owner_cleanup) = {
            let mut identities = self
                .identities
                .lock()
                .expect("gateway browser identity cache poisoned");
            let Some(cached) = identities.get_mut(runtime_instance_id) else {
                self.revoked_runtime_ids
                    .lock()
                    .expect("gateway browser revoked-runtime store poisoned")
                    .insert(runtime_instance_id, expected_authority);
                return Ok(CloseResult {
                    closed: 0,
                    already_closed: true,
                    ..Default::default()
                });
            };
            if cached.authority != expected_authority {
                return Ok(CloseResult {
                    closed: 0,
                    already_closed: true,
                    ..Default::default()
                });
            }
            cached.revocation_pending = true;
            (
                cached.identity.owner_lease_id.clone(),
                cached.identity.runtime_instance_id.clone(),
                cached.authority,
                cached.pending_owner_cleanup.clone(),
            )
        };

        self.revoked_runtime_ids
            .lock()
            .expect("gateway browser revoked-runtime store poisoned")
            .insert(&effective_runtime_id, authority);
        self.clear_runtime_caches(
            &effective_runtime_id,
            Some(&owner_lease_id),
        );

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
        let mut remaining_pending = Vec::new();

        // Always attempt the current lease first, then every superseded lease.
        // A later retry repeats successful revocations idempotently, which is
        // important when one of the independent lane cleanups fails.
        let mut lease_ids = Vec::with_capacity(1 + pending_owner_cleanup.len());
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
                        remaining_pending.push(lease_id);
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
                // its cleanup failed; superseded leases are retained here.
                // The next retry will attempt the current lease again.
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

    fn clear_runtime_caches(
        &self,
        runtime_instance_id: &str,
        owner_lease_id: Option<&OwnerLeaseId>,
    ) {
        self.pending
            .lock()
            .expect("gateway browser pending store poisoned")
            .retain(|_, pending| {
                pending.runtime_instance_id != runtime_instance_id
                    && owner_lease_id
                        .is_none_or(|lease_id| &pending.owner_lease_id != lease_id)
            });
        self.observations
            .lock()
            .expect("gateway browser observation cache poisoned")
            .retain(|lane_key, _| {
                lane_key.runtime_instance_id != runtime_instance_id
            });
    }

    fn clear_owner_lease_caches(&self, owner_lease_id: &OwnerLeaseId) {
        self.pending
            .lock()
            .expect("gateway browser pending store poisoned")
            .retain(|_, pending| &pending.owner_lease_id != owner_lease_id);
    }

    async fn retry_owner_cleanup(
        &self,
        runtime_instance_id: &str,
        current_owner_lease_id: &OwnerLeaseId,
        mut pending_owner_cleanup: Vec<OwnerLeaseId>,
        hub: &BrowserSessionHub,
    ) -> Result<(), BrowserPlatformError> {
        let mut first_error = None;
        let mut remaining = Vec::new();
        for owner_lease_id in pending_owner_cleanup.drain(..) {
            match hub.revoke_owner_lease(&owner_lease_id).await {
                Ok(_) => {}
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                    remaining.push(owner_lease_id);
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
        if let Some(error) = first_error {
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
        pending_owner_cleanup: Vec<OwnerLeaseId>,
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
    /// This covers both signed-child and Remote MCP authorities. It is called
    /// from the Gateway lifecycle sweep because Remote MCP sessions are not
    /// represented by the signed-child capability issuer.
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
                if cached.revocation_pending || !cached.pending_owner_cleanup.is_empty() {
                    Some((
                        runtime_id.clone(),
                        cached.authority,
                        cached.revocation_pending,
                        cached.identity.owner_lease_id.clone(),
                        cached.pending_owner_cleanup.clone(),
                    ))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        futures::future::join_all(pending.into_iter().map(
            |(
                runtime_id,
                authority,
                revocation_pending,
                current_owner,
                pending_owner_cleanup,
            )| {
                let hub = hub.clone();
                async move {
                    if revocation_pending {
                        if let Err(error) = self
                            .revoke_identity_for_authority(
                                &runtime_id,
                                authority,
                            )
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
        ))
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
            .filter(|cached| {
                cached.authority == BrowserAttachmentAuthority::SignedChild
            })
            .fold(BrowserCleanupStatus::default(), |mut status, cached| {
                status.pending_attachments =
                    status.pending_attachments.saturating_add(1);
                status.pending_owner_leases = status
                    .pending_owner_leases
                    .saturating_add(1 + cached.pending_owner_cleanup.len());
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
            .filter(|(_, cached)| {
                cached.authority == BrowserAttachmentAuthority::SignedChild
            })
            .map(|(runtime_id, _)| runtime_id.clone())
            .collect::<Vec<_>>();

        let outcomes = futures::future::join_all(runtime_ids.into_iter().map(
            |runtime_id| async move {
                self.revoke_signed_child_lease(&runtime_id).await
            },
        ))
        .await;
        let mut first_retryable_error = None;
        let mut first_terminal_error = None;
        for outcome in outcomes {
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
    /// Cached identity, approvals, and observations are removed before this
    /// returns. A repeated revoke is a successful no-op and never affects
    /// another child runtime.
    pub async fn revoke_signed_child_lease(
        &self,
        signed_child_lease_id: &str,
    ) -> Result<CloseResult, BrowserPlatformError> {
        self.revoke_identity_for_authority(
            signed_child_lease_id,
            BrowserAttachmentAuthority::SignedChild,
        )
        .await
    }

    /// Reconcile cached browser owners with the process-local signed
    /// capability registry.
    ///
    /// The final in-process `LoopbackCapabilityLease` guard can revoke an
    /// issuer lease without an HTTP revoke request (for example when an ACP
    /// child crashes or its runtime is dropped). The Gateway server calls this
    /// periodically so those owners and their Lane state do not wait for the
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
            .filter(|(runtime_id, cached)| {
                cached.authority == BrowserAttachmentAuthority::SignedChild
                    && !is_active(runtime_id)
            })
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
                Some(&resolved.owner.lane_key.lane_name),
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
        let kind = operation.kind;
        let result = resolved.client.execute(&lane.lane_id, operation).await?;
        if kind == BrowserOperationKind::Observe
            && let Some(text) = result_text(&result.output)
        {
            self.observations
                .lock()
                .expect("gateway browser observation cache poisoned")
                .insert(resolved.owner.lane_key, text.to_owned());
        }
        Ok(result)
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
        if action == "observe" && !result.is_error {
            self.cache_managed_observation(caller, legacy_lane_name, &input, &result);
        }
        Ok(result)
    }

    /// Populate the GW2 observation cache from the shared managed dispatch
    /// path so [`Self::classify`] resolves submit-control accnames against the
    /// same snapshot the model received. Best-effort: a snapshot that cannot
    /// be attributed to an exact owned lane is dropped, never mis-filed.
    fn cache_managed_observation(
        &self,
        caller: &CallerCtx,
        legacy_lane_name: Option<&str>,
        input: &Value,
        result: &ToolResult,
    ) {
        let Ok(envelope) = serde_json::from_str::<Value>(&result.content) else {
            return;
        };
        let Some(text) = envelope
            .get("output")
            .and_then(result_text)
            .or_else(|| result_text(&envelope))
        else {
            return;
        };
        let lane_name = match envelope
            .pointer("/lane/lane_name")
            .and_then(Value::as_str)
            .or(legacy_lane_name)
        {
            Some(lane_name) => Some(lane_name),
            // Without an authoritative lane name a lane_id-addressed snapshot
            // must not be attributed to the default lane.
            None if input.get("lane_id").is_some_and(|v| !v.is_null()) => return,
            None => None,
        };
        let Ok(identity) = self.identity_resolver.resolve(caller) else {
            return;
        };
        let Ok(lane_key) = LaneKey::new(identity.runtime_instance_id, lane_name)
        else {
            return;
        };
        self.observations
            .lock()
            .expect("gateway browser observation cache poisoned")
            .insert(lane_key, text.to_owned());
    }

    /// Resolve a caller-provided selector to this trusted runtime's logical
    /// Lane name for GW2 approval ownership. An unowned lane_id fails at the
    /// bound Hub authorization check.
    pub async fn resolve_lane_selector(
        &self,
        caller: &CallerCtx,
        legacy_lane_name: Option<&str>,
        lane_id: Option<&str>,
    ) -> Result<String, BrowserPlatformError> {
        if legacy_lane_name.is_some() && lane_id.is_some() {
            return Err(BrowserPlatformError::new(
                BrowserErrorCode::OperationNotAllowed,
                "Use either legacy `lane` or `lane_id`, not both.",
                false,
                "Keep lane_id and remove the legacy lane name.",
            ));
        }
        if let Some(lane_id) = lane_id {
            let lane_id = BrowserLaneId::parse(lane_id.to_owned())?;
            let resolved = self.resolve(caller, None)?;
            return resolved
                .client
                .status(&lane_id)
                .await
                .map(|lane| lane.lane_key.lane_name);
        }
        Ok(self
            .resolve(caller, legacy_lane_name)?
            .owner
            .lane_key
            .lane_name)
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

    /// Classify for GW2 without creating an engine. Runtime semantics from the
    /// last observation are reconstructed from the hub result text.
    pub fn classify(
        &self,
        caller: &CallerCtx,
        lane_name: Option<&str>,
        action: &str,
        input: &Value,
    ) -> Result<ApprovalTier, BrowserPlatformError> {
        reject_untrusted_caller_fields(input)?;
        let resolved = self.resolve(caller, lane_name)?;
        let observation = self
            .observations
            .lock()
            .expect("gateway browser observation cache poisoned")
            .get(&resolved.owner.lane_key)
            .cloned();
        Ok(classify_with_observation(action, input, observation.as_deref()))
    }

    /// Stash a sanitized irreversible action. Ownership is the trusted runtime,
    /// lane, user, and lease — never a companion id.
    pub fn stash_pending(
        &self,
        caller: &CallerCtx,
        lane_name: Option<&str>,
        input: Value,
    ) -> Result<Option<String>, BrowserPlatformError> {
        reject_untrusted_caller_fields(&input)?;
        let resolved = self.resolve(caller, lane_name)?;
        let call_id = nomifun_common::generate_id();
        let mut pending = self
            .pending
            .lock()
            .expect("gateway browser pending store poisoned");
        if pending.len() >= MAX_PENDING {
            return Ok(None);
        }
        pending.insert(
            call_id.clone(),
            PendingBrowserAction {
                input,
                lane_name: resolved.owner.lane_key.lane_name,
                runtime_instance_id: resolved.owner.lane_key.runtime_instance_id,
                owner_lease_id: resolved.owner.owner_lease_id,
                user_id: resolved.owner.user_id,
            },
        );
        Ok(Some(call_id))
    }

    /// Atomically consume a pending decision only if the current trusted caller
    /// owns it. A mismatched caller cannot consume another runtime's decision.
    pub fn take_pending_for(
        &self,
        caller: &CallerCtx,
        call_id: &str,
    ) -> Result<Option<PendingBrowserAction>, BrowserPlatformError> {
        let pending = {
            self.pending
                .lock()
                .expect("gateway browser pending store poisoned")
                .get(call_id)
                .cloned()
        };
        let Some(pending) = pending else {
            return Ok(None);
        };
        let resolved = self.resolve(caller, Some(&pending.lane_name))?;
        if resolved.owner != pending.owner() {
            return Err(BrowserPlatformError::new(
                BrowserErrorCode::OperationNotAllowed,
                "This browser approval belongs to another runtime.",
                false,
                "Resolve it from the runtime that requested the action.",
            ));
        }
        Ok(self
            .pending
            .lock()
            .expect("gateway browser pending store poisoned")
            .remove(call_id))
    }

    pub fn pending_count(&self) -> usize {
        self.pending
            .lock()
            .expect("gateway browser pending store poisoned")
            .len()
    }

    /// Execute an approved action through the Hub's Rust-only confirmation
    /// seam. No confirmation bit is ever copied from model JSON.
    pub async fn execute_confirmed(
        &self,
        caller: &CallerCtx,
        pending: PendingBrowserAction,
    ) -> Result<BrowserOperationResult, BrowserPlatformError> {
        let resolved = self.resolve(caller, Some(&pending.lane_name))?;
        if resolved.owner != pending.owner() {
            return Err(BrowserPlatformError::new(
                BrowserErrorCode::OperationNotAllowed,
                "This browser approval belongs to another runtime.",
                false,
                "Resolve it from the runtime that requested the action.",
            ));
        }
        reject_untrusted_caller_fields(&pending.input)?;
        let lane = self
            .open(caller, Some(&pending.lane_name))
            .await?;
        let resolved = self.resolve(caller, Some(&pending.lane_name))?;
        let operation = operation_from_input(&pending.input)?;
        resolved
            .client
            .execute_confirmed(&lane.lane_id, operation)
            .await
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
        let lane_key = LaneKey::new(
            identity.runtime_instance_id.clone(),
            lane_name,
        )?;
        let owner = PendingOwner {
            user_id: identity.user_id.clone(),
            lane_key,
            owner_lease_id: identity.owner_lease_id.clone(),
        };
        let client = hub.bind(identity)?;
        Ok(ResolvedBrowserCaller { client, owner })
    }
}

struct ResolvedBrowserCaller {
    client: BrowserLaneClient,
    owner: PendingOwner,
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

fn classify_with_observation(
    action: &str,
    input: &Value,
    observation: Option<&str>,
) -> ApprovalTier {
    let mut context = ActionContext::default();
    if input
        .get("method")
        .and_then(Value::as_str)
        .is_some_and(|method| method.eq_ignore_ascii_case("post"))
    {
        context.is_cross_origin_post = true;
    }
    if action == "press_key"
        && input
            .get("keys")
            .or_else(|| input.get("key"))
            .and_then(Value::as_str)
            .is_some_and(|keys| {
                keys.split('+')
                    .any(|key| key.trim().eq_ignore_ascii_case("enter"))
            })
    {
        // The gateway cannot synchronously inspect focus. Holding Enter for
        // explicit approval is the safe compatibility behavior.
        context.enter_submits_form = true;
    }
    if action == "click"
        && let Some(reference) = input.get("ref").and_then(Value::as_str)
        && let Some(line) = observation.and_then(|text| observation_line(text, reference))
    {
        context.element_accname = Some(line.to_owned());
        let lower = line.to_ascii_lowercase();
        context.is_submit_control = lower.contains("submit")
            && (lower.contains("button") || lower.contains("input"));
    }
    classify_action(action, &context)
}

fn observation_line<'a>(text: &'a str, reference: &str) -> Option<&'a str> {
    let marker = format!("[ref={reference}]");
    text.lines().find(|line| line.contains(&marker))
}

fn result_text(output: &Value) -> Option<&str> {
    output
        .as_str()
        .or_else(|| output.get("text").and_then(Value::as_str))
        .or_else(|| output.get("yaml").and_then(Value::as_str))
        .or_else(|| output.get("message").and_then(Value::as_str))
        .or_else(|| output.pointer("/result/text").and_then(Value::as_str))
        .or_else(|| output.get("content").and_then(Value::as_str))
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
        BrowserHostDriver, BrowserHostFactory, BrowserHostId,
        BrowserLaneDriver, BrowserLaneId, DriverOperationContext, HostLaunchRequest,
        HostLifecycleState, HubConfig, LaneLaunchRequest,
    };
    use tokio::sync::{Notify, Semaphore};

    use super::*;

    struct Probe {
        active: AtomicUsize,
        maximum: AtomicUsize,
        entered: AtomicUsize,
        confirmed: AtomicUsize,
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
                confirmed: AtomicUsize::new(0),
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
            context: DriverOperationContext,
        ) -> Result<BrowserOperationResult, BrowserPlatformError> {
            if context.trusted_out_of_band_confirmation {
                self.probe.confirmed.fetch_add(1, Ordering::AcqRel);
            }
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
        let probe = Probe::new();
        let factory = Arc::new(FakeFactory {
            launches: AtomicUsize::new(0),
            probe: Arc::clone(&probe),
        });
        let mut config = HubConfig::default();
        config.owner_lease_ttl_ms = owner_lease_ttl_ms;
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
        let user_id =
            nomifun_common::UserId::parse("0190f5fe-7c00-7a00-8000-000000000001")
                .unwrap();
        let conversation_id = nomifun_common::ConversationId::parse(
            "0190f5fe-7c00-7a00-8abc-012345678901",
        )
        .unwrap();
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
    async fn expired_cached_owner_is_reissued_for_the_same_live_runtime() {
        let harness = harness_with_owner_ttl(10);
        let mut first = gateway_caller_without_browser_identity();
        harness
            .registry
            .attach_trusted_identity_with_authority(
                &mut first,
                "remote-session-live",
                None,
                u64::MAX,
                BrowserAttachmentAuthority::RemoteMcpSession,
            )
            .await
            .unwrap();
        let old_owner = first
            .browser_identity
            .as_ref()
            .unwrap()
            .owner_lease_id
            .clone();
        let stale_lane = harness.registry.open(&first, None).await.unwrap();

        tokio::time::sleep(Duration::from_millis(20)).await;

        let mut resumed = gateway_caller_without_browser_identity();
        harness
            .registry
            .attach_trusted_identity_with_authority(
                &mut resumed,
                "remote-session-live",
                None,
                u64::MAX,
                BrowserAttachmentAuthority::RemoteMcpSession,
            )
            .await
            .unwrap();
        let new_owner = resumed
            .browser_identity
            .as_ref()
            .unwrap()
            .owner_lease_id
            .clone();
        assert_ne!(
            old_owner, new_owner,
            "an expired cached owner must be replaced, not returned as stale authority"
        );
        let replacement_lane = harness.registry.open(&resumed, None).await.unwrap();
        assert_ne!(stale_lane.lane_id, replacement_lane.lane_id);

        harness.hub.sweep().await.unwrap();
        let lanes = harness.hub.list_lanes().await;
        assert_eq!(lanes.len(), 1);
        assert_eq!(lanes[0].lane_id, replacement_lane.lane_id);
        assert_eq!(lanes[0].caller.owner_lease_id, new_owner);
    }

    #[tokio::test]
    async fn expired_owner_replacement_cannot_broaden_scope_or_inherit_pending_approval() {
        let harness = harness_with_owner_ttl(10);
        let mut first = gateway_caller_without_browser_identity();
        first.remote = true;
        harness
            .registry
            .attach_trusted_identity_scoped(
                &mut first,
                "remote-session-narrow-replacement",
                None,
                u64::MAX,
                BrowserAttachmentAuthority::RemoteMcpSession,
                BTreeSet::from([
                    BrowserOperationKind::Manage,
                    BrowserOperationKind::Observe,
                ]),
            )
            .await
            .unwrap();
        let old_identity = first.browser_identity.clone().unwrap();
        let old_lane = harness.registry.open(&first, None).await.unwrap();
        let call_id = harness
            .registry
            .stash_pending(
                &first,
                None,
                json!({ "action": "press_key", "keys": "Enter" }),
            )
            .unwrap()
            .unwrap();

        tokio::time::sleep(Duration::from_millis(20)).await;

        let mut replacement = gateway_caller_without_browser_identity();
        replacement.remote = true;
        harness
            .registry
            .attach_trusted_identity_scoped(
                &mut replacement,
                "remote-session-narrow-replacement",
                None,
                u64::MAX,
                BrowserAttachmentAuthority::RemoteMcpSession,
                BTreeSet::from([
                    BrowserOperationKind::Manage,
                    BrowserOperationKind::Observe,
                    BrowserOperationKind::Act,
                ]),
            )
            .await
            .unwrap();
        let replacement_identity = replacement.browser_identity.clone().unwrap();

        assert_eq!(replacement_identity.surface, BrowserSurface::Remote);
        assert_eq!(
            replacement_identity.allowed_operations,
            BTreeSet::from([
                BrowserOperationKind::Manage,
                BrowserOperationKind::Observe,
            ])
        );
        assert_ne!(
            replacement_identity.owner_lease_id,
            old_identity.owner_lease_id
        );
        assert_eq!(
            harness.registry.pending_count(),
            0,
            "pending approval from the superseded owner lease must be discarded"
        );
        assert!(
            harness
                .registry
                .take_pending_for(&replacement, &call_id)
                .unwrap()
                .is_none()
        );
        let replacement_lane = harness.registry.open(&replacement, None).await.unwrap();
        assert_ne!(replacement_lane.lane_id, old_lane.lane_id);
        let error = harness
            .registry
            .open(&first, None)
            .await
            .unwrap_err();
        assert_eq!(error.code, BrowserErrorCode::OwnerLeaseExpired);
    }

    #[tokio::test]
    async fn live_owner_renewal_persists_scope_narrowing() {
        let harness = harness();
        let mut broad = gateway_caller_without_browser_identity();
        broad.remote = true;
        harness
            .registry
            .attach_trusted_identity_scoped(
                &mut broad,
                "remote-session-live-narrowing",
                None,
                u64::MAX,
                BrowserAttachmentAuthority::RemoteMcpSession,
                BTreeSet::from([
                    BrowserOperationKind::Manage,
                    BrowserOperationKind::Observe,
                    BrowserOperationKind::Act,
                ]),
            )
            .await
            .unwrap();
        let owner_lease_id = broad
            .browser_identity
            .as_ref()
            .unwrap()
            .owner_lease_id
            .clone();

        let mut narrow = gateway_caller_without_browser_identity();
        narrow.remote = true;
        harness
            .registry
            .attach_trusted_identity_scoped(
                &mut narrow,
                "remote-session-live-narrowing",
                None,
                u64::MAX,
                BrowserAttachmentAuthority::RemoteMcpSession,
                BTreeSet::from([
                    BrowserOperationKind::Manage,
                    BrowserOperationKind::Observe,
                ]),
            )
            .await
            .unwrap();
        assert_eq!(
            narrow
                .browser_identity
                .as_ref()
                .unwrap()
                .owner_lease_id,
            owner_lease_id
        );

        let mut attempted_broaden = gateway_caller_without_browser_identity();
        attempted_broaden.remote = true;
        harness
            .registry
            .attach_trusted_identity_scoped(
                &mut attempted_broaden,
                "remote-session-live-narrowing",
                None,
                u64::MAX,
                BrowserAttachmentAuthority::RemoteMcpSession,
                BTreeSet::from([
                    BrowserOperationKind::Manage,
                    BrowserOperationKind::Observe,
                    BrowserOperationKind::Act,
                ]),
            )
            .await
            .unwrap();
        assert_eq!(
            attempted_broaden
                .browser_identity
                .unwrap()
                .allowed_operations,
            BTreeSet::from([
                BrowserOperationKind::Manage,
                BrowserOperationKind::Observe,
            ])
        );
    }

    #[tokio::test]
    async fn signed_child_reconciliation_ignores_remote_mcp_attachments() {
        let harness = harness();
        let mut signed = gateway_caller_without_browser_identity();
        harness
            .registry
            .attach_trusted_identity(
                &mut signed,
                "signed-child-inactive",
                Some("attempt-inactive"),
                u64::MAX,
            )
            .await
            .unwrap();
        let mut remote = gateway_caller_without_browser_identity();
        remote.remote = true;
        harness
            .registry
            .attach_trusted_identity_with_authority(
                &mut remote,
                "remote-session-active",
                None,
                u64::MAX,
                BrowserAttachmentAuthority::RemoteMcpSession,
            )
            .await
            .unwrap();

        harness.registry.open(&signed, None).await.unwrap();
        let remote_lane = harness.registry.open(&remote, None).await.unwrap();
        harness
            .registry
            .cleanup_inactive_signed_child_leases(|_| false)
            .await;

        let lanes = harness.hub.list_lanes().await;
        assert_eq!(lanes.len(), 1);
        assert_eq!(lanes[0].lane_id, remote_lane.lane_id);
        let identities = harness
            .registry
            .identities
            .lock()
            .expect("gateway browser identity cache poisoned");
        assert!(!identities.contains_key("signed-child-inactive"));
        assert!(identities.contains_key("remote-session-active"));
        assert_eq!(
            identities["remote-session-active"].authority,
            BrowserAttachmentAuthority::RemoteMcpSession
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
        let mut remote = gateway_caller_without_browser_identity();
        remote.remote = true;
        harness
            .registry
            .attach_trusted_identity_with_authority(
                &mut remote,
                "remote-session-final-sibling",
                None,
                u64::MAX,
                BrowserAttachmentAuthority::RemoteMcpSession,
            )
            .await
            .unwrap();
        let remote_lane = harness.registry.open(&remote, None).await.unwrap();
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
        assert_eq!(lanes[0].lane_id, remote_lane.lane_id);
    }

    #[tokio::test]
    async fn final_signed_child_drain_does_not_consume_remote_authority() {
        let harness = harness();
        let mut remote = gateway_caller_without_browser_identity();
        remote.remote = true;
        harness
            .registry
            .attach_trusted_identity_with_authority(
                &mut remote,
                "remote-session-survives-gateway-drain",
                None,
                u64::MAX,
                BrowserAttachmentAuthority::RemoteMcpSession,
            )
            .await
            .unwrap();
        let remote_lane = harness.registry.open(&remote, None).await.unwrap();

        harness
            .registry
            .drain_signed_child_browser_owners_once()
            .await
            .expect("an empty signed-child postcondition must succeed");
        assert!(harness.registry.signed_child_cleanup_status().is_empty());
        let lanes = harness.hub.list_lanes().await;
        assert_eq!(lanes.len(), 1);
        assert_eq!(lanes[0].lane_id, remote_lane.lane_id);
    }

    #[tokio::test]
    async fn remote_mcp_revoke_is_exact_and_idempotent() {
        let harness = harness();
        let mut remote = gateway_caller_without_browser_identity();
        remote.remote = true;
        harness
            .registry
            .attach_trusted_identity_with_authority(
                &mut remote,
                "remote-session-close",
                None,
                u64::MAX,
                BrowserAttachmentAuthority::RemoteMcpSession,
            )
            .await
            .unwrap();
        harness.registry.open(&remote, None).await.unwrap();

        let first = harness
            .registry
            .revoke_trusted_identity("remote-session-close")
            .await
            .unwrap();
        assert_eq!(first.closed, 1);
        assert!(!first.already_closed);
        assert!(harness.hub.list_lanes().await.is_empty());

        let repeated = harness
            .registry
            .revoke_trusted_identity("remote-session-close")
            .await
            .unwrap();
        assert_eq!(repeated.closed, 0);
        assert!(repeated.already_closed);
    }

    #[tokio::test]
    async fn failed_remote_mcp_revoke_remains_authoritative_until_retry() {
        // A terminal lane-cleanup failure on a host with no surviving lanes is
        // resolved by authoritative host retirement: the revoke succeeds on
        // its first attempt and no retained authority survives.
        let harness = harness();
        let mut remote = gateway_caller_without_browser_identity();
        remote.remote = true;
        harness
            .registry
            .attach_trusted_identity_with_authority(
                &mut remote,
                "remote-session-retired",
                None,
                u64::MAX,
                BrowserAttachmentAuthority::RemoteMcpSession,
            )
            .await
            .unwrap();
        harness.registry.open(&remote, None).await.unwrap();
        harness
            .probe
            .lane_close_failures_remaining
            .store(1, Ordering::Release);
        let result = harness
            .registry
            .revoke_trusted_identity("remote-session-retired")
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
        let mut remote = gateway_caller_without_browser_identity();
        remote.remote = true;
        harness
            .registry
            .attach_trusted_identity_with_authority(
                &mut remote,
                "remote-session-retry",
                None,
                u64::MAX,
                BrowserAttachmentAuthority::RemoteMcpSession,
            )
            .await
            .unwrap();
        harness.registry.open(&remote, None).await.unwrap();
        let mut sibling = gateway_caller_without_browser_identity();
        sibling.remote = true;
        harness
            .registry
            .attach_trusted_identity_with_authority(
                &mut sibling,
                "remote-session-retry-sibling",
                None,
                u64::MAX,
                BrowserAttachmentAuthority::RemoteMcpSession,
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
            .revoke_trusted_identity("remote-session-retry")
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
                .get("remote-session-retry")
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
                .contains_key("remote-session-retry")
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
    async fn final_revoke_cleans_replacement_without_losing_superseded_owner() {
        // A terminal lane-cleanup failure on a host with no surviving lanes is
        // resolved by authoritative host retirement, so the replacement attach
        // consumes the superseded owner immediately.
        let harness = harness_with_owner_ttl(10);
        let mut first = gateway_caller_without_browser_identity();
        first.remote = true;
        harness
            .registry
            .attach_trusted_identity_with_authority(
                &mut first,
                "remote-session-retired-owner",
                None,
                u64::MAX,
                BrowserAttachmentAuthority::RemoteMcpSession,
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
        replacement.remote = true;
        harness
            .registry
            .attach_trusted_identity_with_authority(
                &mut replacement,
                "remote-session-retired-owner",
                None,
                u64::MAX,
                BrowserAttachmentAuthority::RemoteMcpSession,
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
                .get("remote-session-retired-owner")
                .expect("replacement authority must be published");
            assert!(
                cached.pending_owner_cleanup.is_empty(),
                "host retirement resolves the superseded-owner cleanup failure"
            );
            assert_ne!(cached.identity.owner_lease_id, old_owner);
        }
        assert!(harness.hub.list_lanes().await.is_empty());
        assert_eq!(harness.probe.lane_closes.load(Ordering::Acquire), 1);

        // A sibling lane on the shared Primary host makes retirement
        // impossible, so the superseded owner is retained for retry without
        // losing the replacement.
        let harness = harness_with_owner_ttl(10);
        let mut first = gateway_caller_without_browser_identity();
        first.remote = true;
        harness
            .registry
            .attach_trusted_identity_with_authority(
                &mut first,
                "remote-session-superseded",
                None,
                u64::MAX,
                BrowserAttachmentAuthority::RemoteMcpSession,
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
        sibling.remote = true;
        harness
            .registry
            .attach_trusted_identity_with_authority(
                &mut sibling,
                "remote-session-superseded-sibling",
                None,
                u64::MAX,
                BrowserAttachmentAuthority::RemoteMcpSession,
            )
            .await
            .unwrap();
        let sibling_lane = harness.registry.open(&sibling, None).await.unwrap();
        harness
            .probe
            .lane_close_failures_remaining
            .store(1, Ordering::Release);

        let mut replacement = gateway_caller_without_browser_identity();
        replacement.remote = true;
        harness
            .registry
            .attach_trusted_identity_with_authority(
                &mut replacement,
                "remote-session-superseded",
                None,
                u64::MAX,
                BrowserAttachmentAuthority::RemoteMcpSession,
            )
            .await
            .expect(
                "replacement succeeds once the new identity is published; old cleanup remains retryable",
            );

        let replacement_identity = {
            let identities = harness
                .registry
                .identities
                .lock()
                .expect("gateway browser identity cache poisoned");
            let cached = identities
                .get("remote-session-superseded")
                .expect("replacement authority must survive old-owner cleanup failure");
            assert_eq!(cached.pending_owner_cleanup, vec![old_owner.clone()]);
            assert_ne!(cached.identity.owner_lease_id, old_owner);
            cached.identity.clone()
        };
        assert_eq!(
            replacement
                .browser_identity
                .as_ref()
                .expect("successful replacement must publish the new identity")
                .owner_lease_id,
            replacement_identity.owner_lease_id
        );
        let replacement_lane = harness.registry.open(&replacement, None).await.unwrap();

        let result = harness
            .registry
            .revoke_trusted_identity("remote-session-superseded")
            .await
            .unwrap();
        assert_eq!(result.closed, 1);
        assert!(
            !harness
                .registry
                .identities
                .lock()
                .expect("gateway browser identity cache poisoned")
                .contains_key("remote-session-superseded")
        );

        // The final revoke consumed both the replacement lane and the retained
        // superseded-owner cleanup; only the sibling lane survives.
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
        caller.remote = true;
        harness
            .registry
            .attach_trusted_identity_scoped(
                &mut caller,
                "remote-session-observe-only",
                None,
                u64::MAX,
                BrowserAttachmentAuthority::RemoteMcpSession,
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
        harness
            .registry
            .stash_pending(
                &first,
                None,
                json!({ "action": "press_key", "keys": "Enter" }),
            )
            .unwrap()
            .unwrap();
        harness
            .registry
            .stash_pending(
                &second,
                None,
                json!({ "action": "press_key", "keys": "Enter" }),
            )
            .unwrap()
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
        let pending = harness
            .registry
            .pending
            .lock()
            .expect("gateway browser pending store poisoned");
        assert_eq!(pending.len(), 1);
        assert!(pending
            .values()
            .all(|action| action.runtime_instance_id == "signed-child-lease-b"));
        drop(pending);
        let observations = harness
            .registry
            .observations
            .lock()
            .expect("gateway browser observation cache poisoned");
        assert_eq!(observations.len(), 1);
        assert!(observations
            .keys()
            .all(|key| key.runtime_instance_id == "signed-child-lease-b"));
        drop(observations);

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
    async fn managed_observe_feeds_gw2_classification_for_submit_clicks() {
        let harness = harness();
        let caller = caller(&harness.hub, "runtime-observe-cache", "attempt-a");
        let observed = harness
            .registry
            .dispatch_managed(&caller, None, json!({ "action": "observe" }))
            .await
            .unwrap();
        assert!(!observed.is_error, "{}", observed.content);

        let dangerous = harness
            .registry
            .classify(&caller, None, "click", &json!({ "ref": "f0e7" }))
            .unwrap();
        assert_eq!(
            dangerous,
            ApprovalTier::Irreversible,
            "a click on an observed Pay button must be held for approval"
        );
        let benign = harness
            .registry
            .classify(&caller, None, "click", &json!({ "ref": "f0a1" }))
            .unwrap();
        assert_ne!(benign, ApprovalTier::Irreversible);
    }

    #[tokio::test]
    async fn revoked_runtime_tombstones_are_bounded_with_insertion_order_eviction() {
        let harness = harness();
        for index in 0..=REVOKED_RUNTIME_TOMBSTONE_CAPACITY {
            harness
                .registry
                .revoke_trusted_identity(&format!("remote-tombstone-{index}"))
                .await
                .unwrap();
        }

        // Recent revocations keep their anti-resurrection authority.
        let mut newest = gateway_caller_without_browser_identity();
        newest.remote = true;
        let error = harness
            .registry
            .attach_trusted_identity_with_authority(
                &mut newest,
                &format!(
                    "remote-tombstone-{REVOKED_RUNTIME_TOMBSTONE_CAPACITY}"
                ),
                None,
                u64::MAX,
                BrowserAttachmentAuthority::RemoteMcpSession,
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, BrowserErrorCode::OwnerLeaseExpired);

        // The oldest tombstone is evicted instead of retained forever.
        let mut oldest = gateway_caller_without_browser_identity();
        oldest.remote = true;
        harness
            .registry
            .attach_trusted_identity_with_authority(
                &mut oldest,
                "remote-tombstone-0",
                None,
                u64::MAX,
                BrowserAttachmentAuthority::RemoteMcpSession,
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
    fn pending_approval_is_bound_to_runtime_not_companion() {
        let harness = harness();
        let first = caller(&harness.hub, "runtime-owner-a", "attempt-a");
        let second = caller(&harness.hub, "runtime-owner-b", "attempt-b");
        let call_id = harness
            .registry
            .stash_pending(
                &first,
                None,
                json!({ "action": "press_key", "keys": "Enter" }),
            )
            .unwrap()
            .unwrap();
        let error = harness
            .registry
            .take_pending_for(&second, &call_id)
            .unwrap_err();
        assert_eq!(error.code, BrowserErrorCode::OperationNotAllowed);
        assert_eq!(harness.registry.pending_count(), 1);
        assert!(harness
            .registry
            .take_pending_for(&first, &call_id)
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn approved_action_uses_only_the_hub_trusted_confirmation_seam() {
        let harness = harness();
        let caller = caller(&harness.hub, "runtime-confirmed", "attempt-a");
        let call_id = harness
            .registry
            .stash_pending(
                &caller,
                None,
                json!({ "action": "press_key", "keys": "Enter" }),
            )
            .unwrap()
            .unwrap();
        let pending = harness
            .registry
            .take_pending_for(&caller, &call_id)
            .unwrap()
            .unwrap();
        harness
            .registry
            .execute_confirmed(&caller, pending)
            .await
            .unwrap();
        assert_eq!(harness.probe.confirmed.load(Ordering::Acquire), 1);
    }

    #[test]
    fn classifier_preserves_enter_and_observed_dangerous_ref_behavior() {
        assert_eq!(
            classify_with_observation(
                "press_key",
                &json!({ "keys": "Enter" }),
                None,
            ),
            ApprovalTier::Irreversible
        );
        let observation = "- button \"Pay now\" [ref=f0e7]";
        assert!(nomi_browser::accname_is_irreversible(observation));
        assert_eq!(
            classify_with_observation(
                "click",
                &json!({ "ref": "f0e7" }),
                Some(observation),
            ),
            ApprovalTier::Irreversible
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

    #[tokio::test]
    async fn unattributable_lane_id_observation_is_dropped_not_misfiled() {
        let harness = harness();
        let caller = caller(&harness.hub, "runtime-observe-guard", "attempt-guard");
        // An observe result that lost its authoritative lane attribution,
        // e.g. because the post-operation status refresh failed and the
        // facade serialized `"lane": null`.
        let orphaned = ToolResult::text(
            json!({
                "ok": true,
                "action": "observe",
                "lane_id": "lane-unattributable",
                "lane": null,
                "output": { "text": "- button \"Pay now\" [ref=f0e7]" },
            })
            .to_string(),
        );

        harness.registry.cache_managed_observation(
            &caller,
            None,
            &json!({ "action": "observe", "lane_id": "lane-unattributable" }),
            &orphaned,
        );
        {
            let observations = harness
                .registry
                .observations
                .lock()
                .expect("gateway browser observation cache poisoned");
            assert!(
                observations.is_empty(),
                "a lane_id-addressed snapshot without authoritative lane \
                 attribution must be dropped, not filed under another lane"
            );
        }

        // Control: without the caller-supplied lane_id the identical result
        // legitimately attributes to the default lane, so the drop above is
        // the mis-attribution guard and not an unrelated parse failure.
        harness.registry.cache_managed_observation(
            &caller,
            None,
            &json!({ "action": "observe" }),
            &orphaned,
        );
        let observations = harness
            .registry
            .observations
            .lock()
            .expect("gateway browser observation cache poisoned");
        let default_lane = LaneKey::new("runtime-observe-guard", None).unwrap();
        assert_eq!(observations.len(), 1);
        assert_eq!(
            observations.get(&default_lane).map(String::as_str),
            Some("- button \"Pay now\" [ref=f0e7]")
        );
    }

    #[test]
    fn gateway_rejects_every_shared_trusted_owner_field() {
        // F23: the gateway must enforce the ONE shared trusted-owner field
        // list (`nomi_browser::TRUSTED_OWNER_INPUT_FIELDS`). A divergent
        // gateway-local list would make identical requests behave differently
        // across supposedly-equivalent managed browser surfaces.
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
