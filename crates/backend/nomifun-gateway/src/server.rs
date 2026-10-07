//! In-process HTTP half of the Platform Gateway MCP.
//!
//! The nomi engine spawns a SEPARATE stdio process
//! (`nomicore mcp-gateway-stdio`) that cannot share this process's services;
//! it forwards each tool call back here as an authenticated `POST /tool`.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::Json;
use axum::body::{Body, Bytes, to_bytes};
use axum::extract::{DefaultBodyLimit, Extension, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use nomifun_api_types::{
    GATEWAY_CALL_TOOL_OPERATION, GATEWAY_CAPABILITY_DOMAIN,
    GatewayCapabilityClaims, GatewayCapabilityScope, GatewayMcpConfig,
};
use nomifun_common::{
    LOOPBACK_CAPABILITY_RENEW_PATH, LOOPBACK_CAPABILITY_REVOKE_PATH,
    LoopbackCapabilityError, LoopbackCapabilityIssuer,
    LoopbackCapabilityRenewalRequest, LoopbackSessionKind,
};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::{OwnedSemaphorePermit, RwLock, Semaphore};
use tokio::time;
use tower_http::limit::RequestBodyLimitLayer;
use tracing::{debug, info, warn};

use crate::deps::{CallerCtx, CompatibilityCapabilityHost};
use crate::registry::Registry;

const GATEWAY_STOP_WAIT: std::time::Duration =
    std::time::Duration::from_millis(750);
/// Parsed requests are charged to the trusted domain/user/session family
/// shared by every sibling runtime lease. This is a per-task structural
/// ceiling, not a process-wide memory cap.
const GATEWAY_REQUESTS_PER_TASK_FAMILY: usize = 8;
/// Before JSON claims exist, admission must be machine-wide. Capacity scales
/// with both CPU and physical RAM and rejects without queueing; the following
/// hard maximum is only a structural task-count fuse.
const GATEWAY_BODY_READS_PER_LOGICAL_CPU: usize = 8;
const GATEWAY_BODY_READS_UNKNOWN_MEMORY_FALLBACK: usize = 16;
const GATEWAY_BODY_READS_MAX: usize = 512;
/// At most one sixteenth of reported physical RAM can be represented by
/// simultaneously full request bodies. Actual aggregate capacity remains
/// elastic with the host instead of being fixed at (for example) 1 GiB.
const GATEWAY_BODY_READ_MEMORY_DIVISOR: u64 = 16;
#[cfg(not(test))]
const GATEWAY_REQUEST_BODY_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(5);
#[cfg(test)]
const GATEWAY_REQUEST_BODY_TIMEOUT: std::time::Duration =
    std::time::Duration::from_millis(100);

/// Late-bound handle to the gateway dependencies. Unlike the guide /
/// requirement servers (which hold a `Weak` to a singleton that outlives
/// them elsewhere), this slot OWNS the deps bundle: `CompatibilityCapabilityHost` is
/// assembled specifically for this server during router construction and has
/// no other owner. Nothing inside the bundle references the server back, so
/// there is no Arc cycle.
type DepsSlot = Arc<RwLock<Option<Arc<CompatibilityCapabilityHost>>>>;

type StopCompletion = tokio::sync::watch::Receiver<Option<Result<(), String>>>;

type IngressCompletion =
    tokio::sync::watch::Receiver<Option<Result<(), String>>>;

struct GatewayLifecycle {
    http_shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    http_handle: Option<tokio::task::JoinHandle<()>>,
    ingress_completion: Option<IngressCompletion>,
    stop_completion: Option<StopCompletion>,
}

#[cfg(test)]
struct IngressTestGate {
    entered: tokio::sync::Semaphore,
    release: tokio::sync::Semaphore,
}

#[cfg(test)]
impl IngressTestGate {
    fn new() -> Self {
        Self {
            entered: tokio::sync::Semaphore::new(0),
            release: tokio::sync::Semaphore::new(0),
        }
    }
}

#[cfg(test)]
type IngressTestGateSlot = Arc<RwLock<Option<Arc<IngressTestGate>>>>;

#[derive(Clone)]
struct GatewayBodyReadAdmission {
    slots: Arc<Semaphore>,
    #[cfg(test)]
    capacity: usize,
}

impl GatewayBodyReadAdmission {
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
            .saturating_mul(GATEWAY_BODY_READS_PER_LOGICAL_CPU)
            .min(GATEWAY_BODY_READS_MAX);
        let memory_capacity = if total_memory_bytes == 0 {
            GATEWAY_BODY_READS_UNKNOWN_MEMORY_FALLBACK.min(cpu_capacity)
        } else {
            let bytes_per_slot = (nomifun_common::constants::BODY_LIMIT as u64)
                .saturating_mul(GATEWAY_BODY_READ_MEMORY_DIVISOR);
            usize::try_from(total_memory_bytes / bytes_per_slot)
                .unwrap_or(usize::MAX)
                .max(1)
                .min(GATEWAY_BODY_READS_MAX)
        };
        let capacity = cpu_capacity.min(memory_capacity).max(1);
        Self {
            slots: Arc::new(Semaphore::new(capacity)),
            #[cfg(test)]
            capacity,
        }
    }

    fn try_acquire(&self) -> Result<OwnedSemaphorePermit, ()> {
        Arc::clone(&self.slots)
            .try_acquire_owned()
            .map_err(|_| ())
    }
}

#[derive(Clone)]
struct VerifiedGatewayRequest {
    lease_id: Arc<str>,
}

#[derive(serde::Deserialize)]
struct GatewayAuthorizationEnvelope {
    session: Option<GatewayCapabilityClaims>,
}

#[derive(Clone)]
struct GatewayState {
    issuer: Arc<LoopbackCapabilityIssuer>,
    deps: DepsSlot,
    body_read_admission: GatewayBodyReadAdmission,
    #[cfg(test)]
    ingress_test_gate: IngressTestGateSlot,
}

/// In-process HTTP MCP server for the Platform Gateway tools.
pub struct GatewayMcpServer {
    http_addr: SocketAddr,
    issuer: Arc<LoopbackCapabilityIssuer>,
    shutdown_runtime: tokio::runtime::Handle,
    lifecycle: Mutex<GatewayLifecycle>,
    deps_slot: DepsSlot,
    #[cfg(test)]
    ingress_test_gate_slot: IngressTestGateSlot,
}

impl GatewayMcpServer {
    /// Bind a fresh `127.0.0.1:0` listener, mint a root issuer secret, and
    /// start serving `POST /tool`. Deps must be wired separately via
    /// [`set_deps`](Self::set_deps) before the first tool call arrives.
    pub async fn start() -> Result<Self, String> {
        let shutdown_runtime = tokio::runtime::Handle::try_current()
            .map_err(|error| format!("Gateway MCP requires a Tokio runtime: {error}"))?;
        let issuer = Arc::new(LoopbackCapabilityIssuer::random()?);
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| format!("Failed to bind gateway MCP HTTP listener: {e}"))?;
        let http_addr = listener
            .local_addr()
            .map_err(|e| format!("Failed to read gateway MCP local addr: {e}"))?;

        let deps_slot: DepsSlot = Arc::new(RwLock::new(None));
        #[cfg(test)]
        let ingress_test_gate_slot: IngressTestGateSlot =
            Arc::new(RwLock::new(None));

        let state = GatewayState {
            issuer: issuer.clone(),
            deps: deps_slot.clone(),
            body_read_admission: GatewayBodyReadAdmission::for_machine(),
            #[cfg(test)]
            ingress_test_gate: ingress_test_gate_slot.clone(),
        };

        let tool_router = axum::Router::new()
            .route("/tool", axum::routing::post(handle_tool_request))
            // The signed lease lives inside JSON, so only a machine-adaptive
            // body-read fuse can run before parsing. It never queues and hands
            // off to the exact per-lease permit before dispatch.
            .layer(middleware::from_fn_with_state(
                state.clone(),
                enforce_tool_request_bounds,
            ))
            .layer(RequestBodyLimitLayer::new(
                nomifun_common::constants::BODY_LIMIT,
            ))
            .layer(DefaultBodyLimit::disable());
        let app = axum::Router::new()
            .merge(tool_router)
            .route(
                LOOPBACK_CAPABILITY_RENEW_PATH,
                axum::routing::post(handle_capability_renew),
            )
            .route(
                LOOPBACK_CAPABILITY_REVOKE_PATH,
                axum::routing::post(handle_capability_revoke),
            );
        #[cfg(test)]
        let app = app.route(
            "/__test__/ingress-gate",
            axum::routing::get(handle_ingress_test_gate),
        );
        let app = app.with_state(state.clone());

        let (http_shutdown, http_shutdown_rx) =
            tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            let result = axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = http_shutdown_rx.await;
                })
                .await;

            if let Err(e) = result {
                warn!(error = %e, "Gateway MCP axum server exited with error");
            }
        });

        debug!(
            http_port = http_addr.port(),
            "Gateway MCP Server started (axum)"
        );

        Ok(Self {
            http_addr,
            issuer,
            shutdown_runtime,
            lifecycle: Mutex::new(GatewayLifecycle {
                http_shutdown: Some(http_shutdown),
                http_handle: Some(handle),
                ingress_completion: None,
                stop_completion: None,
            }),
            deps_slot,
            #[cfg(test)]
            ingress_test_gate_slot,
        })
    }

    /// Wire the dependency bundle after router construction. Must be called
    /// once before the first tool request arrives.
    pub async fn set_deps(&self, deps: Arc<CompatibilityCapabilityHost>) {
        *self.deps_slot.write().await = Some(deps);
    }

    pub fn http_port(&self) -> u16 {
        self.http_addr.port()
    }

    /// Build the process-private issuer consumed by Agent assemblers. The root
    /// secret remains private and the returned type cannot be serialized.
    pub fn issuer_config(
        &self,
        binary_path: String,
        authoritative_user_id: impl Into<Arc<str>>,
    ) -> GatewayMcpConfig {
        GatewayMcpConfig::from_issuer(
            self.http_addr.port(),
            self.issuer.clone(),
            binary_path,
            authoritative_user_id,
        )
    }

    fn begin_stop(&self) -> (IngressCompletion, StopCompletion) {
        let (
            ingress_completion,
            completion,
            ingress_completion_tx,
            completion_tx,
            http_handle,
        ) = {
            let mut lifecycle = self
                .lifecycle
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let (Some(ingress_completion), Some(completion)) = (
                lifecycle.ingress_completion.as_ref(),
                lifecycle.stop_completion.as_ref(),
            ) {
                return (ingress_completion.clone(), completion.clone());
            }

            if let Some(shutdown) = lifecycle.http_shutdown.take() {
                let _ = shutdown.send(());
                debug!(
                    http_port = self.http_addr.port(),
                    "Gateway MCP graceful stop requested"
                );
            }

            let http_handle = lifecycle.http_handle.take();
            let (ingress_completion_tx, ingress_completion) =
                tokio::sync::watch::channel(None);
            let (completion_tx, completion) =
                tokio::sync::watch::channel(None);
            lifecycle.ingress_completion = Some(ingress_completion.clone());
            lifecycle.stop_completion = Some(completion.clone());
            (
                ingress_completion,
                completion,
                ingress_completion_tx,
                completion_tx,
                http_handle,
            )
        };

        // `Drop` is allowed to run on arbitrary application/OS threads. Always
        // schedule the durable authority back onto the exact runtime that
        // started the listener instead of consulting the dropping thread's
        // ambient runtime. Dropping this supervisor's JoinHandle detaches only
        // the waiter; the runtime still owns the ingress barrier and final
        // exact-owner drain until they publish a terminal result.
        self.shutdown_runtime.spawn(async move {
            let result = finish_stop(
                http_handle,
                ingress_completion_tx,
            )
            .await;
            let _ = completion_tx.send(Some(result));
        });

        (ingress_completion, completion)
    }

    /// Stop HTTP ingress and wait until it is fully quiesced.
    ///
    /// The timeout applies only after Axum has stopped accepting and every
    /// accepted request has completed. Therefore any return from this method is
    /// an authoritative ingress barrier.
    pub async fn stop_and_wait(&self) -> Result<(), String> {
        self.stop_and_wait_for(GATEWAY_STOP_WAIT).await
    }

    /// Wait for the already-started durable shutdown authority to reach its
    /// exact-owner postcondition.
    ///
    /// Unlike [`Self::stop_and_wait`], this method does not impose the short
    /// AppServices-facing cleanup wait. It is useful for diagnostics/tests and
    /// for an owner that explicitly wants to remain attached to the durable
    /// cleanup flight after a previous timeout.
    pub async fn wait_for_shutdown(&self) -> Result<(), String> {
        let (ingress_completion, completion) = self.begin_stop();
        wait_for_stage_completion(
            ingress_completion,
            "Gateway MCP ingress shutdown coordinator ended before reporting completion",
        )
        .await?;
        wait_for_stop_completion(completion).await
    }

    async fn stop_and_wait_for(
        &self,
        wait: std::time::Duration,
    ) -> Result<(), String> {
        let (ingress_completion, completion) = self.begin_stop();
        wait_for_stage_completion(
            ingress_completion,
            "Gateway MCP ingress shutdown coordinator ended before reporting completion",
        )
        .await?;
        match time::timeout(wait, wait_for_stop_completion(completion)).await {
            Ok(result) => result,
            Err(_) => Err(format!(
                "Gateway MCP shutdown exceeded the {} ms wait after HTTP ingress quiesced",
                wait.as_millis()
            )),
        }
    }

    /// Consuming async shutdown convenience for owners that no longer need the
    /// server handle.
    pub async fn shutdown(self) -> Result<(), String> {
        self.stop_and_wait().await
    }

    pub fn stop(&self) {
        // The detached completion task retains both JoinHandles and cleanup
        // authority. Drop remains non-blocking, while later shared-reference
        // callers can still observe the same idempotent shutdown result.
        let _ = self.begin_stop();
    }
}

impl Drop for GatewayMcpServer {
    fn drop(&mut self) {
        self.stop();
    }
}

async fn finish_stop(
    http_handle: Option<tokio::task::JoinHandle<()>>,
    ingress_completion_tx: tokio::sync::watch::Sender<Option<Result<(), String>>>,
) -> Result<(), String> {
    let ingress_result = if let Some(handle) = http_handle {
        match handle.await {
            Ok(()) => Ok(()),
            Err(error) => Err(format!(
                "Gateway MCP HTTP ingress task failed while stopping: {error}"
            )),
        }
    } else {
        Ok(())
    };
    let _ = ingress_completion_tx.send(Some(ingress_result.clone()));
    ingress_result
}

async fn wait_for_stage_completion(
    mut completion: IngressCompletion,
    closed_message: &'static str,
) -> Result<(), String> {
    loop {
        if let Some(result) = completion.borrow().clone() {
            return result;
        }
        if completion.changed().await.is_err() {
            return completion.borrow().clone().unwrap_or_else(|| {
                Err(closed_message.to_owned())
            });
        }
    }
}

async fn wait_for_stop_completion(
    mut completion: StopCompletion,
) -> Result<(), String> {
    loop {
        if let Some(result) = completion.borrow().clone() {
            return result;
        }
        if completion.changed().await.is_err() {
            return completion.borrow().clone().unwrap_or_else(|| {
                Err(
                    "Gateway MCP shutdown coordinator ended before reporting completion"
                        .to_owned(),
                )
            });
        }
    }
}

#[cfg(test)]
async fn handle_ingress_test_gate(
    State(state): State<GatewayState>,
) -> StatusCode {
    let gate = state.ingress_test_gate.read().await.clone();
    if let Some(gate) = gate {
        gate.entered.add_permits(1);
        if let Ok(permit) = gate.release.acquire().await {
            permit.forget();
        }
    }
    StatusCode::NO_CONTENT
}

// ---------------------------------------------------------------------------
// Axum handler
// ---------------------------------------------------------------------------

async fn read_tool_body_with_deadline(
    body: Body,
    deadline: std::time::Duration,
) -> Result<Bytes, Response> {
    match time::timeout(
        deadline,
        to_bytes(body, nomifun_common::constants::BODY_LIMIT),
    )
    .await
    {
        Ok(Ok(bytes)) => Ok(bytes),
        Ok(Err(_)) => Err(gateway_ingress_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "request_body_too_large",
            "The Gateway request body exceeds the configured byte limit.",
            false,
        )),
        Err(_) => Err(gateway_ingress_error(
            StatusCode::REQUEST_TIMEOUT,
            "request_timeout",
            "The Gateway request body was not received before the absolute deadline.",
            true,
        )),
    }
}

fn gateway_ingress_error(
    status: StatusCode,
    code: &str,
    message: &str,
    retryable: bool,
) -> Response {
    let mut response = (
        status,
        Json(json!({
            "error": code,
            "message": message,
            "retryable": retryable,
        })),
    )
        .into_response();
    response
        .headers_mut()
        .insert(header::CONNECTION, HeaderValue::from_static("close"));
    if matches!(
        status,
        StatusCode::TOO_MANY_REQUESTS | StatusCode::SERVICE_UNAVAILABLE
    ) {
        response
            .headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
    }
    response
}

async fn enforce_tool_request_bounds(
    State(state): State<GatewayState>,
    request: Request,
    next: Next,
) -> Response {
    let body_read_permit = match state.body_read_admission.try_acquire() {
        Ok(permit) => permit,
        Err(()) => {
            return gateway_ingress_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "gateway_ingress_saturated",
                "Gateway request-body admission is temporarily saturated.",
                true,
            );
        }
    };

    let (parts, body) = request.into_parts();
    let bytes = match read_tool_body_with_deadline(
        body,
        GATEWAY_REQUEST_BODY_TIMEOUT,
    )
    .await
    {
        Ok(bytes) => bytes,
        Err(response) => return response,
    };
    let envelope = match serde_json::from_slice::<GatewayAuthorizationEnvelope>(&bytes) {
        Ok(envelope) => envelope,
        Err(_) => {
            return gateway_ingress_error(
                StatusCode::BAD_REQUEST,
                "invalid_json",
                "Gateway request body must be valid JSON.",
                false,
            );
        }
    };
    let Some(claims) = envelope.session else {
        return gateway_ingress_error(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Gateway request has no valid bound session authorization.",
            false,
        );
    };
    if claims.scope.validate().is_err()
        || claims.session.kind != LoopbackSessionKind::Conversation
    {
        return gateway_ingress_error(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Gateway request has no valid bound session authorization.",
            false,
        );
    }
    let provided_token = parts
        .headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or("");
    let request_permit = match state.issuer.verify_access_and_acquire(
        GATEWAY_CAPABILITY_DOMAIN,
        &claims,
        provided_token,
        GATEWAY_REQUESTS_PER_TASK_FAMILY,
    ) {
        Ok(permit) => permit,
        Err(LoopbackCapabilityError::CapacityExceeded) => {
            return gateway_ingress_error(
                StatusCode::TOO_MANY_REQUESTS,
                "lease_request_limit",
                "This Gateway task already has the maximum number of in-flight requests.",
                true,
            );
        }
        Err(_) => {
            warn!("Gateway MCP: invalid or unbound session authorization");
            return gateway_ingress_error(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "Gateway request has no valid bound session authorization.",
                false,
            );
        }
    };

    let lease_id: Arc<str> = Arc::from(claims.lease_id.as_str());
    debug_assert_eq!(request_permit.lease_id(), lease_id.as_ref());
    let mut request = Request::from_parts(parts, Body::from(bytes));
    request
        .extensions_mut()
        .insert(VerifiedGatewayRequest { lease_id });
    // From here the exact active lease owns the retained request. Releasing
    // the pre-identity permit avoids turning the adaptive body-read fuse into
    // an aggregate dispatch/concurrency ceiling.
    drop(body_read_permit);
    let response = next.run(request).await;
    drop(request_permit);
    response
}

async fn handle_tool_request(
    State(state): State<GatewayState>,
    Extension(request_authority): Extension<VerifiedGatewayRequest>,
    headers: axum::http::HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let claims = match body
        .get("session")
        .cloned()
        .and_then(|value| serde_json::from_value::<GatewayCapabilityClaims>(value).ok())
    {
        Some(claims)
            if claims.scope.validate().is_ok()
                && claims.session.kind == LoopbackSessionKind::Conversation
                && claims.lease_id == request_authority.lease_id.as_ref() => claims,
        _ => {
            warn!("Gateway MCP: invalid or unbound session authorization");
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "unauthorized"})),
            )
                .into_response();
        }
    };

    if !claims.allows(GATEWAY_CALL_TOOL_OPERATION) {
        warn!("Gateway MCP: tools/call is outside signed capability scope");
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error": "forbidden"})),
        )
            .into_response();
    }

    let operation_id = match nomifun_common::required_idempotency_key(&headers) {
        Ok(value) => value.to_owned(),
        Err(error) => {
            warn!(error, "Gateway MCP: invalid idempotency identity");
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": "invalid_idempotency_key",
                    "message": error,
                })),
            )
                .into_response();
        }
    };

    let tool = body
        .get("tool")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let args = body.get("args").cloned().unwrap_or(Value::Null);
    let Ok(conversation_id) = nomifun_common::ConversationId::parse(&claims.session.session_id) else {
        return (StatusCode::UNAUTHORIZED, Json(json!({"error": "invalid session identity"}))).into_response();
    };
    let user_id = claims.user_id.clone();
    let companion_id = claims.scope.companion_id.clone();
    let ctx = CallerCtx {
        conversation_id: Some(conversation_id),
        user_id,
        companion_id,
        channel_platform: claims.scope.channel_platform.clone(),
        operation_id: Some(operation_id),
        // This in-process server is the INWARD path (bundled agents on loopback);
        // never the external Remote surface.
        ..Default::default()
    };

    let deps = match state.deps.read().await.clone() {
        Some(d) => d,
        None => {
            warn!(tool, "Gateway MCP: deps not available");
            return finish(json!({"error": "service_unavailable"}));
        }
    };

    let is_instance_owner = claims.user_id.as_str() == deps.authoritative_user_id.as_ref();
    if claims.scope.instance_owner != is_instance_owner {
        warn!(user_id = %claims.user_id, "Gateway MCP: signed owner classification disagrees with runtime authority");
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "unauthorized"})),
        )
            .into_response();
    }

    let registry = Registry::global();
    if !registry.contains(&tool) {
        return finish(json!({ "error": format!("Unknown tool: {tool}") }));
    }
    let domains = GatewayMcpConfig::domains_for_profile(&claims.scope.profile);
    if claims.scope.excludes(&tool)
        || !registry.tool_visible_for_caller(ctx.surface(), domains, is_instance_owner, &tool)
    {
        return finish(json!({
            "error": "session_capability_denied",
            "tool": tool,
            "profile": claims.scope.profile,
        }));
    }

    info!(tool, caller = ?ctx.conversation_id, "Gateway MCP: dispatching tool");

    // The capability registry owns every tool and its typed schema. The
    // authenticated session scope is checked above; selected tools execute
    // directly, and unknown names return a structured error.
    let response_body = match registry
        .dispatch_opt(deps.clone(), ctx.clone(), &tool, &args)
        .await
    {
        Some(v) => v,
        None => {
            warn!(tool, "Gateway MCP: unknown tool");
            json!({ "error": format!("Unknown tool: {tool}") })
        }
    };

    finish(response_body)
}

async fn handle_capability_renew(
    State(state): State<GatewayState>,
    Json(request): Json<LoopbackCapabilityRenewalRequest>,
) -> impl IntoResponse {
    match state
        .issuer
        .renew::<GatewayCapabilityScope>(GATEWAY_CAPABILITY_DOMAIN, &request)
    {
        Ok(access)
            if access.claims.scope.validate().is_ok()
                && access.claims.session.kind == LoopbackSessionKind::Conversation =>
        {
            (StatusCode::OK, Json(json!(access))).into_response()
        }
        Ok(_) | Err(_) => {
            warn!("Gateway MCP: invalid capability renewal");
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "unauthorized"})),
            )
                .into_response()
        }
    }
}

async fn handle_capability_revoke(
    State(state): State<GatewayState>,
    Json(request): Json<LoopbackCapabilityRenewalRequest>,
) -> impl IntoResponse {
    match state
        .issuer
        .revoke(GATEWAY_CAPABILITY_DOMAIN, &request)
    {
        Ok(()) => {
            (StatusCode::NO_CONTENT, Json(Value::Null)).into_response()
        }
        Err(_) => (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "unauthorized"})),
        )
            .into_response(),
    }
}

/// Wrap a JSON body as a response and ask the client to close the connection
/// (the stdio bridge runs with `pool_max_idle_per_host(0)` and does not reuse).
fn finish(body: Value) -> axum::response::Response {
    let mut resp = Json(body).into_response();
    resp.headers_mut()
        .insert(header::CONNECTION, HeaderValue::from_static("close"));
    resp
}

// ---------------------------------------------------------------------------
// Shared helpers for the capability handlers
// ---------------------------------------------------------------------------

/// Wrap a serializable payload as a successful tool result.
pub(crate) fn ok<T: serde::Serialize>(payload: T) -> Value {
    match serde_json::to_value(payload) {
        Ok(v) => json!({"result": v}),
        Err(e) => json!({"error": format!("failed to serialize result: {e}")}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_OWNER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000001";
    const TEST_CONVERSATION_ID: &str = "0190f5fe-7c00-7a00-8000-000000000001";
    const OTHER_CONVERSATION_ID: &str = "0190f5fe-7c00-7a00-8000-000000000002";

    struct BoundedToolTestGate {
        entered: Semaphore,
        release: Semaphore,
    }

    impl BoundedToolTestGate {
        fn new() -> Self {
            Self {
                entered: Semaphore::new(0),
                release: Semaphore::new(0),
            }
        }
    }

    async fn bounded_tool_test_handler(
        Extension(gate): Extension<Arc<BoundedToolTestGate>>,
    ) -> StatusCode {
        gate.entered.add_permits(1);
        let permit = gate
            .release
            .acquire()
            .await
            .expect("bounded tool test gate must remain open");
        permit.forget();
        StatusCode::OK
    }

    fn bounded_tool_test_state(
        issuer: Arc<LoopbackCapabilityIssuer>,
        body_read_admission: GatewayBodyReadAdmission,
    ) -> GatewayState {
        GatewayState {
            issuer,
            deps: Arc::new(RwLock::new(None)),
            body_read_admission,
            ingress_test_gate: Arc::new(RwLock::new(None)),
        }
    }

    fn bounded_tool_test_app(
        state: GatewayState,
        gate: Arc<BoundedToolTestGate>,
    ) -> axum::Router {
        axum::Router::new()
            .route("/tool", axum::routing::post(bounded_tool_test_handler))
            .layer(Extension(gate))
            .layer(middleware::from_fn_with_state(
                state,
                enforce_tool_request_bounds,
            ))
            .layer(RequestBodyLimitLayer::new(
                nomifun_common::constants::BODY_LIMIT,
            ))
            .layer(DefaultBodyLimit::disable())
    }

    fn bounded_tool_request(
        child: &nomifun_api_types::GatewayMcpChildConfig,
    ) -> Request {
        let body = serde_json::to_vec(&json!({
            "session": child.bootstrap.access.claims,
            "tool": "test",
            "args": {},
        }))
        .unwrap();
        Request::builder()
            .method("POST")
            .uri("/tool")
            .header(header::CONTENT_TYPE, "application/json")
            .header(
                header::AUTHORIZATION,
                format!("Bearer {}", child.bootstrap.access.token),
            )
            .body(Body::from(body))
            .unwrap()
    }

    #[test]
    fn body_read_admission_scales_with_cpu_and_physical_memory() {
        const GIB: u64 = 1024 * 1024 * 1024;

        let constrained = GatewayBodyReadAdmission::for_resources(8 * GIB, 16);
        let workstation = GatewayBodyReadAdmission::for_resources(64 * GIB, 16);
        let more_cpu = GatewayBodyReadAdmission::for_resources(64 * GIB, 64);
        let unknown_memory = GatewayBodyReadAdmission::for_resources(0, 16);

        assert!(constrained.capacity > 1);
        assert!(workstation.capacity > constrained.capacity);
        assert!(more_cpu.capacity > workstation.capacity);
        assert_eq!(
            unknown_memory.capacity,
            GATEWAY_BODY_READS_UNKNOWN_MEMORY_FALLBACK
        );
        assert!(more_cpu.capacity <= GATEWAY_BODY_READS_MAX);
    }

    #[test]
    fn body_read_admission_is_exact_and_raii_refunds_capacity() {
        let admission = GatewayBodyReadAdmission::for_resources(u64::MAX, 1);
        let mut permits = Vec::new();
        for _ in 0..admission.capacity {
            permits.push(
                admission
                    .try_acquire()
                    .expect("every slot up to capacity must be admitted"),
            );
        }
        assert!(
            admission.try_acquire().is_err(),
            "capacity plus one must reject without queueing"
        );
        drop(permits.pop());
        permits.push(
            admission
                .try_acquire()
                .expect("dropping a body-read permit must restore capacity"),
        );
        drop(permits);
        assert_eq!(
            admission.slots.available_permits(),
            admission.capacity,
            "all normal/error/cancellation paths rely on the same RAII refund"
        );
    }

    #[tokio::test]
    async fn stalled_body_hits_absolute_deadline_and_oversize_is_rejected() {
        use std::convert::Infallible;

        use futures::{StreamExt, stream};

        let stalled = stream::once(async {
            Ok::<_, Infallible>(Bytes::from_static(b"{"))
        })
        .chain(stream::pending::<Result<Bytes, Infallible>>());
        let response = time::timeout(
            std::time::Duration::from_secs(1),
            read_tool_body_with_deadline(
                Body::from_stream(stalled),
                std::time::Duration::from_millis(20),
            ),
        )
        .await
        .expect("absolute body deadline must wake")
        .expect_err("a body that never terminates must time out");
        assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);

        let oversized = vec![0_u8; nomifun_common::constants::BODY_LIMIT + 1];
        let response = read_tool_body_with_deadline(
            Body::from(oversized),
            std::time::Duration::from_secs(1),
        )
        .await
        .expect_err("body limit plus one must be rejected");
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[test]
    fn gateway_uses_eight_in_flight_requests_per_task_family() {
        assert_eq!(GATEWAY_REQUESTS_PER_TASK_FAMILY, 8);
    }

    #[tokio::test]
    async fn ninth_request_on_one_task_family_returns_429_and_completion_refunds() {
        use tower::ServiceExt;

        let issuer = Arc::new(LoopbackCapabilityIssuer::random().unwrap());
        let config = GatewayMcpConfig::from_issuer(
            1,
            Arc::clone(&issuer),
            "/bin/nomicore".into(),
            TEST_OWNER_ID,
        );
        let child = config
            .issue_for_conversation(
                TEST_OWNER_ID,
                TEST_CONVERSATION_ID,
                None,
                None,
                &[],
            )
            .unwrap();
        let admission = GatewayBodyReadAdmission::for_resources(u64::MAX, 64);
        let gate = Arc::new(BoundedToolTestGate::new());
        let app = bounded_tool_test_app(
            bounded_tool_test_state(issuer, admission),
            Arc::clone(&gate),
        );
        let mut calls = tokio::task::JoinSet::new();
        for _ in 0..GATEWAY_REQUESTS_PER_TASK_FAMILY {
            let app = app.clone();
            let request = bounded_tool_request(&child);
            calls.spawn(async move { app.oneshot(request).await.unwrap() });
        }
        let entered = time::timeout(
            std::time::Duration::from_secs(1),
            gate.entered
                .acquire_many(GATEWAY_REQUESTS_PER_TASK_FAMILY as u32),
        )
        .await
        .expect("the exact admitted set must enter the handler")
        .expect("test gate must remain open");
        entered.forget();

        let overloaded = time::timeout(
            std::time::Duration::from_secs(1),
            app.clone().oneshot(bounded_tool_request(&child)),
        )
        .await
        .expect("the over-limit request must reject without queueing")
        .unwrap();
        assert_eq!(overloaded.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            overloaded.headers().get(header::RETRY_AFTER),
            Some(&HeaderValue::from_static("1"))
        );

        gate.release
            .add_permits(GATEWAY_REQUESTS_PER_TASK_FAMILY);
        while let Some(response) = calls.join_next().await {
            assert_eq!(response.unwrap().status(), StatusCode::OK);
        }
        let recovered = tokio::spawn({
            let app = app.clone();
            let request = bounded_tool_request(&child);
            async move { app.oneshot(request).await.unwrap() }
        });
        let entered = time::timeout(
            std::time::Duration::from_secs(1),
            gate.entered.acquire(),
        )
        .await
        .expect("completion must refund one lease slot")
        .expect("test gate must remain open");
        entered.forget();
        gate.release.add_permits(1);
        assert_eq!(recovered.await.unwrap().status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn saturated_pre_identity_body_reader_returns_503_and_timeout_refunds() {
        use std::convert::Infallible;

        use futures::stream;
        use tower::ServiceExt;

        let issuer = Arc::new(LoopbackCapabilityIssuer::random().unwrap());
        let config = GatewayMcpConfig::from_issuer(
            1,
            Arc::clone(&issuer),
            "/bin/nomicore".into(),
            TEST_OWNER_ID,
        );
        let child = config
            .issue_for_conversation(
                TEST_OWNER_ID,
                TEST_CONVERSATION_ID,
                None,
                None,
                &[],
            )
            .unwrap();
        let admission = GatewayBodyReadAdmission {
            slots: Arc::new(Semaphore::new(1)),
            capacity: 1,
        };
        let app = bounded_tool_test_app(
            bounded_tool_test_state(issuer, admission.clone()),
            Arc::new(BoundedToolTestGate::new()),
        );
        let stalled_body = stream::pending::<Result<Bytes, Infallible>>();
        let stalled_request = Request::builder()
            .method("POST")
            .uri("/tool")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from_stream(stalled_body))
            .unwrap();
        let stalled = tokio::spawn({
            let app = app.clone();
            async move { app.oneshot(stalled_request).await.unwrap() }
        });
        time::timeout(std::time::Duration::from_secs(1), async {
            while admission.slots.available_permits() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("stalled request must acquire the sole body-read slot");

        let saturated = app
            .clone()
            .oneshot(bounded_tool_request(&child))
            .await
            .unwrap();
        assert_eq!(saturated.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(stalled.await.unwrap().status(), StatusCode::REQUEST_TIMEOUT);
        assert_eq!(admission.slots.available_permits(), 1);
    }

    fn child(
        server: &GatewayMcpServer,
        user_id: &str,
        conversation_id: &str,
    ) -> nomifun_api_types::GatewayMcpChildConfig {
        server
            .issuer_config("/bin/nomicore".into(), TEST_OWNER_ID)
            .issue_for_conversation(user_id, conversation_id, None, None, &[])
            .unwrap()
    }

    async fn post_tool(port: u16, token: Option<&str>, body: Value) -> (u16, Value) {
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let mut req = client
            .post(format!("http://127.0.0.1:{port}/tool"))
            .header("Idempotency-Key", "gateway-test-operation-v1")
            .json(&body);
        if let Some(t) = token {
            req = req.header("Authorization", format!("Bearer {t}"));
        }
        let resp = req.send().await.unwrap();
        let status = resp.status().as_u16();
        let json: Value = resp.json().await.unwrap_or(Value::Null);
        (status, json)
    }

    async fn post_capability(
        port: u16,
        path: &str,
        request: &LoopbackCapabilityRenewalRequest,
    ) -> (u16, Value) {
        let response = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .post(format!("http://127.0.0.1:{port}{path}"))
            .json(request)
            .send()
            .await
            .unwrap();
        let status = response.status().as_u16();
        let body = response.json().await.unwrap_or(Value::Null);
        (status, body)
    }

    #[tokio::test]
    async fn start_returns_positive_port_and_redacted_issuer() {
        let server = GatewayMcpServer::start().await.unwrap();
        assert!(server.http_port() > 0);
        let debug = format!(
            "{:?}",
            server.issuer_config("/bin/nomicore".into(), TEST_OWNER_ID)
        );
        assert!(debug.contains("[REDACTED]"));
    }

    #[tokio::test]
    async fn stop_and_wait_stops_http_ingress() {
        let server = GatewayMcpServer::start().await.unwrap();
        let addr = server.http_addr;

        server.stop_and_wait().await.unwrap();

        assert!(TcpListener::bind(addr).await.is_ok());
        assert!(server.stop_and_wait().await.is_ok());
    }

    #[tokio::test]
    async fn graceful_stop_waits_for_in_flight_request_and_rejects_new_connections()
    {
        let server = GatewayMcpServer::start().await.unwrap();
        let addr = server.http_addr;
        let gate = Arc::new(IngressTestGate::new());
        *server.ingress_test_gate_slot.write().await = Some(gate.clone());

        let client = reqwest::Client::builder()
            .no_proxy()
            .pool_max_idle_per_host(0)
            .build()
            .unwrap();
        let in_flight = {
            let client = client.clone();
            tokio::spawn(async move {
                client
                    .get(format!(
                        "http://{addr}/__test__/ingress-gate"
                    ))
                    .send()
                    .await
            })
        };
        let entered = time::timeout(
            std::time::Duration::from_secs(1),
            gate.entered.acquire(),
        )
        .await
        .expect("request must enter the handler")
        .expect("test gate must remain open");
        entered.forget();

        let (_, stop) = server.begin_stop();
        time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if TcpListener::bind(addr).await.is_ok() {
                    break;
                }
                time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("graceful shutdown must close the listener promptly");

        assert!(
            time::timeout(
                std::time::Duration::from_millis(25),
                wait_for_stop_completion(stop.clone()),
            )
            .await
            .is_err(),
            "stop completion must wait for an accepted request"
        );

        let error = reqwest::Client::builder()
            .no_proxy()
            .pool_max_idle_per_host(0)
            .build()
            .unwrap()
            .get(format!("http://{addr}/__test__/ingress-gate"))
            .send()
            .await
            .expect_err("a fresh connection after stop must be rejected");
        assert!(
            error.is_connect(),
            "new request should fail at connection time: {error}"
        );

        gate.release.add_permits(1);
        let response = time::timeout(
            std::time::Duration::from_secs(1),
            in_flight,
        )
        .await
        .expect("in-flight request must finish after release")
        .expect("request task must not panic")
        .expect("accepted request must complete successfully");
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        time::timeout(
            std::time::Duration::from_secs(1),
            wait_for_stop_completion(stop),
        )
        .await
        .expect("stop must complete after the in-flight request")
            .expect("graceful stop must succeed");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn drop_from_non_tokio_thread_preserves_in_flight_ingress_barrier() {
        let server = GatewayMcpServer::start().await.unwrap();
        let addr = server.http_addr;
        let gate = Arc::new(IngressTestGate::new());
        *server.ingress_test_gate_slot.write().await = Some(gate.clone());

        let in_flight = tokio::spawn(async move {
            reqwest::Client::builder()
                .no_proxy()
                .pool_max_idle_per_host(0)
                .build()
                .unwrap()
                .get(format!("http://{addr}/__test__/ingress-gate"))
                .send()
                .await
        });
        let entered = time::timeout(
            std::time::Duration::from_secs(1),
            gate.entered.acquire(),
        )
        .await
        .expect("request must enter the handler")
        .expect("test gate must remain open");
        entered.forget();

        std::thread::spawn(move || drop(server))
            .join()
            .expect("non-Tokio drop thread must not panic");

        time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if TcpListener::bind(addr).await.is_ok() {
                    break;
                }
                time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("non-Tokio Drop must still close the listener");
        assert!(
            !in_flight.is_finished(),
            "dropping the server may not discard the accepted request's ingress barrier"
        );

        gate.release.add_permits(1);
        let response = time::timeout(std::time::Duration::from_secs(1), in_flight)
            .await
            .expect("accepted request must finish after release")
            .expect("request task must not panic")
            .expect("accepted request must complete successfully");
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if Arc::strong_count(&gate) == 1 {
                    break;
                }
                time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("durable shutdown supervisor must finish after ingress drains");
    }

    #[tokio::test]
    async fn concurrent_stop_and_wait_calls_share_one_shutdown() {
        let server = GatewayMcpServer::start().await.unwrap();
        let addr = server.http_addr;

        let (first, second, third) = tokio::join!(
            server.stop_and_wait(),
            server.stop_and_wait(),
            server.stop_and_wait()
        );

        assert!(first.is_ok());
        assert_eq!(second, first);
        assert_eq!(third, first);
        assert!(TcpListener::bind(addr).await.is_ok());
        assert!(server.stop_and_wait().await.is_ok());
    }

    #[tokio::test]
    async fn each_start_uses_a_fresh_issuer_secret() {
        let a = GatewayMcpServer::start().await.unwrap();
        let b = GatewayMcpServer::start().await.unwrap();
        let child_a = child(&a, TEST_OWNER_ID, TEST_CONVERSATION_ID);
        let child_b = child(&b, TEST_OWNER_ID, TEST_CONVERSATION_ID);
        assert_ne!(
            child_a.bootstrap.renewal.renewal_proof,
            child_b.bootstrap.renewal.renewal_proof
        );
        assert!(a
            .issuer
            .renew::<GatewayCapabilityScope>(
                GATEWAY_CAPABILITY_DOMAIN,
                &child_b.bootstrap.renewal,
            )
            .is_err());
    }

                #[tokio::test]
    async fn tool_call_requires_auth() {
        let server = GatewayMcpServer::start().await.unwrap();
        let (status, _) = post_tool(
            server.http_port(),
            None,
            json!({"tool": "nomi_list_conversations", "args": {}}),
        )
        .await;
        assert_eq!(status, 401);
    }

    #[tokio::test]
    async fn renewal_restores_server_scope_and_revoke_ends_the_lease() {
        let server = GatewayMcpServer::start().await.unwrap();
        let child = child(&server, TEST_OWNER_ID, TEST_CONVERSATION_ID);
        let original = &child.bootstrap.access.claims;

        let (status, body) = post_capability(
            server.http_port(),
            LOOPBACK_CAPABILITY_RENEW_PATH,
            &child.bootstrap.renewal,
        )
        .await;
        assert_eq!(status, 200);
        let renewed: nomifun_common::LoopbackCapabilityAccess<GatewayCapabilityClaims> =
            serde_json::from_value(body).unwrap();
        assert_eq!(renewed.claims.version, original.version);
        assert_eq!(renewed.claims.lease_id, original.lease_id);
        assert_eq!(renewed.claims.user_id, original.user_id);
        assert_eq!(renewed.claims.session, original.session);
        assert_eq!(renewed.claims.allowed_tools, original.allowed_tools);
        assert_eq!(renewed.claims.scope, original.scope);
        assert_ne!(renewed.claims.nonce, original.nonce);

        let (status, _) = post_capability(
            server.http_port(),
            LOOPBACK_CAPABILITY_REVOKE_PATH,
            &child.bootstrap.renewal,
        )
        .await;
        assert_eq!(status, 204);
        let (status, _) = post_capability(
            server.http_port(),
            LOOPBACK_CAPABILITY_RENEW_PATH,
            &child.bootstrap.renewal,
        )
        .await;
        assert_eq!(status, 401);
    }

    #[tokio::test]
    async fn missing_deps_returns_unavailable() {
        // Server started but set_deps never called.
        let server = GatewayMcpServer::start().await.unwrap();
        let child = child(&server, TEST_OWNER_ID, TEST_CONVERSATION_ID);
        let access = &child.bootstrap.access;
        let (status, body) = post_tool(
            server.http_port(),
            Some(&access.token),
            json!({"tool": "nomi_list_conversations", "args": {}, "session": access.claims}),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(
            body.get("error").and_then(Value::as_str),
            Some("service_unavailable")
        );
    }

    #[tokio::test]
    async fn tampered_cross_conversation_and_expired_claims_are_unauthorized() {
        let server = GatewayMcpServer::start().await.unwrap();
        let child = child(&server, TEST_OWNER_ID, TEST_CONVERSATION_ID);
        let access = &child.bootstrap.access;

        let mut forged = access.claims.clone();
        forged.session = nomifun_common::LoopbackSessionBinding::conversation(OTHER_CONVERSATION_ID);
        let (status, _) = post_tool(
            server.http_port(),
            Some(&access.token),
            json!({"tool": "nomi_list_conversations", "args": {}, "session": forged}),
        )
        .await;
        assert_eq!(status, 401);

        let now = nomifun_common::unix_time_secs();
        let expired = server
            .issuer
            .renew_at::<GatewayCapabilityScope>(
                GATEWAY_CAPABILITY_DOMAIN,
                &child.bootstrap.renewal,
                now.saturating_sub(nomifun_common::LOOPBACK_CAPABILITY_TTL_SECS + 1),
            )
            .unwrap();
        let (status, _) = post_tool(
            server.http_port(),
            Some(&expired.token),
            json!({"tool": "nomi_list_conversations", "args": {}, "session": expired.claims}),
        )
        .await;
        assert_eq!(status, 401);
    }

    #[tokio::test]
    async fn correctly_signed_terminal_binding_is_unauthorized() {
        let server = GatewayMcpServer::start().await.unwrap();
        let child = child(&server, TEST_OWNER_ID, TEST_CONVERSATION_ID);
        let claims = GatewayCapabilityClaims::issue(
            TEST_OWNER_ID,
            nomifun_common::LoopbackSessionBinding::terminal(
                "0190f5fe-7c00-7a00-8000-000000000001",
            ),
            [
                nomifun_api_types::GATEWAY_LIST_TOOLS_OPERATION,
                GATEWAY_CALL_TOOL_OPERATION,
            ],
            child.bootstrap.access.claims.scope.clone(),
        )
        .unwrap();
        let (token, _) = server
            .issuer
            .activate(GATEWAY_CAPABILITY_DOMAIN, &claims)
            .unwrap();
        let (status, _) = post_tool(
            server.http_port(),
            Some(&token),
            json!({"tool": "nomi_list_conversations", "args": {}, "session": claims}),
        )
        .await;
        assert_eq!(status, 401);
    }

    #[tokio::test]
    async fn tools_call_requires_signed_operation_scope() {
        let server = GatewayMcpServer::start().await.unwrap();
        let child = child(&server, TEST_OWNER_ID, TEST_CONVERSATION_ID);
        let mut claims = child.bootstrap.access.claims.clone();
        claims.allowed_tools = vec![nomifun_api_types::GATEWAY_LIST_TOOLS_OPERATION.into()];
        let (token, _) = server
            .issuer
            .activate(GATEWAY_CAPABILITY_DOMAIN, &claims)
            .unwrap();
        let (status, body) = post_tool(
            server.http_port(),
            Some(&token),
            json!({"tool": "nomi_list_conversations", "args": {}, "session": claims}),
        )
        .await;
        assert_eq!(status, 403);
        assert_eq!(body["error"], "forbidden");
    }
}
