//! Remote MCP session authority and lifecycle cleanup.
//!
//! rmcp owns the transport worker, but the Remote front door owns browser
//! attachment policy. This wrapper injects the server-generated logical
//! session id into every MCP request and revokes that exact browser owner when
//! rmcp closes the session (DELETE, idle timeout, or worker exit).

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
#[cfg(feature = "browser-use")]
use std::collections::HashSet;

use futures::{FutureExt, Stream};
use nomifun_common::UserId;
use nomifun_gateway::{GatewayDeps, Registry, Surface};
use rmcp::model::{ClientJsonRpcMessage, GetExtensions, ServerJsonRpcMessage};
use rmcp::transport::streamable_http_server::session::{
    RestoreOutcome, ServerSseMessage, SessionId, SessionManager,
    local::{
        LocalSessionManager, LocalSessionManagerError, LocalSessionWorker,
        create_local_session,
    },
};
use rmcp::transport::common::server_side_http::session_id;
use rmcp::transport::WorkerTransport;
use thiserror::Error;

/// Trusted marker copied into the request context by the session manager.
///
/// Its value is generated and validated by rmcp's server-side session map; a
/// client-supplied `Mcp-Session-Id` is accepted only after `has_session`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RemoteMcpSessionId(pub SessionId);

/// The server-pinned identity for one logical Remote MCP session.
///
/// This is deliberately copied into every message by [`RemoteSessionManager`].
/// The HTTP bearer token is checked on every request, and every later request
/// must still authenticate the installation owner that initialized the session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RemoteMcpSessionIdentity {
    pub session_id: SessionId,
    pub owner_user_id: UserId,
    /// The capability-domain scope selected during `initialize`.
    ///
    /// `None` means the full Remote catalog. This is server-pinned and is
    /// never re-derived from a later request URI.
    pub scope: Option<Vec<String>>,
}

#[derive(Clone, Debug)]
struct RemoteMcpSessionBinding {
    owner_user_id: UserId,
    scope: Option<Vec<String>>,
    request_budget: Arc<RemoteHttpRequestBudget>,
}

impl RemoteMcpSessionBinding {
    fn new(
        owner_user_id: UserId,
        scope: Option<Vec<String>>,
        request_limit: usize,
    ) -> Self {
        Self {
            owner_user_id,
            scope,
            request_budget: RemoteHttpRequestBudget::new(request_limit),
        }
    }
}

const MAX_REMOTE_SCOPE_QUERY_BYTES: usize = 4 * 1024;
const MAX_REMOTE_SCOPE_DOMAINS: usize = 32;
const MAX_REMOTE_SCOPE_DOMAIN_BYTES: usize = 64;
const MAX_REMOTE_SCOPE_BYTES: usize = 1024;
const MAX_REMOTE_BINDING_RETAINED_BYTES: usize = 16 * 1024;
const REMOTE_SESSIONS_PER_LOGICAL_CPU: usize = 32;
const REMOTE_SESSION_ASSUMED_MEMORY_BYTES: u64 = 4 * 1024 * 1024;
const MIN_GLOBAL_REMOTE_SESSIONS: usize = 16;
const MAX_GLOBAL_REMOTE_SESSIONS: usize = 4096;
const MIN_ACTIVE_REMOTE_SESSIONS_PER_TENANT: usize = 16;
const MAX_ACTIVE_REMOTE_SESSIONS_PER_TENANT: usize = 128;
const REMOTE_SESSIONS_PER_TENANT_CPU: usize = 4;
const MIN_REMOTE_INITIALIZE_BURST_PER_TENANT: usize = 24;
const MAX_REMOTE_INFLIGHT_REQUESTS_PER_SESSION: usize = 8;
const MAX_REMOTE_HEADERLESS_REQUESTS_PER_TENANT: usize = 4;
const REMOTE_INITIALIZE_REFILL_INTERVAL: Duration = Duration::from_millis(500);
const REMOTE_RATE_BUCKET_IDLE_RETENTION: Duration = Duration::from_secs(60);
const REMOTE_PROVISIONAL_SESSION_TIMEOUT: Duration = Duration::from_secs(5);
const REMOTE_PROVISIONAL_SWEEP_INTERVAL: Duration = Duration::from_millis(500);
#[cfg(not(test))]
const REMOTE_PROVISIONAL_FINAL_RETRY_INITIAL_WAIT: Duration =
    Duration::from_millis(100);
#[cfg(not(test))]
const REMOTE_PROVISIONAL_FINAL_RETRY_MAX_WAIT: Duration =
    Duration::from_secs(2);
#[cfg(test)]
const REMOTE_PROVISIONAL_FINAL_RETRY_INITIAL_WAIT: Duration =
    Duration::from_millis(5);
#[cfg(test)]
const REMOTE_PROVISIONAL_FINAL_RETRY_MAX_WAIT: Duration =
    Duration::from_millis(25);
#[cfg(feature = "browser-use")]
const MAX_PENDING_REMOTE_BROWSER_CLEANUPS: usize = MAX_GLOBAL_REMOTE_SESSIONS;
#[cfg(feature = "browser-use")]
const MAX_PENDING_REMOTE_BROWSER_CLEANUP_BYTES: usize =
    MAX_PENDING_REMOTE_BROWSER_CLEANUPS * 128;
static NEXT_REMOTE_SESSION_MANAGER_ID: AtomicU64 = AtomicU64::new(1);

/// A fixed structural request gate. It bounds request/response bodies and
/// long-lived SSE responses retained by one logical Remote task without
/// imposing a fixed process-wide memory ceiling.
#[derive(Debug)]
struct RemoteHttpRequestBudget {
    active: AtomicUsize,
    limit: usize,
}

impl RemoteHttpRequestBudget {
    fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            active: AtomicUsize::new(0),
            limit: limit.max(1),
        })
    }

    fn try_acquire(self: &Arc<Self>) -> Option<RemoteHttpRequestPermit> {
        self.try_acquire_below(self.limit)
    }

    fn try_acquire_session(
        self: &Arc<Self>,
        teardown: bool,
    ) -> Option<RemoteHttpRequestPermit> {
        // Keep one of the eight structural slots available for DELETE so a
        // session saturated by long-lived SSE bodies can still be torn down.
        let admission_limit = if teardown {
            self.limit
        } else {
            self.limit.saturating_sub(1).max(1)
        };
        self.try_acquire_below(admission_limit)
    }

    fn try_acquire_below(
        self: &Arc<Self>,
        admission_limit: usize,
    ) -> Option<RemoteHttpRequestPermit> {
        self.active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < admission_limit).then(|| active + 1)
            })
            .ok()
            .map(|_| RemoteHttpRequestPermit {
                budget: Arc::clone(self),
            })
    }

    fn active(&self) -> usize {
        self.active.load(Ordering::Acquire)
    }
}

/// Exact RAII authority for one admitted Remote HTTP request. The router moves
/// this into the response body so streaming responses remain charged until
/// completion or disconnect; cancellation before then drops it normally.
#[derive(Debug)]
#[must_use = "dropping the permit immediately refunds the request slot"]
pub(crate) struct RemoteHttpRequestPermit {
    budget: Arc<RemoteHttpRequestBudget>,
}

impl Drop for RemoteHttpRequestPermit {
    fn drop(&mut self) {
        let previous = self.budget.active.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0, "Remote HTTP request budget underflow");
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RemoteHttpRequestAdmissionError {
    IdentityMismatch,
    CapacityExceeded,
}

#[derive(Clone, Copy, Debug)]
struct RemoteSessionLimits {
    max_active_global: usize,
    max_active_per_tenant: usize,
    max_retained_bytes_global: usize,
    max_retained_bytes_per_tenant: usize,
    max_rate_tenants_global: usize,
    max_binding_retained_bytes: usize,
    initialize_burst_per_tenant: u32,
    initialize_refill_interval: Duration,
    rate_bucket_idle_retention: Duration,
    max_inflight_requests_per_session: usize,
    max_headerless_requests_per_tenant: usize,
}

impl RemoteSessionLimits {
    fn for_machine() -> Self {
        let logical_cpus = std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1);
        let mut system = sysinfo::System::new();
        system.refresh_memory();
        Self::for_resources(system.total_memory(), logical_cpus)
    }

    fn for_resources(total_memory_bytes: u64, logical_cpus: usize) -> Self {
        let cpu_capacity = logical_cpus
            .max(1)
            .saturating_mul(REMOTE_SESSIONS_PER_LOGICAL_CPU);
        let memory_capacity = if total_memory_bytes == 0 {
            // Missing telemetry must remain conservative without imposing a
            // fixed process-wide byte ceiling on healthy machines.
            MIN_GLOBAL_REMOTE_SESSIONS
        } else {
            usize::try_from(
                total_memory_bytes / REMOTE_SESSION_ASSUMED_MEMORY_BYTES,
            )
            .unwrap_or(usize::MAX)
        };
        let max_active_global = cpu_capacity
            .min(memory_capacity)
            .clamp(
                MIN_GLOBAL_REMOTE_SESSIONS,
                MAX_GLOBAL_REMOTE_SESSIONS,
            );
        let max_active_per_tenant = logical_cpus
            .max(1)
            .saturating_mul(REMOTE_SESSIONS_PER_TENANT_CPU)
            .clamp(
                MIN_ACTIVE_REMOTE_SESSIONS_PER_TENANT,
                MAX_ACTIVE_REMOTE_SESSIONS_PER_TENANT,
            )
            .min(max_active_global);
        let initialize_burst_per_tenant = max_active_per_tenant
            .saturating_add(8)
            .max(MIN_REMOTE_INITIALIZE_BURST_PER_TENANT)
            .min(u32::MAX as usize) as u32;
        Self {
            max_active_global,
            max_active_per_tenant,
            max_retained_bytes_global: max_active_global
                .saturating_mul(MAX_REMOTE_BINDING_RETAINED_BYTES),
            max_retained_bytes_per_tenant: max_active_per_tenant
                .saturating_mul(MAX_REMOTE_BINDING_RETAINED_BYTES),
            // Inactive rate buckets are short-lived and hard bounded too. The
            // ceiling scales with machine capacity instead of becoming an
            // attacker-controlled principal-id map.
            max_rate_tenants_global: max_active_global.saturating_mul(2),
            max_binding_retained_bytes: MAX_REMOTE_BINDING_RETAINED_BYTES,
            initialize_burst_per_tenant,
            initialize_refill_interval:
                REMOTE_INITIALIZE_REFILL_INTERVAL,
            rate_bucket_idle_retention:
                REMOTE_RATE_BUCKET_IDLE_RETENTION,
            max_inflight_requests_per_session:
                MAX_REMOTE_INFLIGHT_REQUESTS_PER_SESSION,
            max_headerless_requests_per_tenant:
                MAX_REMOTE_HEADERLESS_REQUESTS_PER_TENANT,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct RemoteSessionTenantKey {
    authoritative_user_id: Arc<str>,
    principal_user_id: UserId,
}

#[derive(Clone, Debug)]
struct RemoteSessionReservation {
    manager_id: u64,
    tenant: RemoteSessionTenantKey,
    retained_bytes: usize,
}

#[derive(Clone, Copy, Debug)]
struct RemoteProvisionalSession {
    manager_id: u64,
    created_at: Instant,
    closing: bool,
}

#[derive(Clone, Debug)]
struct RemoteTenantAdmission {
    active_sessions: usize,
    retained_bytes: usize,
    initialize_tokens: u32,
    last_refill: Instant,
    last_activity: Instant,
    headerless_request_budget: Arc<RemoteHttpRequestBudget>,
}

impl RemoteTenantAdmission {
    fn new(now: Instant, burst: u32, headerless_request_limit: usize) -> Self {
        Self {
            active_sessions: 0,
            retained_bytes: 0,
            initialize_tokens: burst,
            last_refill: now,
            last_activity: now,
            headerless_request_budget: RemoteHttpRequestBudget::new(
                headerless_request_limit,
            ),
        }
    }

    fn refill(&mut self, now: Instant, limits: RemoteSessionLimits) {
        if self.initialize_tokens >= limits.initialize_burst_per_tenant
            || limits.initialize_refill_interval.is_zero()
        {
            self.initialize_tokens = limits.initialize_burst_per_tenant;
            self.last_refill = now;
            return;
        }
        let elapsed = now.saturating_duration_since(self.last_refill);
        let interval_nanos = limits.initialize_refill_interval.as_nanos();
        let refill = elapsed.as_nanos() / interval_nanos;
        if refill == 0 {
            return;
        }
        self.initialize_tokens = self
            .initialize_tokens
            .saturating_add(u32::try_from(refill).unwrap_or(u32::MAX))
            .min(limits.initialize_burst_per_tenant);
        self.last_refill = now;
    }
}

struct RemoteSessionAdmission {
    authoritative_user_id: Arc<str>,
    provisionals: HashMap<SessionId, RemoteProvisionalSession>,
    reservations: HashMap<SessionId, RemoteSessionReservation>,
    tenants: HashMap<RemoteSessionTenantKey, RemoteTenantAdmission>,
    total_retained_bytes: usize,
    limits: RemoteSessionLimits,
}

impl RemoteSessionAdmission {
    fn new(
        authoritative_user_id: Arc<str>,
        limits: RemoteSessionLimits,
    ) -> Self {
        Self {
            authoritative_user_id,
            provisionals: HashMap::new(),
            reservations: HashMap::new(),
            tenants: HashMap::new(),
            total_retained_bytes: 0,
            limits,
        }
    }

    fn tenant_key(
        &self,
        principal_user_id: &UserId,
    ) -> RemoteSessionTenantKey {
        RemoteSessionTenantKey {
            authoritative_user_id: Arc::clone(&self.authoritative_user_id),
            principal_user_id: principal_user_id.clone(),
        }
    }

    fn acquire_headerless_request(
        &mut self,
        principal_user_id: &UserId,
        now: Instant,
    ) -> Result<RemoteHttpRequestPermit, RemoteHttpRequestAdmissionError> {
        self.prune_idle_rate_buckets(now);
        let tenant_key = self.tenant_key(principal_user_id);
        if !self.tenants.contains_key(&tenant_key)
            && self.tenants.len() >= self.limits.max_rate_tenants_global
        {
            return Err(RemoteHttpRequestAdmissionError::CapacityExceeded);
        }
        let tenant = self.tenants.entry(tenant_key).or_insert_with(|| {
            RemoteTenantAdmission::new(
                now,
                self.limits.initialize_burst_per_tenant,
                self.limits.max_headerless_requests_per_tenant,
            )
        });
        tenant.last_activity = now;
        tenant
            .headerless_request_budget
            .try_acquire()
            .ok_or(RemoteHttpRequestAdmissionError::CapacityExceeded)
    }

    fn prune_idle_rate_buckets(&mut self, now: Instant) {
        let limits = self.limits;
        self.tenants.retain(|_, tenant| {
            tenant.refill(now, limits);
            tenant.active_sessions > 0
                || tenant.headerless_request_budget.active() > 0
                || tenant.initialize_tokens
                    < limits.initialize_burst_per_tenant
                || now.saturating_duration_since(tenant.last_activity)
                    < limits.rate_bucket_idle_retention
        });
    }

    fn reserve(
        &mut self,
        session_id: &SessionId,
        manager_id: u64,
        principal_user_id: &UserId,
        retained_bytes: usize,
        now: Instant,
    ) -> Result<(), std::io::Error> {
        if let Some(existing) = self.reservations.get(session_id) {
            let expected_tenant = self.tenant_key(principal_user_id);
            if existing.manager_id == manager_id
                && existing.tenant == expected_tenant
                && existing.retained_bytes == retained_bytes
            {
                return Ok(());
            }
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "Remote MCP session admission authority changed",
            ));
        }
        if self
            .provisionals
            .get(session_id)
            .is_none_or(|provisional| {
                provisional.manager_id != manager_id || provisional.closing
            })
        {
            return Err(remote_session_admission_error(
                "Remote MCP provisional session is unavailable",
            ));
        }
        if retained_bytes > self.limits.max_binding_retained_bytes {
            return Err(remote_session_admission_error(
                "Remote MCP session identity exceeds its retained-memory limit",
            ));
        }

        self.prune_idle_rate_buckets(now);
        let tenant_key = self.tenant_key(principal_user_id);
        if !self.tenants.contains_key(&tenant_key)
            && self.tenants.len() >= self.limits.max_rate_tenants_global
        {
            return Err(remote_session_admission_error(
                "Remote MCP session tenant capacity is temporarily exhausted",
            ));
        }

        let tenant = self
            .tenants
            .entry(tenant_key.clone())
            .or_insert_with(|| {
                RemoteTenantAdmission::new(
                    now,
                    self.limits.initialize_burst_per_tenant,
                    self.limits.max_headerless_requests_per_tenant,
                )
            });
        tenant.refill(now, self.limits);
        tenant.last_activity = now;
        if tenant.initialize_tokens == 0 {
            return Err(remote_session_admission_error(
                "Remote MCP initialize rate limit exceeded",
            ));
        }
        // A rejected initialize remains an initialize attempt; do not refund
        // this token on active-count or byte-cap rejection.
        tenant.initialize_tokens -= 1;

        let tenant_count_allowed =
            tenant.active_sessions < self.limits.max_active_per_tenant;
        let global_bytes_allowed = self
            .total_retained_bytes
            .checked_add(retained_bytes)
            .is_some_and(|bytes| {
                bytes <= self.limits.max_retained_bytes_global
            });
        let tenant_bytes_allowed = tenant
            .retained_bytes
            .checked_add(retained_bytes)
            .is_some_and(|bytes| {
                bytes <= self.limits.max_retained_bytes_per_tenant
            });
        if !tenant_count_allowed
            || !global_bytes_allowed
            || !tenant_bytes_allowed
        {
            return Err(remote_session_admission_error(
                "Remote MCP active session capacity is temporarily exhausted",
            ));
        }

        tenant.active_sessions += 1;
        tenant.retained_bytes += retained_bytes;
        self.total_retained_bytes += retained_bytes;
        self.reservations.insert(
            session_id.clone(),
            RemoteSessionReservation {
                manager_id,
                tenant: tenant_key,
                retained_bytes,
            },
        );
        let removed = self.provisionals.remove(session_id);
        debug_assert!(removed.is_some());
        Ok(())
    }

    fn reserve_provisional(
        &mut self,
        session_id: &SessionId,
        manager_id: u64,
        now: Instant,
    ) -> Result<(), std::io::Error> {
        if self.provisionals.contains_key(session_id)
            || self.reservations.contains_key(session_id)
        {
            return Err(remote_session_admission_error(
                "Remote MCP session id is already registered",
            ));
        }
        if self
            .provisionals
            .len()
            .saturating_add(self.reservations.len())
            >= self.limits.max_active_global
        {
            return Err(remote_session_admission_error(
                "Remote MCP transport session capacity is temporarily exhausted",
            ));
        }
        self.provisionals.insert(
            session_id.clone(),
            RemoteProvisionalSession {
                manager_id,
                created_at: now,
                closing: false,
            },
        );
        Ok(())
    }

    fn claim_expired_provisionals(
        &mut self,
        now: Instant,
        all: bool,
        manager_id: u64,
    ) -> Vec<SessionId> {
        self.provisionals
            .iter_mut()
            .filter_map(|(session_id, provisional)| {
                let expired = provisional.manager_id == manager_id
                    && (all
                        || now.saturating_duration_since(
                            provisional.created_at,
                        ) >= REMOTE_PROVISIONAL_SESSION_TIMEOUT);
                if expired && !provisional.closing {
                    provisional.closing = true;
                    Some(session_id.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    fn finish_provisional_close(
        &mut self,
        session_id: &SessionId,
        manager_id: u64,
    ) {
        if self
            .provisionals
            .get(session_id)
            .is_some_and(|provisional| provisional.manager_id == manager_id)
        {
            self.provisionals.remove(session_id);
        }
    }

    fn retry_provisional_close(
        &mut self,
        session_id: &SessionId,
        manager_id: u64,
    ) {
        if let Some(provisional) = self
            .provisionals
            .get_mut(session_id)
            .filter(|provisional| provisional.manager_id == manager_id)
        {
            provisional.closing = false;
        }
    }

    fn release(
        &mut self,
        session_id: &SessionId,
        manager_id: u64,
        now: Instant,
    ) {
        if self
            .provisionals
            .get(session_id)
            .is_some_and(|provisional| provisional.manager_id == manager_id)
        {
            self.provisionals.remove(session_id);
        }
        let Some(reservation) = self
            .reservations
            .get(session_id)
            .filter(|reservation| reservation.manager_id == manager_id)
            .cloned()
        else {
            return;
        };
        self.reservations.remove(session_id);
        self.total_retained_bytes = self
            .total_retained_bytes
            .saturating_sub(reservation.retained_bytes);
        if let Some(tenant) = self.tenants.get_mut(&reservation.tenant) {
            debug_assert!(tenant.active_sessions > 0);
            debug_assert!(tenant.retained_bytes >= reservation.retained_bytes);
            tenant.active_sessions = tenant.active_sessions.saturating_sub(1);
            tenant.retained_bytes = tenant
                .retained_bytes
                .saturating_sub(reservation.retained_bytes);
            tenant.last_activity = now;
        }
        self.prune_idle_rate_buckets(now);
    }
}

fn remote_session_admission_error(message: &'static str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::WouldBlock, message)
}

fn binding_retained_bytes(
    session_id: &SessionId,
    authoritative_user_id: &str,
    owner_user_id: &UserId,
    scope: Option<&[String]>,
) -> Option<usize> {
    const HASH_ENTRY_OVERHEAD: usize = 64;
    const ALLOCATION_OVERHEAD: usize = 16;
    let mut total = std::mem::size_of::<RemoteMcpSessionBinding>()
        .checked_add(std::mem::size_of::<RemoteHttpRequestBudget>())?
        .checked_add(ALLOCATION_OVERHEAD)?
        .checked_add(std::mem::size_of::<RemoteSessionReservation>())?
        .checked_add(HASH_ENTRY_OVERHEAD.saturating_mul(2))?;
    for retained in [
        session_id.as_ref(),
        authoritative_user_id,
        owner_user_id.as_str(),
    ] {
        total = total
            .checked_add(retained.len())?
            .checked_add(ALLOCATION_OVERHEAD)?;
    }
    if let Some(scope) = scope {
        total = total
            .checked_add(scope.len().saturating_mul(std::mem::size_of::<String>()))?
            .checked_add(ALLOCATION_OVERHEAD)?;
        for domain in scope {
            total = total
                .checked_add(domain.capacity())?
                .checked_add(ALLOCATION_OVERHEAD)?;
        }
    }
    Some(total)
}

#[derive(Clone)]
struct RemoteProvisionalCleanupAuthority {
    shutdown: Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
}

impl RemoteProvisionalCleanupAuthority {
    fn new(
        inner: Arc<LocalSessionManager>,
        admission: Arc<tokio::sync::Mutex<RemoteSessionAdmission>>,
        manager_id: u64,
    ) -> Self {
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(REMOTE_PROVISIONAL_SWEEP_INTERVAL);
            interval.set_missed_tick_behavior(
                tokio::time::MissedTickBehavior::Skip,
            );
            loop {
                let final_drain = tokio::select! {
                    _ = interval.tick() => false,
                    _ = &mut shutdown_rx => true,
                };
                if final_drain {
                    let mut retry_wait =
                        REMOTE_PROVISIONAL_FINAL_RETRY_INITIAL_WAIT;
                    loop {
                        let remaining = sweep_remote_provisional_sessions(
                            &inner,
                            &admission,
                            true,
                            manager_id,
                        )
                        .await;
                        if !remaining {
                            break;
                        }
                        tokio::time::sleep(retry_wait).await;
                        retry_wait = retry_wait
                            .saturating_mul(2)
                            .min(REMOTE_PROVISIONAL_FINAL_RETRY_MAX_WAIT);
                    }
                    break;
                }
                sweep_remote_provisional_sessions(
                    &inner,
                    &admission,
                    false,
                    manager_id,
                )
                .await;
            }
        });
        Self {
            shutdown: Arc::new(std::sync::Mutex::new(Some(shutdown_tx))),
        }
    }
}

impl Drop for RemoteProvisionalCleanupAuthority {
    fn drop(&mut self) {
        if Arc::strong_count(&self.shutdown) != 1 {
            return;
        }
        if let Some(shutdown) = self
            .shutdown
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            let _ = shutdown.send(());
        }
    }
}

async fn sweep_remote_provisional_sessions(
    inner: &LocalSessionManager,
    admission: &tokio::sync::Mutex<RemoteSessionAdmission>,
    all: bool,
    manager_id: u64,
) -> bool {
    let claimed = admission
        .lock()
        .await
        .claim_expired_provisionals(Instant::now(), all, manager_id);
    for session_id in claimed {
        match std::panic::AssertUnwindSafe(inner.close_session(&session_id))
            .catch_unwind()
            .await
        {
            Ok(Ok(())) => {
                admission
                    .lock()
                    .await
                    .finish_provisional_close(&session_id, manager_id);
            }
            Ok(Err(error)) => {
                tracing::debug!(
                    session_id = session_id.as_ref(),
                    error = %error,
                    "Remote MCP provisional cleanup found the transport already closed"
                );
                admission
                    .lock()
                    .await
                    .finish_provisional_close(&session_id, manager_id);
            }
            Err(_) => {
                tracing::error!(
                    session_id = session_id.as_ref(),
                    "Remote MCP provisional transport cleanup panicked; retained for retry"
                );
                admission
                    .lock()
                    .await
                    .retry_provisional_close(&session_id, manager_id);
            }
        }
    }
    admission
        .lock()
        .await
        .provisionals
        .values()
        .any(|provisional| provisional.manager_id == manager_id)
}

fn canonical_scope<'a>(domains: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut scope: Vec<String> = domains
        .into_iter()
        .map(|domain| {
            let mut domain = domain.to_owned();
            domain.shrink_to_fit();
            domain
        })
        .collect();
    scope.sort_unstable();
    scope.dedup();
    scope.shrink_to_fit();
    scope
}

fn raw_query_value<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query.split('&').find_map(|pair| {
        let (candidate, value) = pair.split_once('=').unwrap_or((pair, ""));
        (candidate == key).then_some(value)
    })
}

fn bounded_scope_from_query(
    query: Option<&str>,
) -> Result<Option<Vec<String>>, std::io::Error> {
    let Some(query) = query else {
        return Ok(None);
    };
    if query.len() > MAX_REMOTE_SCOPE_QUERY_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Remote MCP scope query exceeds its byte limit",
        ));
    }
    if let Some(domains) = raw_query_value(query, "domains") {
        if domains.is_empty() {
            return Ok(None);
        }
        let known_specs = Registry::global().tool_specs(Surface::Remote);
        let mut selected = Vec::new();
        let mut retained_bytes = 0usize;
        for (index, raw_domain) in domains.split(',').enumerate() {
            if index >= MAX_REMOTE_SCOPE_DOMAINS {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Remote MCP scope contains too many domains",
                ));
            }
            let domain = raw_domain.trim();
            if domain.is_empty() {
                continue;
            }
            if domain.len() > MAX_REMOTE_SCOPE_DOMAIN_BYTES {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Remote MCP scope domain exceeds its byte limit",
                ));
            }
            retained_bytes = retained_bytes
                .checked_add(domain.len())
                .ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "Remote MCP scope size overflow",
                    )
                })?;
            if retained_bytes > MAX_REMOTE_SCOPE_BYTES {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Remote MCP scope exceeds its retained-byte limit",
                ));
            }
            if !known_specs.iter().any(|spec| spec.domain == domain) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Remote MCP scope contains an unknown or unavailable domain",
                ));
            }
            selected.push(domain);
        }
        return Ok((!selected.is_empty()).then(|| canonical_scope(selected)));
    }
    match raw_query_value(query, "profile") {
        Some("agent") => Ok(Some(canonical_scope(
            crate::AGENT_PROFILE_DOMAINS.iter().copied(),
        ))),
        Some("full") | Some("") | None => Ok(None),
        Some(_) => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Remote MCP scope profile is unknown",
        )),
    }
}

fn binding_accepts_request(
    binding: &RemoteMcpSessionBinding,
    owner_user_id: &UserId,
    requested_scope: Option<&[String]>,
    explicit_scope: bool,
) -> bool {
    binding.owner_user_id == *owner_user_id
        && (!explicit_scope || binding.scope.as_deref() == requested_scope)
}

#[cfg(feature = "browser-use")]
const REMOTE_BROWSER_CLEANUP_INTERVAL: Duration = Duration::from_millis(250);
#[cfg(all(feature = "browser-use", not(test)))]
const REMOTE_BROWSER_FINAL_RETRY_INITIAL_WAIT: Duration =
    Duration::from_millis(250);
#[cfg(all(feature = "browser-use", not(test)))]
const REMOTE_BROWSER_FINAL_RETRY_MAX_WAIT: Duration = Duration::from_secs(5);
#[cfg(all(feature = "browser-use", test))]
const REMOTE_BROWSER_FINAL_RETRY_INITIAL_WAIT: Duration =
    Duration::from_millis(10);
#[cfg(all(feature = "browser-use", test))]
const REMOTE_BROWSER_FINAL_RETRY_MAX_WAIT: Duration =
    Duration::from_millis(50);

#[cfg(feature = "browser-use")]
trait RemoteBrowserCleanupRegistry: Send + Sync {
    fn revoke_trusted_identity<'a>(
        &'a self,
        runtime_instance_id: &'a str,
    ) -> futures::future::BoxFuture<
        'a,
        Result<(), nomifun_browser_platform::BrowserPlatformError>,
    >;

    fn retry_pending_browser_cleanups(
        &self,
    ) -> futures::future::BoxFuture<'_, ()>;
}

#[cfg(feature = "browser-use")]
impl RemoteBrowserCleanupRegistry
    for nomifun_gateway::browser_registry::BrowserRegistry
{
    fn revoke_trusted_identity<'a>(
        &'a self,
        runtime_instance_id: &'a str,
    ) -> futures::future::BoxFuture<
        'a,
        Result<(), nomifun_browser_platform::BrowserPlatformError>,
    > {
        Box::pin(async move {
            nomifun_gateway::browser_registry::BrowserRegistry::revoke_trusted_identity(
                self,
                runtime_instance_id,
            )
            .await
            .map(|_| ())
        })
    }

    fn retry_pending_browser_cleanups(
        &self,
    ) -> futures::future::BoxFuture<'_, ()> {
        Box::pin(async move {
            nomifun_gateway::browser_registry::BrowserRegistry::retry_pending_browser_cleanups(
                self,
            )
            .await;
        })
    }
}

/// Durable retry authority for Remote MCP browser attachments.
///
/// `BrowserRegistry::revoke_trusted_identity` retains failed exact-owner
/// cleanup as `revocation_pending`; this process-lifetime worker keeps retrying
/// those records after DELETE/idle/worker-exit callbacks have returned. The
/// local `pending_sessions` set additionally preserves lifecycle attribution
/// until the registry revoke itself reports success.
#[cfg(feature = "browser-use")]
struct RemoteBrowserCleanupState {
    registry: Arc<dyn RemoteBrowserCleanupRegistry>,
    bindings: Arc<tokio::sync::RwLock<HashMap<SessionId, RemoteMcpSessionBinding>>>,
    admission: Arc<tokio::sync::Mutex<RemoteSessionAdmission>>,
    manager_id: u64,
    pending_sessions: tokio::sync::Mutex<PendingRemoteBrowserCleanups>,
}

#[cfg(feature = "browser-use")]
#[derive(Default)]
struct PendingRemoteBrowserCleanups {
    ids: HashSet<String>,
    retained_bytes: usize,
}

#[cfg(feature = "browser-use")]
impl PendingRemoteBrowserCleanups {
    fn insert(&mut self, session_id: &str) -> bool {
        if self.ids.contains(session_id) {
            return true;
        }
        if self.ids.len() >= MAX_PENDING_REMOTE_BROWSER_CLEANUPS
            || self
                .retained_bytes
                .checked_add(session_id.len())
                .is_none_or(|bytes| {
                    bytes > MAX_PENDING_REMOTE_BROWSER_CLEANUP_BYTES
                })
        {
            return false;
        }
        self.retained_bytes += session_id.len();
        self.ids.insert(session_id.to_owned());
        true
    }

    fn remove(&mut self, session_id: &str) {
        if let Some(removed) = self.ids.take(session_id) {
            self.retained_bytes =
                self.retained_bytes.saturating_sub(removed.len());
        }
    }
}

#[cfg(feature = "browser-use")]
#[derive(Clone)]
struct RemoteBrowserCleanupAuthority {
    state: Arc<RemoteBrowserCleanupState>,
    shutdown: Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
}

#[cfg(feature = "browser-use")]
impl RemoteBrowserCleanupAuthority {
    fn new(
        registry: nomifun_gateway::browser_registry::BrowserRegistry,
        bindings: Arc<tokio::sync::RwLock<HashMap<SessionId, RemoteMcpSessionBinding>>>,
        admission: Arc<tokio::sync::Mutex<RemoteSessionAdmission>>,
        manager_id: u64,
    ) -> Self {
        Self::with_registry(
            Arc::new(registry),
            bindings,
            admission,
            manager_id,
        )
    }

    fn with_registry(
        registry: Arc<dyn RemoteBrowserCleanupRegistry>,
        bindings: Arc<tokio::sync::RwLock<HashMap<SessionId, RemoteMcpSessionBinding>>>,
        admission: Arc<tokio::sync::Mutex<RemoteSessionAdmission>>,
        manager_id: u64,
    ) -> Self {
        let state = Arc::new(RemoteBrowserCleanupState {
            registry,
            bindings,
            admission,
            manager_id,
            pending_sessions: tokio::sync::Mutex::new(
                PendingRemoteBrowserCleanups::default(),
            ),
        });
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
        let worker_state = Arc::clone(&state);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(REMOTE_BROWSER_CLEANUP_INTERVAL);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        retry_remote_browser_cleanups_resilient(&worker_state)
                            .await;
                    }
                    _ = &mut shutdown_rx => {
                        drain_remote_browser_cleanups(&worker_state).await;
                        break;
                    }
                }
            }
        });
        Self {
            state,
            shutdown: Arc::new(std::sync::Mutex::new(Some(shutdown_tx))),
        }
    }

    async fn revoke_or_queue(&self, runtime_instance_id: &str) {
        let retained = self
            .state
            .pending_sessions
            .lock()
            .await
            .insert(runtime_instance_id);
        if !retained {
            // This is reachable only after a host invariant violation: active
            // reservations are adaptively capped below the pending ceiling.
            // Keep the admission debt (backpressure) and rely on the registry's
            // own bounded exact-cleanup ledger rather than allocate more IDs.
            tracing::error!(
                session_id = runtime_instance_id,
                "Remote browser cleanup debt inventory is full"
            );
        }
        try_revoke_remote_browser_session(&self.state, runtime_instance_id).await;
    }

    #[cfg(test)]
    async fn pending_count(&self) -> usize {
        self.state.pending_sessions.lock().await.ids.len()
    }
}

#[cfg(feature = "browser-use")]
impl Drop for RemoteBrowserCleanupAuthority {
    fn drop(&mut self) {
        if Arc::strong_count(&self.shutdown) != 1 {
            return;
        }
        if let Some(shutdown) = self
            .shutdown
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            // The worker owns its state independently. Dropping its JoinHandle
            // never cancels the final drain.
            let _ = shutdown.send(());
        }
    }
}

#[cfg(feature = "browser-use")]
async fn try_revoke_remote_browser_session(
    state: &RemoteBrowserCleanupState,
    runtime_instance_id: &str,
) {
    match state
        .registry
        .revoke_trusted_identity(runtime_instance_id)
        .await
    {
        Ok(_) => {
            // Acquire both authorities before publishing either transition.
            // Cancellation while waiting therefore leaves the exact pending
            // id retryable; after both locks are held, removal + refund have
            // no await point that could strand an admission reservation.
            let mut admission = state.admission.lock().await;
            let mut pending = state.pending_sessions.lock().await;
            pending.remove(runtime_instance_id);
            admission.release(
                &Arc::<str>::from(runtime_instance_id),
                state.manager_id,
                Instant::now(),
            );
        }
        Err(error) => {
            tracing::warn!(
                session_id = runtime_instance_id,
                code = ?error.code,
                "Remote MCP browser attachment revoke failed; retained for retry"
            );
        }
    }
}

#[cfg(feature = "browser-use")]
async fn retry_remote_browser_cleanups(state: &RemoteBrowserCleanupState) {
    let pending = state
        .pending_sessions
        .lock()
        .await
        .ids
        .iter()
        .cloned()
        .collect::<Vec<_>>();
    for runtime_instance_id in pending {
        try_revoke_remote_browser_session(state, &runtime_instance_id).await;
    }
    // Also covers a registry-side pending record left after a caller was
    // cancelled between the registry transition and local bookkeeping.
    state.registry.retry_pending_browser_cleanups().await;
}

#[cfg(feature = "browser-use")]
async fn retry_remote_browser_cleanups_resilient(
    state: &RemoteBrowserCleanupState,
) {
    if std::panic::AssertUnwindSafe(retry_remote_browser_cleanups(state))
        .catch_unwind()
        .await
        .is_err()
    {
        // A registry adapter is an extension boundary. Keep the bounded exact
        // pending set and the same fixed worker alive after a panic instead of
        // losing admission-refund authority.
        tracing::error!(
            "Remote browser cleanup adapter panicked; retained for retry"
        );
    }
}

#[cfg(feature = "browser-use")]
async fn drain_remote_browser_cleanups(state: &RemoteBrowserCleanupState) {
    // Any still-bound session is ending with the manager/router itself. Queue
    // it before the final retry rounds so process shutdown cannot strand it.
    let bound = state
        .bindings
        .read()
        .await
        .keys()
        .map(|id| id.as_ref().to_owned())
        .collect::<Vec<_>>();
    {
        let mut pending = state.pending_sessions.lock().await;
        for session_id in bound {
            let _ = pending.insert(&session_id);
        }
    }
    // The pending set is bounded by the shared session admission ceiling. Keep
    // the one existing cleanup worker alive until every exact runtime id has
    // succeeded; a fixed retry count would otherwise lose the only authority
    // capable of refunding its retained admission reservation.
    let mut retry_wait = REMOTE_BROWSER_FINAL_RETRY_INITIAL_WAIT;
    loop {
        retry_remote_browser_cleanups_resilient(state).await;
        if state.pending_sessions.lock().await.ids.is_empty() {
            break;
        }
        tokio::time::sleep(retry_wait).await;
        retry_wait = retry_wait
            .saturating_mul(2)
            .min(REMOTE_BROWSER_FINAL_RETRY_MAX_WAIT);
    }
}

#[derive(Debug, Error)]
pub(crate) enum RemoteSessionManagerError {
    #[error(transparent)]
    Local(#[from] LocalSessionManagerError),
    #[error("{0}")]
    Transport(#[from] std::io::Error),
}

/// Shared control-plane admission authority for all Remote MCP endpoints that
/// belong to one authoritative user. It bounds sessions and initialize churn
/// across `/mcp`, curated profiles, and any future sibling endpoint without
/// treating the installation owner as the browser task-family identity.
#[derive(Clone)]
pub struct RemoteMcpSessionAdmissionAuthority {
    authoritative_user_id: Arc<str>,
    admission: Arc<tokio::sync::Mutex<RemoteSessionAdmission>>,
}

impl RemoteMcpSessionAdmissionAuthority {
    pub fn for_gateway(deps: &GatewayDeps) -> Self {
        let authoritative_user_id = Arc::clone(&deps.authoritative_user_id);
        Self {
            admission: Arc::new(tokio::sync::Mutex::new(
                RemoteSessionAdmission::new(
                    Arc::clone(&authoritative_user_id),
                    RemoteSessionLimits::for_machine(),
                ),
            )),
            authoritative_user_id,
        }
    }
}

pub(crate) struct RemoteSessionManager {
    manager_id: u64,
    inner: Arc<LocalSessionManager>,
    domains: Option<Vec<String>>,
    bindings: Arc<tokio::sync::RwLock<HashMap<SessionId, RemoteMcpSessionBinding>>>,
    admission: Arc<tokio::sync::Mutex<RemoteSessionAdmission>>,
    _provisional_cleanup: RemoteProvisionalCleanupAuthority,
    #[cfg(feature = "browser-use")]
    browser_cleanup: Option<RemoteBrowserCleanupAuthority>,
}

impl RemoteSessionManager {
    pub(crate) fn with_admission_authority(
        deps: Arc<GatewayDeps>,
        domains: Option<&'static [&'static str]>,
        authority: RemoteMcpSessionAdmissionAuthority,
    ) -> Self {
        let manager_id = NEXT_REMOTE_SESSION_MANAGER_ID.fetch_add(1, Ordering::Relaxed);
        let inner = Arc::new(LocalSessionManager::default());
        let admission = if authority.authoritative_user_id
            == deps.authoritative_user_id
        {
            Arc::clone(&authority.admission)
        } else {
            // A host wiring bug must not merge different users into one
            // admission/rate bucket. Fail isolated while retaining service.
            tracing::error!(
                "Remote MCP admission authority user mismatch; isolating endpoint"
            );
            RemoteMcpSessionAdmissionAuthority::for_gateway(deps.as_ref())
                .admission
        };
        let provisional_cleanup = RemoteProvisionalCleanupAuthority::new(
            Arc::clone(&inner),
            Arc::clone(&admission),
            manager_id,
        );
        let bindings = Arc::new(tokio::sync::RwLock::new(HashMap::new()));
        #[cfg(feature = "browser-use")]
        let browser_cleanup = deps
            .browser_registry
            .clone()
            .map(|registry| {
                RemoteBrowserCleanupAuthority::new(
                    registry,
                    Arc::clone(&bindings),
                    Arc::clone(&admission),
                    manager_id,
                )
            });
        Self {
            manager_id,
            inner,
            domains: domains.map(|domains| canonical_scope(domains.iter().copied())),
            bindings,
            admission,
            _provisional_cleanup: provisional_cleanup,
            #[cfg(feature = "browser-use")]
            browser_cleanup,
        }
    }

    fn scope_from_message(
        &self,
        message: &ClientJsonRpcMessage,
    ) -> Result<(Option<Vec<String>>, bool), std::io::Error> {
        if let Some(domains) = &self.domains {
            return Ok((Some(domains.clone()), true));
        }
        let parts = match message {
            ClientJsonRpcMessage::Request(request) => request
                .request
                .extensions()
                .get::<axum::http::request::Parts>(),
            ClientJsonRpcMessage::Notification(notification) => notification
                .notification
                .extensions()
                .get::<axum::http::request::Parts>(),
            ClientJsonRpcMessage::Response(_) | ClientJsonRpcMessage::Error(_) => None,
        }
        .ok_or_else(|| {
            std::io::Error::other(
                "authenticated Remote MCP request has no HTTP request parts",
            )
        })?;
        let explicit = parts.uri.query().is_some_and(|query| {
            query.split('&').any(|pair| {
                let key = pair.split_once('=').map_or(pair, |(key, _)| key);
                key == "domains" || key == "profile"
            })
        });
        Ok((bounded_scope_from_query(parts.uri.query())?, explicit))
    }

    fn owner_from_message(
        message: &ClientJsonRpcMessage,
    ) -> Result<UserId, std::io::Error> {
        let parts = match message {
            ClientJsonRpcMessage::Request(request) => request
                .request
                .extensions()
                .get::<axum::http::request::Parts>(),
            ClientJsonRpcMessage::Notification(notification) => notification
                .notification
                .extensions()
                .get::<axum::http::request::Parts>(),
            ClientJsonRpcMessage::Response(_) | ClientJsonRpcMessage::Error(_) => None,
        }
            .ok_or_else(|| {
                std::io::Error::other(
                    "authenticated Remote MCP request has no HTTP request parts",
                )
            })?;
        parts
            .extensions
            .get::<crate::router::RemoteInstanceOwner>()
            .map(|owner| owner.0.clone())
            .ok_or_else(|| {
                std::io::Error::other(
                    "authenticated Remote MCP request has no canonical installation owner identity",
                )
            })
    }

    async fn inject_pinned_identity(
        &self,
        id: &SessionId,
        message: &mut ClientJsonRpcMessage,
        pin_if_missing: bool,
    ) -> Result<(), std::io::Error> {
        let owner_user_id = Self::owner_from_message(message)?;
        let (requested_scope, explicit_scope) = self.scope_from_message(message)?;
        // Admission reservation and binding publication share one lock order
        // and have no await/cancellation point between them. A session can
        // therefore never retain one without the other.
        let mut admission = self.admission.lock().await;
        let mut bindings = self.bindings.write().await;
        match bindings.get(id) {
            Some(binding)
                if !binding_accepts_request(
                    binding,
                    &owner_user_id,
                    requested_scope.as_deref(),
                    explicit_scope,
                ) =>
            {
                let message = if binding.owner_user_id != owner_user_id {
                    "Remote MCP session is bound to a different installation owner"
                } else {
                    "Remote MCP session is bound to a different capability scope"
                };
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    message,
                ));
            }
            None if pin_if_missing => {
                let retained_bytes = binding_retained_bytes(
                    id,
                    admission.authoritative_user_id.as_ref(),
                    &owner_user_id,
                    requested_scope.as_deref(),
                )
                .ok_or_else(|| {
                    remote_session_admission_error(
                        "Remote MCP session retained-memory accounting overflowed",
                    )
                })?;
                admission.reserve(
                    id,
                    self.manager_id,
                    &owner_user_id,
                    retained_bytes,
                    Instant::now(),
                )?;
                bindings.insert(
                    id.clone(),
                    RemoteMcpSessionBinding::new(
                        owner_user_id.clone(),
                        requested_scope.clone(),
                        admission.limits.max_inflight_requests_per_session,
                    ),
                );
            }
            None => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "Remote MCP session has no pinned identity",
                ));
            }
            Some(_) => {
                if !admission.reservations.contains_key(id) {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "Remote MCP session has no retained admission authority",
                    ));
                }
            }
        }
        let binding = bindings
            .get(id)
            .expect("session binding inserted or already present")
            .clone();
        drop(bindings);
        drop(admission);
        message.insert_extension(RemoteMcpSessionIdentity {
            session_id: id.clone(),
            owner_user_id: binding.owner_user_id,
            scope: binding.scope,
        });
        // Keep the narrower marker for code/tests that only need the logical
        // session id. The identity above is the authoritative owner pin.
        message.insert_extension(RemoteMcpSessionId(id.clone()));
        Ok(())
    }

    async fn close_session_durably(
        &self,
        id: &SessionId,
    ) -> Result<(), RemoteSessionManagerError> {
        let id = id.clone();
        let inner = Arc::clone(&self.inner);
        let bindings = Arc::clone(&self.bindings);
        let admission = Arc::clone(&self.admission);
        let manager_id = self.manager_id;
        #[cfg(feature = "browser-use")]
        let browser_cleanup = self.browser_cleanup.clone();
        tokio::spawn(async move {
            // LocalSessionManager removes the transport handle before awaiting
            // worker shutdown. The detached finalizer then publishes the
            // binding refund even when the original HTTP future is cancelled.
            let result = std::panic::AssertUnwindSafe(
                inner.close_session(&id),
            )
            .catch_unwind()
            .await;
            let mut admission = admission.lock().await;
            bindings.write().await.remove(&id);
            #[cfg(feature = "browser-use")]
            let defer_refund = browser_cleanup.is_some();
            #[cfg(not(feature = "browser-use"))]
            let defer_refund = false;
            if !defer_refund {
                admission.release(&id, manager_id, Instant::now());
            }
            drop(admission);
            #[cfg(feature = "browser-use")]
            if let Some(cleanup) = browser_cleanup.as_ref() {
                cleanup.revoke_or_queue(id.as_ref()).await;
            }
            match result {
                Ok(result) => result.map_err(RemoteSessionManagerError::Local),
                Err(_) => Err(RemoteSessionManagerError::Transport(
                    std::io::Error::other(
                        "Remote MCP transport close panicked after exact lifecycle cleanup",
                    ),
                )),
            }
        })
        .await
        .map_err(|error| {
            std::io::Error::other(format!(
                "Remote MCP session finalizer failed: {error}"
            ))
        })??;
        Ok(())
    }

    async fn discard_failed_initialization(&self, id: &SessionId) {
        // `StreamableHttpService` allocates the LocalSession worker before it
        // forwards initialize.  If initialize fails, rmcp does not get as far
        // as its normal worker-exit callback, so close the worker here instead
        // of leaving it alive until the init timeout.
        if let Err(error) = self.close_session_durably(id).await {
            tracing::debug!(
                session_id = id.as_ref(),
                error = %error,
                "Remote MCP initialize cleanup found the session worker already closed"
            );
        }
    }

    pub(crate) async fn acquire_http_request_permit(
        &self,
        id: Option<&SessionId>,
        owner_user_id: &UserId,
        headerless_post: bool,
        teardown: bool,
    ) -> Result<Option<RemoteHttpRequestPermit>, RemoteHttpRequestAdmissionError>
    {
        if let Some(id) = id {
            let bindings = self.bindings.read().await;
            let Some(binding) = bindings.get(id) else {
                return Err(
                    RemoteHttpRequestAdmissionError::IdentityMismatch,
                );
            };
            if binding.owner_user_id != *owner_user_id {
                return Err(
                    RemoteHttpRequestAdmissionError::IdentityMismatch,
                );
            }
            // Linearize admission while the binding read authority is still
            // held. Exact close/replacement needs the write lock and therefore
            // cannot create an old-budget/new-binding ABA window.
            return binding
                .request_budget
                .try_acquire_session(teardown)
                .map(Some)
                .ok_or(RemoteHttpRequestAdmissionError::CapacityExceeded);
        }
        if headerless_post {
            return self
                .admission
                .lock()
                .await
                .acquire_headerless_request(owner_user_id, Instant::now())
                .map(Some);
        }
        Ok(None)
    }
}

impl SessionManager for RemoteSessionManager {
    type Error = RemoteSessionManagerError;
    type Transport = WorkerTransport<LocalSessionWorker>;

    async fn create_session(
        &self,
    ) -> Result<(SessionId, Self::Transport), Self::Error> {
        let id = session_id();
        self.admission
            .lock()
            .await
            .reserve_provisional(&id, self.manager_id, Instant::now())?;

        // Construct locally so the provisional authority exists before the
        // session becomes visible in rmcp's map. Cancellation at the map lock
        // or after insertion is recovered by the background exact-id sweep.
        let (handle, worker) =
            create_local_session(id.clone(), self.inner.session_config.clone());
        let mut sessions = self.inner.sessions.write().await;
        if sessions.contains_key(&id) {
            drop(sessions);
            self.admission
                .lock()
                .await
                .finish_provisional_close(&id, self.manager_id);
            return Err(std::io::Error::other(
                "Remote MCP generated a duplicate session id",
            )
            .into());
        }
        sessions.insert(id.clone(), handle);
        drop(sessions);
        Ok((id, WorkerTransport::spawn(worker)))
    }

    async fn initialize_session(
        &self,
        id: &SessionId,
        mut message: ClientJsonRpcMessage,
    ) -> Result<ServerJsonRpcMessage, Self::Error> {
        if let Err(error) = self.inject_pinned_identity(id, &mut message, true).await {
            self.discard_failed_initialization(id).await;
            return Err(error.into());
        }
        match self.inner.initialize_session(id, message).await {
            Ok(response) => Ok(response),
            Err(error) => {
                self.discard_failed_initialization(id).await;
                Err(RemoteSessionManagerError::Local(error))
            }
        }
    }

    async fn has_session(&self, id: &SessionId) -> Result<bool, Self::Error> {
        Ok(self.inner.has_session(id).await?)
    }

    async fn close_session(&self, id: &SessionId) -> Result<(), Self::Error> {
        self.close_session_durably(id).await
    }

    async fn create_stream(
        &self,
        id: &SessionId,
        mut message: ClientJsonRpcMessage,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + Sync + 'static, Self::Error>
    {
        self.inject_pinned_identity(id, &mut message, false).await?;
        Ok(self.inner.create_stream(id, message).await?)
    }

    async fn accept_message(
        &self,
        id: &SessionId,
        mut message: ClientJsonRpcMessage,
    ) -> Result<(), Self::Error> {
        match &message {
            ClientJsonRpcMessage::Response(_) | ClientJsonRpcMessage::Error(_) => {
                if !self.bindings.read().await.contains_key(id) {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "Remote MCP session has no pinned identity",
                    )
                    .into());
                }
            }
            _ => self.inject_pinned_identity(id, &mut message, false).await?,
        }
        Ok(self.inner.accept_message(id, message).await?)
    }

    async fn create_standalone_stream(
        &self,
        id: &SessionId,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + Sync + 'static, Self::Error>
    {
        if !self.bindings.read().await.contains_key(id) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "Remote MCP session has no pinned identity",
            )
            .into());
        }
        Ok(self.inner.create_standalone_stream(id).await?)
    }

    async fn resume(
        &self,
        id: &SessionId,
        last_event_id: String,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + Sync + 'static, Self::Error>
    {
        if !self.bindings.read().await.contains_key(id) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "Remote MCP session has no pinned identity",
            )
            .into());
        }
        Ok(self.inner.resume(id, last_event_id).await?)
    }

    async fn restore_session(
        &self,
        _id: SessionId,
    ) -> Result<RestoreOutcome<Self::Transport>, Self::Error> {
        // Do not delegate rmcp's restore implementation: LocalSessionManager
        // accepts the caller-supplied id when restoring, which would weaken
        // the invariant that session ids are server-generated and validated by
        // `has_session`.  This front door has no persisted, authenticated
        // owner/scope binding to restore, so fail closed instead.
        Ok(RestoreOutcome::NotSupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "browser-use")]
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[cfg(feature = "browser-use")]
    use futures::FutureExt;
    #[cfg(feature = "browser-use")]
    use nomifun_browser_platform::{
        BrowserErrorCode, BrowserPlatformError,
    };

    fn owner() -> UserId {
        UserId::new()
    }

    fn session_id(value: &str) -> SessionId {
        Arc::<str>::from(value)
    }

    fn constrained_limits(
        max_active_global: usize,
        max_active_per_tenant: usize,
        initialize_burst_per_tenant: u32,
    ) -> RemoteSessionLimits {
        let mut limits =
            RemoteSessionLimits::for_resources(16 * 1024 * 1024 * 1024, 8);
        limits.max_active_global = max_active_global;
        limits.max_active_per_tenant = max_active_per_tenant;
        limits.max_retained_bytes_global = max_active_global
            .saturating_mul(MAX_REMOTE_BINDING_RETAINED_BYTES);
        limits.max_retained_bytes_per_tenant = max_active_per_tenant
            .saturating_mul(MAX_REMOTE_BINDING_RETAINED_BYTES);
        limits.max_rate_tenants_global = max_active_global.saturating_mul(2);
        limits.initialize_burst_per_tenant = initialize_burst_per_tenant;
        limits.initialize_refill_interval = Duration::from_secs(60);
        limits
    }

    fn admit_test_session(
        admission: &mut RemoteSessionAdmission,
        id: &SessionId,
        manager_id: u64,
        owner_user_id: &UserId,
        now: Instant,
    ) -> Result<(), std::io::Error> {
        admission.reserve_provisional(id, manager_id, now)?;
        let retained_bytes = binding_retained_bytes(
            id,
            admission.authoritative_user_id.as_ref(),
            owner_user_id,
            Some(&canonical_scope(["agent"])),
        )
        .expect("bounded test binding size");
        admission.reserve(
            id,
            manager_id,
            owner_user_id,
            retained_bytes,
            now,
        )
    }

    #[cfg(feature = "browser-use")]
    fn binding() -> RemoteMcpSessionBinding {
        RemoteMcpSessionBinding::new(
            owner(),
            Some(canonical_scope(["browser"])),
            MAX_REMOTE_INFLIGHT_REQUESTS_PER_SESSION,
        )
    }

    #[cfg(feature = "browser-use")]
    fn cleanup_error() -> BrowserPlatformError {
        BrowserPlatformError::new(
            BrowserErrorCode::BrowserUnavailable,
            "Synthetic Remote browser cleanup failure.",
            true,
            "Retry the authoritative cleanup.",
        )
    }

    #[cfg(feature = "browser-use")]
    #[derive(Default)]
    struct CleanupRegistryProbe {
        panics_remaining: AtomicUsize,
        failures_remaining: AtomicUsize,
        retry_calls: AtomicUsize,
        revocations: tokio::sync::Mutex<Vec<String>>,
        notify: tokio::sync::Notify,
    }

    #[cfg(feature = "browser-use")]
    impl CleanupRegistryProbe {
        fn fail_next(count: usize) -> Arc<Self> {
            Self::panic_then_fail(0, count)
        }

        fn panic_then_fail(panics: usize, failures: usize) -> Arc<Self> {
            Arc::new(Self {
                panics_remaining: AtomicUsize::new(panics),
                failures_remaining: AtomicUsize::new(failures),
                ..Default::default()
            })
        }

        async fn revocations(&self) -> Vec<String> {
            self.revocations.lock().await.clone()
        }

        async fn wait_for_revocations(&self, expected: usize) {
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    if self.revocations.lock().await.len() >= expected {
                        break;
                    }
                    self.notify.notified().await;
                }
            })
            .await
            .expect("Remote browser cleanup did not reach the registry");
        }
    }

    #[cfg(feature = "browser-use")]
    impl RemoteBrowserCleanupRegistry for CleanupRegistryProbe {
        fn revoke_trusted_identity<'a>(
            &'a self,
            runtime_instance_id: &'a str,
        ) -> futures::future::BoxFuture<
            'a,
            Result<(), BrowserPlatformError>,
        > {
            async move {
                self.revocations
                    .lock()
                    .await
                    .push(runtime_instance_id.to_owned());
                self.notify.notify_waiters();
                if self
                    .panics_remaining
                    .fetch_update(
                        Ordering::AcqRel,
                        Ordering::Acquire,
                        |remaining| {
                            (remaining > 0).then(|| remaining - 1)
                        },
                    )
                    .is_ok()
                {
                    panic!("synthetic Remote browser cleanup panic");
                }
                if self
                    .failures_remaining
                    .fetch_update(
                        Ordering::AcqRel,
                        Ordering::Acquire,
                        |remaining| {
                            (remaining > 0).then(|| remaining - 1)
                        },
                    )
                    .is_ok()
                {
                    Err(cleanup_error())
                } else {
                    Ok(())
                }
            }
            .boxed()
        }

        fn retry_pending_browser_cleanups(
            &self,
        ) -> futures::future::BoxFuture<'_, ()> {
            async move {
                self.retry_calls.fetch_add(1, Ordering::AcqRel);
                self.notify.notify_waiters();
            }
            .boxed()
        }
    }

    #[cfg(feature = "browser-use")]
    fn cleanup_authority(
        registry: Arc<CleanupRegistryProbe>,
        bindings: Arc<
            tokio::sync::RwLock<HashMap<SessionId, RemoteMcpSessionBinding>>,
        >,
        admission: Arc<tokio::sync::Mutex<RemoteSessionAdmission>>,
        manager_id: u64,
    ) -> RemoteBrowserCleanupAuthority {
        RemoteBrowserCleanupAuthority::with_registry(
            registry,
            bindings,
            admission,
            manager_id,
        )
    }

    #[cfg(feature = "browser-use")]
    fn test_manager(
        inner: LocalSessionManager,
        registry: Arc<CleanupRegistryProbe>,
    ) -> RemoteSessionManager {
        let manager_id =
            NEXT_REMOTE_SESSION_MANAGER_ID.fetch_add(1, Ordering::Relaxed);
        let inner = Arc::new(inner);
        let bindings = Arc::new(tokio::sync::RwLock::new(HashMap::new()));
        let admission = Arc::new(tokio::sync::Mutex::new(
            RemoteSessionAdmission::new(
                Arc::<str>::from(
                    "0190f5fe-7c00-7a00-8000-000000000001",
                ),
                RemoteSessionLimits::for_resources(16 * 1024 * 1024 * 1024, 8),
            ),
        ));
        let provisional_cleanup = RemoteProvisionalCleanupAuthority::new(
            Arc::clone(&inner),
            Arc::clone(&admission),
            manager_id,
        );
        RemoteSessionManager {
            manager_id,
            inner,
            domains: None,
            bindings: Arc::clone(&bindings),
            admission: Arc::clone(&admission),
            _provisional_cleanup: provisional_cleanup,
            browser_cleanup: Some(cleanup_authority(
                registry,
                bindings,
                admission,
                manager_id,
            )),
        }
    }

    #[cfg(feature = "browser-use")]
    fn manager_with_cleanup(
        registry: Arc<CleanupRegistryProbe>,
    ) -> RemoteSessionManager {
        test_manager(LocalSessionManager::default(), registry)
    }

    #[cfg(feature = "browser-use")]
    fn manager_with_cleanup_and_keep_alive(
        registry: Arc<CleanupRegistryProbe>,
        keep_alive: Duration,
    ) -> Arc<RemoteSessionManager> {
        let mut inner = LocalSessionManager::default();
        inner.session_config.keep_alive = Some(keep_alive);
        Arc::new(test_manager(inner, registry))
    }

    #[cfg(feature = "browser-use")]
    fn initialize_request() -> ClientJsonRpcMessage {
        use rmcp::model::{
            ClientCapabilities, ClientRequest, Implementation,
            InitializeRequest, InitializeRequestParams, RequestId,
        };

        ClientJsonRpcMessage::request(
            ClientRequest::InitializeRequest(InitializeRequest::new(
                InitializeRequestParams::new(
                    ClientCapabilities::default(),
                    Implementation::new("nomifun-public-session-test", "1"),
                ),
            )),
            RequestId::Number(1),
        )
    }

    #[cfg(feature = "browser-use")]
    async fn create_and_bind_session(
        manager: &RemoteSessionManager,
    ) -> (SessionId, RemoteBrowserCleanupAuthority) {
        let (id, transport) = manager.create_session().await.unwrap();
        drop(transport);
        let binding = binding();
        let retained_bytes = binding_retained_bytes(
            &id,
            manager
                .admission
                .lock()
                .await
                .authoritative_user_id
                .as_ref(),
            &binding.owner_user_id,
            binding.scope.as_deref(),
        )
        .unwrap();
        let mut admission = manager.admission.lock().await;
        let mut bindings = manager.bindings.write().await;
        admission
            .reserve(
                &id,
                manager.manager_id,
                &binding.owner_user_id,
                retained_bytes,
                Instant::now(),
            )
            .unwrap();
        bindings.insert(id.clone(), binding);
        drop(bindings);
        drop(admission);
        let cleanup = manager
            .browser_cleanup
            .as_ref()
            .expect("test manager has browser cleanup")
            .clone();
        (id, cleanup)
    }

    #[cfg(feature = "browser-use")]
    async fn wait_until(
        description: &str,
        mut predicate: impl AsyncFnMut() -> bool,
    ) {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if predicate().await {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {description}"));
    }

    #[test]
    fn session_binding_pins_owner_and_scope() {
        let first = owner();
        let second = owner();
        let binding = RemoteMcpSessionBinding::new(
            first.clone(),
            Some(canonical_scope(["files", "agent"])),
            MAX_REMOTE_INFLIGHT_REQUESTS_PER_SESSION,
        );
        assert!(binding_accepts_request(
            &binding,
            &first,
            Some(&canonical_scope(["agent", "files"])),
            true
        ));
        assert!(!binding_accepts_request(
            &binding,
            &first,
            Some(&canonical_scope(["agent", "files", "browser"])),
            true
        ));
        assert!(!binding_accepts_request(
            &binding,
            &second,
            Some(&canonical_scope(["agent", "files"])),
            true
        ));
        // A later request without a query cannot widen the initialize scope;
        // the handler receives the pinned scope from the identity marker.
        assert!(binding_accepts_request(&binding, &first, None, false));
    }

    #[test]
    fn aggregate_session_capacity_scales_with_cpu_and_memory() {
        let small = RemoteSessionLimits::for_resources(4 * 1024 * 1024 * 1024, 4);
        let large =
            RemoteSessionLimits::for_resources(64 * 1024 * 1024 * 1024, 32);
        assert!(large.max_active_global > small.max_active_global);
        assert!(
            large.max_retained_bytes_global > small.max_retained_bytes_global
        );
        assert!(large.max_active_per_tenant >= small.max_active_per_tenant);
        assert!(large.max_active_global <= MAX_GLOBAL_REMOTE_SESSIONS);
    }

    #[tokio::test]
    async fn remote_http_request_budget_is_exact_and_cancellation_safe() {
        let budget = RemoteHttpRequestBudget::new(
            MAX_REMOTE_INFLIGHT_REQUESTS_PER_SESSION,
        );

        // Seven ordinary requests leave the eighth structural slot available
        // for exact DELETE teardown even when all ordinary bodies are SSE.
        let ordinary = (0..MAX_REMOTE_INFLIGHT_REQUESTS_PER_SESSION - 1)
            .map(|_| {
                budget
                    .try_acquire_session(false)
                    .expect("ordinary request below reserved teardown slot")
            })
            .collect::<Vec<_>>();
        assert!(budget.try_acquire_session(false).is_none());
        let teardown = budget
            .try_acquire_session(true)
            .expect("reserved teardown slot");
        assert_eq!(budget.active(), MAX_REMOTE_INFLIGHT_REQUESTS_PER_SESSION);
        assert!(budget.try_acquire_session(true).is_none());
        drop(teardown);
        drop(ordinary);
        assert_eq!(budget.active(), 0);

        // Contended CAS admission is exact: no more than eight of 64 callers
        // can hold a permit, and all are refunded after their bodies finish.
        let barrier = Arc::new(tokio::sync::Barrier::new(65));
        let attempts = Arc::new(AtomicUsize::new(0));
        let (release_tx, release_rx) = tokio::sync::watch::channel(false);
        let mut tasks = Vec::new();
        for _ in 0..64 {
            let budget = Arc::clone(&budget);
            let barrier = Arc::clone(&barrier);
            let attempts = Arc::clone(&attempts);
            let mut release_rx = release_rx.clone();
            tasks.push(tokio::spawn(async move {
                barrier.wait().await;
                let permit = budget.try_acquire();
                attempts.fetch_add(1, Ordering::AcqRel);
                let Some(_permit) = permit else {
                    return false;
                };
                while !*release_rx.borrow() {
                    release_rx.changed().await.unwrap();
                }
                true
            }));
        }
        barrier.wait().await;
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if attempts.load(Ordering::Acquire) == 64
                    && budget.active()
                        == MAX_REMOTE_INFLIGHT_REQUESTS_PER_SESSION
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("all contended HTTP admissions must finish");
        release_tx.send(true).unwrap();
        let mut admitted = 0;
        for task in tasks {
            admitted += usize::from(task.await.unwrap());
        }
        assert_eq!(admitted, MAX_REMOTE_INFLIGHT_REQUESTS_PER_SESSION);
        assert_eq!(budget.active(), 0);

        // Dropping a cancelled request future drops its permit synchronously;
        // no async cleanup worker or timeout is needed to recover this slot.
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let cancelled_budget = Arc::clone(&budget);
        let cancelled = tokio::spawn(async move {
            let _permit = cancelled_budget.try_acquire().unwrap();
            let _ = entered_tx.send(());
            futures::future::pending::<()>().await;
        });
        entered_rx.await.unwrap();
        assert_eq!(budget.active(), 1);
        cancelled.abort();
        assert!(cancelled.await.unwrap_err().is_cancelled());
        assert_eq!(budget.active(), 0);
    }

    #[tokio::test]
    async fn headerless_initialize_gate_is_per_tenant_and_not_prunable_inflight() {
        let mut limits = constrained_limits(8, 8, 16);
        limits.max_headerless_requests_per_tenant = 2;
        limits.rate_bucket_idle_retention = Duration::ZERO;
        let mut admission = RemoteSessionAdmission::new(
            Arc::<str>::from("test-user"),
            limits,
        );
        let first = owner();
        let second = owner();
        let now = Instant::now();

        let first_a = admission
            .acquire_headerless_request(&first, now)
            .unwrap();
        let first_b = admission
            .acquire_headerless_request(&first, now)
            .unwrap();
        assert!(matches!(
            admission.acquire_headerless_request(&first, now),
            Err(RemoteHttpRequestAdmissionError::CapacityExceeded)
        ));
        let second_a = admission
            .acquire_headerless_request(&second, now)
            .expect("another tenant has an independent headerless gate");

        admission.prune_idle_rate_buckets(now + Duration::from_secs(1));
        assert!(admission.tenants.contains_key(&admission.tenant_key(&first)));
        drop(first_a);
        let replacement = admission
            .acquire_headerless_request(&first, Instant::now())
            .expect("RAII drop refunds exactly one tenant slot");
        drop((first_b, second_a, replacement));
        admission.prune_idle_rate_buckets(now + Duration::from_secs(2));
        assert!(admission.tenants.is_empty());
    }

    #[tokio::test]
    async fn response_body_owns_remote_http_permit_until_drop() {
        let budget = RemoteHttpRequestBudget::new(1);
        let permit = budget.try_acquire().unwrap();
        let response = axum::response::Response::new(
            axum::body::Body::from("streaming response"),
        );
        let response =
            crate::router::response_with_request_permit(response, permit);
        assert_eq!(budget.active(), 1);
        let (parts, body) = response.into_parts();
        drop(parts);
        assert_eq!(
            budget.active(),
            1,
            "response headers must not refund a streaming body permit"
        );
        drop(body);
        assert_eq!(budget.active(), 0);
    }

    #[test]
    fn hostile_same_tenant_initialize_storm_is_hard_bounded() {
        let user = Arc::<str>::from("test-user");
        let limits = constrained_limits(12, 3, 12);
        let mut admission = RemoteSessionAdmission::new(user, limits);
        let owner = owner();
        let now = Instant::now();
        let mut accepted = 0;
        for index in 0..12 {
            let id = session_id(&format!("storm-{index}"));
            match admit_test_session(
                &mut admission,
                &id,
                7,
                &owner,
                now,
            ) {
                Ok(()) => accepted += 1,
                Err(_) => admission.release(&id, 7, now),
            }
        }
        assert_eq!(accepted, 3);
        assert_eq!(admission.reservations.len(), 3);
        assert!(admission.provisionals.is_empty());
        let usage = admission
            .tenants
            .get(&admission.tenant_key(&owner))
            .expect("tenant rate/usage record");
        assert_eq!(usage.active_sessions, 3);
        assert!(
            usage.retained_bytes <= limits.max_retained_bytes_per_tenant
        );
    }

    #[test]
    fn independent_tasks_remain_distinct_below_the_safety_fuse() {
        let limits = constrained_limits(8, 4, 8);
        let mut admission =
            RemoteSessionAdmission::new(Arc::<str>::from("test-user"), limits);
        let first_owner = owner();
        let second_owner = owner();
        let now = Instant::now();
        for (id, manager_id, owner_user_id) in [
            ("task-a", 1, &first_owner),
            ("task-b", 1, &first_owner),
            ("task-c", 2, &second_owner),
        ] {
            admit_test_session(
                &mut admission,
                &session_id(id),
                manager_id,
                owner_user_id,
                now,
            )
            .unwrap();
        }
        assert_eq!(admission.reservations.len(), 3);
        assert_eq!(admission.tenants.len(), 2);
        assert_ne!(
            admission.reservations[&session_id("task-a")].tenant,
            admission.reservations[&session_id("task-c")].tenant
        );
    }

    #[test]
    fn close_refunds_exact_session_and_reuses_one_slot() {
        let limits = constrained_limits(2, 1, 4);
        let mut admission =
            RemoteSessionAdmission::new(Arc::<str>::from("test-user"), limits);
        let owner = owner();
        let now = Instant::now();
        let first = session_id("refund-first");
        admit_test_session(&mut admission, &first, 1, &owner, now)
            .unwrap();
        admission.release(&first, 1, now);
        assert!(admission.reservations.is_empty());
        assert_eq!(admission.total_retained_bytes, 0);

        let replacement = session_id("refund-replacement");
        admit_test_session(
            &mut admission,
            &replacement,
            1,
            &owner,
            now,
        )
        .unwrap();
        assert_eq!(admission.reservations.len(), 1);
    }

    #[test]
    fn initialize_rate_rejection_leaves_no_half_registered_session() {
        let limits = constrained_limits(8, 8, 2);
        let mut admission =
            RemoteSessionAdmission::new(Arc::<str>::from("test-user"), limits);
        let owner = owner();
        let now = Instant::now();
        for index in 0..2 {
            admit_test_session(
                &mut admission,
                &session_id(&format!("rate-{index}")),
                1,
                &owner,
                now,
            )
            .unwrap();
        }
        let rejected = session_id("rate-rejected");
        assert!(
            admit_test_session(
                &mut admission,
                &rejected,
                1,
                &owner,
                now,
            )
            .is_err()
        );
        admission.release(&rejected, 1, now);
        assert!(!admission.reservations.contains_key(&rejected));
        assert!(!admission.provisionals.contains_key(&rejected));
        assert_eq!(admission.reservations.len(), 2);
    }

    #[test]
    fn remote_scope_is_bounded_and_unknown_domains_fail_closed() {
        assert_eq!(
            bounded_scope_from_query(Some("domains=agent,files,agent"))
                .unwrap(),
            Some(canonical_scope(["agent", "files"]))
        );
        assert!(
            bounded_scope_from_query(Some("domains=not-a-real-domain"))
                .is_err()
        );
        let too_many = format!(
            "domains={}",
            std::iter::repeat_n("agent", MAX_REMOTE_SCOPE_DOMAINS + 1)
                .collect::<Vec<_>>()
                .join(",")
        );
        assert!(bounded_scope_from_query(Some(&too_many)).is_err());
        let oversized = format!(
            "domains={}",
            "x".repeat(MAX_REMOTE_SCOPE_DOMAIN_BYTES + 1)
        );
        assert!(bounded_scope_from_query(Some(&oversized)).is_err());
        assert!(
            bounded_scope_from_query(Some(&format!(
                "x={}",
                "y".repeat(MAX_REMOTE_SCOPE_QUERY_BYTES)
            )))
            .is_err()
        );
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn close_session_revokes_only_the_exact_remote_session() {
        let registry = CleanupRegistryProbe::fail_next(0);
        let manager = manager_with_cleanup(Arc::clone(&registry));
        let (first, cleanup) = create_and_bind_session(&manager).await;
        let (second, _) = create_and_bind_session(&manager).await;

        manager.close_session(&first).await.unwrap();

        assert_eq!(
            registry.revocations().await,
            vec![first.as_ref().to_owned()]
        );
        assert!(!manager.bindings.read().await.contains_key(&first));
        assert!(manager.bindings.read().await.contains_key(&second));
        assert!(manager.has_session(&second).await.unwrap());
        assert_eq!(cleanup.pending_count().await, 0);

        manager.close_session(&second).await.unwrap();
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn abandoned_preinitialize_session_is_swept_by_exact_id() {
        let registry = CleanupRegistryProbe::fail_next(0);
        let manager = manager_with_cleanup(registry);
        let (id, transport) = manager.create_session().await.unwrap();
        drop(transport);
        {
            let mut admission = manager.admission.lock().await;
            let provisional = admission
                .provisionals
                .get_mut(&id)
                .expect("new transport has provisional authority");
            provisional.created_at = Instant::now()
                .checked_sub(REMOTE_PROVISIONAL_SESSION_TIMEOUT)
                .unwrap();
        }

        sweep_remote_provisional_sessions(
            manager.inner.as_ref(),
            manager.admission.as_ref(),
            false,
            manager.manager_id,
        )
        .await;

        assert!(!manager.has_session(&id).await.unwrap());
        let admission = manager.admission.lock().await;
        assert!(!admission.provisionals.contains_key(&id));
        assert!(!admission.reservations.contains_key(&id));
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn public_transport_layers_reject_before_any_session_registration() {
        use rmcp::ServerHandler;
        use rmcp::transport::streamable_http_server::{
            StreamableHttpServerConfig, StreamableHttpService,
        };
        use tower::ServiceExt;
        use tower_http::limit::RequestBodyLimitLayer;

        #[derive(Clone, Copy)]
        struct TestHandler;
        impl ServerHandler for TestHandler {}

        let registry = CleanupRegistryProbe::fail_next(0);
        let manager = Arc::new(manager_with_cleanup(registry));
        let service = StreamableHttpService::new(
            || Ok(TestHandler),
            Arc::clone(&manager),
            StreamableHttpServerConfig::default().disable_allowed_hosts(),
        );
        let app = axum::Router::new()
            .fallback_service(service)
            .layer(RequestBodyLimitLayer::new(
                nomifun_common::constants::BODY_LIMIT,
            ))
            .layer(axum::middleware::from_fn(
                crate::router::initialize_preflight_middleware,
            ));

        let oversized = app
            .clone()
            .oneshot(
                axum::http::Request::post("/")
                    .body(axum::body::Body::from(vec![
                        b'x';
                        nomifun_common::constants::BODY_LIMIT + 1
                    ]))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            oversized.status(),
            axum::http::StatusCode::PAYLOAD_TOO_LARGE
        );

        for _ in 0..16 {
            let response = app
                .clone()
                .oneshot(
                    axum::http::Request::post("/")
                        .body(axum::body::Body::from(
                            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#,
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
        }

        assert!(manager.inner.sessions.read().await.is_empty());
        assert!(manager.bindings.read().await.is_empty());
        let admission = manager.admission.lock().await;
        assert!(admission.provisionals.is_empty());
        assert!(admission.reservations.is_empty());
        assert_eq!(admission.total_retained_bytes, 0);
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn worker_idle_exit_flows_through_close_session_and_revoke() {
        use rmcp::ServerHandler;
        use rmcp::transport::streamable_http_server::{
            StreamableHttpServerConfig, StreamableHttpService,
        };

        #[derive(Clone, Copy)]
        struct TestHandler;
        impl ServerHandler for TestHandler {}

        let registry = CleanupRegistryProbe::fail_next(0);
        let manager = manager_with_cleanup_and_keep_alive(
            Arc::clone(&registry),
            Duration::from_millis(40),
        );
        let service = StreamableHttpService::new(
            || Ok(TestHandler),
            Arc::clone(&manager),
            StreamableHttpServerConfig::default()
                .disable_allowed_hosts()
                .with_sse_keep_alive(None),
        );
        let app = axum::Router::new().nest_service("/mcp", service);

        let response = tower::ServiceExt::oneshot(
            app,
            axum::http::Request::post("/mcp")
                .header("host", "localhost")
                .header("content-type", "application/json")
                .header("accept", "application/json, text/event-stream")
                .extension(crate::router::RemoteInstanceOwner(owner()))
                .body(axum::body::Body::from(
                    r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
        if response.status() != axum::http::StatusCode::OK {
            let status = response.status();
            let body =
                axum::body::to_bytes(response.into_body(), usize::MAX)
                    .await
                    .expect("read failed initialize response");
            panic!(
                "initialize fixture returned {status}: {}",
                String::from_utf8_lossy(&body)
            );
        }
        let id = session_id(
            response.headers()["mcp-session-id"]
                .to_str()
                .expect("session id header"),
        );

        wait_until("idle worker session removal", || async {
            !manager.has_session(&id).await.unwrap()
        })
        .await;
        registry.wait_for_revocations(1).await;
        assert_eq!(
            registry.revocations().await,
            vec![id.as_ref().to_owned()]
        );
        assert!(!manager.bindings.read().await.contains_key(&id));
        let admission = manager.admission.lock().await;
        assert!(!admission.reservations.contains_key(&id));
        assert!(!admission.provisionals.contains_key(&id));
        assert_eq!(admission.total_retained_bytes, 0);
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn failed_initialization_revokes_and_discards_the_session() {
        let registry = CleanupRegistryProbe::fail_next(0);
        let manager = manager_with_cleanup(Arc::clone(&registry));
        let (id, transport) = manager.create_session().await.unwrap();
        drop(transport);

        let mut message = initialize_request();
        let ClientJsonRpcMessage::Request(request) = &mut message else {
            unreachable!("initialize helper returns a request")
        };
        let (mut parts, _) = axum::http::Request::builder()
            .uri("/mcp")
            .body(())
            .unwrap()
            .into_parts();
        parts
            .extensions
            .insert(crate::router::RemoteInstanceOwner(owner()));
        request.request.extensions_mut().insert(parts);

        let result = manager.initialize_session(&id, message).await;
        assert!(result.is_err(), "the detached worker must fail initialize");
        assert!(!manager.has_session(&id).await.unwrap());
        assert!(!manager.bindings.read().await.contains_key(&id));
        assert_eq!(
            registry.revocations().await,
            vec![id.as_ref().to_owned()]
        );
        let admission = manager.admission.lock().await;
        assert!(!admission.reservations.contains_key(&id));
        assert!(!admission.provisionals.contains_key(&id));
        assert_eq!(admission.total_retained_bytes, 0);
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn failed_revoke_is_retried_by_the_durable_worker() {
        let registry = CleanupRegistryProbe::fail_next(1);
        let manager = manager_with_cleanup(Arc::clone(&registry));
        let (id, cleanup) = create_and_bind_session(&manager).await;

        manager.close_session(&id).await.unwrap();
        assert_eq!(cleanup.pending_count().await, 1);
        assert!(manager.admission.lock().await.reservations.contains_key(&id));

        registry.wait_for_revocations(2).await;
        wait_until("durable cleanup pending set to drain", || async {
            cleanup.pending_count().await == 0
        })
        .await;
        assert_eq!(
            registry.revocations().await,
            vec![id.as_ref().to_owned(), id.as_ref().to_owned()]
        );
        assert!(!manager.admission.lock().await.reservations.contains_key(&id));
        assert!(registry.retry_calls.load(Ordering::Acquire) >= 1);
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn cancelled_successful_revoke_keeps_exact_refund_debt() {
        let registry = CleanupRegistryProbe::fail_next(0);
        let manager_id =
            NEXT_REMOTE_SESSION_MANAGER_ID.fetch_add(1, Ordering::Relaxed);
        let id = session_id("cancelled-refund-publication");
        let binding = binding();
        let admission = Arc::new(tokio::sync::Mutex::new(
            RemoteSessionAdmission::new(
                Arc::<str>::from("test-user"),
                RemoteSessionLimits::for_resources(
                    16 * 1024 * 1024 * 1024,
                    8,
                ),
            ),
        ));
        {
            let mut admission_guard = admission.lock().await;
            admission_guard
                .reserve_provisional(&id, manager_id, Instant::now())
                .unwrap();
            let retained_bytes = binding_retained_bytes(
                &id,
                admission_guard.authoritative_user_id.as_ref(),
                &binding.owner_user_id,
                binding.scope.as_deref(),
            )
            .unwrap();
            admission_guard
                .reserve(
                    &id,
                    manager_id,
                    &binding.owner_user_id,
                    retained_bytes,
                    Instant::now(),
                )
                .unwrap();
        }
        let state = Arc::new(RemoteBrowserCleanupState {
            registry: Arc::clone(&registry)
                as Arc<dyn RemoteBrowserCleanupRegistry>,
            bindings: Arc::new(tokio::sync::RwLock::new(HashMap::from([(
                id.clone(),
                binding,
            )]))),
            admission: Arc::clone(&admission),
            manager_id,
            pending_sessions: tokio::sync::Mutex::new(
                PendingRemoteBrowserCleanups::default(),
            ),
        });
        assert!(state.pending_sessions.lock().await.insert(id.as_ref()));

        // Force the cleanup future to stop after registry success but before it
        // can acquire admission. The pending id must remain retryable until the
        // atomic pending-remove + reservation-refund publication can complete.
        let admission_guard = admission.lock().await;
        let task_state = Arc::clone(&state);
        let id_for_task = id.as_ref().to_owned();
        let cleanup_task = tokio::spawn(async move {
            try_revoke_remote_browser_session(&task_state, &id_for_task).await;
        });
        registry.wait_for_revocations(1).await;
        cleanup_task.abort();
        assert!(cleanup_task.await.unwrap_err().is_cancelled());
        drop(admission_guard);

        assert_eq!(state.pending_sessions.lock().await.ids.len(), 1);
        assert!(admission.lock().await.reservations.contains_key(&id));
        try_revoke_remote_browser_session(&state, id.as_ref()).await;
        assert!(state.pending_sessions.lock().await.ids.is_empty());
        assert!(!admission.lock().await.reservations.contains_key(&id));
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn final_cleanup_survives_panics_and_more_than_four_failures() {
        let registry = CleanupRegistryProbe::panic_then_fail(2, 8);
        let manager = manager_with_cleanup(Arc::clone(&registry));
        let (id, cleanup) = create_and_bind_session(&manager).await;
        let admission = Arc::clone(&manager.admission);

        // The manager and last external authority both disappear while the
        // session is still bound. Its one fixed worker must survive adapter
        // panics and retry beyond the historical four-attempt cutoff.
        drop(manager);
        drop(cleanup);
        registry.wait_for_revocations(11).await;
        wait_until("final cleanup admission refund", || async {
            !admission.lock().await.reservations.contains_key(&id)
        })
        .await;
        assert_eq!(
            registry
                .revocations()
                .await
                .iter()
                .filter(|revoked| revoked.as_str() == id.as_ref())
                .count(),
            11
        );
        assert!(registry.retry_calls.load(Ordering::Acquire) >= 9);
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn cleanup_authority_drop_drains_still_bound_sessions() {
        let registry = CleanupRegistryProbe::fail_next(0);
        let bindings = Arc::new(tokio::sync::RwLock::new(HashMap::new()));
        let first = session_id("remote-drop-first");
        let second = session_id("remote-drop-second");
        bindings.write().await.insert(first.clone(), binding());
        bindings.write().await.insert(second.clone(), binding());
        let manager_id =
            NEXT_REMOTE_SESSION_MANAGER_ID.fetch_add(1, Ordering::Relaxed);
        let admission = Arc::new(tokio::sync::Mutex::new(
            RemoteSessionAdmission::new(
                Arc::<str>::from("test-user"),
                RemoteSessionLimits::for_resources(16 * 1024 * 1024 * 1024, 8),
            ),
        ));
        let cleanup = cleanup_authority(
            Arc::clone(&registry),
            Arc::clone(&bindings),
            admission,
            manager_id,
        );

        drop(cleanup);
        registry.wait_for_revocations(2).await;

        let mut revoked = registry.revocations().await;
        revoked.sort();
        assert_eq!(
            revoked,
            vec![first.as_ref().to_owned(), second.as_ref().to_owned()]
        );
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn stale_session_cleanup_cannot_revoke_a_replacement_session() {
        let registry = CleanupRegistryProbe::fail_next(0);
        let manager = manager_with_cleanup(Arc::clone(&registry));
        let (stale, _) = create_and_bind_session(&manager).await;
        let (replacement, _) = create_and_bind_session(&manager).await;

        manager.close_session(&stale).await.unwrap();

        assert_eq!(
            registry.revocations().await,
            vec![stale.as_ref().to_owned()]
        );
        assert!(manager.bindings.read().await.contains_key(&replacement));
        assert!(manager.has_session(&replacement).await.unwrap());

        manager.close_session(&replacement).await.unwrap();
    }
}
