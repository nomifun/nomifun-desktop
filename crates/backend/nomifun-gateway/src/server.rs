//! In-process HTTP half of the Platform Gateway MCP.
//!
//! ACP CLIs and the nomi engine spawn a SEPARATE stdio process
//! (`nomicore mcp-gateway-stdio`) that cannot share this process's services;
//! it forwards each tool call back here as an authenticated `POST /tool`.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::IntoResponse;
use nomifun_api_types::{
    GATEWAY_CALL_TOOL_OPERATION, GATEWAY_CAPABILITY_DOMAIN,
    GatewayCapabilityClaims, GatewayCapabilityScope, GatewayMcpConfig,
};
use nomifun_common::{
    LOOPBACK_CAPABILITY_RENEW_PATH, LOOPBACK_CAPABILITY_REVOKE_PATH,
    LoopbackCapabilityIssuer, LoopbackCapabilityRenewalRequest, LoopbackSessionKind,
};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tokio::time;
#[cfg(feature = "browser-use")]
use tokio::time::MissedTickBehavior;
use tracing::{debug, info, warn};

use crate::deps::{CallerCtx, GatewayDeps};
use crate::registry::Registry;

#[cfg(feature = "browser-use")]
const BROWSER_OWNER_SWEEP_INTERVAL: std::time::Duration =
    std::time::Duration::from_millis(500);
const BROWSER_REVOKE_WAIT: std::time::Duration =
    std::time::Duration::from_millis(750);

/// Late-bound handle to the gateway dependencies. Unlike the guide /
/// requirement servers (which hold a `Weak` to a singleton that outlives
/// them elsewhere), this slot OWNS the deps bundle: `GatewayDeps` is
/// assembled specifically for this server during router construction and has
/// no other owner. Nothing inside the bundle references the server back, so
/// there is no Arc cycle.
type DepsSlot = Arc<RwLock<Option<Arc<GatewayDeps>>>>;
#[cfg(feature = "browser-use")]
type BrowserRegistrySlot =
    Arc<RwLock<Option<crate::browser_registry::BrowserRegistry>>>;

type StopCompletion =
    tokio::sync::watch::Receiver<Option<Result<(), String>>>;

type IngressCompletion =
    tokio::sync::watch::Receiver<Option<Result<(), String>>>;

struct GatewayCleanupWorker {
    shutdown: tokio::sync::watch::Sender<bool>,
    handle: tokio::task::JoinHandle<()>,
}

struct GatewayLifecycle {
    http_shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    http_handle: Option<tokio::task::JoinHandle<()>>,
    browser_cleanup_worker: Option<GatewayCleanupWorker>,
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
struct GatewayState {
    issuer: Arc<LoopbackCapabilityIssuer>,
    deps: DepsSlot,
    #[cfg(feature = "browser-use")]
    browser_registry: BrowserRegistrySlot,
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
    #[cfg(feature = "browser-use")]
    browser_registry_slot: BrowserRegistrySlot,
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
        #[cfg(feature = "browser-use")]
        let browser_registry_slot: BrowserRegistrySlot =
            Arc::new(RwLock::new(None));
        #[cfg(test)]
        let ingress_test_gate_slot: IngressTestGateSlot =
            Arc::new(RwLock::new(None));

        let state = GatewayState {
            issuer: issuer.clone(),
            deps: deps_slot.clone(),
            #[cfg(feature = "browser-use")]
            browser_registry: browser_registry_slot.clone(),
            #[cfg(test)]
            ingress_test_gate: ingress_test_gate_slot.clone(),
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
            );
        #[cfg(test)]
        let app = app.route(
            "/__test__/ingress-gate",
            axum::routing::get(handle_ingress_test_gate),
        );
        let app = app.with_state(state.clone());

        #[cfg(feature = "browser-use")]
        let browser_cleanup_worker = {
            let cleanup_state = state;
            let (shutdown_tx, mut shutdown_rx) =
                tokio::sync::watch::channel(false);
            let handle = tokio::spawn(async move {
                let mut interval = time::interval(BROWSER_OWNER_SWEEP_INTERVAL);
                interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
                loop {
                    tokio::select! {
                        _ = interval.tick() => {
                            sweep_gateway_browser_owners(&cleanup_state).await;
                        }
                        changed = shutdown_rx.changed() => {
                            if changed.is_err() || *shutdown_rx.borrow() {
                                drain_gateway_browser_owners_until_clean(&cleanup_state).await;
                                return;
                            }
                        }
                    }
                }
            });
            Some(GatewayCleanupWorker {
                shutdown: shutdown_tx,
                handle,
            })
        };
        #[cfg(not(feature = "browser-use"))]
        let browser_cleanup_worker = None;

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
                browser_cleanup_worker,
                ingress_completion: None,
                stop_completion: None,
            }),
            deps_slot,
            #[cfg(feature = "browser-use")]
            browser_registry_slot,
            #[cfg(test)]
            ingress_test_gate_slot,
        })
    }

    /// Wire the dependency bundle after router construction. Must be called
    /// once before the first tool request arrives.
    pub async fn set_deps(&self, deps: Arc<GatewayDeps>) {
        #[cfg(feature = "browser-use")]
        {
            *self.browser_registry_slot.write().await =
                deps.browser_registry.clone();
        }
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
            browser_cleanup_worker,
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
            let browser_cleanup_worker =
                lifecycle.browser_cleanup_worker.take();
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
                browser_cleanup_worker,
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
                browser_cleanup_worker,
                ingress_completion_tx,
            )
            .await;
            let _ = completion_tx.send(Some(result));
        });

        (ingress_completion, completion)
    }

    /// Stop HTTP ingress, wait until it is fully quiesced, and then wait briefly
    /// for the durable browser owner drain.
    ///
    /// The timeout applies only after Axum has stopped accepting and every
    /// accepted request has completed. Therefore any return from this method is
    /// an authoritative ingress barrier. A cleanup timeout detaches only the
    /// caller; the shared durable cleanup flight retains exact-owner authority.
    pub async fn stop_and_wait(&self) -> Result<(), String> {
        self.stop_and_wait_for(BROWSER_REVOKE_WAIT).await
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
                "Gateway MCP browser owner cleanup exceeded the {} ms shutdown wait after HTTP ingress quiesced; durable cleanup continues",
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
    browser_cleanup_worker: Option<GatewayCleanupWorker>,
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
    ingress_result?;

    if let Some(worker) = browser_cleanup_worker {
        let _ = worker.shutdown.send(true);
        match worker.handle.await {
            Ok(()) => Ok(()),
            Err(error) => Err(format!(
                "Gateway MCP browser cleanup task failed while stopping: {error}"
            )),
        }
    } else {
        Ok(())
    }
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

#[cfg(feature = "browser-use")]
async fn gateway_browser_registry(
    state: &GatewayState,
) -> Option<crate::browser_registry::BrowserRegistry> {
    state.browser_registry.read().await.clone()
}

#[cfg(feature = "browser-use")]
async fn sweep_gateway_browser_owners(state: &GatewayState) {
    let Some(registry) = gateway_browser_registry(state).await else {
        return;
    };
    let issuer = Arc::clone(&state.issuer);
    registry
        .cleanup_inactive_signed_child_leases(|lease_id| {
            issuer.is_lease_active(GATEWAY_CAPABILITY_DOMAIN, lease_id)
        })
        .await;
    registry.retry_pending_browser_cleanups().await;
}

#[cfg(feature = "browser-use")]
async fn drain_gateway_browser_owners_until_clean(state: &GatewayState) {
    let Some(registry) = gateway_browser_registry(state).await else {
        return;
    };

    // Ingress is already quiesced, so the set of SignedChild runtimes can only
    // shrink. Keep the exact-owner authority alive until its postcondition is
    // true; callers may time out without cancelling this task.
    loop {
        match registry.drain_signed_child_browser_owners_once().await {
            Ok(()) => return,
            Err(error) => {
                let status = registry.signed_child_cleanup_status();
                warn!(
                    code = ?error.code,
                    retryable = error.retryable,
                    pending_attachments = status.pending_attachments,
                    pending_owner_leases = status.pending_owner_leases,
                    revocation_pending_attachments =
                        status.revocation_pending_attachments,
                    "Gateway MCP browser owner cleanup remains pending; retrying"
                );
                time::sleep(BROWSER_OWNER_SWEEP_INTERVAL).await;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Axum handler
// ---------------------------------------------------------------------------

async fn handle_tool_request(
    State(state): State<GatewayState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let provided_token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");

    let claims = match body
        .get("session")
        .cloned()
        .and_then(|value| serde_json::from_value::<GatewayCapabilityClaims>(value).ok())
    {
        Some(claims)
            if claims.scope.validate().is_ok()
                && claims.session.kind == LoopbackSessionKind::Conversation
                && state
                    .issuer
                    .verify_access(GATEWAY_CAPABILITY_DOMAIN, &claims, provided_token)
                    .is_ok() => claims,
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
        session_mode: claims.scope.session_mode.clone(),
        operation_id: Some(operation_id),
        // This in-process server is the INWARD path (bundled agents on loopback);
        // never the external Remote surface.
        ..Default::default()
    };
    #[cfg(feature = "browser-use")]
    let ctx = ctx;

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

    // Browser identity/owner attachment is deliberately the last pre-dispatch
    // step. The registry can renew or issue a Hub owner lease, so no rejected
    // request may reach it before owner classification, tool visibility, and
    // argument validation have all succeeded.
    #[cfg(feature = "browser-use")]
    let ctx = if tool.starts_with("nomi_browser_")
        && let Some(registry) = deps.browser_registry.clone()
    {
        match preflight_and_attach_browser_identity(
            registry,
            ctx,
            &tool,
            &args,
            claims.lease_id.clone(),
            claims.expires_at_unix_secs.saturating_mul(1_000),
        )
        .await
        {
            Ok(ctx) => ctx,
            Err(error) => return finish(error),
        }
    } else {
        ctx
    };

    info!(tool, caller = ?ctx.conversation_id, "Gateway MCP: dispatching tool");

    // The capability registry is the single authority: it owns every tool,
    // generates its schema, and enforces the danger-tier × surface permission
    // gate. An unknown name returns a structured error the agent can recover from.
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

#[cfg(feature = "browser-use")]
async fn preflight_and_attach_browser_identity(
    registry: crate::browser_registry::BrowserRegistry,
    ctx: CallerCtx,
    tool: &str,
    args: &Value,
    lease_id: String,
    capability_expires_at_ms: u64,
) -> Result<CallerCtx, Value> {
    match Registry::global().validate_arguments(tool, args) {
        Some(Ok(())) => {}
        Some(Err(error)) => return Err(error),
        None => return Err(json!({ "error": format!("Unknown tool: {tool}") })),
    }
    registry
        .validate_managed_request(&ctx, tool, args)
        .await
        .map_err(crate::browser_registry::platform_error_to_value)?;
    let ctx = attach_browser_identity(
        registry.clone(),
        ctx,
        lease_id,
        capability_expires_at_ms,
    )
    .await?;
    // The owner-scoped lane_id check needs the trusted identity, which only
    // exists after attachment. Re-validate so an unowned handle fails here
    // rather than surfacing later from the bound Hub dispatch.
    registry
        .validate_managed_request(&ctx, tool, args)
        .await
        .map_err(crate::browser_registry::platform_error_to_value)?;
    Ok(ctx)
}

#[cfg(feature = "browser-use")]
async fn attach_browser_identity(
    registry: crate::browser_registry::BrowserRegistry,
    mut ctx: CallerCtx,
    lease_id: String,
    capability_expires_at_ms: u64,
) -> Result<CallerCtx, Value> {
    registry
        .attach_trusted_identity(
            &mut ctx,
            &lease_id,
            None,
            capability_expires_at_ms,
        )
        .await
        .map_err(crate::browser_registry::platform_error_to_value)?;
    Ok(ctx)
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
            // The signed capability is already irreversibly revoked at this
            // point. Tear down its browser owner immediately rather than
            // waiting for the Hub lease sweep. Cleanup is deliberately never
            // attempted for an invalid renewal proof.
            #[cfg(feature = "browser-use")]
            {
                let registry = state
                    .deps
                    .read()
                    .await
                    .as_ref()
                    .and_then(|deps| deps.browser_registry.clone());
                if let Some(registry) = registry
                    && let Err(error) = registry
                        .revoke_signed_child_lease(&request.lease_id)
                        .await
                {
                    warn!(
                        lease_id = %request.lease_id,
                        code = ?error.code,
                        "Gateway MCP: browser owner cleanup after capability revoke failed"
                    );
                }
            }
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
    use nomifun_common::UserId;
    #[cfg(feature = "browser-use")]
    use nomifun_browser_platform::{
        BrowserHostDriver, BrowserHostFactory, BrowserHostId,
        BrowserLaneDriver, BrowserOperation, BrowserOperationResult,
        BrowserPlatformError, BrowserSessionHub, DriverOperationContext,
        HostLaunchRequest, HostLifecycleState, HubConfig, LaneLaunchRequest,
    };
    #[cfg(feature = "browser-use")]
    use std::sync::atomic::{AtomicUsize, Ordering};

    const TEST_OWNER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000001";
    const TEST_CONVERSATION_ID: &str = "0190f5fe-7c00-7a00-8000-000000000001";
    const OTHER_CONVERSATION_ID: &str = "0190f5fe-7c00-7a00-8000-000000000002";

    #[cfg(feature = "browser-use")]
    struct BrowserCloseProbe {
        lane_closes: AtomicUsize,
    }

    #[cfg(feature = "browser-use")]
    struct GatedBrowserCloseProbe {
        lane_closes: AtomicUsize,
        close_started: tokio::sync::Semaphore,
        close_release: tokio::sync::Semaphore,
    }

    #[cfg(feature = "browser-use")]
    struct GatedBrowserLane {
        probe: Arc<GatedBrowserCloseProbe>,
    }

    #[cfg(feature = "browser-use")]
    #[async_trait::async_trait]
    impl BrowserLaneDriver for GatedBrowserLane {
        async fn execute(
            &self,
            _operation: BrowserOperation,
            _context: DriverOperationContext,
        ) -> Result<BrowserOperationResult, BrowserPlatformError> {
            Ok(BrowserOperationResult::default())
        }

        async fn close(&self) -> Result<(), BrowserPlatformError> {
            self.probe.close_started.add_permits(1);
            let permit = self
                .probe
                .close_release
                .acquire()
                .await
                .expect("test close gate must remain open");
            permit.forget();
            self.probe.lane_closes.fetch_add(1, Ordering::AcqRel);
            Ok(())
        }
    }

    #[cfg(feature = "browser-use")]
    struct GatedBrowserHost {
        host_id: BrowserHostId,
        probe: Arc<GatedBrowserCloseProbe>,
    }

    #[cfg(feature = "browser-use")]
    #[async_trait::async_trait]
    impl BrowserHostDriver for GatedBrowserHost {
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
            Ok(Arc::new(GatedBrowserLane {
                probe: Arc::clone(&self.probe),
            }))
        }

        async fn shutdown(&self) -> Result<(), BrowserPlatformError> {
            Ok(())
        }
    }

    #[cfg(feature = "browser-use")]
    struct GatedBrowserFactory {
        probe: Arc<GatedBrowserCloseProbe>,
    }

    #[cfg(feature = "browser-use")]
    #[async_trait::async_trait]
    impl BrowserHostFactory for GatedBrowserFactory {
        async fn launch(
            &self,
            request: HostLaunchRequest,
        ) -> Result<Arc<dyn BrowserHostDriver>, BrowserPlatformError> {
            Ok(Arc::new(GatedBrowserHost {
                host_id: request.host_id,
                probe: Arc::clone(&self.probe),
            }))
        }
    }

    #[cfg(feature = "browser-use")]
    struct TestBrowserLane {
        probe: Arc<BrowserCloseProbe>,
    }

    #[cfg(feature = "browser-use")]
    #[async_trait::async_trait]
    impl BrowserLaneDriver for TestBrowserLane {
        async fn execute(
            &self,
            _operation: BrowserOperation,
            _context: DriverOperationContext,
        ) -> Result<BrowserOperationResult, BrowserPlatformError> {
            Ok(BrowserOperationResult::default())
        }

        async fn close(&self) -> Result<(), BrowserPlatformError> {
            self.probe.lane_closes.fetch_add(1, Ordering::AcqRel);
            Ok(())
        }
    }

    #[cfg(feature = "browser-use")]
    struct TestBrowserHost {
        host_id: BrowserHostId,
        probe: Arc<BrowserCloseProbe>,
    }

    #[cfg(feature = "browser-use")]
    #[async_trait::async_trait]
    impl BrowserHostDriver for TestBrowserHost {
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
            Ok(Arc::new(TestBrowserLane {
                probe: Arc::clone(&self.probe),
            }))
        }

        async fn shutdown(&self) -> Result<(), BrowserPlatformError> {
            Ok(())
        }
    }

    #[cfg(feature = "browser-use")]
    struct TestBrowserFactory {
        probe: Arc<BrowserCloseProbe>,
    }

    #[cfg(feature = "browser-use")]
    #[async_trait::async_trait]
    impl BrowserHostFactory for TestBrowserFactory {
        async fn launch(
            &self,
            request: HostLaunchRequest,
        ) -> Result<Arc<dyn BrowserHostDriver>, BrowserPlatformError> {
            Ok(Arc::new(TestBrowserHost {
                host_id: request.host_id,
                probe: Arc::clone(&self.probe),
            }))
        }
    }

    fn child(
        server: &GatewayMcpServer,
        user_id: &str,
        conversation_id: &str,
    ) -> nomifun_api_types::GatewayMcpChildConfig {
        server
            .issuer_config("/bin/nomicore".into(), TEST_OWNER_ID)
            .issue_for_conversation(user_id, conversation_id, None, None, None, &[])
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

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn final_browser_drain_starts_only_after_in_flight_request_finishes() {
        let server = GatewayMcpServer::start().await.unwrap();
        let addr = server.http_addr;
        let ingress_gate = Arc::new(IngressTestGate::new());
        *server.ingress_test_gate_slot.write().await =
            Some(ingress_gate.clone());

        let close_probe = Arc::new(GatedBrowserCloseProbe {
            lane_closes: AtomicUsize::new(0),
            close_started: tokio::sync::Semaphore::new(0),
            close_release: tokio::sync::Semaphore::new(0),
        });
        let hub = BrowserSessionHub::new(
            Arc::new(GatedBrowserFactory {
                probe: close_probe.clone(),
            }),
            HubConfig::default(),
        );
        let registry =
            crate::browser_registry::BrowserRegistry::from_hub(hub.clone());
        *server.browser_registry_slot.write().await = Some(registry.clone());

        let ctx = CallerCtx {
            conversation_id: Some(
                nomifun_common::ConversationId::parse(TEST_CONVERSATION_ID)
                    .unwrap(),
            ),
            user_id: UserId::parse(TEST_OWNER_ID).unwrap(),
            ..Default::default()
        };
        let signed_child =
            child(&server, TEST_OWNER_ID, TEST_CONVERSATION_ID);
        let runtime_lease_id =
            signed_child.bootstrap.access.claims.lease_id.clone();
        let ctx = attach_browser_identity(
            registry,
            ctx,
            runtime_lease_id,
            u64::MAX,
        )
        .await
        .unwrap();
        server
            .browser_registry_slot
            .read()
            .await
            .clone()
            .expect("registry must be installed")
            .open(&ctx, None)
            .await
            .unwrap();
        assert_eq!(hub.list_lanes().await.len(), 1);

        let client = reqwest::Client::builder()
            .no_proxy()
            .pool_max_idle_per_host(0)
            .build()
            .unwrap();
        let in_flight = tokio::spawn(async move {
            client
                .get(format!("http://{addr}/__test__/ingress-gate"))
                .send()
                .await
        });
        let entered = time::timeout(
            std::time::Duration::from_secs(1),
            ingress_gate.entered.acquire(),
        )
        .await
        .expect("request must enter the handler")
        .expect("test gate must remain open");
        entered.forget();

        let (_, stop) = server.begin_stop();
        assert!(
            time::timeout(
                std::time::Duration::from_millis(50),
                close_probe.close_started.acquire(),
            )
            .await
            .is_err(),
            "final cleanup must not take its snapshot while ingress is active"
        );
        assert_eq!(hub.list_lanes().await.len(), 1);

        ingress_gate.release.add_permits(1);
        let response = time::timeout(
            std::time::Duration::from_secs(1),
            in_flight,
        )
        .await
        .expect("in-flight request must finish after release")
        .expect("request task must not panic")
        .expect("accepted request must complete successfully");
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        let close_started = time::timeout(
            std::time::Duration::from_secs(1),
            close_probe.close_started.acquire(),
        )
        .await
        .expect("cleanup must start after ingress drains")
        .expect("close gate must remain open");
        close_started.forget();
        close_probe.close_release.add_permits(1);

        time::timeout(
            std::time::Duration::from_secs(2),
            wait_for_stop_completion(stop),
        )
        .await
        .expect("stop must complete after cleanup is released")
        .expect("ordered shutdown must succeed");
        assert!(hub.list_lanes().await.is_empty());
        assert_eq!(close_probe.lane_closes.load(Ordering::Acquire), 1);
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
    async fn stop_timeout_keeps_shared_flight_for_retry() {
        let server = GatewayMcpServer::start().await.unwrap();
        let (shutdown_tx, mut shutdown_rx) =
            tokio::sync::watch::channel(false);
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let delayed_cleanup = tokio::spawn(async move {
            while !*shutdown_rx.borrow() {
                if shutdown_rx.changed().await.is_err() {
                    return;
                }
            }
            let _ = release_rx.await;
        });
        {
            let mut lifecycle = server
                .lifecycle
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(existing) = lifecycle.browser_cleanup_worker.replace(
                GatewayCleanupWorker {
                    shutdown: shutdown_tx,
                    handle: delayed_cleanup,
                },
            ) {
                existing.handle.abort();
            }
        }

        let error = server
            .stop_and_wait_for(std::time::Duration::from_millis(1))
            .await
            .unwrap_err();
        assert!(error.contains("durable cleanup continues"));
        assert!(
            TcpListener::bind(server.http_addr).await.is_ok(),
            "a cleanup timeout may not return before ingress is quiesced"
        );

        release_tx.send(()).unwrap();
        time::timeout(
            std::time::Duration::from_secs(1),
            server.wait_for_shutdown(),
        )
            .await
            .expect("durable cleanup must finish after release")
            .expect("wait_for_shutdown must observe the original flight");
        assert!(
            server
                .stop_and_wait_for(std::time::Duration::from_millis(1))
                .await
                .is_ok(),
            "successful flight result must be cached"
        );
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

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn gateway_browser_identity_uses_lease_as_runtime_not_attempt() {
        let probe = Arc::new(BrowserCloseProbe {
            lane_closes: AtomicUsize::new(0),
        });
        let hub = BrowserSessionHub::new(
            Arc::new(TestBrowserFactory {
                probe: Arc::clone(&probe),
            }),
            HubConfig::default(),
        );
        let registry =
            crate::browser_registry::BrowserRegistry::from_hub(hub.clone());
        let runtime_lease_id = "signed-gateway-runtime-lease";
        let ctx = CallerCtx {
            conversation_id: Some(
                nomifun_common::ConversationId::parse(TEST_CONVERSATION_ID)
                    .unwrap(),
            ),
            user_id: UserId::parse(TEST_OWNER_ID).unwrap(),
            ..Default::default()
        };

        let ctx = attach_browser_identity(
            registry.clone(),
            ctx,
            runtime_lease_id.to_owned(),
            u64::MAX,
        )
        .await
        .unwrap();
        let identity = ctx
            .browser_identity
            .as_ref()
            .expect("gateway must attach a browser identity");
        assert_eq!(identity.runtime_instance_id, runtime_lease_id);
        assert_eq!(
            identity.attempt_id, None,
            "a capability lease is runtime authority, not execution-attempt metadata"
        );

        registry.open(&ctx, None).await.unwrap();
        assert_eq!(hub.list_lanes().await.len(), 1);
        let revoked = registry
            .revoke_signed_child_lease(runtime_lease_id)
            .await
            .unwrap();
        assert_eq!(revoked.closed, 1);
        assert!(!revoked.already_closed);
        assert!(hub.list_lanes().await.is_empty());
        assert_eq!(probe.lane_closes.load(Ordering::Acquire), 1);
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn invalid_browser_arguments_do_not_create_or_renew_a_browser_lease() {
        let close_probe = Arc::new(BrowserCloseProbe {
            lane_closes: AtomicUsize::new(0),
        });
        let hub = BrowserSessionHub::new(
            Arc::new(TestBrowserFactory {
                probe: Arc::clone(&close_probe),
            }),
            HubConfig::default(),
        );
        let browser_registry =
            crate::browser_registry::BrowserRegistry::from_hub(hub);
        let ctx = CallerCtx {
            conversation_id: Some(
                nomifun_common::ConversationId::parse(TEST_CONVERSATION_ID)
                    .unwrap(),
            ),
            user_id: UserId::parse(TEST_OWNER_ID).unwrap(),
            ..Default::default()
        };

        let invalid = preflight_and_attach_browser_identity(
            browser_registry.clone(),
            ctx.clone(),
            "nomi_browser_open",
            &json!({"lane_name": 7}),
            "signed-browser-lease".to_owned(),
            u64::MAX,
        )
        .await
        .unwrap_err();
        assert!(
            invalid
                .get("error")
                .and_then(Value::as_str)
                .is_some_and(|error| error.contains("invalid arguments")),
            "invalid args must be rejected by typed preflight: {invalid}"
        );
        assert_eq!(
            browser_registry.signed_child_cleanup_status(),
            crate::browser_registry::BrowserCleanupStatus::default(),
            "invalid args must not create a Browser identity or owner lease"
        );

        preflight_and_attach_browser_identity(
            browser_registry.clone(),
            ctx.clone(),
            "nomi_browser_open",
            &json!({"lane_name": "default"}),
            "signed-browser-lease".to_owned(),
            u64::MAX,
        )
        .await
        .expect("valid args must attach the Browser owner");
        assert_eq!(
            browser_registry.signed_child_cleanup_status().pending_attachments,
            1,
            "valid args must attach one signed-child Browser owner"
        );

        let invalid = preflight_and_attach_browser_identity(
            browser_registry.clone(),
            ctx,
            "nomi_browser_open",
            &json!({"lane_name": 7}),
            "signed-browser-lease".to_owned(),
            u64::MAX,
        )
        .await
        .unwrap_err();
        assert!(
            invalid
                .get("error")
                .and_then(Value::as_str)
                .is_some_and(|error| error.contains("invalid arguments")),
            "invalid renewal must be rejected before Browser owner renewal: {invalid}"
        );
        assert_eq!(
            browser_registry.signed_child_cleanup_status().pending_owner_leases,
            1,
            "invalid args must not renew or replace the existing Browser owner lease"
        );
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn preflight_accepts_forked_lane_id_for_owner_and_rejects_sibling() {
        let close_probe = Arc::new(BrowserCloseProbe {
            lane_closes: AtomicUsize::new(0),
        });
        let hub = BrowserSessionHub::new(
            Arc::new(TestBrowserFactory {
                probe: Arc::clone(&close_probe),
            }),
            HubConfig::default(),
        );
        let browser_registry =
            crate::browser_registry::BrowserRegistry::from_hub(hub);
        let ctx = CallerCtx {
            conversation_id: Some(
                nomifun_common::ConversationId::parse(TEST_CONVERSATION_ID)
                    .unwrap(),
            ),
            user_id: UserId::parse(TEST_OWNER_ID).unwrap(),
            ..Default::default()
        };

        // nomi_browser_fork returns the owner-scoped Lane handle.
        let attached = preflight_and_attach_browser_identity(
            browser_registry.clone(),
            ctx.clone(),
            "nomi_browser_fork",
            &json!({"lane_name": "research"}),
            "signed-browser-lease-fork".to_owned(),
            u64::MAX,
        )
        .await
        .expect("fork request must attach the Browser owner");
        let forked = browser_registry
            .dispatch_managed(
                &attached,
                None,
                json!({"action": "browser_fork", "lane_name": "research"}),
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

        // A follow-up request carrying that lane_id starts from a fresh
        // per-request CallerCtx with no browser identity; the transport
        // preflight must accept it for the same signed lease.
        preflight_and_attach_browser_identity(
            browser_registry.clone(),
            ctx.clone(),
            "nomi_browser_status",
            &json!({"lane_id": lane_id}),
            "signed-browser-lease-fork".to_owned(),
            u64::MAX,
        )
        .await
        .expect("the owning lease must be able to target its forked lane_id");

        // A different signed lease is a different trusted runtime; the same
        // handle must still fail closed after identity attachment.
        let error = preflight_and_attach_browser_identity(
            browser_registry.clone(),
            ctx,
            "nomi_browser_status",
            &json!({"lane_id": lane_id}),
            "signed-browser-lease-sibling".to_owned(),
            u64::MAX,
        )
        .await
        .expect_err("an unowned lane handle must be rejected at preflight");
        assert_eq!(
            error.get("code").and_then(Value::as_str),
            Some("operation_not_allowed"),
            "{error}"
        );
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
