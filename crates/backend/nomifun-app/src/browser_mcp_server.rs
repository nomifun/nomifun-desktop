//! Authenticated loopback adapter from ACP browser stdio children to the
//! process-wide [`BrowserSessionHub`].
//!
//! This server is a transport boundary only. It never launches Chromium and it
//! never exposes a CDP endpoint, debugging port, profile path, cookie, or
//! storage value. The signed child capability supplies immutable user,
//! conversation, runtime, ACP audience, tool, and browser-operation scope; the
//! server derives the platform caller identity exclusively from those verified
//! claims.

use std::collections::{BTreeSet, HashMap};
use std::io;
use std::net::{Shutdown, SocketAddr};
use std::pin::Pin;
use std::sync::{Arc, Mutex as StdMutex, RwLock, Weak};
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::Json;
use axum::extract::{DefaultBodyLimit, FromRequest, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::serve::Listener;
use axum::Extension;
use nomi_browser::{
    ManagedBrowserFacade, TRUSTED_OWNER_INPUT_FIELDS, managed_result_envelope,
};
use nomifun_api_types::{
    BROWSER_CAPABILITY_DOMAIN, BROWSER_MCP_TOOL_NAMES, BrowserCapabilityClaims,
    BrowserCapabilityOperation, BrowserCapabilityScope, BrowserCapabilitySurface,
    BrowserMcpConfig, browser_tool_operation,
};
use nomifun_browser_platform::{
    BrowserErrorCode, BrowserOperationKind, BrowserPlatformError, BrowserSessionHub,
    BrowserSurface, CallerIdentity, OwnerLeaseId, TaskResourceFamilyKey,
};
use nomifun_common::{
    LOOPBACK_CAPABILITY_RENEW_PATH, LOOPBACK_CAPABILITY_REVOKE_PATH,
    LoopbackCapabilityError, LoopbackCapabilityIssuer, LoopbackCapabilityRenewalRequest,
    LoopbackSessionKind,
};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};
use tower_http::limit::RequestBodyLimitLayer;
use tracing::{debug, warn};

const REVOKED_LEASE_SWEEP_INTERVAL: Duration = Duration::from_millis(500);
const CLEANUP_RETRY_WAIT: Duration = Duration::from_millis(50);
const CLEANUP_RETRY_MAX_WAIT: Duration = Duration::from_secs(2);
const REVOKED_BINDING_CLEANUP_CONCURRENCY: usize = 16;
/// Accepted requests get a bounded grace period during App shutdown. Both the
/// complete-body deadline and normal task-admission wait fit inside it. A
/// request that still has not returned is cancelled by the outer middleware;
/// this timeout is the final supervisor failsafe, not an operation/RSS cap.
const BROWSER_MCP_INGRESS_SHUTDOWN_GRACE: Duration = Duration::from_secs(8);
const BROWSER_MCP_PERIODIC_CLEANUP_STOP_GRACE: Duration = Duration::from_millis(250);
/// Bounds the bytes retained by one request before its signed task identity can
/// be verified. Browser tool inputs are paths, selectors, text, and small JSON
/// options; screenshots and downloads flow in the response direction.
const BROWSER_MCP_REQUEST_BODY_LIMIT_BYTES: usize = 128 * 1024;
/// Absolute, not idle, deadline for producing the complete signed JSON
/// envelope. This prevents a slow chunked sender from keeping one pre-identity
/// Hyper task alive forever by dripping a byte before each idle timeout.
const BROWSER_MCP_REQUEST_BODY_TIMEOUT: Duration = Duration::from_secs(5);
const BROWSER_MCP_PRE_AUTH_MIN_INGRESS: usize = 64;
const BROWSER_MCP_PRE_AUTH_PER_CPU: usize = 8;
const BROWSER_MCP_PRE_AUTH_MAX_INGRESS: usize = 512;
const BROWSER_MCP_PRE_AUTH_MIN_CONNECTIONS: usize = 64;
const BROWSER_MCP_PRE_AUTH_CONNECTIONS_PER_CPU: usize = 8;
const BROWSER_MCP_PRE_AUTH_MAX_CONNECTIONS: usize = 512;
/// The MCP ingress budget is per trusted user-visible task family, never
/// process-global. Sibling runtimes in one conversation share this boundary.
/// Four active calls leave room for normal parallel observation while the
/// platform's deeper weighted-operation scheduler remains authoritative.
const BROWSER_MCP_TASK_ACTIVE_REQUESTS: usize = 4;
/// Waiting requests retain a parsed body and HTTP task, so bound them
/// separately from active calls. Total retained calls per task are therefore
/// `ACTIVE + QUEUED`.
const BROWSER_MCP_TASK_QUEUED_REQUESTS: usize = 12;
const BROWSER_MCP_TASK_QUEUE_WAIT: Duration = Duration::from_secs(5);
const MODEL_IDENTITY_INPUT_FIELDS: &[&str] = &[
    "identity",
    "identity_mode",
    "authenticated",
    "auth_identity",
    "profile",
    "account",
];
type HubSlot = Arc<RwLock<Weak<BrowserSessionHub>>>;

#[derive(Default)]
struct AcceptedConnectionRegistry {
    next_id: u64,
    shutdown_handles: HashMap<u64, std::net::TcpStream>,
}

#[derive(Clone)]
struct PreAuthConnectionIngress {
    slots: Arc<Semaphore>,
    stopping: Arc<AtomicBool>,
    accepted: Arc<StdMutex<AcceptedConnectionRegistry>>,
    #[cfg(test)]
    capacity: usize,
}

impl PreAuthConnectionIngress {
    fn new() -> Self {
        let logical_cpus = std::thread::available_parallelism()
            .map(|parallelism| parallelism.get())
            .unwrap_or(4);
        let capacity = logical_cpus
            .saturating_mul(BROWSER_MCP_PRE_AUTH_CONNECTIONS_PER_CPU)
            .clamp(
                BROWSER_MCP_PRE_AUTH_MIN_CONNECTIONS,
                BROWSER_MCP_PRE_AUTH_MAX_CONNECTIONS,
            );
        Self::with_capacity(capacity)
    }

    fn with_capacity(capacity: usize) -> Self {
        assert!(capacity > 0);
        Self {
            slots: Arc::new(Semaphore::new(capacity)),
            stopping: Arc::new(AtomicBool::new(false)),
            accepted: Arc::new(StdMutex::new(AcceptedConnectionRegistry::default())),
            #[cfg(test)]
            capacity,
        }
    }

    fn register(&self, shutdown_handle: std::net::TcpStream) -> Option<u64> {
        let mut accepted = self
            .accepted
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.stopping.load(Ordering::Acquire) {
            let _ = shutdown_handle.shutdown(Shutdown::Both);
            return None;
        }
        let mut connection_id = accepted.next_id.wrapping_add(1).max(1);
        while accepted.shutdown_handles.contains_key(&connection_id) {
            connection_id = connection_id.wrapping_add(1).max(1);
        }
        accepted.next_id = connection_id;
        accepted
            .shutdown_handles
            .insert(connection_id, shutdown_handle);
        Some(connection_id)
    }

    fn unregister(&self, connection_id: u64) {
        self.accepted
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .shutdown_handles
            .remove(&connection_id);
    }

    fn stop_and_close_all(&self) {
        // Store before taking the registry lock. An accept racing before the
        // store is drained below; one racing after it self-closes in register.
        self.stopping.store(true, Ordering::Release);
        let shutdown_handles = {
            let mut accepted = self
                .accepted
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            std::mem::take(&mut accepted.shutdown_handles)
        };
        for (_, stream) in shutdown_handles {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }
}

struct BoundedTcpListener {
    listener: TcpListener,
    ingress: PreAuthConnectionIngress,
}

impl BoundedTcpListener {
    fn new(listener: TcpListener, ingress: PreAuthConnectionIngress) -> Self {
        Self { listener, ingress }
    }
}

struct BoundedTcpStream {
    stream: TcpStream,
    ingress: PreAuthConnectionIngress,
    connection_id: Option<u64>,
    /// Socket-lifetime transport authority. Unlike request admission this is
    /// deliberately not released after auth: keep-alive/pipelined traffic must
    /// never leave an unaccounted idle FD or Hyper connection task behind.
    _permit: OwnedSemaphorePermit,
}

impl Drop for BoundedTcpStream {
    fn drop(&mut self) {
        if let Some(connection_id) = self.connection_id.take() {
            self.ingress.unregister(connection_id);
        }
    }
}

impl AsyncRead for BoundedTcpStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(context, buffer)
    }
}

impl AsyncWrite for BoundedTcpStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        Pin::new(&mut self.stream).poll_write(context, buffer)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Result<(), io::Error>> {
        Pin::new(&mut self.stream).poll_flush(context)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Result<(), io::Error>> {
        Pin::new(&mut self.stream).poll_shutdown(context)
    }

    fn is_write_vectored(&self) -> bool {
        self.stream.is_write_vectored()
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffers: &[io::IoSlice<'_>],
    ) -> Poll<Result<usize, io::Error>> {
        Pin::new(&mut self.stream).poll_write_vectored(context, buffers)
    }
}

impl Listener for BoundedTcpListener {
    type Io = BoundedTcpStream;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let permit = Arc::clone(&self.ingress.slots)
                .acquire_owned()
                .await
                .expect("browser MCP connection semaphore remains open");
            let (stream, address) = Listener::accept(&mut self.listener).await;
            let std_stream = match stream.into_std() {
                Ok(stream) => stream,
                Err(error) => {
                    warn!(%error, "Browser MCP could not register an accepted socket");
                    continue;
                }
            };
            let shutdown_handle = match std_stream.try_clone() {
                Ok(stream) => stream,
                Err(error) => {
                    warn!(%error, "Browser MCP could not clone an accepted socket shutdown handle");
                    continue;
                }
            };
            let stream = match TcpStream::from_std(std_stream) {
                Ok(stream) => stream,
                Err(error) => {
                    warn!(%error, "Browser MCP could not restore an accepted async socket");
                    continue;
                }
            };
            let connection_id = self.ingress.register(shutdown_handle);
            return (
                BoundedTcpStream {
                    stream,
                    ingress: self.ingress.clone(),
                    connection_id,
                    _permit: permit,
                },
                address,
            );
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.listener.local_addr()
    }
}

#[derive(Clone)]
struct BrowserMcpRequestShutdown {
    signal: Arc<tokio::sync::watch::Sender<bool>>,
}

impl Default for BrowserMcpRequestShutdown {
    fn default() -> Self {
        let (signal, _) = tokio::sync::watch::channel(false);
        Self {
            signal: Arc::new(signal),
        }
    }
}

impl BrowserMcpRequestShutdown {
    fn cancel(&self) {
        self.signal.send_replace(true);
    }

    async fn cancelled(&self) {
        let mut signal = self.signal.subscribe();
        if *signal.borrow() {
            return;
        }
        loop {
            if signal.changed().await.is_err() || *signal.borrow_and_update() {
                return;
            }
        }
    }
}

#[derive(Clone)]
struct PreAuthIngress {
    slots: Arc<Semaphore>,
    capacity: usize,
    accepting_requests: Arc<AtomicBool>,
    request_shutdown: BrowserMcpRequestShutdown,
}

impl PreAuthIngress {
    fn new(
        accepting_requests: Arc<AtomicBool>,
        request_shutdown: BrowserMcpRequestShutdown,
    ) -> Self {
        let logical_cpus = std::thread::available_parallelism()
            .map(|parallelism| parallelism.get())
            .unwrap_or(4);
        let capacity = logical_cpus
            .saturating_mul(BROWSER_MCP_PRE_AUTH_PER_CPU)
            .clamp(
                BROWSER_MCP_PRE_AUTH_MIN_INGRESS,
                BROWSER_MCP_PRE_AUTH_MAX_INGRESS,
            );
        Self {
            slots: Arc::new(Semaphore::new(capacity)),
            capacity,
            accepting_requests,
            request_shutdown,
        }
    }

    #[cfg(test)]
    fn with_capacity(capacity: usize) -> Self {
        assert!(capacity > 0);
        Self {
            slots: Arc::new(Semaphore::new(capacity)),
            capacity,
            accepting_requests: Arc::new(AtomicBool::new(true)),
            request_shutdown: BrowserMcpRequestShutdown::default(),
        }
    }
}

/// Cloneable only because Axum request extensions clone values. `release`
/// clears the shared permit through every clone, making the handoff to the
/// verified per-task quota immediate instead of retaining a process-wide slot
/// for the browser operation itself.
#[derive(Clone)]
struct PreAuthIngressGuard {
    permit: Arc<StdMutex<Option<OwnedSemaphorePermit>>>,
}

impl PreAuthIngressGuard {
    fn new(permit: OwnedSemaphorePermit) -> Self {
        Self {
            permit: Arc::new(StdMutex::new(Some(permit))),
        }
    }

    fn release(&self) {
        self.permit
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
    }
}

async fn enforce_pre_auth_ingress(
    State(ingress): State<PreAuthIngress>,
    mut request: Request,
    next: Next,
) -> Response {
    if !ingress.accepting_requests.load(Ordering::Acquire) {
        return browser_mcp_ingress_stopped();
    }
    let permit = match Arc::clone(&ingress.slots).try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => return pre_auth_ingress_saturated(ingress.capacity),
    };
    if !ingress.accepting_requests.load(Ordering::Acquire) {
        drop(permit);
        return browser_mcp_ingress_stopped();
    }
    request
        .extensions_mut()
        .insert(PreAuthIngressGuard::new(permit));
    let mut response = tokio::select! {
        biased;
        _ = ingress.request_shutdown.cancelled() => browser_mcp_ingress_stopped(),
        response = next.run(request) => response,
    };
    // This bridge never benefits from keep-alive: one stdio call maps to one
    // authenticated HTTP request. Closing every response promptly minimizes
    // socket-lifetime occupancy while the bounded Listener remains the final
    // defence against clients that never send request headers.
    response
        .headers_mut()
        .insert(header::CONNECTION, HeaderValue::from_static("close"));
    response
}

struct TimedJson<T>(T);

impl<S, T> FromRequest<S> for TimedJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = Response;

    async fn from_request(
        request: Request,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        extract_json_with_deadline(
            request,
            state,
            BROWSER_MCP_REQUEST_BODY_TIMEOUT,
        )
        .await
        .map(Self)
    }
}

async fn extract_json_with_deadline<S, T>(
    request: Request,
    state: &S,
    deadline: Duration,
) -> Result<T, Response>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    match tokio::time::timeout(deadline, Json::<T>::from_request(request, state)).await {
        Ok(Ok(Json(value))) => Ok(value),
        Ok(Err(rejection)) => Err(rejection.into_response()),
        Err(_) => Err(request_body_timeout()),
    }
}

#[derive(Clone, Default)]
struct TaskRequestAdmissions {
    /// Weak values make completed tasks disappear without an explicit task-end
    /// callback. Lazy and periodic pruning remove the small key shells too.
    entries: Arc<StdMutex<HashMap<String, Weak<TaskRequestAdmission>>>>,
}

struct TaskRequestAdmission {
    active: Arc<Semaphore>,
    outstanding: Arc<Semaphore>,
}

impl Default for TaskRequestAdmission {
    fn default() -> Self {
        Self {
            active: Arc::new(Semaphore::new(BROWSER_MCP_TASK_ACTIVE_REQUESTS)),
            outstanding: Arc::new(Semaphore::new(
                BROWSER_MCP_TASK_ACTIVE_REQUESTS + BROWSER_MCP_TASK_QUEUED_REQUESTS,
            )),
        }
    }
}

/// Owns both permits and a strong limiter reference. Keeping the limiter alive
/// is important: otherwise a task whose map entry is only `Weak` could create a
/// second limiter while an accepted request still holds semaphore permits.
struct TaskRequestPermit {
    entries: Arc<StdMutex<HashMap<String, Weak<TaskRequestAdmission>>>>,
    task_resource_family_key: String,
    limiter: Option<Arc<TaskRequestAdmission>>,
    outstanding: Option<OwnedSemaphorePermit>,
    active: Option<OwnedSemaphorePermit>,
}

impl Drop for TaskRequestPermit {
    fn drop(&mut self) {
        // Release capacity before inspecting strong references so a waiter
        // that is already bound to this limiter can advance normally.
        self.active.take();
        self.outstanding.take();
        let Some(limiter) = self.limiter.take() else {
            return;
        };
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let is_current = entries
            .get(&self.task_resource_family_key)
            .is_some_and(|entry| entry.ptr_eq(&Arc::downgrade(&limiter)));
        if is_current && Arc::strong_count(&limiter) == 1 {
            entries.remove(&self.task_resource_family_key);
        }
        // Drop the final strong reference while map lookup/replacement is
        // serialized, preventing a transient second limiter for this task.
        drop(limiter);
    }
}

impl TaskRequestPermit {
    async fn activate(
        mut self,
        queue_wait: Duration,
    ) -> Result<Self, TaskRequestAdmissionError> {
        let active_semaphore = Arc::clone(
            &self
                .limiter
                .as_ref()
                .expect("task request limiter remains owned while acquiring")
                .active,
        );
        let deadline = tokio::time::Instant::now() + queue_wait;
        let active = tokio::time::timeout_at(deadline, active_semaphore.acquire_owned())
            .await
            .map_err(|_| TaskRequestAdmissionError::QueueTimeout)?
            .map_err(|_| TaskRequestAdmissionError::OutstandingLimit)?;
        self.active = Some(active);
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TaskRequestAdmissionError {
    OutstandingLimit,
    QueueTimeout,
}

impl TaskRequestAdmissions {
    fn admission_for(&self, task_resource_family_key: &str) -> Arc<TaskRequestAdmission> {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(admission) = entries
            .get(task_resource_family_key)
            .and_then(Weak::upgrade)
        {
            return admission;
        }
        let admission = Arc::new(TaskRequestAdmission::default());
        entries.insert(
            task_resource_family_key.to_owned(),
            Arc::downgrade(&admission),
        );
        admission
    }

    fn reserve(
        &self,
        task_resource_family_key: &str,
    ) -> Result<TaskRequestPermit, TaskRequestAdmissionError> {
        let limiter = self.admission_for(task_resource_family_key);
        let outstanding = Arc::clone(&limiter.outstanding)
            .try_acquire_owned()
            .map_err(|_| TaskRequestAdmissionError::OutstandingLimit)?;
        Ok(TaskRequestPermit {
            entries: Arc::clone(&self.entries),
            task_resource_family_key: task_resource_family_key.to_owned(),
            limiter: Some(limiter),
            outstanding: Some(outstanding),
            active: None,
        })
    }

    async fn acquire(
        &self,
        task_resource_family_key: &str,
        queue_wait: Duration,
    ) -> Result<TaskRequestPermit, TaskRequestAdmissionError> {
        self.reserve(task_resource_family_key)?.activate(queue_wait).await
    }

    fn prune(&self) {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|_, admission| admission.strong_count() > 0);
    }

    #[cfg(test)]
    fn get(&self, task_resource_family_key: &str) -> Option<Arc<TaskRequestAdmission>> {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(task_resource_family_key)
            .and_then(Weak::upgrade)
    }

    #[cfg(test)]
    fn entry_count(&self) -> usize {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }
}

#[derive(Clone, Debug, Default)]
enum BrowserMcpShutdownStage {
    #[default]
    Pending,
    Complete(Result<(), Arc<str>>),
}

#[derive(Clone, Debug, Default)]
struct BrowserMcpShutdownStatus {
    serve: BrowserMcpShutdownStage,
    cleanup: BrowserMcpShutdownStage,
}

impl BrowserMcpShutdownStatus {
    fn completed_result(&self) -> Option<Result<(), String>> {
        let (
            BrowserMcpShutdownStage::Complete(serve),
            BrowserMcpShutdownStage::Complete(cleanup),
        ) = (&self.serve, &self.cleanup)
        else {
            return None;
        };
        Some(aggregate_browser_mcp_shutdown_results([
            serve.clone(),
            cleanup.clone(),
        ]))
    }

    #[cfg(test)]
    fn timeout_result(&self, wait: Duration) -> Result<(), String> {
        // The flight may have completed between `timeout` firing and this
        // snapshot. Prefer the cached terminal result over manufacturing an
        // empty/spurious timeout error in that race.
        if let Some(result) = self.completed_result() {
            return result;
        }
        let mut errors = Vec::new();
        match &self.serve {
            BrowserMcpShutdownStage::Pending => errors.push(format!(
                "Browser MCP HTTP ingress shutdown exceeded the {} ms shutdown wait",
                wait.as_millis()
            )),
            BrowserMcpShutdownStage::Complete(Err(error)) => {
                errors.push(error.to_string());
            }
            BrowserMcpShutdownStage::Complete(Ok(())) => {}
        }
        match &self.cleanup {
            BrowserMcpShutdownStage::Pending => errors.push(format!(
                "Browser MCP cleanup exceeded the {} ms shutdown wait; durable cleanup detached",
                wait.as_millis()
            )),
            BrowserMcpShutdownStage::Complete(Err(error)) => {
                errors.push(error.to_string());
            }
            BrowserMcpShutdownStage::Complete(Ok(())) => {}
        }
        Err(errors.join("; "))
    }
}

fn aggregate_browser_mcp_shutdown_results<const N: usize>(
    results: [Result<(), Arc<str>>; N],
) -> Result<(), String> {
    let errors = results
        .into_iter()
        .filter_map(Result::err)
        .map(|error| error.to_string())
        .collect::<Vec<_>>();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn update_browser_mcp_shutdown_stage(
    completion: &tokio::sync::watch::Sender<BrowserMcpShutdownStatus>,
    update: impl FnOnce(&mut BrowserMcpShutdownStatus),
) {
    completion.send_modify(update);
}

fn browser_mcp_task_result(
    result: Result<Result<(), String>, tokio::task::JoinError>,
    task_name: &str,
) -> Result<(), Arc<str>> {
    match result {
        Ok(result) => result.map_err(Arc::<str>::from),
        Err(error) => Err(Arc::from(format!(
            "Browser MCP {task_name} task failed while stopping: {error}"
        ))),
    }
}

struct BrowserMcpShutdownWorkerGuard {
    completion: tokio::sync::watch::Sender<BrowserMcpShutdownStatus>,
}

impl BrowserMcpShutdownWorkerGuard {
    fn new(
        completion: tokio::sync::watch::Sender<BrowserMcpShutdownStatus>,
    ) -> Self {
        Self { completion }
    }
}

impl Drop for BrowserMcpShutdownWorkerGuard {
    fn drop(&mut self) {
        update_browser_mcp_shutdown_stage(&self.completion, |status| {
            if matches!(status.serve, BrowserMcpShutdownStage::Pending) {
                status.serve = BrowserMcpShutdownStage::Complete(Err(
                    Arc::from(
                        "Browser MCP shutdown worker stopped before HTTP ingress shutdown completed",
                    ),
                ));
            }
            if matches!(status.cleanup, BrowserMcpShutdownStage::Pending) {
                status.cleanup = BrowserMcpShutdownStage::Complete(Err(
                    Arc::from(
                        "Browser MCP shutdown worker stopped before durable cleanup completed",
                    ),
                ));
            }
        });
    }
}

fn browser_mcp_supervisor_failed(
    status: &BrowserMcpShutdownStatus,
) -> Result<(), String> {
    let mut errors = Vec::new();
    match &status.serve {
        BrowserMcpShutdownStage::Pending => errors.push(
            "Browser MCP shutdown supervisor stopped before HTTP ingress shutdown completed"
                .to_owned(),
        ),
        BrowserMcpShutdownStage::Complete(Err(error)) => {
            errors.push(error.to_string());
        }
        BrowserMcpShutdownStage::Complete(Ok(())) => {}
    }
    match &status.cleanup {
        BrowserMcpShutdownStage::Pending => errors.push(
            "Browser MCP shutdown supervisor stopped before durable cleanup completed"
                .to_owned(),
        ),
        BrowserMcpShutdownStage::Complete(Err(error)) => {
            errors.push(error.to_string());
        }
        BrowserMcpShutdownStage::Complete(Ok(())) => {}
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn browser_mcp_shutdown_channel_closed(
    completion: &tokio::sync::watch::Receiver<BrowserMcpShutdownStatus>,
) -> Result<(), String> {
    let status = completion.borrow().clone();
    if let Some(result) = status.completed_result() {
        result
    } else {
        browser_mcp_supervisor_failed(&status)
    }
}

async fn wait_for_browser_mcp_shutdown(
    mut completion: tokio::sync::watch::Receiver<BrowserMcpShutdownStatus>,
) -> Result<(), String> {
    loop {
        if let Some(result) = completion.borrow().completed_result() {
            return result;
        }
        if completion.changed().await.is_err() {
            return browser_mcp_shutdown_channel_closed(&completion);
        }
    }
}

#[cfg(test)]
async fn wait_for_browser_mcp_ingress(
    mut completion: tokio::sync::watch::Receiver<BrowserMcpShutdownStatus>,
) -> Result<(), String> {
    loop {
        let serve = match &completion.borrow().serve {
            BrowserMcpShutdownStage::Pending => None,
            BrowserMcpShutdownStage::Complete(result) => Some(
                result
                    .clone()
                    .map_err(|error| error.to_string()),
            ),
        };
        if let Some(result) = serve {
            return result;
        }
        if completion.changed().await.is_err() {
            let status = completion.borrow().clone();
            return match status.serve {
                BrowserMcpShutdownStage::Complete(result) => {
                    result.map_err(|error| error.to_string())
                }
                BrowserMcpShutdownStage::Pending => Err(
                    "Browser MCP shutdown supervisor stopped before HTTP ingress shutdown completed"
                        .to_owned(),
                ),
            };
        }
    }
}

struct BrowserMcpLifecycleState {
    serve_task:
        Option<tokio::task::JoinHandle<Result<(), String>>>,
    serve_shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    cleanup_task: Option<tokio::task::JoinHandle<()>>,
    cleanup_shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    stop_started: bool,
}

struct BrowserMcpLifecycle {
    state: StdMutex<BrowserMcpLifecycleState>,
    completion: tokio::sync::watch::Sender<BrowserMcpShutdownStatus>,
    runtime: tokio::runtime::Handle,
    cleanup_state: BrowserMcpState,
    request_shutdown: BrowserMcpRequestShutdown,
    connection_ingress: PreAuthConnectionIngress,
}

impl BrowserMcpLifecycle {
    fn begin_stop(&self) {
        // Request cancellation covers parsed/active handlers, while explicit
        // socket shutdown covers connections that have not produced headers
        // and therefore never entered Axum middleware.
        self.request_shutdown.cancel();
        self.connection_ingress.stop_and_close_all();
        let flight = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if state.stop_started {
                None
            } else {
                state.stop_started = true;
                Some((
                    state.serve_task.take(),
                    state.serve_shutdown.take(),
                    state.cleanup_task.take(),
                    state.cleanup_shutdown.take(),
                ))
            }
        };

        let Some((
            serve_task,
            serve_shutdown,
            cleanup_task,
            cleanup_shutdown,
        )) = flight
        else {
            return;
        };
        // Signal graceful shutdown synchronously so the listener stops
        // accepting new connections as soon as the shared stop flight starts.
        // The request-level cancellation and socket registry make accepted
        // work quiesce before the final binding drain begins.
        let serve_shutdown_result = match serve_shutdown {
            Some(shutdown) => shutdown.send(()).map_err(|_| {
                Arc::from(
                    "Browser MCP HTTP ingress graceful-shutdown receiver was unavailable",
                )
            }),
            None => Err(Arc::from(
                "Browser MCP HTTP ingress graceful-shutdown signal was missing",
            )),
        };
        let completion = self.completion.clone();
        let cleanup_state = self.cleanup_state.clone();
        // This worker owns both JoinHandles. Dropping or timing out any caller
        // only drops that caller's watch receiver; it cannot cancel this stop
        // flight or consume the authoritative handles.
        drop(self.runtime.spawn(async move {
            let _worker_guard =
                BrowserMcpShutdownWorkerGuard::new(completion.clone());
            let serve_task_result = match serve_task {
                Some(mut task) => match tokio::time::timeout(
                    BROWSER_MCP_INGRESS_SHUTDOWN_GRACE,
                    &mut task,
                )
                .await
                {
                    Ok(result) => browser_mcp_task_result(result, "HTTP ingress"),
                    Err(_) => {
                        task.abort();
                        let _ = task.await;
                        Err(Arc::from(format!(
                            "Browser MCP HTTP ingress shutdown exceeded {} ms; the supervisor aborted the listener task and continued authoritative owner cleanup",
                            BROWSER_MCP_INGRESS_SHUTDOWN_GRACE.as_millis()
                        )))
                    }
                },
                None => Err(Arc::from(
                    "Browser MCP HTTP ingress task was missing at shutdown",
                )),
            };
            let serve_result = aggregate_browser_mcp_shutdown_results([
                serve_shutdown_result,
                serve_task_result,
            ])
            .map_err(Arc::<str>::from);
            update_browser_mcp_shutdown_stage(
                &completion,
                |status| {
                    status.serve =
                        BrowserMcpShutdownStage::Complete(serve_result);
                },
            );

            let cleanup_signal_error = match cleanup_shutdown {
                Some(shutdown) => shutdown.send(()).err().map(|_| {
                    "Browser MCP periodic cleanup worker was unavailable at shutdown"
                        .to_owned()
                }),
                None => Some(
                    "Browser MCP periodic cleanup shutdown signal was missing".to_owned(),
                ),
            };
            let cleanup_task_error = match cleanup_task {
                Some(mut task) => match tokio::time::timeout(
                    BROWSER_MCP_PERIODIC_CLEANUP_STOP_GRACE,
                    &mut task,
                )
                .await
                {
                    Ok(result) => result.err().map(|error| {
                        format!(
                            "Browser MCP periodic cleanup task failed while stopping: {error}"
                        )
                    }),
                    Err(_) => {
                        task.abort();
                        let _ = task.await;
                        Some(format!(
                            "Browser MCP periodic cleanup task exceeded {} ms and was cancelled before the authoritative drain",
                            BROWSER_MCP_PERIODIC_CLEANUP_STOP_GRACE.as_millis()
                        ))
                    }
                },
                None => Some(
                    "Browser MCP periodic cleanup task was missing at shutdown".to_owned(),
                ),
            };
            for error in cleanup_signal_error
                .into_iter()
                .chain(cleanup_task_error)
            {
                warn!(
                    %error,
                    "Browser MCP periodic cleanup worker failed; continuing with the authoritative final drain"
                );
            }

            // The periodic worker is only an optimization. The shutdown
            // supervisor owns the authoritative post-ingress drain and retries
            // until the binding inventory is empty. A cancelled periodic task,
            // a transient Hub failure, or a timed-out waiter therefore cannot
            // publish cleanup success or discard exact-owner cleanup authority.
            drain_all_bindings(&cleanup_state).await;
            update_browser_mcp_shutdown_stage(
                &completion,
                |status| {
                    status.cleanup =
                        BrowserMcpShutdownStage::Complete(Ok(()));
                },
            );
        }));
    }

    async fn wait_for_stop(&self) -> Result<(), String> {
        self.begin_stop();
        wait_for_browser_mcp_shutdown(self.completion.subscribe()).await
    }

    #[cfg(test)]
    async fn wait_for_stop_for(&self, wait: Duration) -> Result<(), String> {
        self.begin_stop();
        let completion = self.completion.subscribe();
        // The caller-visible timeout applies only to owner cleanup. Returning
        // before this stage would let AppServices tear down the Hub while an
        // already accepted request could still publish a fresh owner.
        let _serve_result =
            wait_for_browser_mcp_ingress(completion.clone()).await;
        match tokio::time::timeout(
            wait,
            wait_for_browser_mcp_shutdown(completion),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => self.completion.borrow().timeout_result(wait),
        }
    }
}

#[derive(Clone)]
struct OwnerBinding {
    owner_lease_id: OwnerLeaseId,
    policy: OwnerBindingPolicy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OwnerBindingPolicy {
    user_id: String,
    conversation_id: Option<String>,
    runtime_instance_id: String,
    agent_id: Option<String>,
    surface: BrowserSurface,
    allowed_operations: BTreeSet<BrowserOperationKind>,
}

#[derive(Default)]
struct OwnerBindingState {
    binding: Option<OwnerBinding>,
    /// Owner leases whose exact Hub cleanup failed remain here until retry
    /// succeeds. New transitions maintain the fail-closed invariant that a
    /// non-empty pending inventory has no current binding and cannot mint a
    /// replacement. The Vec remains able to drain legacy/interrupted states
    /// which may already contain more than one exact authority.
    pending_owner_cleanup: Vec<OwnerLeaseId>,
    /// A tombstone prevents a request which already captured an `Arc` to an
    /// entry from recreating a Hub lease after capability cleanup detached the
    /// entry from the process-wide map.
    revoked: bool,
}

#[derive(Default)]
struct OwnerBindingEntry {
    /// Serializes only the lifecycle transition for this capability. The
    /// guard may span Hub I/O, but it is never acquired while the global
    /// `bindings` map lock is held, so a slow cleanup for one capability
    /// cannot stall unrelated ACP runtimes.
    operation: Mutex<()>,
    state: Mutex<OwnerBindingState>,
    #[cfg(test)]
    ensure_captured: tokio::sync::Notify,
    #[cfg(test)]
    cleanup_captured: tokio::sync::Notify,
}

#[derive(Clone)]
struct BrowserMcpState {
    issuer: Arc<LoopbackCapabilityIssuer>,
    hub: HubSlot,
    bindings: Arc<Mutex<HashMap<String, Arc<OwnerBindingEntry>>>>,
    request_admissions: TaskRequestAdmissions,
    #[cfg(test)]
    owner_lease_issues: Arc<AtomicUsize>,
    #[cfg(test)]
    revoked_cleanup_active: Arc<AtomicUsize>,
    #[cfg(test)]
    revoked_cleanup_peak: Arc<AtomicUsize>,
    #[cfg(test)]
    request_started: Arc<tokio::sync::Notify>,
}

/// Process-local HTTP half of the ACP browser bridge.
pub(crate) struct BrowserMcpServer {
    http_addr: SocketAddr,
    issuer: Arc<LoopbackCapabilityIssuer>,
    hub: HubSlot,
    accepting_requests: Arc<AtomicBool>,
    lifecycle: BrowserMcpLifecycle,
    #[cfg(test)]
    pre_auth_ingress: PreAuthIngress,
    #[cfg(test)]
    pre_auth_connections: PreAuthConnectionIngress,
    #[cfg(test)]
    state: BrowserMcpState,
}

impl BrowserMcpServer {
    /// Bind a loopback-only ephemeral port and start the scoped capability
    /// lifecycle and tool forwarding routes.
    pub(crate) async fn start() -> Result<Self, String> {
        let issuer = Arc::new(LoopbackCapabilityIssuer::random()?);
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|error| format!("Failed to bind browser MCP loopback listener: {error}"))?;
        let http_addr = listener
            .local_addr()
            .map_err(|error| format!("Failed to read browser MCP loopback address: {error}"))?;
        let hub = Arc::new(RwLock::new(Weak::new()));
        let state = BrowserMcpState {
            issuer: Arc::clone(&issuer),
            hub: Arc::clone(&hub),
            bindings: Arc::new(Mutex::new(HashMap::new())),
            request_admissions: TaskRequestAdmissions::default(),
            #[cfg(test)]
            owner_lease_issues: Arc::new(AtomicUsize::new(0)),
            #[cfg(test)]
            revoked_cleanup_active: Arc::new(AtomicUsize::new(0)),
            #[cfg(test)]
            revoked_cleanup_peak: Arc::new(AtomicUsize::new(0)),
            #[cfg(test)]
            request_started: Arc::new(tokio::sync::Notify::new()),
        };
        let accepting_requests = Arc::new(AtomicBool::new(true));
        let request_shutdown = BrowserMcpRequestShutdown::default();
        let pre_auth_ingress = PreAuthIngress::new(
            Arc::clone(&accepting_requests),
            request_shutdown.clone(),
        );
        let pre_auth_connections = PreAuthConnectionIngress::new();
        let bounded_listener = BoundedTcpListener::new(
            listener,
            pre_auth_connections.clone(),
        );

        let app = axum::Router::new()
            .route("/tool", axum::routing::post(handle_tool_request))
            .route(
                LOOPBACK_CAPABILITY_RENEW_PATH,
                axum::routing::post(handle_capability_renew),
            )
            .route(
                LOOPBACK_CAPABILITY_REVOKE_PATH,
                axum::routing::post(handle_capability_revoke),
            )
            .with_state(state.clone())
            // The task identity lives in the signed JSON envelope, so the
            // only safe pre-parse defence is a strict per-request byte cap.
            // This layer rejects an oversized declared Content-Length before
            // buffering and also enforces the same cap on chunked bodies.
            .layer(DefaultBodyLimit::disable())
            .layer(RequestBodyLimitLayer::new(
                BROWSER_MCP_REQUEST_BODY_LIMIT_BYTES,
            ))
            // This is a bounded pre-auth transport failsafe, not a browser
            // resource/RSS cap. It covers only body read + claim verification,
            // rejects without queueing, and hands off immediately to the
            // verified per-task limiter before any browser work begins.
            .layer(middleware::from_fn_with_state(
                pre_auth_ingress.clone(),
                enforce_pre_auth_ingress,
            ));

        let (serve_shutdown, serve_shutdown_rx) =
            tokio::sync::oneshot::channel();
        let serve_task = tokio::spawn(async move {
            axum::serve(bounded_listener, app)
                .with_graceful_shutdown(async move {
                    let _ = serve_shutdown_rx.await;
                })
                .await
                .map_err(|error| {
                    warn!(%error, "Browser MCP loopback server exited with an error");
                    format!("Browser MCP loopback server exited with an error: {error}")
                })
        });
        let (cleanup_shutdown, mut cleanup_rx) = tokio::sync::oneshot::channel();
        let cleanup_state = state.clone();
        let cleanup_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(REVOKED_LEASE_SWEEP_INTERVAL);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        cleanup_state.request_admissions.prune();
                        retry_pending_owner_cleanups(&cleanup_state).await;
                        cleanup_revoked_bindings(&cleanup_state).await;
                    }
                    _ = &mut cleanup_rx => {
                        break;
                    }
                }
            }
        });
        let (completion, _) =
            tokio::sync::watch::channel(BrowserMcpShutdownStatus::default());
        let runtime = tokio::runtime::Handle::current();

        debug!("Browser MCP loopback server started");
        Ok(Self {
            http_addr,
            issuer,
            hub,
            accepting_requests,
            lifecycle: BrowserMcpLifecycle {
                state: StdMutex::new(BrowserMcpLifecycleState {
                    serve_task: Some(serve_task),
                    serve_shutdown: Some(serve_shutdown),
                    cleanup_task: Some(cleanup_task),
                    cleanup_shutdown: Some(cleanup_shutdown),
                    stop_started: false,
                }),
                completion,
                runtime,
                cleanup_state: state.clone(),
                request_shutdown,
                connection_ingress: pre_auth_connections.clone(),
            },
            #[cfg(test)]
            pre_auth_ingress,
            #[cfg(test)]
            pre_auth_connections,
            #[cfg(test)]
            state,
        })
    }

    /// Late-wire the one application-owned browser authority.
    pub(crate) fn set_hub(&self, hub: Weak<BrowserSessionHub>) {
        *self
            .hub
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = hub;
    }

    pub(crate) fn issuer_config(&self, binary_path: String) -> BrowserMcpConfig {
        BrowserMcpConfig::from_issuer(
            self.http_addr.port(),
            Arc::clone(&self.issuer),
            binary_path,
        )
    }

    fn begin_stop(&self) {
        // Close admission synchronously before the async graceful-shutdown
        // signal is polled. A fresh request can still complete a TCP handshake
        // during that scheduling window, but it is rejected before body read,
        // task admission, owner binding, or browser dispatch.
        self.accepting_requests.store(false, Ordering::Release);
        self.lifecycle.begin_stop();
    }

    /// Stop ingress and wait for the authoritative exact-owner cleanup barrier.
    ///
    /// This is the AppServices shutdown barrier: it does not return while an
    /// accepted HTTP request or a retained owner binding can still race Hub
    /// shutdown. Transient cleanup failures are retried by the durable worker
    /// until the binding inventory reaches its empty postcondition.
    pub(crate) async fn stop_and_wait(&self) -> Result<(), String> {
        self.begin_stop();
        let result = self.lifecycle.wait_for_stop().await;
        if let Err(error) = &result {
            warn!(%error, "Browser MCP async shutdown did not finish cleanly");
        }
        result
    }

    /// Consuming async shutdown convenience for owners that no longer need the
    /// server handle.
    #[cfg(test)]
    pub(crate) async fn shutdown(self) -> Result<(), String> {
        self.stop_and_wait().await
    }

    #[cfg(test)]
    async fn stop_and_wait_for(&self, wait: Duration) -> Result<(), String> {
        self.begin_stop();
        let result = self.lifecycle.wait_for_stop_for(wait).await;
        if let Err(error) = &result {
            warn!(%error, "Browser MCP async shutdown did not finish cleanly");
        }
        result
    }

    fn stop(&self) {
        // Synchronous teardown cannot wait. The detached supervisor owns both
        // task handles and continues the ordered ingress stop and final drain.
        self.begin_stop();
    }
}

impl Drop for BrowserMcpServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn validate_browser_claims(
    claims: &BrowserCapabilityClaims,
) -> Result<(), LoopbackCapabilityError> {
    claims.validate_renewable_shape()?;
    claims.scope.validate(&claims.session)?;
    if claims.session.kind != LoopbackSessionKind::Conversation
        || claims.scope.surface != BrowserCapabilitySurface::Acp
        || !claims
            .scope
            .allows(BrowserCapabilityOperation::Manage)
        || claims.allowed_tools.iter().any(|tool| {
            !BROWSER_MCP_TOOL_NAMES.contains(&tool.as_str())
                || browser_tool_operation(tool)
                    .is_none_or(|operation| !claims.scope.allows(operation))
        })
    {
        return Err(LoopbackCapabilityError::InvalidIdentity);
    }
    Ok(())
}

async fn handle_tool_request(
    State(state): State<BrowserMcpState>,
    headers: axum::http::HeaderMap,
    Extension(pre_auth_guard): Extension<PreAuthIngressGuard>,
    TimedJson(body): TimedJson<Value>,
) -> Response {
    #[cfg(test)]
    state.request_started.notify_one();
    let presented_token = bearer_token(&headers);
    let claims: BrowserCapabilityClaims = match body
        .get("session")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
    {
        Some(claims)
            if state
                .issuer
                .verify_access(BROWSER_CAPABILITY_DOMAIN, &claims, presented_token)
                .is_ok()
                && validate_browser_claims(&claims).is_ok() =>
        {
            claims
        }
        _ => {
            warn!("Browser MCP rejected an invalid, expired, or missing capability");
            return unauthorized();
        }
    };

    let tool = body.get("tool").and_then(Value::as_str).unwrap_or("");
    let Some(capability_operation) = browser_tool_operation(tool) else {
        return forbidden();
    };
    if !claims.allows(tool) || !claims.scope.allows(capability_operation) {
        warn!(tool, "Browser MCP tool is outside the signed capability scope");
        return forbidden();
    }
    // Only verified, immutable claims may select an accounting bucket. The
    // Owner lease and runtime are intentionally excluded when a signed
    // conversation exists: sibling runtimes are one user-visible task family.
    let task_resource_family_key = task_resource_family_key_from_claims(&claims);
    let reservation = match state
        .request_admissions
        .reserve(&task_resource_family_key)
    {
        Ok(reservation) => reservation,
        Err(TaskRequestAdmissionError::OutstandingLimit) => {
            return resource_exhausted("mcp_task_request_limit");
        }
        Err(TaskRequestAdmissionError::QueueTimeout) => unreachable!(
            "reserving a total task slot never waits for an active slot"
        ),
    };
    // Atomic resource-accounting handoff: after a verified task owns one of
    // its total slots, the process-wide pre-auth transport slot is no longer
    // retained. Long browser operations therefore do not consume a global
    // ingress budget or impose an aggregate browser-memory ceiling.
    pre_auth_guard.release();
    let _request_permit = match reservation.activate(BROWSER_MCP_TASK_QUEUE_WAIT).await {
        Ok(permit) => permit,
        Err(TaskRequestAdmissionError::QueueTimeout) => {
            return resource_exhausted("mcp_task_queue_timeout");
        }
        Err(TaskRequestAdmissionError::OutstandingLimit) => {
            return resource_exhausted("mcp_task_request_limit");
        }
    };
    let input = body
        .get("args")
        .cloned()
        .filter(Value::is_object)
        .unwrap_or_else(|| Value::Object(Map::new()));
    if let Err(error) = reject_model_identity_fields(&input) {
        warn!(
            tool,
            "Browser MCP rejected model-controlled browser identity policy"
        );
        return finish(platform_error_json(error));
    }
    let Some(hub) = upgrade_hub(&state) else {
        return finish(platform_error_json(BrowserPlatformError::new(
            BrowserErrorCode::BrowserUnavailable,
            "The managed browser service is unavailable.",
            true,
            "Retry after the application browser service is ready.",
        )));
    };

    let owner_lease_id = match ensure_owner_binding(&state, &hub, &claims).await {
        Ok(owner_lease_id) => owner_lease_id,
        Err(error) => return finish(platform_error_json(error)),
    };
    let caller = caller_from_claims(&claims, owner_lease_id);
    let client = match hub.bind(caller) {
        Ok(client) => client,
        Err(error) => return finish(platform_error_json(error)),
    };
    let facade = ManagedBrowserFacade::new(client, None);
    finish(managed_result_envelope(facade.execute(tool, &input).await))
}

async fn handle_capability_renew(
    State(state): State<BrowserMcpState>,
    Extension(pre_auth_guard): Extension<PreAuthIngressGuard>,
    TimedJson(request): TimedJson<LoopbackCapabilityRenewalRequest>,
) -> Response {
    match state
        .issuer
        .renew::<BrowserCapabilityScope>(BROWSER_CAPABILITY_DOMAIN, &request)
    {
        Ok(access) if validate_browser_claims(&access.claims).is_ok() => {
            pre_auth_guard.release();
            Json(access).into_response()
        }
        _ => unauthorized(),
    }
}

async fn handle_capability_revoke(
    State(state): State<BrowserMcpState>,
    Extension(pre_auth_guard): Extension<PreAuthIngressGuard>,
    TimedJson(request): TimedJson<LoopbackCapabilityRenewalRequest>,
) -> Response {
    match state
        .issuer
        .revoke(BROWSER_CAPABILITY_DOMAIN, &request)
    {
        Ok(()) => {
            pre_auth_guard.release();
            cleanup_binding(&state, &request.lease_id).await;
            StatusCode::NO_CONTENT.into_response()
        }
        Err(_) => unauthorized(),
    }
}

fn bearer_token(headers: &axum::http::HeaderMap) -> &str {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or("")
}

fn upgrade_hub(state: &BrowserMcpState) -> Option<Arc<BrowserSessionHub>> {
    state
        .hub
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .upgrade()
}

async fn ensure_owner_binding(
    state: &BrowserMcpState,
    hub: &BrowserSessionHub,
    claims: &BrowserCapabilityClaims,
) -> Result<OwnerLeaseId, BrowserPlatformError> {
    let trusted_policy = owner_binding_policy_from_claims(claims);
    let entry = {
        let mut bindings = state.bindings.lock().await;
        if !state
            .issuer
            .is_lease_active(BROWSER_CAPABILITY_DOMAIN, &claims.lease_id)
        {
            return Err(owner_binding_expired());
        }
        Arc::clone(
            bindings
                .entry(claims.lease_id.clone())
                .or_insert_with(|| Arc::new(OwnerBindingEntry::default())),
        )
    };
    #[cfg(test)]
    entry.ensure_captured.notify_one();
    // Serialize renewal/replacement only for this signed capability. Browser
    // calls from unrelated ACP runtimes must not wait behind a slow Hub
    // cleanup for another capability. This lock is deliberately acquired
    // after the global map lock has been released.
    let _operation_guard = entry.operation.lock().await;
    let (revoked, existing) = {
        let state_guard = entry.state.lock().await;
        (state_guard.revoked, state_guard.binding.clone())
    };
    if revoked
        || !state
            .issuer
            .is_lease_active(BROWSER_CAPABILITY_DOMAIN, &claims.lease_id)
    {
        return Err(owner_binding_expired());
    }
    if let Some(existing) = existing {
        if existing.policy != trusted_policy {
            return Err(owner_binding_policy_mismatch());
        }
        if hub.renew_owner_lease(&existing.owner_lease_id).is_ok() {
            if state
                .issuer
                .is_lease_active(BROWSER_CAPABILITY_DOMAIN, &claims.lease_id)
            {
                return Ok(existing.owner_lease_id);
            }
            {
                let mut state_guard = entry.state.lock().await;
                state_guard.revoked = true;
            }
            let _ = hub.revoke_owner_lease(&existing.owner_lease_id).await;
            return Err(owner_binding_expired());
        }
        // Close any lanes whose owner lease expired before replacing it. This
        // also prevents an old LaneKey from being mistaken for the new owner.
        if let Err(error) = hub.revoke_owner_lease(&existing.owner_lease_id).await {
            let mut state_guard = entry.state.lock().await;
            if !state_guard
                .pending_owner_cleanup
                .iter()
                .any(|lease_id| lease_id == &existing.owner_lease_id)
            {
                state_guard
                    .pending_owner_cleanup
                    .push(existing.owner_lease_id.clone());
            }
            // Fail closed: once the current lease has been revoked in the Hub,
            // only the exact pending-cleanup authority may remain reachable.
            // Publishing a replacement before that cleanup succeeds would let
            // one capability accumulate unbounded owner generations.
            state_guard.binding = None;
            drop(state_guard);
            warn!(
                code = ?error.code,
                "Browser MCP could not close an expired owner; blocking replacement until exact cleanup succeeds"
            );
            return Err(error);
        }
        // Cleanup completed, so this capability may publish exactly one new
        // current owner below.
        let mut state_guard = entry.state.lock().await;
        state_guard.binding = None;
        drop(state_guard);
    } else {
        let pending_owner_cleanup = {
            let state_guard = entry.state.lock().await;
            state_guard.pending_owner_cleanup.clone()
        };
        if !pending_owner_cleanup.is_empty() {
            // `ensure_owner_binding` already owns this capability's
            // lifecycle gate. Calling the public retry helper here would try
            // to acquire the same non-reentrant mutex a second time and
            // deadlock precisely when a superseded owner needs a retry.
            retry_pending_owner_cleanup_locked(&entry, hub).await?;
        }
    }

    if !state
        .issuer
        .is_lease_active(BROWSER_CAPABILITY_DOMAIN, &claims.lease_id)
    {
        return Err(owner_binding_expired());
    }

    // Acquire the state guard before issuing the synchronous Hub lease. There
    // must be no cancellation point between lease creation and publishing its
    // binding; otherwise cancellation could strand an owner lease with no
    // entry for the revoked-capability sweep to discover.
    let mut state_guard = entry.state.lock().await;
    if state_guard.revoked
        || !state
            .issuer
            .is_lease_active(BROWSER_CAPABILITY_DOMAIN, &claims.lease_id)
    {
        return Err(owner_binding_expired());
    }
    let lease = hub.issue_owner_lease(
        trusted_policy.user_id.clone(),
        trusted_policy.conversation_id.clone(),
        trusted_policy.runtime_instance_id.clone(),
    )?;
    #[cfg(test)]
    state.owner_lease_issues.fetch_add(1, Ordering::AcqRel);
    let owner_lease_id = lease.lease_id.clone();
    state_guard.binding = Some(OwnerBinding {
        owner_lease_id: owner_lease_id.clone(),
        policy: trusted_policy,
    });
    drop(state_guard);

    if !state
        .issuer
        .is_lease_active(BROWSER_CAPABILITY_DOMAIN, &claims.lease_id)
    {
        // The binding is already published before this await, so cancellation
        // cannot lose cleanup authority: the periodic revoked-lease sweep can
        // find and revoke it.
        {
            let mut state_guard = entry.state.lock().await;
            state_guard.revoked = true;
        }
        if hub.revoke_owner_lease(&lease.lease_id).await.is_ok() {
            clear_binding_if_matches(&entry, &lease.lease_id).await;
            remove_binding_entry_if_current(state, &claims.lease_id, &entry).await;
        }
        return Err(owner_binding_expired());
    }
    Ok(owner_lease_id)
}

async fn clear_binding_if_matches(entry: &OwnerBindingEntry, owner_lease_id: &OwnerLeaseId) {
    let mut state_guard = entry.state.lock().await;
    if state_guard
        .binding
        .as_ref()
        .is_some_and(|binding| &binding.owner_lease_id == owner_lease_id)
    {
        state_guard.binding.take();
    }
}

fn owner_binding_expired() -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::OwnerLeaseExpired,
        "The browser capability has expired.",
        false,
        "Request a fresh browser capability.",
    )
}

fn owner_binding_policy_mismatch() -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::InvalidCallerIdentity,
        "The browser capability does not match the established signed owner policy.",
        false,
        "Renew the original signed browser capability or request a fresh runtime.",
    )
}

fn owner_binding_policy_from_claims(
    claims: &BrowserCapabilityClaims,
) -> OwnerBindingPolicy {
    OwnerBindingPolicy {
        user_id: claims.user_id.to_string(),
        conversation_id: claims.session.conversation_id.clone(),
        runtime_instance_id: claims.scope.runtime_instance_id.clone(),
        agent_id: claims.scope.agent_id.clone(),
        surface: BrowserSurface::Acp,
        allowed_operations: claims
            .scope
            .allowed_operations
            .iter()
            .copied()
            .map(platform_operation)
            .collect(),
    }
}

/// Mirrors [`CallerIdentity::task_resource_family_key`] before an owner lease
/// exists. Every input is immutable and server-verified capability state.
fn task_resource_family_key_from_claims(claims: &BrowserCapabilityClaims) -> String {
    let user_id = claims.user_id.to_string();
    TaskResourceFamilyKey::from_trusted_parts(
        &user_id,
        claims.session.conversation_id.as_deref(),
        &claims.scope.runtime_instance_id,
        None,
        None,
        BrowserSurface::Acp,
    )
    .into_string()
}

fn caller_from_claims(
    claims: &BrowserCapabilityClaims,
    owner_lease_id: OwnerLeaseId,
) -> CallerIdentity {
    let policy = owner_binding_policy_from_claims(claims);
    CallerIdentity {
        user_id: policy.user_id,
        conversation_id: policy.conversation_id,
        runtime_instance_id: policy.runtime_instance_id,
        agent_id: policy.agent_id,
        companion_id: None,
        execution_id: None,
        step_id: None,
        attempt_id: None,
        remote_connection_id: None,
        surface: policy.surface,
        owner_lease_id,
        capability_expires_at_ms: claims.expires_at_unix_secs.saturating_mul(1_000),
        allowed_operations: policy.allowed_operations,
    }
}

fn platform_operation(operation: BrowserCapabilityOperation) -> BrowserOperationKind {
    match operation {
        BrowserCapabilityOperation::Manage => BrowserOperationKind::Manage,
        BrowserCapabilityOperation::Navigate => BrowserOperationKind::Navigate,
        BrowserCapabilityOperation::Observe => BrowserOperationKind::Observe,
        BrowserCapabilityOperation::Act => BrowserOperationKind::Act,
        BrowserCapabilityOperation::Screenshot => BrowserOperationKind::Screenshot,
        BrowserCapabilityOperation::Tabs => BrowserOperationKind::Tabs,
        BrowserCapabilityOperation::Download => BrowserOperationKind::Download,
        BrowserCapabilityOperation::Debug => BrowserOperationKind::Debug,
        BrowserCapabilityOperation::Crawl => BrowserOperationKind::Crawl,
    }
}

fn reject_model_identity_fields(input: &Value) -> Result<(), BrowserPlatformError> {
    let Some(object) = input.as_object() else {
        return Ok(());
    };
    let Some(field) = MODEL_IDENTITY_INPUT_FIELDS
        .iter()
        .chain(TRUSTED_OWNER_INPUT_FIELDS.iter())
        .find(|field| object.contains_key(**field))
    else {
        return Ok(());
    };
    Err(BrowserPlatformError::new(
        BrowserErrorCode::InvalidCallerIdentity,
        format!("Browser field `{field}` is selected by trusted host policy."),
        false,
        "Remove trusted owner and identity-selection fields from Browser tool arguments.",
    ))
}

fn platform_error_json(error: BrowserPlatformError) -> Value {
    json!({ "error": error })
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "error": "unauthorized" })),
    )
        .into_response()
}

fn forbidden() -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(json!({ "error": "forbidden" })),
    )
        .into_response()
}

fn resource_exhausted(reason_code: &'static str) -> Response {
    let mut response = (
        StatusCode::TOO_MANY_REQUESTS,
        Json(json!({
            "error": {
                "code": "resource_exhausted",
                "message": "This browser task has too many requests in flight.",
                "retryable": true,
                "next_action": "Retry after an earlier browser request completes.",
                "metadata": {
                    "reason_code": reason_code,
                    "active_limit": BROWSER_MCP_TASK_ACTIVE_REQUESTS,
                    "queued_limit": BROWSER_MCP_TASK_QUEUED_REQUESTS,
                }
            }
        })),
    )
        .into_response();
    response
        .headers_mut()
        .insert(header::CONNECTION, HeaderValue::from_static("close"));
    response
        .headers_mut()
        .insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
    response
}

fn pre_auth_ingress_saturated(capacity: usize) -> Response {
    let mut response = (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({
            "error": {
                "code": "resource_exhausted",
                "message": "The local browser request ingress is temporarily saturated.",
                "retryable": true,
                "next_action": "Retry after an earlier request body has been verified.",
                "metadata": {
                    "reason_code": "mcp_pre_auth_ingress_limit",
                    "transport_capacity": capacity,
                }
            }
        })),
    )
        .into_response();
    response
        .headers_mut()
        .insert(header::CONNECTION, HeaderValue::from_static("close"));
    response
        .headers_mut()
        .insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
    response
}

fn browser_mcp_ingress_stopped() -> Response {
    let mut response = (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({
            "error": {
                "code": "browser_shutting_down",
                "message": "The browser request bridge is shutting down.",
                "retryable": true,
                "next_action": "Retry after the application is ready."
            }
        })),
    )
        .into_response();
    response
        .headers_mut()
        .insert(header::CONNECTION, HeaderValue::from_static("close"));
    response
}

fn request_body_timeout() -> Response {
    let mut response = (
        StatusCode::REQUEST_TIMEOUT,
        Json(json!({
            "error": {
                "code": "request_timeout",
                "message": "The browser request body was not received before the deadline.",
                "retryable": true,
                "next_action": "Retry the browser request on a healthy local connection."
            }
        })),
    )
        .into_response();
    response
        .headers_mut()
        .insert(header::CONNECTION, HeaderValue::from_static("close"));
    response
}

fn finish(body: Value) -> Response {
    let mut response = Json(body).into_response();
    response
        .headers_mut()
        .insert(header::CONNECTION, HeaderValue::from_static("close"));
    response
}

async fn cleanup_binding(state: &BrowserMcpState, capability_lease_id: &str) {
    let Some(entry) = ({
        let bindings = state.bindings.lock().await;
        bindings.get(capability_lease_id).cloned()
    }) else {
        return;
    };

    #[cfg(test)]
    entry.cleanup_captured.notify_one();
    // Do not hold the global bindings map lock while waiting for this
    // capability's lifecycle transition or for Hub cleanup.
    let _operation_guard = entry.operation.lock().await;
    {
        let mut state_guard = entry.state.lock().await;
        state_guard.revoked = true;
    }

    let (binding, pending_owner_cleanup) = {
        let state_guard = entry.state.lock().await;
        (
            state_guard.binding.clone(),
            state_guard.pending_owner_cleanup.clone(),
        )
    };
    if binding.is_none() && pending_owner_cleanup.is_empty() {
        remove_binding_entry_if_current(state, capability_lease_id, &entry).await;
        return;
    }
    let Some(hub) = upgrade_hub(state) else {
        warn!("Browser MCP cannot clean an owner while the Hub is unavailable");
        return;
    };
    let mut owner_lease_ids = Vec::with_capacity(
        usize::from(binding.is_some()) + pending_owner_cleanup.len(),
    );
    if let Some(binding) = binding {
        owner_lease_ids.push(binding.owner_lease_id);
    }
    for lease_id in pending_owner_cleanup {
        if !owner_lease_ids.iter().any(|current| current == &lease_id) {
            owner_lease_ids.push(lease_id);
        }
    }

    let mut remaining = Vec::new();
    let mut current_closed = false;
    let current_owner = owner_lease_ids.first().cloned();
    let mut first_error = None;
    for owner_lease_id in owner_lease_ids {
        match hub.revoke_owner_lease(&owner_lease_id).await {
            Ok(_) => {
                if current_owner.as_ref() == Some(&owner_lease_id) {
                    current_closed = true;
                }
            }
            Err(error) => {
                if first_error.is_none() {
                    first_error = Some(error);
                }
                remaining.push(owner_lease_id);
            }
        }
    }

    {
        let mut state_guard = entry.state.lock().await;
        if current_closed {
            state_guard.binding = None;
        }
        state_guard.pending_owner_cleanup = remaining.clone();
    }
    if let Some(error) = first_error {
        warn!(
            code = ?error.code,
            "Browser MCP failed to clean an owner after capability revocation"
        );
        // Keep the tombstoned entry and every failed exact owner in the map so
        // the periodic sweep can retry without losing cleanup authority.
        return;
    }
    if entry.state.lock().await.binding.is_none()
        && entry.state.lock().await.pending_owner_cleanup.is_empty()
    {
        remove_binding_entry_if_current(state, capability_lease_id, &entry).await;
    }
}

async fn remove_binding_entry_if_current(
    state: &BrowserMcpState,
    capability_lease_id: &str,
    entry: &Arc<OwnerBindingEntry>,
) {
    let mut bindings = state.bindings.lock().await;
    if bindings
        .get(capability_lease_id)
        .is_some_and(|current| Arc::ptr_eq(current, entry))
    {
        bindings.remove(capability_lease_id);
    }
}

async fn cleanup_revoked_bindings(state: &BrowserMcpState) {
    let revoked: Vec<String> = state
        .bindings
        .lock()
        .await
        .keys()
        .filter(|lease_id| {
            !state
                .issuer
                .is_lease_active(BROWSER_CAPABILITY_DOMAIN, lease_id)
        })
        .cloned()
        .collect();
    let mut revoked = revoked.into_iter();
    let mut cleanups = tokio::task::JoinSet::new();
    loop {
        while cleanups.len() < REVOKED_BINDING_CLEANUP_CONCURRENCY {
            let Some(lease_id) = revoked.next() else {
                break;
            };
            let state = state.clone();
            cleanups.spawn(async move {
                #[cfg(test)]
                let _task_guard = RevokedCleanupTaskGuard::enter(&state);
                cleanup_binding(&state, &lease_id).await;
            });
        }
        let Some(result) = cleanups.join_next().await else {
            break;
        };
        if let Err(error) = result {
            warn!(%error, "Browser MCP revoked-binding cleanup task failed");
        }
    }
}

#[cfg(test)]
struct RevokedCleanupTaskGuard {
    active: Arc<AtomicUsize>,
}

#[cfg(test)]
impl RevokedCleanupTaskGuard {
    fn enter(state: &BrowserMcpState) -> Self {
        let active = state
            .revoked_cleanup_active
            .fetch_add(1, Ordering::AcqRel)
            .saturating_add(1);
        state
            .revoked_cleanup_peak
            .fetch_max(active, Ordering::AcqRel);
        Self {
            active: Arc::clone(&state.revoked_cleanup_active),
        }
    }
}

#[cfg(test)]
impl Drop for RevokedCleanupTaskGuard {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::AcqRel);
    }
}

async fn retry_pending_owner_cleanup_for_entry(
    entry: &OwnerBindingEntry,
    hub: &BrowserSessionHub,
) -> Result<(), BrowserPlatformError> {
    let _operation_guard = entry.operation.lock().await;
    retry_pending_owner_cleanup_locked(entry, hub).await
}

/// Retry pending exact-owner cleanup while the per-capability lifecycle gate
/// is already held by the caller.
///
/// Keeping this as a separate, non-locking helper is important for
/// `ensure_owner_binding`: a replacement request may need to drain a
/// superseded owner before publishing a new binding, but the outer ensure
/// transition already owns `entry.operation`.
async fn retry_pending_owner_cleanup_locked(
    entry: &OwnerBindingEntry,
    hub: &BrowserSessionHub,
) -> Result<(), BrowserPlatformError> {
    let pending = entry.state.lock().await.pending_owner_cleanup.clone();
    if pending.is_empty() {
        return Ok(());
    }
    let mut remaining = Vec::new();
    let mut first_error = None;
    for owner_lease_id in pending {
        if let Err(error) = hub.revoke_owner_lease(&owner_lease_id).await {
            if first_error.is_none() {
                first_error = Some(error);
            }
            remaining.push(owner_lease_id);
        }
    }
    entry.state.lock().await.pending_owner_cleanup = remaining;
    first_error.map_or(Ok(()), Err)
}

async fn retry_pending_owner_cleanups(state: &BrowserMcpState) {
    let entries: Vec<Arc<OwnerBindingEntry>> = {
        let bindings = state.bindings.lock().await;
        bindings.values().cloned().collect()
    };
    let Some(hub) = upgrade_hub(state) else {
        return;
    };
    for entry in entries {
        // Do not acquire every active capability's lifecycle gate on every
        // 500 ms sweep. Entries without exact pending debt need no retry, and
        // waiting behind an ordinary in-flight request here would head-of-line
        // block revoked cleanup for the whole server.
        if entry.state.lock().await.pending_owner_cleanup.is_empty() {
            continue;
        }
        if let Err(error) = retry_pending_owner_cleanup_for_entry(&entry, &hub).await {
            warn!(
                code = ?error.code,
                "Browser MCP pending owner cleanup retry failed"
            );
        }
    }
}

async fn drain_all_bindings(state: &BrowserMcpState) {
    let mut retry_wait = CLEANUP_RETRY_WAIT;
    loop {
        let capability_lease_ids: Vec<String> = {
            let bindings = state.bindings.lock().await;
            bindings.keys().cloned().collect()
        };
        if capability_lease_ids.is_empty() {
            return;
        }
        let attempted = capability_lease_ids.len();

        // The server is going away. Revoke the process-local capability lease
        // directly (the server is the issuer authority), then clean the exact
        // Hub owner. HTTP is already quiesced before this authoritative loop
        // starts, so no request can publish another entry after an empty
        // snapshot.
        for capability_lease_id in capability_lease_ids {
            nomifun_common::LoopbackCapabilityLease::new(
                Arc::clone(&state.issuer),
                BROWSER_CAPABILITY_DOMAIN,
                capability_lease_id.clone(),
            )
            .revoke();
            cleanup_binding(state, &capability_lease_id).await;
        }
        let remaining = state.bindings.lock().await.len();
        if remaining == 0 {
            return;
        }
        // Permanent Hub/driver failure retains exact cleanup authority, but it
        // must not turn one detached shutdown worker into a 20 Hz hot loop.
        // Any progress resets latency; an unchanged inventory backs off to a
        // small fixed ceiling while continuing forever under worker ownership.
        tokio::time::sleep(retry_wait).await;
        retry_wait = next_cleanup_retry_wait(retry_wait, remaining < attempted);
    }
}

fn next_cleanup_retry_wait(current: Duration, made_progress: bool) -> Duration {
    if made_progress {
        CLEANUP_RETRY_WAIT
    } else {
        current
            .checked_mul(2)
            .unwrap_or(CLEANUP_RETRY_MAX_WAIT)
            .min(CLEANUP_RETRY_MAX_WAIT)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use nomifun_common::LoopbackCapabilityLease;
    use nomifun_browser_platform::{
        BrowserHostDriver, BrowserHostFactory, BrowserHostId, BrowserLaneDriver,
        BrowserProfileFootprint,
        BrowserOperation, BrowserOperationResult, DriverOperationContext,
        HostLaunchRequest, HostLifecycleState, HubConfig, LaneLaunchRequest,
    };

    use super::*;

    const USER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000001";
    const CONVERSATION_ID: &str = "0190f5fe-7c00-7a00-8000-000000000002";

    struct FakeLane;

    #[async_trait]
    impl BrowserLaneDriver for FakeLane {
        async fn execute(
            &self,
            operation: BrowserOperation,
            _context: DriverOperationContext,
        ) -> Result<BrowserOperationResult, BrowserPlatformError> {
            Ok(BrowserOperationResult {
                output: json!({ "action": operation.action, "input": operation.input }),
                ..Default::default()
            })
        }

        async fn close(&self) -> Result<(), BrowserPlatformError> {
            Ok(())
        }
    }

    struct FakeHost {
        host_id: BrowserHostId,
    }

    #[async_trait]
    impl BrowserHostDriver for FakeHost {
        fn host_id(&self) -> BrowserHostId {
            self.host_id.clone()
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
            _request: LaneLaunchRequest,
        ) -> Result<Arc<dyn BrowserLaneDriver>, BrowserPlatformError> {
            Ok(Arc::new(FakeLane))
        }

        async fn shutdown(&self) -> Result<(), BrowserPlatformError> {
            Ok(())
        }
    }

    struct FakeFactory {
        launches: AtomicUsize,
    }

    #[async_trait]
    impl BrowserHostFactory for FakeFactory {
        async fn launch(
            &self,
            request: HostLaunchRequest,
        ) -> Result<Arc<dyn BrowserHostDriver>, BrowserPlatformError> {
            self.launches.fetch_add(1, Ordering::AcqRel);
            Ok(Arc::new(FakeHost {
                host_id: request.host_id,
            }))
        }
    }

    struct CleanupFailingLane {
        cleanup_blocked: Arc<AtomicBool>,
    }

    #[async_trait]
    impl BrowserLaneDriver for CleanupFailingLane {
        async fn execute(
            &self,
            operation: BrowserOperation,
            _context: DriverOperationContext,
        ) -> Result<BrowserOperationResult, BrowserPlatformError> {
            Ok(BrowserOperationResult {
                output: json!({ "action": operation.action, "input": operation.input }),
                ..Default::default()
            })
        }

        async fn close(&self) -> Result<(), BrowserPlatformError> {
            if self.cleanup_blocked.load(Ordering::Acquire) {
                return Err(BrowserPlatformError::new(
                    BrowserErrorCode::BrowserUnavailable,
                    "Synthetic owner cleanup failure.",
                    true,
                    "Release the synthetic cleanup fault and retry.",
                ));
            }
            Ok(())
        }
    }

    struct CleanupFailingHost {
        host_id: BrowserHostId,
        cleanup_blocked: Arc<AtomicBool>,
    }

    #[async_trait]
    impl BrowserHostDriver for CleanupFailingHost {
        fn host_id(&self) -> BrowserHostId {
            self.host_id.clone()
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
            _request: LaneLaunchRequest,
        ) -> Result<Arc<dyn BrowserLaneDriver>, BrowserPlatformError> {
            Ok(Arc::new(CleanupFailingLane {
                cleanup_blocked: Arc::clone(&self.cleanup_blocked),
            }))
        }

        async fn shutdown(&self) -> Result<(), BrowserPlatformError> {
            if self.cleanup_blocked.load(Ordering::Acquire) {
                return Err(BrowserPlatformError::new(
                    BrowserErrorCode::BrowserUnavailable,
                    "Synthetic owner host retirement failure.",
                    true,
                    "Release the synthetic cleanup fault and retry.",
                ));
            }
            Ok(())
        }
    }

    struct CleanupFailingFactory {
        cleanup_blocked: Arc<AtomicBool>,
    }

    #[async_trait]
    impl BrowserHostFactory for CleanupFailingFactory {
        async fn launch(
            &self,
            request: HostLaunchRequest,
        ) -> Result<Arc<dyn BrowserHostDriver>, BrowserPlatformError> {
            Ok(Arc::new(CleanupFailingHost {
                host_id: request.host_id,
                cleanup_blocked: Arc::clone(&self.cleanup_blocked),
            }))
        }
    }

    async fn setup() -> (
        BrowserMcpServer,
        Arc<BrowserSessionHub>,
        nomifun_api_types::BrowserMcpChildConfig,
    ) {
        let mut config = HubConfig::default();
        config.owner_lease_ttl_ms = 60_000;
        let hub = Arc::new(BrowserSessionHub::new(
            Arc::new(FakeFactory {
                launches: AtomicUsize::new(0),
            }),
            config,
        ));
        let server = BrowserMcpServer::start().await.unwrap();
        server.set_hub(Arc::downgrade(&hub));
        let child = server
            .issuer_config("nomicore".into())
            .issue_for_conversation(USER_ID, CONVERSATION_ID, Some("agent-test"))
            .unwrap();
        (server, hub, child)
    }

    async fn call_tool(
        child: &nomifun_api_types::BrowserMcpChildConfig,
        tool: &str,
    ) -> reqwest::Response {
        call_tool_with_args(
            child,
            tool,
            json!({ "url": "https://example.test" }),
        )
        .await
    }

    async fn call_tool_with_args(
        child: &nomifun_api_types::BrowserMcpChildConfig,
        tool: &str,
        args: Value,
    ) -> reqwest::Response {
        try_call_tool_with_args(child, tool, args).await.unwrap()
    }

    async fn try_call_tool_with_args(
        child: &nomifun_api_types::BrowserMcpChildConfig,
        tool: &str,
        args: Value,
    ) -> Result<reqwest::Response, reqwest::Error> {
        reqwest::Client::new()
            .post(format!(
                "http://127.0.0.1:{}/tool",
                child.bootstrap.port
            ))
            .bearer_auth(&child.bootstrap.access.token)
            .json(&json!({
                "session": child.bootstrap.access.claims,
                "tool": tool,
                "args": args,
            }))
            .send()
            .await
    }

    async fn wait_for_admission_saturation(
        admission: &TaskRequestAdmission,
    ) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while admission.outstanding.available_permits() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("task request admission did not saturate");
    }

    #[tokio::test]
    async fn one_signed_task_keeps_a_hard_boundary_under_one_thousand_attempts() {
        let (server, _hub, child) = setup().await;
        let task_resource_key =
            task_resource_family_key_from_claims(&child.bootstrap.access.claims);
        let admissions = server.state.request_admissions.clone();
        let admission = admissions.admission_for(&task_resource_key);

        let mut active = Vec::new();
        for _ in 0..BROWSER_MCP_TASK_ACTIVE_REQUESTS {
            active.push(
                admissions
                    .acquire(&task_resource_key, Duration::from_secs(1))
                    .await
                    .expect("active request must be admitted"),
            );
        }
        assert_eq!(admission.active.available_permits(), 0);

        let mut queued = Vec::new();
        for _ in 0..BROWSER_MCP_TASK_QUEUED_REQUESTS {
            let admissions = admissions.clone();
            let task_resource_key = task_resource_key.clone();
            queued.push(tokio::spawn(async move {
                admissions
                    .acquire(&task_resource_key, Duration::from_secs(30))
                    .await
            }));
        }
        wait_for_admission_saturation(&admission).await;

        let mut flood = tokio::task::JoinSet::new();
        for _ in 0..1_000 {
            let admissions = admissions.clone();
            let task_resource_key = task_resource_key.clone();
            flood.spawn(async move {
                admissions
                    .acquire(&task_resource_key, Duration::from_secs(1))
                    .await
            });
        }
        while let Some(result) = flood.join_next().await {
            match result.expect("flood task must not panic") {
                Err(TaskRequestAdmissionError::OutstandingLimit) => {}
                Err(other) => panic!("unexpected admission error: {other:?}"),
                Ok(_) => panic!("a saturated task exceeded its hard request boundary"),
            }
        }
        assert_eq!(admissions.entry_count(), 1);
        assert_eq!(admission.outstanding.available_permits(), 0);

        for queued_request in queued {
            queued_request.abort();
            let _ = queued_request.await;
        }
        assert_eq!(
            admission.outstanding.available_permits(),
            BROWSER_MCP_TASK_QUEUED_REQUESTS,
            "cancelled queue waiters must release every outstanding slot"
        );
        drop(active);
        assert_eq!(
            admission.outstanding.available_permits(),
            BROWSER_MCP_TASK_ACTIVE_REQUESTS + BROWSER_MCP_TASK_QUEUED_REQUESTS
        );

        drop(admission);
        admissions.prune();
        assert_eq!(
            admissions.entry_count(),
            0,
            "weak admission entries must not retain completed task keys"
        );
    }

    #[tokio::test]
    async fn sibling_runtimes_share_conversation_ingress_but_other_conversations_do_not() {
        let (server, _hub, first) = setup().await;
        let sibling = server
            .issuer_config("nomicore".into())
            .issue_for_conversation(USER_ID, CONVERSATION_ID, Some("agent-sibling"))
            .unwrap();
        let other = server
            .issuer_config("nomicore".into())
            .issue_for_conversation(
                USER_ID,
                "0190f5fe-7c00-7a00-8000-000000000003",
                Some("agent-other"),
            )
            .unwrap();
        let first_family =
            task_resource_family_key_from_claims(&first.bootstrap.access.claims);
        let sibling_family =
            task_resource_family_key_from_claims(&sibling.bootstrap.access.claims);
        let other_family =
            task_resource_family_key_from_claims(&other.bootstrap.access.claims);
        assert_eq!(first_family, sibling_family);
        assert_ne!(first_family, other_family);

        let admissions = server.state.request_admissions.clone();
        let mut active = Vec::new();
        for _ in 0..BROWSER_MCP_TASK_ACTIVE_REQUESTS {
            active.push(
                admissions
                    .acquire(&first_family, Duration::from_secs(1))
                    .await
                    .unwrap(),
            );
        }
        assert!(matches!(
            admissions
                .acquire(&sibling_family, Duration::from_millis(1))
                .await,
            Err(TaskRequestAdmissionError::QueueTimeout)
        ));
        let independent = admissions
            .acquire(&other_family, Duration::from_millis(1))
            .await
            .expect("another conversation must retain independent capacity");
        drop(independent);
        drop(active);
    }

    #[tokio::test]
    async fn queue_timeout_and_cancellation_storm_restore_all_task_slots() {
        let admissions = TaskRequestAdmissions::default();
        let task_resource_key = "verified-user:verified-runtime";
        let admission = admissions.admission_for(task_resource_key);
        let mut active = Vec::new();
        for _ in 0..BROWSER_MCP_TASK_ACTIVE_REQUESTS {
            active.push(
                admissions
                    .acquire(task_resource_key, Duration::from_secs(1))
                    .await
                    .expect("active request must be admitted"),
            );
        }

        for _ in 0..64 {
            let mut queued = Vec::new();
            for _ in 0..BROWSER_MCP_TASK_QUEUED_REQUESTS {
                let admissions = admissions.clone();
                queued.push(tokio::spawn(async move {
                    admissions
                        .acquire(task_resource_key, Duration::from_secs(30))
                        .await
                }));
            }
            wait_for_admission_saturation(&admission).await;
            for queued_request in queued {
                queued_request.abort();
                let _ = queued_request.await;
            }
            assert_eq!(
                admission.outstanding.available_permits(),
                BROWSER_MCP_TASK_QUEUED_REQUESTS
            );
        }

        let timeout = admissions
            .acquire(task_resource_key, Duration::from_millis(1))
            .await;
        assert!(matches!(
            timeout,
            Err(TaskRequestAdmissionError::QueueTimeout)
        ));
        assert_eq!(
            admission.outstanding.available_permits(),
            BROWSER_MCP_TASK_QUEUED_REQUESTS,
            "a timed-out queue waiter must release its total slot"
        );
        drop(active);
        drop(admission);
        admissions.prune();
        assert_eq!(admissions.entry_count(), 0);
    }

    #[tokio::test]
    async fn saturated_task_does_not_reduce_another_tasks_capacity() {
        let admissions = TaskRequestAdmissions::default();
        let first_key = "verified-user:first-runtime";
        let second_key = "verified-user:second-runtime";
        let first = admissions.admission_for(first_key);
        let mut first_active = Vec::new();
        for _ in 0..BROWSER_MCP_TASK_ACTIVE_REQUESTS {
            first_active.push(
                admissions
                    .acquire(first_key, Duration::from_secs(1))
                    .await
                    .unwrap(),
            );
        }
        let mut first_queued = Vec::new();
        for _ in 0..BROWSER_MCP_TASK_QUEUED_REQUESTS {
            let admissions = admissions.clone();
            first_queued.push(tokio::spawn(async move {
                admissions
                    .acquire(first_key, Duration::from_secs(30))
                    .await
            }));
        }
        wait_for_admission_saturation(&first).await;

        let second = admissions
            .acquire(second_key, Duration::from_millis(50))
            .await
            .expect("there must be no process-global request semaphore");
        let second_admission = admissions.get(second_key).unwrap();
        assert_eq!(
            second_admission.active.available_permits(),
            BROWSER_MCP_TASK_ACTIVE_REQUESTS - 1
        );
        drop(second);

        for queued_request in first_queued {
            queued_request.abort();
            let _ = queued_request.await;
        }
        drop(first_active);
        drop(first);
        drop(second_admission);
        admissions.prune();
        assert_eq!(admissions.entry_count(), 0);
    }

    #[tokio::test]
    async fn concurrent_http_calls_from_one_token_are_bounded_before_owner_waits() {
        const REQUESTS: usize = 64;
        const RETAINED: usize =
            BROWSER_MCP_TASK_ACTIVE_REQUESTS + BROWSER_MCP_TASK_QUEUED_REQUESTS;

        let (server, _hub, child) = setup().await;
        let claims = child.bootstrap.access.claims.clone();
        let task_resource_key = task_resource_family_key_from_claims(&claims);
        let entry = Arc::new(OwnerBindingEntry::default());
        server
            .state
            .bindings
            .lock()
            .await
            .insert(claims.lease_id.clone(), Arc::clone(&entry));
        let owner_gate = entry.operation.lock().await;

        let mut calls = tokio::task::JoinSet::new();
        for index in 0..REQUESTS {
            let child = child.clone();
            calls.spawn(async move {
                try_call_tool_with_args(
                    &child,
                    "navigate",
                    json!({ "url": format!("https://example.test/{index}") }),
                )
                .await
            });
        }

        let admission = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Some(admission) =
                    server.state.request_admissions.get(&task_resource_key)
                    && admission.outstanding.available_permits() == 0
                {
                    break admission;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("HTTP requests did not reach the per-task boundary");
        assert_eq!(admission.active.available_permits(), 0);

        for _ in 0..(REQUESTS - RETAINED) {
            let response = tokio::time::timeout(Duration::from_secs(2), calls.join_next())
                .await
                .expect("over-limit HTTP request did not fail promptly")
                .expect("request set ended before every overload response")
                .expect("request task panicked")
                .expect("request transport failed");
            assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
            let body: Value = response.json().await.unwrap();
            assert_eq!(
                body.pointer("/error/code").and_then(Value::as_str),
                Some("resource_exhausted")
            );
        }
        assert_eq!(admission.outstanding.available_permits(), 0);

        drop(owner_gate);
        let mut completed = 0;
        while let Some(response) = calls.join_next().await {
            let response = response
                .expect("request task panicked")
                .expect("request transport failed");
            assert_eq!(response.status(), StatusCode::OK);
            completed += 1;
        }
        assert_eq!(completed, RETAINED);
    }

    #[tokio::test]
    async fn oversized_body_is_rejected_before_task_admission() {
        let (server, _hub, child) = setup().await;
        let raw = serde_json::to_vec(&json!({
            "session": child.bootstrap.access.claims.clone(),
            "tool": "type",
            "args": {
                "text": "x".repeat(BROWSER_MCP_REQUEST_BODY_LIMIT_BYTES),
            },
        }))
        .unwrap();
        assert!(raw.len() > BROWSER_MCP_REQUEST_BODY_LIMIT_BYTES);
        let response = reqwest::Client::new()
            .post(format!(
                "http://127.0.0.1:{}/tool",
                child.bootstrap.port
            ))
            .bearer_auth(&child.bootstrap.access.token)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(raw)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        server.state.request_admissions.prune();
        assert_eq!(server.state.request_admissions.entry_count(), 0);
        assert!(server.state.bindings.lock().await.is_empty());
        assert_eq!(
            server.pre_auth_ingress.slots.available_permits(),
            server.pre_auth_ingress.capacity,
            "body rejection must return the pre-auth ingress permit"
        );
    }

    #[tokio::test]
    async fn stalled_chunked_body_hits_an_absolute_read_deadline() {
        use std::convert::Infallible;

        use axum::body::{Body, Bytes};
        use futures_util::{StreamExt, stream};

        // Yield one syntactically incomplete frame, then remain pending
        // forever. This models a chunked sender that never terminates its JSON
        // envelope; an idle-reset timeout could be defeated by periodically
        // adding frames, while this absolute extraction deadline cannot.
        let body = stream::once(async {
            Ok::<_, Infallible>(Bytes::from_static(b"{"))
        })
        .chain(stream::pending::<Result<Bytes, Infallible>>());
        let request = Request::builder()
            .method("POST")
            .uri("/tool")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::TRANSFER_ENCODING, "chunked")
            .body(Body::from_stream(body))
            .unwrap();

        let response = tokio::time::timeout(
            Duration::from_secs(1),
            extract_json_with_deadline::<_, Value>(
                request,
                &(),
                Duration::from_millis(20),
            ),
        )
        .await
        .expect("the absolute body deadline did not wake")
        .unwrap_err();
        assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
    }

    #[tokio::test]
    async fn one_thousand_stalled_bodies_are_bounded_by_pre_auth_ingress() {
        use std::convert::Infallible;

        use axum::body::{Body, Bytes};
        use futures_util::{StreamExt, stream};
        use tower::ServiceExt;

        const CAPACITY: usize = 16;
        const REQUESTS: usize = 1_000;

        async fn stalled_handler(
            Extension(_guard): Extension<PreAuthIngressGuard>,
            request: Request,
        ) -> Response {
            match extract_json_with_deadline::<_, Value>(
                request,
                &(),
                Duration::from_secs(30),
            )
            .await
            {
                Ok(_) => StatusCode::OK.into_response(),
                Err(response) => response,
            }
        }

        let ingress = PreAuthIngress::with_capacity(CAPACITY);
        let app = axum::Router::new()
            .route("/tool", axum::routing::post(stalled_handler))
            .layer(DefaultBodyLimit::disable())
            .layer(RequestBodyLimitLayer::new(
                BROWSER_MCP_REQUEST_BODY_LIMIT_BYTES,
            ))
            .layer(middleware::from_fn_with_state(
                ingress.clone(),
                enforce_pre_auth_ingress,
            ));
        let mut calls = tokio::task::JoinSet::new();
        for _ in 0..REQUESTS {
            let app = app.clone();
            calls.spawn(async move {
                let body = stream::once(async {
                    Ok::<_, Infallible>(Bytes::from_static(b"{"))
                })
                .chain(stream::pending::<Result<Bytes, Infallible>>());
                let request = Request::builder()
                    .method("POST")
                    .uri("/tool")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::TRANSFER_ENCODING, "chunked")
                    .body(Body::from_stream(body))
                    .unwrap();
                app.oneshot(request).await.unwrap()
            });
        }

        for _ in 0..(REQUESTS - CAPACITY) {
            let response = tokio::time::timeout(Duration::from_secs(3), calls.join_next())
                .await
                .expect("pre-auth overload was queued instead of rejected")
                .expect("request set ended before every overload response")
                .expect("pre-auth request task panicked");
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        }
        assert_eq!(
            ingress.slots.available_permits(),
            0,
            "only the fixed pre-auth capacity may retain stalled bodies"
        );

        calls.abort_all();
        while calls.join_next().await.is_some() {}
        assert_eq!(
            ingress.slots.available_permits(),
            CAPACITY,
            "cancelling every stalled body must restore every transport slot"
        );
    }

    #[tokio::test]
    async fn bounded_listener_caps_idle_pre_header_connections_and_recovers() {
        const CAPACITY: usize = 4;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let ingress = PreAuthConnectionIngress::with_capacity(CAPACITY);
        let mut bounded = BoundedTcpListener::new(listener, ingress.clone());
        let (accepted_tx, mut accepted_rx) = tokio::sync::mpsc::unbounded_channel();
        let accepter = tokio::spawn(async move {
            for _ in 0..=CAPACITY {
                let (stream, _) = bounded.accept().await;
                if accepted_tx.send(stream).is_err() {
                    return;
                }
            }
        });

        let mut clients = Vec::new();
        let mut accepted = Vec::new();
        for _ in 0..CAPACITY {
            clients.push(TcpStream::connect(address).await.unwrap());
            accepted.push(
                tokio::time::timeout(Duration::from_secs(1), accepted_rx.recv())
                    .await
                    .expect("bounded listener did not accept an available slot")
                    .unwrap(),
            );
        }
        assert_eq!(ingress.slots.available_permits(), 0);

        // The TCP handshake may sit in the OS backlog, but Axum must not own a
        // fifth FD/Hyper task while all four socket-lifetime permits are held.
        clients.push(
            tokio::time::timeout(Duration::from_secs(1), TcpStream::connect(address))
                .await
                .expect("backlogged loopback connect stalled")
                .unwrap(),
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(50), accepted_rx.recv())
                .await
                .is_err(),
            "the listener accepted beyond its connection high-water mark"
        );

        drop(accepted.remove(0));
        accepted.push(
            tokio::time::timeout(Duration::from_secs(1), accepted_rx.recv())
                .await
                .expect("dropping an idle socket did not restore acceptance")
                .unwrap(),
        );
        accepter.await.unwrap();
        assert_eq!(ingress.slots.available_permits(), 0);
        drop(accepted);
        drop(clients);
        assert_eq!(
            ingress.slots.available_permits(),
            CAPACITY,
            "socket Drop must return every connection-lifetime permit"
        );
    }

    #[tokio::test]
    async fn shutdown_releases_idle_pre_header_connection_authority() {
        use tokio::io::AsyncReadExt;

        let (server, _hub, _child) = setup().await;
        let mut idle_client = TcpStream::connect(server.http_addr).await.unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                // One permit belongs to the accepted idle socket and Axum may
                // reserve another while its next accept is pending.
                if server
                    .pre_auth_connections
                    .slots
                    .available_permits()
                    <= server.pre_auth_connections.capacity.saturating_sub(1)
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the idle socket was never accepted");

        tokio::time::timeout(Duration::from_secs(1), server.stop_and_wait())
            .await
            .expect("shutdown retained an idle pre-header socket")
            .unwrap();
        let mut byte = [0_u8; 1];
        let read = tokio::time::timeout(Duration::from_secs(1), idle_client.read(&mut byte))
            .await
            .expect("the accepted raw socket remained readable forever");
        assert!(
            matches!(read, Ok(0) | Err(_)),
            "shutdown must close, not merely stop accepting, the raw socket"
        );
        drop(idle_client);
        assert_eq!(
            server
                .pre_auth_connections
                .slots
                .available_permits(),
            server.pre_auth_connections.capacity,
            "serve shutdown must drop both accepted and pending-accept permits"
        );
        assert!(
            server
                .pre_auth_connections
                .accepted
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .shutdown_handles
                .is_empty(),
            "shutdown must not retain duplicate socket-close handles"
        );
    }

    #[tokio::test]
    async fn pre_auth_cap_is_released_before_long_post_auth_work() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        use axum::body::Body;
        use tower::ServiceExt;

        const CAPACITY: usize = 8;
        const VERIFIED_TASKS: usize = CAPACITY * 3;

        #[derive(Clone)]
        struct PostAuthHold {
            entered: Arc<AtomicUsize>,
            release: Arc<tokio::sync::Notify>,
        }

        async fn post_auth_handler(
            State(state): State<PostAuthHold>,
            Extension(guard): Extension<PreAuthIngressGuard>,
            TimedJson(_body): TimedJson<Value>,
        ) -> StatusCode {
            // Model the real handler's verified-task handoff. Work after this
            // point may be long lived without owning a global transport slot.
            guard.release();
            state.entered.fetch_add(1, Ordering::AcqRel);
            state.release.notified().await;
            StatusCode::OK
        }

        let ingress = PreAuthIngress::with_capacity(CAPACITY);
        let hold = PostAuthHold {
            entered: Arc::new(AtomicUsize::new(0)),
            release: Arc::new(tokio::sync::Notify::new()),
        };
        let app = axum::Router::new()
            .route("/tool", axum::routing::post(post_auth_handler))
            .with_state(hold.clone())
            .layer(DefaultBodyLimit::disable())
            .layer(RequestBodyLimitLayer::new(
                BROWSER_MCP_REQUEST_BODY_LIMIT_BYTES,
            ))
            .layer(middleware::from_fn_with_state(
                ingress.clone(),
                enforce_pre_auth_ingress,
            ));
        let mut calls = tokio::task::JoinSet::new();
        for batch in 1..=VERIFIED_TASKS / CAPACITY {
            for _ in 0..CAPACITY {
                let app = app.clone();
                calls.spawn(async move {
                    let request = Request::builder()
                        .method("POST")
                        .uri("/tool")
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from("{}"))
                        .unwrap();
                    app.oneshot(request).await.unwrap()
                });
            }
            tokio::time::timeout(Duration::from_secs(2), async {
                while hold.entered.load(Ordering::Acquire) != batch * CAPACITY {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("the transport cap leaked into post-auth task work");
        }
        assert_eq!(
            ingress.slots.available_permits(),
            CAPACITY,
            "verified long-lived work must not retain pre-auth slots"
        );
        hold.release.notify_waiters();
        while let Some(response) = calls.join_next().await {
            assert_eq!(response.unwrap().status(), StatusCode::OK);
        }
    }

    #[tokio::test]
    async fn forged_task_identity_cannot_allocate_an_admission_bucket() {
        let (server, _hub, child) = setup().await;
        let mut forged = child.bootstrap.access.claims.clone();
        forged.scope.runtime_instance_id =
            "0190f5fe-7c00-7a00-8000-000000000099".to_owned();
        assert!(validate_browser_claims(&forged).is_ok());
        let response = reqwest::Client::new()
            .post(format!(
                "http://127.0.0.1:{}/tool",
                child.bootstrap.port
            ))
            .bearer_auth(&child.bootstrap.access.token)
            .json(&json!({
                "session": forged,
                "tool": "navigate",
                "args": { "url": "https://example.test/forged" },
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        server.state.request_admissions.prune();
        assert_eq!(server.state.request_admissions.entry_count(), 0);
        assert!(server.state.bindings.lock().await.is_empty());
    }

    #[tokio::test]
    async fn valid_acp_capability_enters_the_shared_hub() {
        let (_server, hub, child) = setup().await;
        let response = call_tool(&child, "navigate").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["result"]["action"], "navigate");
        let lanes = hub.list_lanes().await;
        assert_eq!(lanes.len(), 1);
        assert_eq!(lanes[0].caller.surface, BrowserSurface::Acp);
        assert_eq!(lanes[0].caller.user_id, USER_ID);
        assert_eq!(
            lanes[0].caller.conversation_id.as_deref(),
            Some(CONVERSATION_ID)
        );
        assert_eq!(lanes[0].caller.agent_id.as_deref(), Some("agent-test"));
        assert_eq!(
            lanes[0].caller.runtime_instance_id,
            child.bootstrap.access.claims.scope.runtime_instance_id
        );
        assert_eq!(
            lanes[0].caller.task_resource_family_key().into_string(),
            task_resource_family_key_from_claims(&child.bootstrap.access.claims),
            "MCP admission and Hub accounting must share the exact stable task key"
        );
        assert!(
            lanes[0]
                .caller
                .allowed_operations
                .contains(&BrowserOperationKind::Debug)
        );
        assert!(
            !child.bootstrap.access.claims.allows("evaluate"),
            "the Debug operation class enables read-only diagnostics, not arbitrary script execution"
        );
        for tool in ["get_console_logs", "get_page_errors", "get_network_log"] {
            assert!(
                child.bootstrap.access.claims.allows(tool),
                "read-only diagnostic tool {tool} must be granted"
            );
        }
    }

    #[tokio::test]
    async fn unavailable_hub_cannot_create_an_owner_binding() {
        let (server, hub, child) = setup().await;
        server.set_hub(Weak::<BrowserSessionHub>::new());

        let response = call_tool(&child, "navigate").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value = response.json().await.unwrap();
        assert!(body.get("error").is_some(), "unexpected response: {body}");
        assert!(server.state.bindings.lock().await.is_empty());
        assert_eq!(server.state.owner_lease_issues.load(Ordering::Acquire), 0);

        server.set_hub(Arc::downgrade(&hub));
        server.stop_and_wait().await.unwrap();
    }

    #[tokio::test]
    async fn active_owner_bindings_scale_with_distinct_signed_tasks() {
        const TASKS: usize = 64;
        let (server, hub, _unused_child) = setup().await;

        for index in 0..TASKS {
            let agent_id = format!("elastic-agent-{index}");
            // Independent user-visible tasks means independent *conversations*:
            // the Hub's owner-lease budget is charged per task-resource family,
            // and sibling agents inside one conversation deliberately share it
            // (32 active generations per family). Varying only the agent id
            // would exercise that shared per-family cap, not MCP scaling.
            let conversation_id = format!("0190f5fe-7c00-7a00-8000-{:012x}", index + 1);
            let child = server
                .issuer_config("nomicore".into())
                .issue_for_conversation(
                    USER_ID,
                    &conversation_id,
                    Some(&agent_id),
                )
                .unwrap();
            ensure_owner_binding(&server.state, &hub, &child.bootstrap.access.claims)
                .await
                .unwrap();
        }

        assert_eq!(server.state.bindings.lock().await.len(), TASKS);
        assert_eq!(
            server.state.owner_lease_issues.load(Ordering::Acquire),
            TASKS,
            "MCP must not impose a process-wide cap on independent active tasks"
        );
        server.stop_and_wait().await.unwrap();
        assert!(server.state.bindings.lock().await.is_empty());
    }

    #[tokio::test]
    async fn owner_binding_rejects_claim_policy_changes_before_renewal_or_replacement() {
        let (server, hub, child) = setup().await;
        let claims = child.bootstrap.access.claims.clone();
        let owner = ensure_owner_binding(&server.state, &hub, &claims)
            .await
            .unwrap();

        for changed in [
            {
                let mut changed = claims.clone();
                changed.scope.allowed_operations = vec![
                    BrowserCapabilityOperation::Manage,
                    BrowserCapabilityOperation::Navigate,
                    BrowserCapabilityOperation::Observe,
                    BrowserCapabilityOperation::Act,
                    BrowserCapabilityOperation::Screenshot,
                    BrowserCapabilityOperation::Tabs,
                    BrowserCapabilityOperation::Download,
                    BrowserCapabilityOperation::Debug,
                ];
                changed.allowed_tools.retain(|tool| {
                    browser_tool_operation(tool)
                        .is_some_and(|operation| changed.scope.allows(operation))
                });
                changed
            },
            {
                let mut changed = claims.clone();
                changed.scope.agent_id = Some("agent-forged".to_owned());
                changed
            },
        ] {
            assert!(validate_browser_claims(&changed).is_ok());
            let error = ensure_owner_binding(&server.state, &hub, &changed)
                .await
                .unwrap_err();
            assert_eq!(error.code, BrowserErrorCode::InvalidCallerIdentity);
            assert_eq!(
                server
                    .state
                    .bindings
                    .lock()
                    .await
                    .get(&claims.lease_id)
                    .unwrap()
                    .state
                    .lock()
                    .await
                    .binding
                    .as_ref()
                    .unwrap()
                    .owner_lease_id,
                owner
            );
        }
    }

    #[tokio::test]
    async fn expired_acp_owner_replacement_keeps_signed_policy_and_rejects_old_lane() {
        let mut config = HubConfig::default();
        config.owner_lease_ttl_ms = 10;
        let hub = Arc::new(BrowserSessionHub::new(
            Arc::new(FakeFactory {
                launches: AtomicUsize::new(0),
            }),
            config,
        ));
        let server = BrowserMcpServer::start().await.unwrap();
        server.set_hub(Arc::downgrade(&hub));
        let child = server
            .issuer_config("nomicore".into())
            .issue_for_conversation(USER_ID, CONVERSATION_ID, Some("agent-test"))
            .unwrap();
        let claims = child.bootstrap.access.claims.clone();

        let old_owner = ensure_owner_binding(&server.state, &hub, &claims)
            .await
            .unwrap();
        let old_caller = caller_from_claims(&claims, old_owner.clone());
        let old_client = hub.bind(old_caller.clone()).unwrap();
        let old_lane = old_client
            .open(None, nomifun_browser_platform::BrowserIdentityMode::Primary, None)
            .await
            .unwrap()
            .lane()
            .clone();

        tokio::time::sleep(Duration::from_millis(20)).await;

        let replacement_owner = ensure_owner_binding(&server.state, &hub, &claims)
            .await
            .unwrap();
        assert_ne!(replacement_owner, old_owner);
        let replacement_caller = caller_from_claims(&claims, replacement_owner);
        assert_eq!(replacement_caller.surface, BrowserSurface::Acp);
        assert_eq!(
            replacement_caller.allowed_operations,
            owner_binding_policy_from_claims(&claims).allowed_operations
        );
        let replacement_client = hub.bind(replacement_caller).unwrap();
        let replacement_lane = replacement_client
            .open(None, nomifun_browser_platform::BrowserIdentityMode::Primary, None)
            .await
            .unwrap()
            .lane()
            .clone();
        assert_ne!(replacement_lane.lane_id, old_lane.lane_id);

        let old_error = old_client.status(&old_lane.lane_id).await.unwrap_err();
        assert_eq!(old_error.code, BrowserErrorCode::OwnerLeaseExpired);
    }

    #[tokio::test]
    async fn owner_cleanup_failure_blocks_unbounded_replacement_generations() {
        let cleanup_blocked = Arc::new(AtomicBool::new(true));
        let mut config = HubConfig::default();
        config.owner_lease_ttl_ms = 10;
        let hub = Arc::new(BrowserSessionHub::new(
            Arc::new(CleanupFailingFactory {
                cleanup_blocked: Arc::clone(&cleanup_blocked),
            }),
            config,
        ));
        let server = BrowserMcpServer::start().await.unwrap();
        server.set_hub(Arc::downgrade(&hub));
        let child = server
            .issuer_config("nomicore".into())
            .issue_for_conversation(USER_ID, CONVERSATION_ID, Some("agent-test"))
            .unwrap();
        let claims = child.bootstrap.access.claims.clone();
        // Keep this binding outside the live server state's periodic sweeper so
        // the test controls every failed cleanup/replacement transition.
        let state = BrowserMcpState {
            issuer: Arc::clone(&server.issuer),
            hub: Arc::clone(&server.hub),
            bindings: Arc::new(Mutex::new(HashMap::new())),
            request_admissions: TaskRequestAdmissions::default(),
            owner_lease_issues: Arc::new(AtomicUsize::new(0)),
            revoked_cleanup_active: Arc::new(AtomicUsize::new(0)),
            revoked_cleanup_peak: Arc::new(AtomicUsize::new(0)),
            request_started: Arc::new(tokio::sync::Notify::new()),
        };

        let old_owner = ensure_owner_binding(&state, &hub, &claims)
            .await
            .unwrap();
        let old_client = hub
            .bind(caller_from_claims(&claims, old_owner.clone()))
            .unwrap();
        old_client
            .open(
                None,
                nomifun_browser_platform::BrowserIdentityMode::Primary,
                None,
            )
            .await
            .unwrap();
        assert_eq!(state.owner_lease_issues.load(Ordering::Acquire), 1);

        tokio::time::sleep(Duration::from_millis(20)).await;
        for _ in 0..64 {
            let error = ensure_owner_binding(&state, &hub, &claims)
                .await
                .expect_err("an uncleared exact owner must block every replacement");
            assert_eq!(
                error
                    .metadata
                    .get("cleanup_pending")
                    .and_then(Value::as_bool),
                Some(true),
                "every retained MCP owner must correspond to Hub-owned cleanup debt"
            );
        }

        let entry = state
            .bindings
            .lock()
            .await
            .get(&claims.lease_id)
            .cloned()
            .expect("failed cleanup must retain its binding entry");
        {
            let binding_state = entry.state.lock().await;
            assert!(
                binding_state.binding.is_none(),
                "a revoked current owner must not remain publishable"
            );
            assert_eq!(
                binding_state.pending_owner_cleanup,
                vec![old_owner.clone()],
                "retries must deduplicate the exact pending owner"
            );
        }
        assert_eq!(
            state.owner_lease_issues.load(Ordering::Acquire),
            1,
            "permanent cleanup failure must not mint owner generations"
        );

        cleanup_blocked.store(false, Ordering::Release);
        let replacement = ensure_owner_binding(&state, &hub, &claims)
            .await
            .expect("replacement may be issued after exact cleanup recovers");
        assert_ne!(replacement, old_owner);
        assert_eq!(state.owner_lease_issues.load(Ordering::Acquire), 2);
        assert!(entry.state.lock().await.pending_owner_cleanup.is_empty());

        let renewed = ensure_owner_binding(&state, &hub, &claims)
            .await
            .expect("the single replacement should renew normally");
        assert_eq!(renewed, replacement);
        assert_eq!(
            state.owner_lease_issues.load(Ordering::Acquire),
            2,
            "recovery may publish exactly one replacement"
        );

        cleanup_binding(&state, &claims.lease_id).await;
        server.stop_and_wait().await.unwrap();
    }

    #[tokio::test]
    async fn pending_owner_cleanup_completes_before_replacement_without_deadlock() {
        let (server, hub, child) = setup().await;
        let claims = child.bootstrap.access.claims.clone();
        // Use a detached binding inventory so the server's periodic sweep
        // cannot race this exact replacement interleaving.
        let state = BrowserMcpState {
            issuer: Arc::clone(&server.issuer),
            hub: Arc::clone(&server.hub),
            bindings: Arc::new(Mutex::new(HashMap::new())),
            request_admissions: TaskRequestAdmissions::default(),
            owner_lease_issues: Arc::new(AtomicUsize::new(0)),
            revoked_cleanup_active: Arc::new(AtomicUsize::new(0)),
            revoked_cleanup_peak: Arc::new(AtomicUsize::new(0)),
            request_started: Arc::new(tokio::sync::Notify::new()),
        };

        let old_owner = ensure_owner_binding(&state, &hub, &claims)
            .await
            .unwrap();
        let old_client = hub
            .bind(caller_from_claims(&claims, old_owner.clone()))
            .unwrap();
        let old_lane = old_client
            .open(
                None,
                nomifun_browser_platform::BrowserIdentityMode::Primary,
                None,
            )
            .await
            .unwrap()
            .lane()
            .clone();
        let entry = state
            .bindings
            .lock()
            .await
            .get(&claims.lease_id)
            .cloned()
            .expect("the initial ensure must publish a binding entry");

        // Model an interrupted replacement after it detached the expired
        // current owner but retained exact-owner cleanup authority. The next
        // ensure already owns `entry.operation` when it encounters this
        // state. Calling the locking retry wrapper from there deadlocks on the
        // same non-reentrant mutex; the timeout makes that old failure mode
        // deterministic without relying on a sleep.
        {
            let _operation_guard = entry.operation.lock().await;
            let mut binding_state = entry.state.lock().await;
            let binding = binding_state
                .binding
                .take()
                .expect("the initial owner binding must still be current");
            assert_eq!(binding.owner_lease_id, old_owner);
            binding_state
                .pending_owner_cleanup
                .push(binding.owner_lease_id);
        }

        let replacement_owner = tokio::time::timeout(
            Duration::from_secs(1),
            ensure_owner_binding(&state, &hub, &claims),
        )
        .await
        .expect("replacement deadlocked while retrying pending owner cleanup")
        .unwrap();

        assert_ne!(replacement_owner, old_owner);
        assert!(
            hub.renew_owner_lease(&old_owner).is_err(),
            "pending exact-owner cleanup must finish before replacement returns"
        );
        assert!(
            hub.list_lanes().await.is_empty(),
            "pending cleanup must close the superseded owner's Lane"
        );
        {
            let binding_state = entry.state.lock().await;
            assert!(
                binding_state.pending_owner_cleanup.is_empty(),
                "successful retry must clear the pending cleanup inventory"
            );
            assert_eq!(
                binding_state
                    .binding
                    .as_ref()
                    .expect("replacement must publish a current owner")
                    .owner_lease_id,
                replacement_owner
            );
        }

        let replacement_client = hub
            .bind(caller_from_claims(&claims, replacement_owner.clone()))
            .unwrap();
        let replacement_lane = replacement_client
            .open(
                None,
                nomifun_browser_platform::BrowserIdentityMode::Primary,
                None,
            )
            .await
            .unwrap()
            .lane()
            .clone();
        assert_ne!(replacement_lane.lane_id, old_lane.lane_id);

        cleanup_binding(&state, &claims.lease_id).await;
        assert!(state.bindings.lock().await.is_empty());
        assert!(hub.list_lanes().await.is_empty());
        server.stop_and_wait().await.unwrap();
    }

    async fn assert_revoked_cleanup_window(binding_count: usize) {
        let (server, _hub, _child) = setup().await;
        let state = BrowserMcpState {
            issuer: Arc::clone(&server.issuer),
            hub: Arc::clone(&server.hub),
            bindings: Arc::new(Mutex::new(HashMap::new())),
            request_admissions: TaskRequestAdmissions::default(),
            owner_lease_issues: Arc::new(AtomicUsize::new(0)),
            revoked_cleanup_active: Arc::new(AtomicUsize::new(0)),
            revoked_cleanup_peak: Arc::new(AtomicUsize::new(0)),
            request_started: Arc::new(tokio::sync::Notify::new()),
        };
        let entries = (0..binding_count)
            .map(|_| Arc::new(OwnerBindingEntry::default()))
            .collect::<Vec<_>>();
        {
            let mut bindings = state.bindings.lock().await;
            for (index, entry) in entries.iter().enumerate() {
                bindings.insert(
                    format!("already-revoked-capability-{binding_count}-{index}"),
                    Arc::clone(entry),
                );
            }
        }
        let mut operation_guards = Vec::with_capacity(entries.len());
        for entry in &entries {
            operation_guards.push(entry.operation.lock().await);
        }

        let sweep_state = state.clone();
        let sweep = tokio::spawn(async move {
            cleanup_revoked_bindings(&sweep_state).await;
        });
        let expected_window = binding_count.min(REVOKED_BINDING_CLEANUP_CONCURRENCY);
        tokio::time::timeout(Duration::from_secs(1), async {
            while state.revoked_cleanup_active.load(Ordering::Acquire) < expected_window {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("revoked cleanup tasks did not fill their fixed window");
        assert_eq!(
            state.revoked_cleanup_peak.load(Ordering::Acquire),
            expected_window
        );

        if binding_count > REVOKED_BINDING_CLEANUP_CONCURRENCY {
            drop(operation_guards.remove(0));
            tokio::time::timeout(Duration::from_secs(1), async {
                while state.bindings.lock().await.len() == binding_count {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("freeing one window slot did not advance the sweep");
            assert_eq!(
                state.revoked_cleanup_peak.load(Ordering::Acquire),
                REVOKED_BINDING_CLEANUP_CONCURRENCY,
                "the N+1 binding must reuse a completed slot instead of spawning another task"
            );
        }

        drop(operation_guards);
        tokio::time::timeout(Duration::from_secs(1), sweep)
            .await
            .expect("bounded revoked cleanup sweep did not finish")
            .unwrap();
        assert_eq!(state.revoked_cleanup_active.load(Ordering::Acquire), 0);
        assert!(state.bindings.lock().await.is_empty());
        server.stop_and_wait().await.unwrap();
    }

    #[tokio::test]
    async fn revoked_binding_sweep_has_a_fixed_n_and_n_plus_one_task_window() {
        assert_revoked_cleanup_window(REVOKED_BINDING_CLEANUP_CONCURRENCY).await;
        assert_revoked_cleanup_window(REVOKED_BINDING_CLEANUP_CONCURRENCY + 1).await;
    }

    #[tokio::test]
    async fn read_only_debug_tools_are_granted_but_evaluate_is_not() {
        let (_server, _hub, child) = setup().await;
        for (tool, args) in [
            ("get_console_logs", json!({})),
            ("get_page_errors", json!({})),
            ("get_network_log", json!({"include_bodies": false})),
        ] {
            let response = call_tool_with_args(&child, tool, args).await;
            assert_eq!(response.status(), StatusCode::OK, "{tool}");
            let body: Value = response.json().await.unwrap();
            assert_eq!(body["result"]["action"], tool, "{body}");
        }
        assert_eq!(
            call_tool(&child, "evaluate").await.status(),
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn acp_management_and_lane_id_share_the_owner_scoped_contract() {
        let (_server, hub, child) = setup().await;
        let opened = call_tool_with_args(
            &child,
            "browser_open",
            json!({"lane_name": "research"}),
        )
        .await;
        assert_eq!(opened.status(), StatusCode::OK);
        let opened: Value = opened.json().await.unwrap();
        let lane_id = opened
            .pointer("/result/lane/lane_id")
            .and_then(Value::as_str)
            .expect("browser_open returns a short handle")
            .to_owned();

        let navigated = call_tool_with_args(
            &child,
            "navigate",
            json!({"url": "https://example.test/research", "lane_id": lane_id}),
        )
        .await;
        assert_eq!(navigated.status(), StatusCode::OK);
        let navigated: Value = navigated.json().await.unwrap();
        assert_eq!(
            navigated.pointer("/result/lane_id").and_then(Value::as_str),
            Some(lane_id.as_str())
        );

        let listed = call_tool_with_args(&child, "browser_list", json!({}))
            .await;
        let listed: Value = listed.json().await.unwrap();
        assert_eq!(
            listed
                .pointer("/result/lanes")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(1)
        );
        assert_eq!(hub.list_lanes().await.len(), 1);

        let closed = call_tool_with_args(
            &child,
            "browser_close",
            json!({"lane_id": lane_id}),
        )
        .await;
        let closed: Value = closed.json().await.unwrap();
        assert_eq!(
            closed.pointer("/result/closed").and_then(Value::as_u64),
            Some(1)
        );
        assert!(hub.list_lanes().await.is_empty());
    }

    #[tokio::test]
    async fn acp_lane_id_cannot_cross_sibling_runtime_ownership() {
        let (server, hub, first) = setup().await;
        let sibling = server
            .issuer_config("nomicore".into())
            .issue_for_conversation(USER_ID, CONVERSATION_ID, Some("agent-sibling"))
            .unwrap();
        let opened = call_tool_with_args(&first, "browser_open", json!({})).await;
        let opened: Value = opened.json().await.unwrap();
        let lane_id = opened
            .pointer("/result/lane/lane_id")
            .and_then(Value::as_str)
            .unwrap()
            .to_owned();

        let crossed = call_tool_with_args(
            &sibling,
            "browser_status",
            json!({"lane_id": lane_id}),
        )
        .await;
        assert_eq!(crossed.status(), StatusCode::OK);
        let crossed: Value = crossed.json().await.unwrap();
        assert!(
            crossed.get("error").is_some(),
            "an unowned lane handle must fail as a tool error: {crossed}"
        );
        assert_eq!(hub.list_lanes().await.len(), 1);
    }

    #[tokio::test]
    async fn acp_crawl_many_uses_and_cleans_only_hub_lanes() {
        let (_server, hub, child) = setup().await;
        let response = call_tool_with_args(
            &child,
            "browser_crawl_many",
            json!({
                "urls": [
                    "https://example.test/a",
                    "https://example.test/b",
                    "https://example.test/c"
                ],
                "concurrency": 2,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value = response.json().await.unwrap();
        let results = body
            .pointer("/result/results")
            .and_then(Value::as_array)
            .expect("one crawl terminal result per URL");
        assert_eq!(results.len(), 3);
        assert_eq!(results[0]["url"], "https://example.test/a");
        assert_eq!(results[1]["url"], "https://example.test/b");
        assert_eq!(results[2]["url"], "https://example.test/c");
        assert!(
            hub.list_lanes().await.is_empty(),
            "crawl_many must await Hub Lane cleanup"
        );
    }

    #[tokio::test]
    async fn model_identity_fields_fail_closed_before_binding_or_facade_dispatch() {
        let (server, hub, child) = setup().await;
        for field in MODEL_IDENTITY_INPUT_FIELDS {
            let response = call_tool_with_args(
                &child,
                "browser_open",
                json!({
                    "lane_name": "research",
                    (*field): "model-controlled",
                }),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK, "{field}");
            let body: Value = response.json().await.unwrap();
            assert_eq!(
                body.pointer("/error/code").and_then(Value::as_str),
                Some("invalid_caller_identity"),
                "{field} must fail at the MCP boundary: {body}"
            );
            assert!(
                server.state.bindings.lock().await.is_empty(),
                "{field} must be rejected before an owner binding is created"
            );
            assert!(
                hub.list_lanes().await.is_empty(),
                "{field} must be rejected before facade dispatch or Host launch"
            );
        }
    }

    #[tokio::test]
    async fn trusted_owner_fields_fail_closed_before_binding_or_facade_dispatch() {
        let (server, hub, child) = setup().await;
        assert!(
            TRUSTED_OWNER_INPUT_FIELDS.contains(&"runtime_cleanup_key"),
            "the exact runtime cleanup key must remain host-owned"
        );
        for field in TRUSTED_OWNER_INPUT_FIELDS {
            let response = call_tool_with_args(
                &child,
                "browser_open",
                json!({
                    "lane_name": "research",
                    (*field): "model-controlled",
                }),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK, "{field}");
            let body: Value = response.json().await.unwrap();
            assert_eq!(
                body.pointer("/error/code").and_then(Value::as_str),
                Some("invalid_caller_identity"),
                "{field} must fail at the MCP boundary: {body}"
            );
            assert!(
                server.state.bindings.lock().await.is_empty(),
                "{field} must be rejected before an owner binding is created"
            );
            assert!(
                hub.list_lanes().await.is_empty(),
                "{field} must be rejected before facade dispatch or Host launch"
            );
        }
    }

    #[tokio::test]
    async fn sibling_acp_runtimes_in_one_conversation_get_distinct_lanes() {
        let (server, hub, first) = setup().await;
        let second = server
            .issuer_config("nomicore".into())
            .issue_for_conversation(USER_ID, CONVERSATION_ID, Some("agent-sibling"))
            .unwrap();
        assert_eq!(call_tool(&first, "navigate").await.status(), StatusCode::OK);
        assert_eq!(
            call_tool(&second, "navigate").await.status(),
            StatusCode::OK
        );

        let lanes = hub.list_lanes().await;
        assert_eq!(lanes.len(), 2);
        let runtimes = lanes
            .iter()
            .map(|lane| lane.caller.runtime_instance_id.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(runtimes.len(), 2);
        assert!(runtimes.contains(first.bootstrap.access.claims.scope.runtime_instance_id.as_str()));
        assert!(
            runtimes.contains(
                second
                    .bootstrap
                    .access
                    .claims
                    .scope
                    .runtime_instance_id
                    .as_str()
            )
        );
    }

    #[tokio::test]
    async fn tool_outside_signed_operation_scope_fails_closed() {
        let (_server, _hub, child) = setup().await;
        let response = call_tool(&child, "evaluate").await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn dropping_parent_capability_guard_closes_owner_lanes() {
        let (_server, hub, child) = setup().await;
        assert_eq!(call_tool(&child, "navigate").await.status(), StatusCode::OK);
        assert_eq!(hub.list_lanes().await.len(), 1);
        drop(child);
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if hub.list_lanes().await.is_empty() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("revoked capability must clean its owner lanes");
    }

    #[tokio::test]
    async fn async_shutdown_waits_for_binding_cleanup() {
        let (server, hub, child) = setup().await;
        assert_eq!(call_tool(&child, "navigate").await.status(), StatusCode::OK);
        assert_eq!(hub.list_lanes().await.len(), 1);

        server.shutdown().await.unwrap();

        assert!(
            hub.list_lanes().await.is_empty(),
            "async shutdown must wait for owner-lane cleanup"
        );
    }

    #[tokio::test]
    async fn concurrent_and_repeated_stop_waiters_share_one_flight() {
        let (server, hub, child) = setup().await;
        let http_addr = server.http_addr;
        assert_eq!(call_tool(&child, "navigate").await.status(), StatusCode::OK);
        assert_eq!(hub.list_lanes().await.len(), 1);

        let (first, second) = tokio::join!(
            server.stop_and_wait(),
            server.stop_and_wait(),
        );
        first.unwrap();
        second.unwrap();
        assert!(hub.list_lanes().await.is_empty());
        assert!(
            TcpListener::bind(http_addr).await.is_ok(),
            "successful shutdown must include completed ingress teardown"
        );

        // Completion is cached. A later waiter observes the same terminal
        // result rather than treating consumed task handles as success.
        server.stop_and_wait().await.unwrap();
    }

    #[tokio::test]
    async fn shutdown_cancels_in_flight_handler_before_final_binding_cleanup() {
        let (server, hub, child) = setup().await;
        let capability_lease_id = child.bootstrap.access.claims.lease_id.clone();
        let entry = Arc::new(OwnerBindingEntry::default());
        server
            .state
            .bindings
            .lock()
            .await
            .insert(capability_lease_id.clone(), Arc::clone(&entry));

        // Hold the per-capability transition after HTTP has accepted the
        // request. This makes the connection observably in flight while
        // graceful shutdown closes the listener.
        let operation_guard = entry.operation.lock().await;
        let request_started = server.state.request_started.notified();
        let request_child = child.clone();
        let in_flight = tokio::spawn(async move {
            try_call_tool_with_args(
                &request_child,
                "navigate",
                json!({ "url": "https://example.test/in-flight" }),
            )
            .await
        });
        request_started.await;

        server.begin_stop();
        assert!(
            server
                .issuer
                .is_lease_active(BROWSER_CAPABILITY_DOMAIN, &capability_lease_id),
            "the final binding drain must retain authority while cleanup is pinned"
        );

        let cancelled = tokio::time::timeout(Duration::from_secs(1), in_flight)
            .await
            .expect("shutdown did not cancel the blocked handler")
            .unwrap();
        if let Ok(response) = cancelled {
            assert!(response.status().is_server_error());
        }
        assert!(
            entry.state.lock().await.binding.is_none(),
            "the cancelled handler must not publish an owner"
        );

        let unexpected_handler_start =
            server.state.request_started.notified();
        let rejected = try_call_tool_with_args(
            &child,
            "navigate",
            json!({ "url": "https://example.test/rejected-after-stop" }),
        )
        .await;
        if let Ok(response) = rejected {
            assert!(
                response.status().is_server_error(),
                "a scheduling-race TCP handshake must still fail before the handler: {}",
                response.status()
            );
        }
        assert!(
            tokio::time::timeout(
                Duration::from_millis(25),
                unexpected_handler_start,
            )
            .await
            .is_err(),
            "a post-stop request reached the tool handler"
        );
        assert!(
            entry.state.lock().await.binding.is_none(),
            "a post-stop request created an owner binding"
        );

        drop(operation_guard);
        server.stop_and_wait().await.unwrap();

        assert!(
            !server
                .issuer
                .is_lease_active(BROWSER_CAPABILITY_DOMAIN, &capability_lease_id),
            "the final drain must revoke the capability after HTTP has quiesced"
        );
        assert!(
            hub.list_lanes().await.is_empty(),
            "the authoritative post-cancellation drain must leave no owner resources"
        );
    }

    #[test]
    fn shutdown_timeout_aggregates_ingress_and_cleanup_state() {
        let error = BrowserMcpShutdownStatus::default()
            .timeout_result(Duration::from_millis(7))
            .unwrap_err();
        assert!(error.contains("HTTP ingress shutdown exceeded"));
        assert!(error.contains("cleanup exceeded"));
        assert!(error.contains("durable cleanup detached"));
    }

    #[test]
    fn permanent_cleanup_failure_uses_a_bounded_exponential_backoff() {
        let mut wait = CLEANUP_RETRY_WAIT;
        let mut observed = Vec::new();
        for _ in 0..16 {
            observed.push(wait);
            wait = next_cleanup_retry_wait(wait, false);
        }
        assert_eq!(observed[0], CLEANUP_RETRY_WAIT);
        assert_eq!(wait, CLEANUP_RETRY_MAX_WAIT);
        assert!(observed.into_iter().all(|delay| {
            delay >= CLEANUP_RETRY_WAIT && delay <= CLEANUP_RETRY_MAX_WAIT
        }));
        assert_eq!(
            next_cleanup_retry_wait(wait, true),
            CLEANUP_RETRY_WAIT,
            "any completed binding should restore prompt retry latency"
        );
    }

    #[tokio::test]
    async fn timeout_then_retry_waits_for_the_same_durable_stop_flight() {
        let (server, hub, child) = setup().await;
        assert_eq!(call_tool(&child, "navigate").await.status(), StatusCode::OK);
        let entry = server
            .state
            .bindings
            .lock()
            .await
            .get(&child.bootstrap.access.claims.lease_id)
            .cloned()
            .expect("tool call publishes an owner binding");

        // Pin the exact-owner drain so the first wait times out. The timeout
        // must not own or abort the cleanup task.
        let operation_guard = entry.operation.lock().await;
        let error = server
            .stop_and_wait_for(Duration::from_millis(1))
            .await
            .unwrap_err();
        assert!(
            error.contains("durable cleanup detached"),
            "unexpected timeout error: {error}"
        );
        assert_eq!(hub.list_lanes().await.len(), 1);

        drop(operation_guard);
        server.stop_and_wait().await.unwrap();
        assert!(hub.list_lanes().await.is_empty());
        server.stop_and_wait().await.unwrap();
    }

    #[tokio::test]
    async fn cancelling_stuck_periodic_cleanup_preserves_owned_final_drain() {
        let (server, hub, child) = setup().await;
        assert_eq!(call_tool(&child, "navigate").await.status(), StatusCode::OK);
        let capability_lease_id = child.bootstrap.access.claims.lease_id.clone();
        let entry = server
            .state
            .bindings
            .lock()
            .await
            .get(&capability_lease_id)
            .cloned()
            .expect("tool call publishes an owner binding");
        let operation_guard = entry.operation.lock().await;
        let cleanup_captured = entry.cleanup_captured.notified();
        LoopbackCapabilityLease::new(
            Arc::clone(&server.issuer),
            BROWSER_CAPABILITY_DOMAIN,
            capability_lease_id,
        )
        .revoke();
        tokio::time::timeout(Duration::from_secs(1), cleanup_captured)
            .await
            .expect("periodic cleanup did not capture the revoked binding");

        let error = server
            .stop_and_wait_for(Duration::from_millis(1))
            .await
            .unwrap_err();
        assert!(error.contains("cleanup exceeded"), "unexpected error: {error}");
        tokio::time::sleep(BROWSER_MCP_PERIODIC_CLEANUP_STOP_GRACE * 2).await;
        assert_eq!(hub.list_lanes().await.len(), 1);
        assert!(
            !server.state.bindings.lock().await.is_empty(),
            "aborting the optional periodic worker must retain exact cleanup authority"
        );

        drop(operation_guard);
        tokio::time::timeout(Duration::from_secs(1), server.stop_and_wait())
            .await
            .expect("the owned final drain did not resume")
            .unwrap();
        assert!(server.state.bindings.lock().await.is_empty());
        assert!(hub.list_lanes().await.is_empty());
    }

    #[tokio::test]
    async fn finite_cleanup_wait_cancels_http_without_losing_owned_cleanup() {
        let (server, hub, child) = setup().await;
        let capability_lease_id = child.bootstrap.access.claims.lease_id.clone();
        let sibling = server
            .issuer_config("nomicore".into())
            .issue_for_conversation(USER_ID, CONVERSATION_ID, Some("agent-sibling"))
            .unwrap();
        assert_eq!(
            call_tool(&sibling, "navigate").await.status(),
            StatusCode::OK
        );
        let sibling_entry = server
            .state
            .bindings
            .lock()
            .await
            .get(&sibling.bootstrap.access.claims.lease_id)
            .cloned()
            .expect("sibling tool call publishes an owner binding");
        let entry = Arc::new(OwnerBindingEntry::default());
        server
            .state
            .bindings
            .lock()
            .await
            .insert(capability_lease_id, Arc::clone(&entry));

        let operation_guard = entry.operation.lock().await;
        let ensure_captured = entry.ensure_captured.notified();
        let cleanup_guard = sibling_entry.operation.lock().await;
        let request_started = server.state.request_started.notified();
        let request_child = child.clone();
        let in_flight = tokio::spawn(async move {
            try_call_tool_with_args(
                &request_child,
                "navigate",
                json!({ "url": "https://example.test/in-flight-timeout" }),
            )
            .await
        });
        request_started.await;
        ensure_captured.await;

        let error = tokio::time::timeout(
            Duration::from_secs(1),
            server.stop_and_wait_for(Duration::from_millis(1)),
        )
        .await
        .expect("shutdown did not cancel the blocked HTTP handler")
        .unwrap_err();
        assert!(
            error.contains("cleanup exceeded"),
            "the finite caller wait may expire while owned cleanup continues: {error}"
        );
        let in_flight = tokio::time::timeout(Duration::from_secs(1), in_flight)
            .await
            .expect("the blocked HTTP handler survived shutdown cancellation")
            .unwrap();
        if let Ok(response) = in_flight {
            assert!(response.status().is_server_error());
        }
        assert!(
            !server.state.bindings.lock().await.is_empty(),
            "the timed-out caller must leave exact-owner cleanup authority intact"
        );
        assert_eq!(
            hub.list_lanes().await.len(),
            1,
            "handler cancellation must not discard the sibling owner's live cleanup debt"
        );
        drop(operation_guard);
        drop(cleanup_guard);
        server.stop_and_wait().await.unwrap();
        assert!(hub.list_lanes().await.is_empty());
    }

    #[tokio::test]
    async fn cleanup_success_requires_an_empty_binding_inventory() {
        let (server, hub, child) = setup().await;
        assert_eq!(call_tool(&child, "navigate").await.status(), StatusCode::OK);
        let entry = server
            .state
            .bindings
            .lock()
            .await
            .get(&child.bootstrap.access.claims.lease_id)
            .cloned()
            .expect("tool call publishes an owner binding");
        let operation_guard = entry.operation.lock().await;

        let waiter = server.stop_and_wait();
        tokio::pin!(waiter);
        assert!(
            tokio::time::timeout(Duration::from_millis(25), &mut waiter)
                .await
                .is_err(),
            "the authoritative barrier must remain pending while cleanup is pinned"
        );
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if matches!(
                    server.lifecycle.completion.borrow().serve,
                    BrowserMcpShutdownStage::Complete(_)
                ) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("HTTP ingress must quiesce while final binding cleanup is pinned");
        assert!(
            matches!(
                server.lifecycle.completion.borrow().cleanup,
                BrowserMcpShutdownStage::Pending
            ),
            "cleanup must remain pending while an exact owner binding is retained"
        );
        assert!(
            !server.state.bindings.lock().await.is_empty(),
            "a retained binding is the durable cleanup authority"
        );

        drop(operation_guard);
        waiter.await.unwrap();
        assert!(server.state.bindings.lock().await.is_empty());
        assert!(hub.list_lanes().await.is_empty());
        assert!(
            matches!(
                server.lifecycle.completion.borrow().cleanup,
                BrowserMcpShutdownStage::Complete(Ok(()))
            ),
            "cleanup success must coincide with the empty-inventory postcondition"
        );
    }

    #[tokio::test]
    async fn cleanup_timeout_retries_until_the_hub_is_available_again() {
        let (server, hub, child) = setup().await;
        assert_eq!(call_tool(&child, "navigate").await.status(), StatusCode::OK);
        server.set_hub(Weak::<BrowserSessionHub>::new());

        let error = server
            .stop_and_wait_for(Duration::from_millis(10))
            .await
            .unwrap_err();
        assert!(
            error.contains("cleanup exceeded"),
            "unexpected timeout error: {error}"
        );
        assert!(
            matches!(
                server.lifecycle.completion.borrow().cleanup,
                BrowserMcpShutdownStage::Pending
            ),
            "an unavailable Hub must not be published as cleanup success"
        );
        assert!(
            !server.state.bindings.lock().await.is_empty(),
            "the tombstoned binding must retain exact-owner retry authority"
        );

        server.set_hub(Arc::downgrade(&hub));
        tokio::time::timeout(Duration::from_secs(1), server.stop_and_wait())
            .await
            .expect("durable cleanup must resume when the Hub returns")
            .unwrap();
        assert!(server.state.bindings.lock().await.is_empty());
        assert!(hub.list_lanes().await.is_empty());
    }

    #[tokio::test]
    async fn shutdown_drains_current_and_replaced_exact_owner_leases() {
        let (server, hub, child) = setup().await;
        let claims = child.bootstrap.access.claims.clone();
        let old_owner = ensure_owner_binding(&server.state, &hub, &claims)
            .await
            .unwrap();
        let replacement = hub
            .issue_owner_lease(
                claims.user_id.to_string(),
                claims.session.conversation_id.clone(),
                claims.scope.runtime_instance_id.clone(),
            )
            .unwrap()
            .lease_id;
        let entry = server
            .state
            .bindings
            .lock()
            .await
            .get(&claims.lease_id)
            .cloned()
            .expect("owner binding must exist");
        {
            let _operation_guard = entry.operation.lock().await;
            let mut binding = entry.state.lock().await;
            binding.binding = Some(OwnerBinding {
                owner_lease_id: replacement.clone(),
                policy: owner_binding_policy_from_claims(&claims),
            });
            binding.pending_owner_cleanup.push(old_owner.clone());
        }

        server.stop_and_wait().await.unwrap();

        assert!(
            hub.renew_owner_lease(&old_owner).is_err(),
            "shutdown must revoke the superseded exact owner"
        );
        assert!(
            hub.renew_owner_lease(&replacement).is_err(),
            "shutdown must also revoke the replacement exact owner"
        );
        assert!(server.state.bindings.lock().await.is_empty());
    }

    #[tokio::test]
    async fn drop_signals_stop_and_detaches_durable_cleanup() {
        let (server, hub, child) = setup().await;
        assert_eq!(call_tool(&child, "navigate").await.status(), StatusCode::OK);
        let entry = server
            .state
            .bindings
            .lock()
            .await
            .get(&child.bootstrap.access.claims.lease_id)
            .cloned()
            .expect("tool call publishes an owner binding");
        let operation_guard = entry.operation.lock().await;

        drop(server);
        assert_eq!(hub.list_lanes().await.len(), 1);
        drop(operation_guard);

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if hub.list_lanes().await.is_empty() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("Drop must detach, not abort, the durable cleanup flight");
    }

    #[tokio::test]
    async fn detached_binding_entry_cannot_recreate_owner_after_capability_revocation() {
        let (server, hub, child) = setup().await;
        let claims = child.bootstrap.access.claims.clone();
        let capability_lease_id = claims.lease_id.clone();
        let state = BrowserMcpState {
            issuer: Arc::clone(&server.issuer),
            hub: Arc::clone(&server.hub),
            bindings: Arc::new(Mutex::new(HashMap::new())),
            request_admissions: TaskRequestAdmissions::default(),
            owner_lease_issues: Arc::new(AtomicUsize::new(0)),
            revoked_cleanup_active: Arc::new(AtomicUsize::new(0)),
            revoked_cleanup_peak: Arc::new(AtomicUsize::new(0)),
            request_started: Arc::new(tokio::sync::Notify::new()),
        };
        let entry = Arc::new(OwnerBindingEntry::default());
        state
            .bindings
            .lock()
            .await
            .insert(capability_lease_id.clone(), Arc::clone(&entry));

        // Pin ensure_owner_binding after it has captured the map entry but
        // before it can issue a Hub lease. This is the old detached-entry
        // interleaving: cleanup removes the entry, then ensure resumes with
        // its stale Arc.
        let entry_guard = entry.state.lock().await;
        let captured = entry.ensure_captured.notified();
        let ensure_state = state.clone();
        let ensure_hub = Arc::clone(&hub);
        let ensure = tokio::spawn(async move {
            ensure_owner_binding(&ensure_state, &ensure_hub, &claims).await
        });
        captured.await;

        LoopbackCapabilityLease::new(
            Arc::clone(&server.issuer),
            BROWSER_CAPABILITY_DOMAIN,
            capability_lease_id.clone(),
        )
        .revoke();
        let cleanup_state = state.clone();
        let cleanup_lease_id = capability_lease_id.clone();
        let cleanup =
            tokio::spawn(async move { cleanup_binding(&cleanup_state, &cleanup_lease_id).await });
        tokio::task::yield_now().await;
        drop(entry_guard);

        cleanup.await.unwrap();
        let error = ensure.await.unwrap().unwrap_err();
        assert_eq!(error.code, BrowserErrorCode::OwnerLeaseExpired);
        assert!(state.bindings.lock().await.is_empty());
        assert!(
            entry.state.lock().await.revoked,
            "cleanup must tombstone every detached entry"
        );
        assert!(hub.list_lanes().await.is_empty());
        assert_eq!(
            hub.revoke_owner_lease(&OwnerLeaseId::new())
                .await
                .unwrap()
                .closed,
            0
        );
    }

    #[tokio::test]
    async fn slow_cleanup_does_not_block_sibling_capability() {
        let (server, hub, first) = setup().await;
        let sibling = server
            .issuer_config("nomicore".into())
            .issue_for_conversation(USER_ID, CONVERSATION_ID, Some("agent-sibling"))
            .unwrap();
        let first_claims = first.bootstrap.access.claims.clone();
        let sibling_claims = sibling.bootstrap.access.claims.clone();
        let state = BrowserMcpState {
            issuer: Arc::clone(&server.issuer),
            hub: Arc::clone(&server.hub),
            bindings: Arc::new(Mutex::new(HashMap::new())),
            request_admissions: TaskRequestAdmissions::default(),
            owner_lease_issues: Arc::new(AtomicUsize::new(0)),
            revoked_cleanup_active: Arc::new(AtomicUsize::new(0)),
            revoked_cleanup_peak: Arc::new(AtomicUsize::new(0)),
            request_started: Arc::new(tokio::sync::Notify::new()),
        };

        let first_owner = ensure_owner_binding(&state, &hub, &first_claims)
            .await
            .unwrap();
        let sibling_owner = ensure_owner_binding(&state, &hub, &sibling_claims)
            .await
            .unwrap();
        let first_entry = state
            .bindings
            .lock()
            .await
            .get(&first_claims.lease_id)
            .cloned()
            .unwrap();

        // Hold only capability A's lifecycle lock. cleanup_binding(A) must
        // release the global map lock before it waits here, otherwise ensuring
        // the unrelated sibling capability will time out.
        let first_operation_guard = first_entry.operation.lock().await;
        let cleanup_captured = first_entry.cleanup_captured.notified();
        LoopbackCapabilityLease::new(
            Arc::clone(&server.issuer),
            BROWSER_CAPABILITY_DOMAIN,
            first_claims.lease_id.clone(),
        )
        .revoke();
        let cleanup_state = state.clone();
        let first_capability_lease_id = first_claims.lease_id.clone();
        let cleanup = tokio::spawn(async move {
            cleanup_binding(&cleanup_state, &first_capability_lease_id).await;
        });
        cleanup_captured.await;

        let ensured_sibling = tokio::time::timeout(
            Duration::from_secs(1),
            ensure_owner_binding(&state, &hub, &sibling_claims),
        )
        .await
        .expect("sibling ensure waited behind another capability's cleanup")
        .unwrap();
        assert_eq!(ensured_sibling, sibling_owner);
        assert!(hub.renew_owner_lease(&sibling_owner).is_ok());

        drop(first_operation_guard);
        cleanup.await.unwrap();
        let bindings = state.bindings.lock().await;
        assert!(!bindings.contains_key(&first_claims.lease_id));
        assert!(bindings.contains_key(&sibling_claims.lease_id));
        drop(bindings);
        assert!(hub.renew_owner_lease(&first_owner).is_err());
        assert!(hub.renew_owner_lease(&sibling_owner).is_ok());
    }

    #[tokio::test]
    async fn explicit_child_revoke_closes_only_its_owner_lanes_immediately() {
        let (server, hub, child) = setup().await;
        let sibling = server
            .issuer_config("nomicore".into())
            .issue_for_conversation(USER_ID, CONVERSATION_ID, Some("agent-sibling"))
            .unwrap();
        assert_eq!(call_tool(&child, "navigate").await.status(), StatusCode::OK);
        assert_eq!(
            call_tool(&sibling, "navigate").await.status(),
            StatusCode::OK
        );
        assert_eq!(hub.list_lanes().await.len(), 2);

        let response = reqwest::Client::new()
            .post(format!(
                "http://127.0.0.1:{}{}",
                child.bootstrap.port, LOOPBACK_CAPABILITY_REVOKE_PATH
            ))
            .json(&child.bootstrap.renewal)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        let lanes = hub.list_lanes().await;
        assert_eq!(lanes.len(), 1);
        assert_eq!(
            lanes[0].caller.runtime_instance_id,
            sibling.bootstrap.access.claims.scope.runtime_instance_id
        );
        assert_eq!(
            call_tool(&child, "navigate").await.status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            call_tool(&sibling, "navigate").await.status(),
            StatusCode::OK
        );
    }
}
