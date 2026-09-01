//! Transport-only admission and lifecycle for the canonical Remote MCP front door.
//!
//! rmcp's session id is a connection identity only. Product work always uses
//! the explicit `AgentSessionId` carried by the canonical MCP operations.

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use futures::{FutureExt, Stream};
use nomifun_common::UserId;
use rmcp::{
    model::{ClientJsonRpcMessage, GetExtensions, ServerJsonRpcMessage},
    transport::{
        WorkerTransport,
        common::server_side_http::session_id,
        streamable_http_server::session::{
            RestoreOutcome, ServerSseMessage, SessionId, SessionManager,
            local::{
                LocalSessionManager, LocalSessionManagerError, LocalSessionWorker,
                create_local_session,
            },
        },
    },
};
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RemoteMcpSessionId(pub SessionId);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RemoteMcpSessionIdentity {
    pub session_id: SessionId,
    pub owner_user_id: UserId,
}

#[derive(Clone, Debug)]
struct RemoteMcpSessionBinding {
    owner_user_id: UserId,
    request_budget: Arc<RemoteHttpRequestBudget>,
}

impl RemoteMcpSessionBinding {
    fn new(owner_user_id: UserId, request_limit: usize) -> Self {
        Self {
            owner_user_id,
            request_budget: RemoteHttpRequestBudget::new(request_limit),
        }
    }
}

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
static NEXT_REMOTE_SESSION_MANAGER_ID: AtomicU64 = AtomicU64::new(1);

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
        let limit = if teardown {
            self.limit
        } else {
            self.limit.saturating_sub(1).max(1)
        };
        self.try_acquire_below(limit)
    }

    fn try_acquire_below(
        self: &Arc<Self>,
        limit: usize,
    ) -> Option<RemoteHttpRequestPermit> {
        self.active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < limit).then_some(active + 1)
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

/// A request permit must share the original budget's atomic counter.
///
/// This wrapper avoids exposing the budget internals while keeping the permit
/// cheap and cancellation-safe.
#[derive(Debug)]
#[must_use = "dropping the permit releases the request slot"]
pub(crate) struct RemoteHttpRequestPermit {
    budget: Arc<RemoteHttpRequestBudget>,
}

impl Drop for RemoteHttpRequestPermit {
    fn drop(&mut self) {
        let previous = self.budget.active.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0);
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
        let cpus = std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1);
        let mut system = sysinfo::System::new();
        system.refresh_memory();
        Self::for_resources(cpus, system.total_memory())
    }

    fn for_resources(cpus: usize, memory_bytes: u64) -> Self {
        let cpus = cpus.max(1);
        let cpu_capacity = cpus.saturating_mul(REMOTE_SESSIONS_PER_LOGICAL_CPU);
        let memory_capacity = usize::try_from(
            memory_bytes / REMOTE_SESSION_ASSUMED_MEMORY_BYTES,
        )
        .unwrap_or(usize::MAX);
        let max_active_global = cpu_capacity
            .min(memory_capacity)
            .clamp(MIN_GLOBAL_REMOTE_SESSIONS, MAX_GLOBAL_REMOTE_SESSIONS);
        let max_active_per_tenant = cpus
            .saturating_mul(REMOTE_SESSIONS_PER_TENANT_CPU)
            .clamp(
                MIN_ACTIVE_REMOTE_SESSIONS_PER_TENANT,
                MAX_ACTIVE_REMOTE_SESSIONS_PER_TENANT,
            )
            .min(max_active_global);
        let burst = max_active_per_tenant
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
            max_rate_tenants_global: max_active_global.saturating_mul(2),
            max_binding_retained_bytes: MAX_REMOTE_BINDING_RETAINED_BYTES,
            initialize_burst_per_tenant: burst,
            initialize_refill_interval: REMOTE_INITIALIZE_REFILL_INTERVAL,
            rate_bucket_idle_retention: REMOTE_RATE_BUCKET_IDLE_RETENTION,
            max_inflight_requests_per_session: MAX_REMOTE_INFLIGHT_REQUESTS_PER_SESSION,
            max_headerless_requests_per_tenant: MAX_REMOTE_HEADERLESS_REQUESTS_PER_TENANT,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct TenantKey {
    owner_user_id: UserId,
}

#[derive(Clone, Debug)]
struct Reservation {
    manager_id: u64,
    tenant: TenantKey,
    retained_bytes: usize,
}

#[derive(Clone, Copy, Debug)]
struct Provisional {
    manager_id: u64,
    created_at: Instant,
    closing: bool,
}

#[derive(Clone, Debug)]
struct TenantAdmission {
    active_sessions: usize,
    retained_bytes: usize,
    initialize_tokens: u32,
    last_refill: Instant,
    last_activity: Instant,
    headerless_budget: Arc<RemoteHttpRequestBudget>,
}

impl TenantAdmission {
    fn new(now: Instant, limits: RemoteSessionLimits) -> Self {
        Self {
            active_sessions: 0,
            retained_bytes: 0,
            initialize_tokens: limits.initialize_burst_per_tenant,
            last_refill: now,
            last_activity: now,
            headerless_budget: RemoteHttpRequestBudget::new(
                limits.max_headerless_requests_per_tenant,
            ),
        }
    }

    fn refill(&mut self, now: Instant, limits: RemoteSessionLimits) {
        let elapsed = now.saturating_duration_since(self.last_refill);
        let interval = limits.initialize_refill_interval.as_nanos();
        if interval == 0 {
            self.initialize_tokens = limits.initialize_burst_per_tenant;
            self.last_refill = now;
            return;
        }
        let refill = elapsed.as_nanos() / interval;
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
    tenants: HashMap<TenantKey, TenantAdmission>,
    reservations: HashMap<SessionId, Reservation>,
    provisionals: HashMap<SessionId, Provisional>,
    total_retained_bytes: usize,
    limits: RemoteSessionLimits,
}

impl RemoteSessionAdmission {
    fn new(limits: RemoteSessionLimits) -> Self {
        Self {
            tenants: HashMap::new(),
            reservations: HashMap::new(),
            provisionals: HashMap::new(),
            total_retained_bytes: 0,
            limits,
        }
    }

    fn tenant(&self, owner: &UserId) -> TenantKey {
        TenantKey {
            owner_user_id: owner.clone(),
        }
    }

    fn acquire_headerless(
        &mut self,
        owner: &UserId,
        now: Instant,
    ) -> Result<RemoteHttpRequestPermit, RemoteHttpRequestAdmissionError> {
        self.prune(now);
        let key = self.tenant(owner);
        if !self.tenants.contains_key(&key)
            && self.tenants.len() >= self.limits.max_rate_tenants_global
        {
            return Err(RemoteHttpRequestAdmissionError::CapacityExceeded);
        }
        let tenant = self
            .tenants
            .entry(key)
            .or_insert_with(|| TenantAdmission::new(now, self.limits));
        tenant.last_activity = now;
        tenant
            .headerless_budget
            .try_acquire()
            .ok_or(RemoteHttpRequestAdmissionError::CapacityExceeded)
    }

    fn reserve_provisional(
        &mut self,
        id: &SessionId,
        manager_id: u64,
        now: Instant,
    ) -> Result<(), std::io::Error> {
        if self.provisionals.contains_key(id) || self.reservations.contains_key(id) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "Remote MCP session id is already registered",
            ));
        }
        if self
            .provisionals
            .len()
            .saturating_add(self.reservations.len())
            >= self.limits.max_active_global
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "Remote MCP transport session capacity is temporarily exhausted",
            ));
        }
        self.provisionals.insert(
            id.clone(),
            Provisional {
                manager_id,
                created_at: now,
                closing: false,
            },
        );
        Ok(())
    }

    fn reserve(
        &mut self,
        id: &SessionId,
        manager_id: u64,
        owner: &UserId,
        retained_bytes: usize,
        now: Instant,
    ) -> Result<(), std::io::Error> {
        if retained_bytes > self.limits.max_binding_retained_bytes {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "Remote MCP session identity exceeds its retained-memory limit",
            ));
        }
        if self
            .provisionals
            .get(id)
            .is_none_or(|provisional| {
                provisional.manager_id != manager_id || provisional.closing
            })
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "Remote MCP provisional session is unavailable",
            ));
        }
        let key = self.tenant(owner);
        let tenant = self
            .tenants
            .entry(key.clone())
            .or_insert_with(|| TenantAdmission::new(now, self.limits));
        tenant.refill(now, self.limits);
        if tenant.initialize_tokens == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "Remote MCP initialize rate limit exceeded",
            ));
        }
        if tenant.active_sessions >= self.limits.max_active_per_tenant
            || self.total_retained_bytes.saturating_add(retained_bytes)
                > self.limits.max_retained_bytes_global
            || tenant.retained_bytes.saturating_add(retained_bytes)
                > self.limits.max_retained_bytes_per_tenant
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "Remote MCP active session capacity is temporarily exhausted",
            ));
        }
        tenant.initialize_tokens -= 1;
        tenant.active_sessions += 1;
        tenant.retained_bytes += retained_bytes;
        self.total_retained_bytes += retained_bytes;
        self.reservations.insert(
            id.clone(),
            Reservation {
                manager_id,
                tenant: key,
                retained_bytes,
            },
        );
        self.provisionals.remove(id);
        Ok(())
    }

    fn release(&mut self, id: &SessionId, manager_id: u64, now: Instant) {
        self.provisionals.remove(id);
        let Some(reservation) = self
            .reservations
            .get(id)
            .filter(|reservation| reservation.manager_id == manager_id)
            .cloned()
        else {
            return;
        };
        self.reservations.remove(id);
        self.total_retained_bytes = self
            .total_retained_bytes
            .saturating_sub(reservation.retained_bytes);
        if let Some(tenant) = self.tenants.get_mut(&reservation.tenant) {
            tenant.active_sessions = tenant.active_sessions.saturating_sub(1);
            tenant.retained_bytes = tenant
                .retained_bytes
                .saturating_sub(reservation.retained_bytes);
            tenant.last_activity = now;
        }
        self.prune(now);
    }

    fn claim_expired(
        &mut self,
        now: Instant,
        manager_id: u64,
        all: bool,
    ) -> Vec<SessionId> {
        self.provisionals
            .iter_mut()
            .filter_map(|(id, provisional)| {
                if provisional.manager_id == manager_id
                    && !provisional.closing
                    && (all
                        || now.saturating_duration_since(provisional.created_at)
                            >= REMOTE_PROVISIONAL_SESSION_TIMEOUT)
                {
                    provisional.closing = true;
                    Some(id.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    fn finish_provisional(&mut self, id: &SessionId, manager_id: u64) {
        if self
            .provisionals
            .get(id)
            .is_some_and(|provisional| provisional.manager_id == manager_id)
        {
            self.provisionals.remove(id);
        }
    }

    fn prune(&mut self, now: Instant) {
        let limits = self.limits;
        self.tenants.retain(|_, tenant| {
            tenant.refill(now, limits);
            tenant.active_sessions > 0
                || tenant.headerless_budget.active() > 0
                || tenant.initialize_tokens < limits.initialize_burst_per_tenant
                || now.saturating_duration_since(tenant.last_activity)
                    < limits.rate_bucket_idle_retention
        });
    }
}

#[derive(Clone)]
struct ProvisionalCleanupAuthority {
    shutdown: Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
}

impl ProvisionalCleanupAuthority {
    fn new(
        inner: Arc<LocalSessionManager>,
        admission: Arc<tokio::sync::Mutex<RemoteSessionAdmission>>,
        manager_id: u64,
    ) -> Self {
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(REMOTE_PROVISIONAL_SWEEP_INTERVAL);
            loop {
                let final_drain = tokio::select! {
                    _ = interval.tick() => false,
                    _ = &mut shutdown_rx => true,
                };
                let ids = admission
                    .lock()
                    .await
                    .claim_expired(Instant::now(), manager_id, final_drain);
                for id in ids {
                    let _ = inner.close_session(&id).await;
                    admission.lock().await.finish_provisional(&id, manager_id);
                }
                if final_drain {
                    break;
                }
            }
        });
        Self {
            shutdown: Arc::new(std::sync::Mutex::new(Some(shutdown_tx))),
        }
    }
}

impl Drop for ProvisionalCleanupAuthority {
    fn drop(&mut self) {
        if Arc::strong_count(&self.shutdown) != 1 {
            return;
        }
        if let Some(sender) = self
            .shutdown
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            let _ = sender.send(());
        }
    }
}

#[derive(Debug, Error)]
pub(crate) enum RemoteSessionManagerError {
    #[error(transparent)]
    Local(#[from] LocalSessionManagerError),
    #[error("{0}")]
    Transport(#[from] std::io::Error),
}

#[derive(Clone)]
pub struct RemoteMcpSessionAdmissionAuthority {
    admission: Arc<tokio::sync::Mutex<RemoteSessionAdmission>>,
    owner_user_id: UserId,
}

impl RemoteMcpSessionAdmissionAuthority {
    pub fn for_owner(owner_user_id: &UserId) -> Self {
        Self {
            admission: Arc::new(tokio::sync::Mutex::new(
                RemoteSessionAdmission::new(RemoteSessionLimits::for_machine()),
            )),
            owner_user_id: owner_user_id.clone(),
        }
    }
}

pub(crate) struct RemoteSessionManager {
    manager_id: u64,
    inner: Arc<LocalSessionManager>,
    bindings: Arc<tokio::sync::RwLock<HashMap<SessionId, RemoteMcpSessionBinding>>>,
    admission: Arc<tokio::sync::Mutex<RemoteSessionAdmission>>,
    _provisional_cleanup: ProvisionalCleanupAuthority,
}

impl RemoteSessionManager {
    pub(crate) fn with_owner_admission_authority(
        owner_user_id: UserId,
        authority: RemoteMcpSessionAdmissionAuthority,
    ) -> Self {
        let manager_id = NEXT_REMOTE_SESSION_MANAGER_ID.fetch_add(1, Ordering::Relaxed);
        let admission = if authority.owner_user_id == owner_user_id {
            Arc::clone(&authority.admission)
        } else {
            tracing::error!(
                "canonical Remote MCP admission authority owner mismatch; isolating endpoint"
            );
            RemoteMcpSessionAdmissionAuthority::for_owner(&owner_user_id).admission
        };
        let inner = Arc::new(LocalSessionManager::default());
        let provisional_cleanup = ProvisionalCleanupAuthority::new(
            Arc::clone(&inner),
            Arc::clone(&admission),
            manager_id,
        );
        Self {
            manager_id,
            inner,
            bindings: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            admission,
            _provisional_cleanup: provisional_cleanup,
        }
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
        .ok_or_else(|| std::io::Error::other("Remote MCP request has no HTTP request parts"))?;
        parts
            .extensions
            .get::<crate::router::RemoteInstanceOwner>()
            .map(|owner| owner.0.clone())
            .ok_or_else(|| std::io::Error::other("Remote MCP request has no owner identity"))
    }

    async fn inject_identity(
        &self,
        id: &SessionId,
        message: &mut ClientJsonRpcMessage,
        pin_if_missing: bool,
    ) -> Result<(), std::io::Error> {
        let owner = Self::owner_from_message(message)?;
        let mut admission = self.admission.lock().await;
        let mut bindings = self.bindings.write().await;
        match bindings.get(id) {
            Some(binding) if binding.owner_user_id != owner => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "Remote MCP session is bound to a different installation owner",
                ));
            }
            None if pin_if_missing => {
                let retained_bytes = id
                    .as_ref()
                    .len()
                    .saturating_add(owner.as_ref().len())
                    .saturating_add(256);
                admission.reserve(
                    id,
                    self.manager_id,
                    &owner,
                    retained_bytes,
                    Instant::now(),
                )?;
                bindings.insert(
                    id.clone(),
                    RemoteMcpSessionBinding::new(
                        owner.clone(),
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
            Some(_) if !admission.reservations.contains_key(id) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "Remote MCP session has no retained admission authority",
                ));
            }
            Some(_) => {}
        }
        let binding = bindings
            .get(id)
            .expect("session binding must exist after admission")
            .clone();
        drop(bindings);
        drop(admission);
        message.insert_extension(RemoteMcpSessionIdentity {
            session_id: id.clone(),
            owner_user_id: binding.owner_user_id,
        });
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
        tokio::spawn(async move {
            let result = std::panic::AssertUnwindSafe(inner.close_session(&id))
                .catch_unwind()
                .await;
            let mut admission = admission.lock().await;
            bindings.write().await.remove(&id);
            admission.release(&id, manager_id, Instant::now());
            match result {
                Ok(Ok(())) => Ok(()),
                Ok(Err(error)) => Err(RemoteSessionManagerError::Local(error)),
                Err(_) => Err(RemoteSessionManagerError::Transport(
                    std::io::Error::other("Remote MCP session close panicked"),
                )),
            }
        })
        .await
        .map_err(|error| {
            RemoteSessionManagerError::Transport(std::io::Error::other(
                format!("Remote MCP session finalizer failed: {error}"),
            ))
        })??;
        Ok(())
    }

    pub(crate) async fn acquire_http_request_permit(
        &self,
        id: Option<&SessionId>,
        owner: &UserId,
        headerless_post: bool,
        teardown: bool,
    ) -> Result<Option<RemoteHttpRequestPermit>, RemoteHttpRequestAdmissionError> {
        if let Some(id) = id {
            let bindings = self.bindings.read().await;
            let Some(binding) = bindings.get(id) else {
                return Err(RemoteHttpRequestAdmissionError::IdentityMismatch);
            };
            if binding.owner_user_id != *owner {
                return Err(RemoteHttpRequestAdmissionError::IdentityMismatch);
            }
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
                .acquire_headerless(owner, Instant::now())
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
        let (handle, worker) =
            create_local_session(id.clone(), self.inner.session_config.clone());
        let mut sessions = self.inner.sessions.write().await;
        sessions.insert(id.clone(), handle);
        drop(sessions);
        Ok((id, WorkerTransport::spawn(worker)))
    }

    async fn initialize_session(
        &self,
        id: &SessionId,
        mut message: ClientJsonRpcMessage,
    ) -> Result<ServerJsonRpcMessage, Self::Error> {
        if let Err(error) = self.inject_identity(id, &mut message, true).await {
            let _ = self.close_session_durably(id).await;
            return Err(error.into());
        }
        match self.inner.initialize_session(id, message).await {
            Ok(response) => Ok(response),
            Err(error) => {
                let _ = self.close_session_durably(id).await;
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
        self.inject_identity(id, &mut message, false).await?;
        Ok(self.inner.create_stream(id, message).await?)
    }

    async fn accept_message(
        &self,
        id: &SessionId,
        mut message: ClientJsonRpcMessage,
    ) -> Result<(), Self::Error> {
        if !matches!(
            message,
            ClientJsonRpcMessage::Response(_) | ClientJsonRpcMessage::Error(_)
        ) {
            self.inject_identity(id, &mut message, false).await?;
        } else if !self.bindings.read().await.contains_key(id) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "Remote MCP session has no pinned identity",
            )
            .into());
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
        Ok(RestoreOutcome::NotSupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admission_limits_scale_without_a_fixed_global_byte_cap() {
        let limits = RemoteSessionLimits::for_resources(8, 32 * 1024 * 1024 * 1024);
        assert!(limits.max_active_global >= MIN_GLOBAL_REMOTE_SESSIONS);
        assert!(limits.max_active_global <= MAX_GLOBAL_REMOTE_SESSIONS);
        assert!(limits.max_active_per_tenant <= limits.max_active_global);
    }

    #[tokio::test]
    async fn request_budget_reserves_a_slot_and_refunds_on_drop() {
        let budget = RemoteHttpRequestBudget::new(1);
        let permit = budget.try_acquire().expect("first request slot");
        assert!(budget.try_acquire().is_none());
        drop(permit);
        assert!(budget.try_acquire().is_some());
    }
}
