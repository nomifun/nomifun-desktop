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
use std::net::SocketAddr;
use std::sync::{Arc, Mutex as StdMutex, RwLock, Weak};
use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use nomi_browser::{ManagedBrowserFacade, managed_result_envelope};
use nomifun_api_types::{
    BROWSER_CAPABILITY_DOMAIN, BROWSER_MCP_TOOL_NAMES, BrowserCapabilityClaims,
    BrowserCapabilityOperation, BrowserCapabilityScope, BrowserCapabilitySurface,
    BrowserMcpConfig, browser_tool_operation,
};
use nomifun_browser_platform::{
    BrowserErrorCode, BrowserOperationKind, BrowserPlatformError, BrowserSessionHub,
    BrowserSurface, CallerIdentity, OwnerLeaseId,
};
use nomifun_common::{
    LOOPBACK_CAPABILITY_RENEW_PATH, LOOPBACK_CAPABILITY_REVOKE_PATH,
    LoopbackCapabilityError, LoopbackCapabilityIssuer, LoopbackCapabilityRenewalRequest,
    LoopbackSessionKind,
};
use serde_json::{Map, Value, json};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tracing::{debug, warn};

const REVOKED_LEASE_SWEEP_INTERVAL: Duration = Duration::from_millis(500);
const CLEANUP_RETRY_WAIT: Duration = Duration::from_millis(50);
const MODEL_IDENTITY_INPUT_FIELDS: &[&str] = &[
    "identity",
    "identity_mode",
    "authenticated",
    "auth_identity",
    "profile",
    "account",
];
type HubSlot = Arc<RwLock<Weak<BrowserSessionHub>>>;

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
}

impl BrowserMcpLifecycle {
    fn begin_stop(&self) {
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
        // The supervisor below then joins the server, which waits for every
        // already accepted request before the final binding drain begins.
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
                Some(task) => {
                    browser_mcp_task_result(task.await, "HTTP ingress")
                }
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
                Some(task) => task.await.err().map(|error| {
                    format!(
                        "Browser MCP periodic cleanup task failed while stopping: {error}"
                    )
                }),
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
    /// Owner leases superseded after a Hub renewal/close failure remain here
    /// until their exact lease cleanup succeeds. The current binding and these
    /// entries are deliberately separate so a replacement cannot make an old
    /// owner unreachable.
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
    #[cfg(test)]
    request_started: Arc<tokio::sync::Notify>,
}

/// Process-local HTTP half of the ACP browser bridge.
pub(crate) struct BrowserMcpServer {
    http_addr: SocketAddr,
    issuer: Arc<LoopbackCapabilityIssuer>,
    hub: HubSlot,
    lifecycle: BrowserMcpLifecycle,
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
            #[cfg(test)]
            request_started: Arc::new(tokio::sync::Notify::new()),
        };

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
            .with_state(state.clone());

        let (serve_shutdown, serve_shutdown_rx) =
            tokio::sync::oneshot::channel();
        let serve_task = tokio::spawn(async move {
            axum::serve(listener, app)
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
            },
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
        self.lifecycle.begin_stop();
    }

    /// Stop ingress and wait for the authoritative exact-owner cleanup barrier.
    ///
    /// This is the AppServices shutdown barrier: it does not return while an
    /// accepted HTTP request or a retained owner binding can still race Hub
    /// shutdown. Transient cleanup failures are retried by the durable worker
    /// until the binding inventory reaches its empty postcondition.
    pub(crate) async fn stop_and_wait(&self) -> Result<(), String> {
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
    Json(body): Json<Value>,
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
    Json(request): Json<LoopbackCapabilityRenewalRequest>,
) -> Response {
    match state
        .issuer
        .renew::<BrowserCapabilityScope>(BROWSER_CAPABILITY_DOMAIN, &request)
    {
        Ok(access) if validate_browser_claims(&access.claims).is_ok() => {
            Json(access).into_response()
        }
        _ => unauthorized(),
    }
}

async fn handle_capability_revoke(
    State(state): State<BrowserMcpState>,
    Json(request): Json<LoopbackCapabilityRenewalRequest>,
) -> Response {
    match state
        .issuer
        .revoke(BROWSER_CAPABILITY_DOMAIN, &request)
    {
        Ok(()) => {
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
        let mut pending_owner_cleanup = {
            let state_guard = entry.state.lock().await;
            state_guard.pending_owner_cleanup.clone()
        };
        if let Err(error) = hub.revoke_owner_lease(&existing.owner_lease_id).await {
            if !pending_owner_cleanup
                .iter()
                .any(|lease_id| lease_id == &existing.owner_lease_id)
            {
                pending_owner_cleanup.push(existing.owner_lease_id.clone());
            }
            let mut state_guard = entry.state.lock().await;
            state_guard.pending_owner_cleanup = pending_owner_cleanup.clone();
            warn!(
                code = ?error.code,
                "Browser MCP could not close an expired owner before replacement; retaining exact owner for retry"
            );
        }
        // The replacement below publishes the new current owner while the old
        // lease remains in `pending_owner_cleanup` when its close failed.
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
        .find(|field| object.contains_key(**field))
    else {
        return Ok(());
    };
    Err(BrowserPlatformError::new(
        BrowserErrorCode::InvalidCallerIdentity,
        format!(
            "Browser identity field `{field}` is selected by trusted host policy."
        ),
        false,
        "Remove identity-selection fields from Browser tool arguments.",
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
    let mut cleanups = tokio::task::JoinSet::new();
    for lease_id in revoked {
        let state = state.clone();
        cleanups.spawn(async move {
            cleanup_binding(&state, &lease_id).await;
        });
    }
    while let Some(result) = cleanups.join_next().await {
        if let Err(error) = result {
            warn!(%error, "Browser MCP revoked-binding cleanup task failed");
        }
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
        if let Err(error) = retry_pending_owner_cleanup_for_entry(&entry, &hub).await {
            warn!(
                code = ?error.code,
                "Browser MCP pending owner cleanup retry failed"
            );
        }
    }
}

async fn drain_all_bindings(state: &BrowserMcpState) {
    loop {
        let capability_lease_ids: Vec<String> = {
            let bindings = state.bindings.lock().await;
            bindings.keys().cloned().collect()
        };
        if capability_lease_ids.is_empty() {
            return;
        }

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
        retry_pending_owner_cleanups(state).await;
        cleanup_revoked_bindings(state).await;
        if state.bindings.lock().await.is_empty() {
            return;
        }
        tokio::time::sleep(CLEANUP_RETRY_WAIT).await;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use nomifun_common::LoopbackCapabilityLease;
    use nomifun_browser_platform::{
        BrowserHostDriver, BrowserHostFactory, BrowserHostId, BrowserLaneDriver,
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
    async fn pending_owner_cleanup_completes_before_replacement_without_deadlock() {
        let (server, hub, child) = setup().await;
        let claims = child.bootstrap.access.claims.clone();
        // Use a detached binding inventory so the server's periodic sweep
        // cannot race this exact replacement interleaving.
        let state = BrowserMcpState {
            issuer: Arc::clone(&server.issuer),
            hub: Arc::clone(&server.hub),
            bindings: Arc::new(Mutex::new(HashMap::new())),
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
    async fn graceful_shutdown_drains_in_flight_request_before_final_binding_cleanup() {
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
            "final binding drain must wait for the accepted request to finish"
        );
        assert!(
            !in_flight.is_finished(),
            "the accepted request must remain in flight while its transition is pinned"
        );

        let rejected = try_call_tool_with_args(
            &child,
            "navigate",
            json!({ "url": "https://example.test/rejected-after-stop" }),
        )
        .await;
        assert!(
            rejected.is_err(),
            "a fresh connection must not be accepted after ingress shutdown"
        );

        drop(operation_guard);
        let response = in_flight.await.unwrap().unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        server.stop_and_wait().await.unwrap();

        assert!(
            !server
                .issuer
                .is_lease_active(BROWSER_CAPABILITY_DOMAIN, &capability_lease_id),
            "the final drain must revoke the capability after HTTP has quiesced"
        );
        assert!(
            hub.list_lanes().await.is_empty(),
            "the owner published by the final in-flight request must be drained"
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
    async fn finite_cleanup_wait_still_waits_for_http_quiescence_before_timing_out() {
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

        let waiter = server.stop_and_wait_for(Duration::from_millis(1));
        tokio::pin!(waiter);
        assert!(
            tokio::time::timeout(Duration::from_millis(25), &mut waiter)
                .await
                .is_err(),
            "the cleanup wait must not let a caller timeout bypass HTTP quiescence"
        );
        assert!(
            !in_flight.is_finished(),
            "the accepted request must still be held by the per-capability gate"
        );

        drop(operation_guard);
        let in_flight = in_flight.await.unwrap();
        if let Ok(response) = in_flight {
            assert_eq!(response.status(), StatusCode::OK);
        }
        let error = waiter.await.unwrap_err();
        assert!(
            error.contains("cleanup exceeded"),
            "the finite wait may time out only after ingress quiesces: {error}"
        );
        assert!(
            !server.state.bindings.lock().await.is_empty(),
            "the timed-out caller must leave exact-owner cleanup authority intact"
        );
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
