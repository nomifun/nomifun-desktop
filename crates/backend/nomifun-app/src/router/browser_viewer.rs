//! Authenticated embedded-browser viewer WebSocket.
//!
//! This is deliberately a separate socket from the application's JSON
//! realtime channel: one connection carries binary JPEG frames and
//! lane-scoped JSON control/input messages. Authentication and the single-use
//! viewer grant are both checked before the HTTP upgrade is accepted.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::extract::ws::{CloseFrame, Message, WebSocket};
use axum::extract::{Path, Query, State, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use base64::Engine as _;
use nomifun_browser_platform::{
    BrowserErrorCode, BrowserIdentityMode, BrowserLaneId, BrowserLaneSnapshot, BrowserOperation,
    BrowserOperationKind, BrowserOperationResult, BrowserPlatformError,
    BrowserSessionHub, ControlLease, ViewerGrant,
};
use nomifun_realtime::WsHandlerState;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{Mutex, watch};
use uuid::Uuid;

use super::browser_management::browser_url_projection::project_renderer_url;

const MAX_INBOUND_MESSAGE_BYTES: usize = 16 * 1024;
const MAX_OUTBOUND_JPEG_BYTES: usize = 4 * 1024 * 1024;
const MAX_WS_WRITE_BUFFER_BYTES: usize = 8 * 1024 * 1024;
const MAX_INBOUND_MESSAGES_PER_SECOND: u32 = 120;
const MAX_NAVIGATION_URL_BYTES: usize = 4 * 1024;
const MAX_TEXT_INPUT_BYTES: usize = 256;
const MAX_FRAME_WIDTH: u32 = 1600;
const MAX_FRAME_HEIGHT: u32 = 1200;
// 10 FPS intentionally stays below the product ceiling of 12 FPS.
const SCREENSHOT_POLL_INTERVAL: Duration = Duration::from_millis(100);
// Defense in depth: even if a CDP backend is misconfigured, the application
// transport never publishes faster than the 12 FPS product ceiling.
const MIN_STREAM_FRAME_INTERVAL: Duration = Duration::from_millis(84);
const PENDING_CONTROL_CLAIM_TTL_MS: u64 = 5_000;
// Viewer sockets are long-lived and are not registered with the shared
// realtime WebSocket manager, so they need their own bounded re-auth check.
const VIEWER_REAUTH_INTERVAL: Duration = Duration::from_secs(10);
// Keep enough already-published frame bindings to cover normal renderer and
// network backpressure without allowing an unbounded per-viewer target map.
// At the 12 FPS transport ceiling this represents more than 20 seconds of
// history; an older token fails closed and the renderer can retry on a fresh
// frame.
const MAX_PUBLISHED_FRAME_BINDINGS: usize = 256;
// JavaScript must be able to echo the JSON number without loss of precision.
const MAX_SAFE_FRAME_VERSION: u64 = 9_007_199_254_740_991;

#[derive(Clone)]
pub struct BrowserViewerState {
    authority: Arc<dyn BrowserViewerAuthority>,
    ws_auth: WsHandlerState,
    allow_local_webview_origins: bool,
    control_holders: Arc<Mutex<HashMap<BrowserLaneId, ViewerControlHolder>>>,
    next_control_generation: Arc<AtomicU64>,
    control_mutations: Arc<Mutex<HashMap<BrowserLaneId, Arc<Mutex<()>>>>>,
}

#[derive(Clone)]
struct ViewerControlHolder {
    user_id: String,
    connection_id: String,
    lease_id: Option<String>,
    generation: u64,
    expires_at_ms: u64,
    revoked: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ViewerControlClaim {
    user_id: String,
    connection_id: String,
    generation: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct ViewerControlRevocation {
    user_id: String,
    lane_id: BrowserLaneId,
    generation: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ControlAcquisition {
    ExplicitTakeover,
    Automatic,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ViewerClaimStatus {
    Current,
    Expired,
    Revoked,
    Replaced,
    Missing,
}

impl BrowserViewerState {
    pub fn new(
        hub: Arc<BrowserSessionHub>,
        ws_auth: WsHandlerState,
        allow_local_webview_origins: bool,
    ) -> Self {
        Self {
            authority: hub,
            ws_auth,
            allow_local_webview_origins,
            control_holders: Arc::new(Mutex::new(HashMap::new())),
            next_control_generation: Arc::new(AtomicU64::new(1)),
            control_mutations: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    async fn claim_control_holder(
        &self,
        user_id: &str,
        lane_id: &BrowserLaneId,
        connection_id: &str,
    ) -> Result<ViewerControlClaim, BrowserPlatformError> {
        let now_ms = epoch_ms();
        let mut holders = self.control_holders.lock().await;
        if let Some(current) = holders.get(lane_id) {
            let same_holder =
                current.user_id == user_id && current.connection_id == connection_id;
            if !current.revoked && !same_holder && current.expires_at_ms > now_ms {
                return Err(BrowserPlatformError::new(
                    BrowserErrorCode::LaneControlledByUser,
                    "Another viewer currently controls this browser lane.",
                    true,
                    "Wait for that viewer to return control or for its lease to expire.",
                )
                .for_lane(lane_id.clone()));
            }
            if same_holder && !current.revoked && current.expires_at_ms > now_ms {
                return Ok(ViewerControlClaim {
                    user_id: current.user_id.clone(),
                    connection_id: current.connection_id.clone(),
                    generation: current.generation,
                });
            }
        }
        // A short pending claim closes the race between two concurrent
        // takeover calls. It is replaced by the Hub lease expiry as soon as
        // acquisition succeeds. An expired holder is atomically replaced.
        let generation = self
            .next_control_generation
            .fetch_add(1, Ordering::Relaxed);
        let claim = ViewerControlClaim {
            user_id: user_id.to_owned(),
            connection_id: connection_id.to_owned(),
            generation,
        };
        holders.insert(
            lane_id.clone(),
            ViewerControlHolder {
                user_id: claim.user_id.clone(),
                connection_id: claim.connection_id.clone(),
                lease_id: None,
                generation: claim.generation,
                expires_at_ms: now_ms.saturating_add(PENDING_CONTROL_CLAIM_TTL_MS),
                revoked: false,
            },
        );
        Ok(claim)
    }

    async fn lane_control_mutation(&self, lane_id: &BrowserLaneId) -> Arc<Mutex<()>> {
        let mut mutations = self.control_mutations.lock().await;
        mutations
            .entry(lane_id.clone())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    async fn update_control_holder(
        &self,
        lane_id: &BrowserLaneId,
        claim: &ViewerControlClaim,
        lease: &ControlLease,
    ) -> bool {
        let mut holders = self.control_holders.lock().await;
        if let Some(holder) = holders.get_mut(lane_id)
            && holder.user_id == claim.user_id
            && holder.connection_id == claim.connection_id
            && holder.generation == claim.generation
            && !holder.revoked
        {
            holder.lease_id = Some(lease.lease_id.clone());
            holder.expires_at_ms = lease.expires_at_ms;
            return true;
        }
        false
    }

    async fn release_control_holder(
        &self,
        lane_id: &BrowserLaneId,
        claim: &ViewerControlClaim,
    ) -> bool {
        let mut holders = self.control_holders.lock().await;
        let should_remove = holders.get(lane_id).is_some_and(|current| {
            current.user_id == claim.user_id
                && current.connection_id == claim.connection_id
                && current.generation == claim.generation
        });
        if should_remove {
            holders.remove(lane_id);
        }
        should_remove
    }

    async fn claim_status(
        &self,
        lane_id: &BrowserLaneId,
        claim: &ViewerControlClaim,
    ) -> ViewerClaimStatus {
        let holders = self.control_holders.lock().await;
        let Some(current) = holders.get(lane_id) else {
            return ViewerClaimStatus::Missing;
        };
        if current.generation != claim.generation
            || current.user_id != claim.user_id
            || current.connection_id != claim.connection_id
        {
            return ViewerClaimStatus::Replaced;
        }
        if current.revoked {
            ViewerClaimStatus::Revoked
        } else if current.expires_at_ms <= epoch_ms() {
            ViewerClaimStatus::Expired
        } else {
            ViewerClaimStatus::Current
        }
    }

    /// Captures the exact holder generation that an HTTP return-control
    /// request intends to revoke. Completion must use
    /// [`Self::finish_control_revocation`] so a concurrent explicit takeover
    /// cannot be mistaken for the older holder.
    pub(crate) async fn begin_control_revocation(
        &self,
        user_id: &str,
        lane_id: &BrowserLaneId,
    ) -> ViewerControlRevocation {
        let holders = self.control_holders.lock().await;
        let generation = holders
            .get(lane_id)
            .filter(|current| current.user_id == user_id)
            .map(|current| current.generation);
        ViewerControlRevocation {
            user_id: user_id.to_owned(),
            lane_id: lane_id.clone(),
            generation,
        }
    }

    /// Idempotently revokes only the holder generation captured before the
    /// authoritative Hub return. If another takeover installed a newer holder
    /// while the Hub call was in flight, that holder is preserved.
    pub(crate) async fn finish_control_revocation(
        &self,
        revocation: ViewerControlRevocation,
    ) -> bool {
        let Some(generation) = revocation.generation else {
            return false;
        };
        let mut holders = self.control_holders.lock().await;
        let should_remove = holders.get(&revocation.lane_id).is_some_and(|current| {
            current.user_id == revocation.user_id
                && current.generation == generation
                && !current.revoked
        });
        if should_remove {
            holders.remove(&revocation.lane_id);
        }
        should_remove
    }

    /// Performs the authoritative Hub return and updates the route-local
    /// holder under the same lane-scoped mutation gate. Keeping the gate
    /// lane-local prevents a slow return on one lane from serializing any
    /// other lane.
    pub(crate) async fn return_control_and_revoke(
        &self,
        user_id: &str,
        lane_id: &BrowserLaneId,
    ) -> Result<bool, BrowserPlatformError> {
        let gate = self.lane_control_mutation(lane_id).await;
        let _guard = gate.lock().await;
        let revocation = self.begin_control_revocation(user_id, lane_id).await;
        let returned = self
            .authority
            .return_control_for_user(user_id, lane_id)
            .await?;
        // Hub false is still a successful stale-holder cleanup case: the
        // local route may have outlived the authoritative lease.
        self.finish_control_revocation(revocation).await;
        Ok(returned)
    }

    /// Returns control for the exact lease held by one viewer connection.
    ///
    /// The HTTP endpoint intentionally retains user+lane-wide revocation
    /// semantics, but a viewer socket must never be able to revoke another
    /// socket's lease merely because it shares the same user and lane. The
    /// route-local claim and opaque Hub lease id are both checked before the
    /// authoritative exact-lease release.
    async fn return_control_for_connection(
        &self,
        user_id: &str,
        lane_id: &BrowserLaneId,
        claim: Option<&ViewerControlClaim>,
        lease_id: Option<&str>,
    ) -> Result<bool, BrowserPlatformError> {
        let claim = claim.ok_or_else(|| control_returned_error(lane_id))?;
        let lease_id = lease_id.ok_or_else(|| control_returned_error(lane_id))?;
        let gate = self.lane_control_mutation(lane_id).await;
        let _guard = gate.lock().await;

        let exact_holder = {
            let holders = self.control_holders.lock().await;
            holders.get(lane_id).is_some_and(|holder| {
                claim.user_id == user_id
                    && holder.user_id == user_id
                    && holder.connection_id == claim.connection_id
                    && holder.generation == claim.generation
                    && holder.lease_id.as_deref() == Some(lease_id)
                    && !holder.revoked
            })
        };

        // A stale connection may legitimately arrive after the HTTP
        // user+lane return-control endpoint has already released its lease.
        // Treat that as an idempotent no-op, but never call the Hub: doing so
        // could release a newer lease installed by another connection.
        if !exact_holder {
            return Ok(false);
        }

        let returned = self
            .authority
            .return_control_for_lease(user_id, lane_id, lease_id)
            .await?;

        let mut holders = self.control_holders.lock().await;
        let still_exact = holders.get(lane_id).is_some_and(|holder| {
            holder.user_id == user_id
                && holder.connection_id == claim.connection_id
                && holder.generation == claim.generation
                && holder.lease_id.as_deref() == Some(lease_id)
        });
        if still_exact {
            holders.remove(lane_id);
        }
        Ok(returned)
    }

    /// Compatibility helper for narrow unit tests and old callers. Production
    /// return-control paths must use [`Self::return_control_and_revoke`] so the
    /// Hub operation and holder revocation share one lane gate.
    #[cfg(test)]
    async fn clear_control_holder_for_user(
        &self,
        user_id: &str,
        lane_id: &BrowserLaneId,
    ) -> bool {
        let gate = self.lane_control_mutation(lane_id).await;
        let _guard = gate.lock().await;
        let revocation = self.begin_control_revocation(user_id, lane_id).await;
        self.finish_control_revocation(revocation).await
    }

    #[cfg(test)]
    fn with_authority(
        authority: Arc<dyn BrowserViewerAuthority>,
        ws_auth: WsHandlerState,
        allow_local_webview_origins: bool,
    ) -> Self {
        Self {
            authority,
            ws_auth,
            allow_local_webview_origins,
            control_holders: Arc::new(Mutex::new(HashMap::new())),
            next_control_generation: Arc::new(AtomicU64::new(1)),
            control_mutations: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

/// Returns a fully-state-bound route group. Do not wrap this route in the
/// ordinary HTTP auth middleware: browser WebSockets cannot set the desktop
/// local-trust header, so the handler reuses the application's established
/// WebSocket token extractor/authenticator instead.
pub fn browser_viewer_routes(state: BrowserViewerState) -> Router {
    Router::new()
        .route(
            "/api/browser/lanes/{id}/view",
            get(browser_viewer_upgrade),
        )
        .with_state(state)
}

#[derive(Debug, Deserialize)]
struct ViewerQuery {
    token: String,
}

struct AuthorizedViewer {
    user_id: String,
    lane_id: BrowserLaneId,
    identity_mode: BrowserIdentityMode,
    selected_protocol: Option<String>,
    // Retained only in memory for re-authentication. It is deliberately not
    // included in any response or Debug representation.
    auth_token: String,
}

async fn browser_viewer_upgrade(
    State(state): State<BrowserViewerState>,
    Path(raw_lane_id): Path<String>,
    Query(query): Query<ViewerQuery>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let authorized = match authorize_upgrade(&state, &headers, &raw_lane_id, &query.token).await {
        Ok(authorized) => authorized,
        Err(rejection) => return rejection.into_response(),
    };

    let ws = ws
        .read_buffer_size(MAX_INBOUND_MESSAGE_BYTES)
        .write_buffer_size(64 * 1024)
        .max_write_buffer_size(MAX_WS_WRITE_BUFFER_BYTES)
        .max_message_size(MAX_INBOUND_MESSAGE_BYTES)
        .max_frame_size(MAX_INBOUND_MESSAGE_BYTES);
    let ws = match authorized.selected_protocol.clone() {
        Some(protocol) => ws.protocols([protocol]),
        None => ws,
    };
    ws.on_upgrade(move |socket| async move {
        handle_viewer_socket(socket, state, authorized).await;
    })
}

async fn authorize_upgrade(
    state: &BrowserViewerState,
    headers: &HeaderMap,
    raw_lane_id: &str,
    viewer_token: &str,
) -> Result<AuthorizedViewer, ViewerRejection> {
    // Origin is deliberately checked before either credential. This avoids
    // consuming a one-shot grant on a cross-site handshake and avoids turning
    // the endpoint into a credential-validity oracle for hostile origins.
    let origin_disposition = validate_origin(headers, state.allow_local_webview_origins)
        .map_err(|_| ViewerRejection::forbidden("Viewer origin is not allowed."))?;

    let lane_id = BrowserLaneId::parse(raw_lane_id.to_owned())
        .map_err(|_| ViewerRejection::not_found("Browser lane not found."))?;
    if !valid_viewer_token_shape(viewer_token) {
        return Err(ViewerRejection::forbidden("Viewer access denied."));
    }

    let requested_protocol = first_requested_protocol(headers);
    let (user_id, selected_protocol, auth_token) = match origin_disposition {
        OriginDisposition::LocalWebview => {
            // A local development page is cross-origin with the backend. Do
            // not let an ambient session cookie authorize it: the desktop
            // trust/JWT must be explicitly bound to this WebSocket handshake
            // as the first requested subprotocol.
            let protocol = requested_protocol.ok_or_else(|| {
                ViewerRejection::forbidden(
                    "Cross-origin viewer access requires local desktop trust.",
                )
            })?;
            let auth_token = protocol.to_owned();
            let user_id = (state.ws_auth.token_authenticator)(&auth_token).ok_or_else(|| {
                ViewerRejection::forbidden("Application authentication failed.")
            })?;
            (user_id, Some(auth_token.clone()), auth_token)
        }
        OriginDisposition::SameOrigin => {
            let auth_token = (state.ws_auth.token_extractor)(headers).ok_or_else(|| {
                ViewerRejection::forbidden("Application authentication required.")
            })?;
            let user_id = (state.ws_auth.token_authenticator)(&auth_token).ok_or_else(|| {
                ViewerRejection::forbidden("Application authentication failed.")
            })?;
            // If the browser explicitly requested an authenticated protocol,
            // echo it even when the shared extractor preferred an ambient
            // cookie. Never echo an arbitrary, unauthenticated protocol.
            let selected_protocol = requested_protocol
                .filter(|protocol| {
                    (state.ws_auth.token_authenticator)(protocol).as_deref()
                        == Some(user_id.as_str())
                })
                .map(str::to_owned);
            (user_id, selected_protocol, auth_token)
        }
    };

    state
        .authority
        .consume_viewer_token(&user_id, &lane_id, viewer_token)
        .await
        .map_err(ViewerRejection::from_platform)?;
    let identity_mode = state
        .authority
        .viewer_snapshot(&user_id, &lane_id)
        .await
        .map_err(ViewerRejection::from_platform)?
        .identity_mode;

    Ok(AuthorizedViewer {
        user_id,
        lane_id,
        identity_mode,
        selected_protocol,
        auth_token,
    })
}

fn valid_viewer_token_shape(token: &str) -> bool {
    token.len() == 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn first_requested_protocol(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::SEC_WEBSOCKET_PROTOCOL)
        .and_then(|value| value.to_str().ok())
        .and_then(|protocols| protocols.split(',').next())
        .map(str::trim)
        .filter(|protocol| !protocol.is_empty())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OriginDisposition {
    SameOrigin,
    LocalWebview,
}

fn validate_origin(
    headers: &HeaderMap,
    allow_local_webview: bool,
) -> Result<OriginDisposition, ()> {
    let mut hosts = headers.get_all(header::HOST).iter();
    let host = hosts
        .next()
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|host| !host.is_empty())
        .ok_or(())?;
    if hosts.next().is_some() {
        return Err(());
    }
    let mut origins = headers.get_all(header::ORIGIN).iter();
    let origin = origins
        .next()
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|origin| !origin.is_empty() && !origin.contains(','))
        .ok_or(())?;
    if origins.next().is_some() {
        return Err(());
    }
    let uri = origin.parse::<axum::http::Uri>().map_err(|_| ())?;
    let scheme = uri.scheme_str().ok_or(())?;
    let authority = uri.authority().ok_or(())?;
    if uri
        .path_and_query()
        .is_some_and(|path_and_query| path_and_query.as_str() != "/")
        || authority.as_str().contains('@')
    {
        return Err(());
    }
    if matches!(scheme, "http" | "https")
        && authority.as_str().eq_ignore_ascii_case(host)
    {
        return Ok(OriginDisposition::SameOrigin);
    }
    if !allow_local_webview {
        return Err(());
    }

    let origin_host = authority
        .host()
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or_else(|| authority.host());
    let is_tauri_origin = (scheme == "tauri" && origin_host.eq_ignore_ascii_case("localhost"))
        || (matches!(scheme, "http" | "https")
            && origin_host.eq_ignore_ascii_case("tauri.localhost"));
    let is_loopback_dev_origin = matches!(scheme, "http" | "https")
        && (origin_host.eq_ignore_ascii_case("localhost")
            || origin_host == "127.0.0.1"
            || origin_host == "::1");
    if is_tauri_origin || is_loopback_dev_origin {
        Ok(OriginDisposition::LocalWebview)
    } else {
        Err(())
    }
}

#[derive(Debug)]
struct ViewerRejection {
    status: StatusCode,
    code: &'static str,
    message: &'static str,
}

impl ViewerRejection {
    fn forbidden(message: &'static str) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            code: "viewer_access_denied",
            message,
        }
    }

    fn not_found(message: &'static str) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "lane_not_found",
            message,
        }
    }

    fn from_platform(error: BrowserPlatformError) -> Self {
        let (status, code, message) = match error.code {
            BrowserErrorCode::LaneNotFound => (
                StatusCode::NOT_FOUND,
                "lane_not_found",
                "Browser lane not found.",
            ),
            BrowserErrorCode::BrowserShuttingDown | BrowserErrorCode::BrowserUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "browser_unavailable",
                "Browser viewer is temporarily unavailable.",
            ),
            BrowserErrorCode::ViewerTokenExpired => (
                StatusCode::FORBIDDEN,
                "viewer_token_expired",
                "Viewer access denied.",
            ),
            BrowserErrorCode::ViewerTokenConsumed => (
                StatusCode::FORBIDDEN,
                "viewer_token_consumed",
                "Viewer access denied.",
            ),
            _ => (
                StatusCode::FORBIDDEN,
                "viewer_access_denied",
                "Viewer access denied.",
            ),
        };
        Self {
            status,
            code,
            message,
        }
    }
}

impl IntoResponse for ViewerRejection {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({
                "code": self.code,
                "message": self.message,
            })),
        )
            .into_response()
    }
}

#[derive(Clone)]
enum FrameUpdate {
    Frame {
        bytes: Arc<[u8]>,
        width: u32,
        height: u32,
        target_id: String,
        mode: ViewerStreamMode,
        snapshot: BrowserLaneSnapshot,
    },
    Failed {
        code: BrowserErrorCode,
        message: String,
        retryable: bool,
    },
}

#[derive(Clone, Copy)]
enum ViewerStreamMode {
    Screencast,
    ScreenshotPollingFallback,
}

impl ViewerStreamMode {
    fn label(self) -> &'static str {
        match self {
            Self::Screencast => "screencast",
            Self::ScreenshotPollingFallback => "screenshot_polling_fallback",
        }
    }
}

#[derive(Clone, Debug)]
struct DisplayedFrame {
    width: u32,
    height: u32,
    target_id: String,
    // Opaque to the renderer. The id scopes the token to this socket while
    // the monotonically increasing version identifies one published JPEG.
    frame_id: String,
    frame_version: u64,
}

struct PublishedFrameBindings {
    connection_id: String,
    next_version: u64,
    frames: VecDeque<DisplayedFrame>,
}

impl PublishedFrameBindings {
    fn new(connection_id: String) -> Self {
        Self {
            connection_id,
            next_version: 1,
            frames: VecDeque::with_capacity(MAX_PUBLISHED_FRAME_BINDINGS),
        }
    }

    fn reserve(
        &mut self,
        width: u32,
        height: u32,
        target_id: String,
    ) -> Result<DisplayedFrame, BrowserPlatformError> {
        if self.next_version > MAX_SAFE_FRAME_VERSION {
            return Err(viewer_stream_error(
                "The browser viewer frame sequence was exhausted.",
            ));
        }
        let frame = DisplayedFrame {
            width,
            height,
            target_id,
            frame_id: self.connection_id.clone(),
            frame_version: self.next_version,
        };
        // Never reuse a version on one connection: an ancient token must not
        // become valid for a different JPEG after a counter wrap.
        self.next_version += 1;
        Ok(frame)
    }

    fn commit(&mut self, frame: DisplayedFrame) {
        debug_assert_eq!(frame.frame_id, self.connection_id);
        debug_assert!(frame.frame_version > 0 && frame.frame_version < self.next_version);
        if self.frames.len() == MAX_PUBLISHED_FRAME_BINDINGS {
            self.frames.pop_front();
        }
        self.frames.push_back(frame.clone());
    }

    fn resolve(
        &self,
        frame_id: Option<&str>,
        frame_version: Option<u64>,
    ) -> Result<DisplayedFrame, BrowserPlatformError> {
        let (Some(frame_id), Some(frame_version)) = (frame_id, frame_version) else {
            return Err(stale_viewer_frame_error(
                "Viewer input is not bound to a displayed browser frame.",
            ));
        };
        if frame_id != self.connection_id || frame_version == 0 {
            return Err(stale_viewer_frame_error(
                "Viewer input references an invalid browser frame.",
            ));
        }
        self.frames
            .iter()
            .find(|frame| frame.frame_version == frame_version)
            .cloned()
            .ok_or_else(|| {
                stale_viewer_frame_error(
                    "The displayed browser frame is no longer available for input.",
                )
            })
    }
}

struct ViewerConnection {
    connection_id: String,
    control_lease: Option<ControlLease>,
    control_claim: Option<ViewerControlClaim>,
    explicitly_revoked: bool,
    rate: MessageRate,
}

impl ViewerConnection {
    fn new() -> Self {
        Self {
            connection_id: Uuid::now_v7().to_string(),
            control_lease: None,
            control_claim: None,
            explicitly_revoked: false,
            rate: MessageRate::new(),
        }
    }
}

struct MessageRate {
    window_started: Instant,
    count: u32,
}

impl MessageRate {
    fn new() -> Self {
        Self {
            window_started: Instant::now(),
            count: 0,
        }
    }

    fn allow(&mut self) -> bool {
        let now = Instant::now();
        if now.duration_since(self.window_started) >= Duration::from_secs(1) {
            self.window_started = now;
            self.count = 0;
        }
        self.count = self.count.saturating_add(1);
        self.count <= MAX_INBOUND_MESSAGES_PER_SECOND
    }
}

async fn handle_viewer_socket(
    mut socket: WebSocket,
    state: BrowserViewerState,
    authorized: AuthorizedViewer,
) {
    let mut connection = ViewerConnection::new();
    let mut reauth = tokio::time::interval(VIEWER_REAUTH_INTERVAL);
    reauth.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // Do not make the first interval tick an immediate duplicate of the
    // upgrade-time authentication check.
    reauth.tick().await;
    if let Err(error) = state
        .authority
        .viewer_connected(
            &authorized.user_id,
            &authorized.lane_id,
            &connection.connection_id,
        )
        .await
    {
        send_platform_error(&mut socket, &error).await;
        close_policy(&mut socket, "browser lane unavailable").await;
        return;
    }
    let initial = match state
        .authority
        .viewer_snapshot(&authorized.user_id, &authorized.lane_id)
        .await
    {
        Ok(snapshot) => snapshot,
        Err(error) => {
            send_platform_error(&mut socket, &error).await;
            close_policy(&mut socket, "browser lane unavailable").await;
            let _ = state
                .authority
                .viewer_disconnected(
                    &authorized.user_id,
                    &authorized.lane_id,
                    &connection.connection_id,
                )
                .await;
            return;
        }
    };
    if send_json(
        &mut socket,
        viewer_metadata(
            "ready",
            &initial,
            None,
            Some("screencast"),
            None,
            None,
        ),
    )
    .await
    .is_err()
    {
        let _ = state
            .authority
            .viewer_disconnected(
                &authorized.user_id,
                &authorized.lane_id,
                &connection.connection_id,
            )
            .await;
        return;
    }

    let mut published_frames = PublishedFrameBindings::new(connection.connection_id.clone());
    let (frame_tx, mut frame_rx) = watch::channel::<Option<Arc<FrameUpdate>>>(None);
    let producer = tokio::spawn(frame_producer(
        state.authority.clone(),
        authorized.user_id.clone(),
        authorized.lane_id.clone(),
        frame_tx,
    ));
    let mut viewer_is_streaming = false;

    loop {
        tokio::select! {
            _ = reauth.tick() => {
                if !viewer_auth_still_valid(&state, &authorized) {
                    send_protocol_error(
                        &mut socket,
                        "viewer_auth_expired",
                        "Viewer authentication expired.",
                        false,
                    ).await;
                    close_policy(&mut socket, "viewer authentication expired").await;
                    break;
                }
            }
            changed = frame_rx.changed() => {
                if changed.is_err() {
                    break;
                }
                let update = frame_rx.borrow_and_update().clone();
                let Some(update) = update else {
                    continue;
                };
                match update.as_ref() {
                    FrameUpdate::Frame { bytes, width, height, target_id, mode, snapshot } => {
                        if !viewer_is_streaming {
                            if let Err(error) = state
                                .authority
                                .viewer_streaming(
                                    &authorized.user_id,
                                    &authorized.lane_id,
                                    &connection.connection_id,
                                )
                                .await
                            {
                                send_platform_error(&mut socket, &error).await;
                                break;
                            }
                            viewer_is_streaming = true;
                        }
                        let displayed_frame = match published_frames.reserve(
                            *width,
                            *height,
                            target_id.clone(),
                        ) {
                            Ok(frame) => frame,
                            Err(error) => {
                                send_platform_error(&mut socket, &error).await;
                                break;
                            }
                        };
                        if send_json(
                            &mut socket,
                            viewer_metadata(
                                "frame",
                                snapshot,
                                Some((*width, *height)),
                                Some(mode.label()),
                                Some(target_id),
                                Some(&displayed_frame),
                            ),
                        ).await.is_err() {
                            break;
                        }
                        if socket.send(Message::Binary(bytes.to_vec().into())).await.is_err() {
                            break;
                        }
                        // A reserved token is intentionally unresolvable while
                        // only its JSON metadata is in flight. Commit it only
                        // after the matching JPEG has been accepted by the WS
                        // sink, so input can never jump to a not-yet-sent
                        // target. Older committed frame tokens remain valid.
                        published_frames.commit(displayed_frame);
                    }
                    FrameUpdate::Failed { code, message, retryable } => {
                        if let Err(error) = state
                            .authority
                            .viewer_failed(
                                &authorized.user_id,
                                &authorized.lane_id,
                                &connection.connection_id,
                            )
                            .await
                        {
                            send_platform_error(&mut socket, &error).await;
                            break;
                        }
                        viewer_is_streaming = false;
                        if send_json(
                            &mut socket,
                            json!({
                                "type": "stream_error",
                                "code": code,
                                "message": message,
                                "recoverable": retryable,
                            }),
                        ).await.is_err() {
                            break;
                        }
                    }
                }
            }
            received = socket.recv() => {
                let Some(received) = received else {
                    break;
                };
                let message = match received {
                    Ok(message) => message,
                    Err(_) => break,
                };
                if !connection.rate.allow() {
                    send_protocol_error(
                        &mut socket,
                        "viewer_rate_limited",
                        "Viewer input rate exceeded the connection limit.",
                        true,
                    ).await;
                    close_policy(&mut socket, "viewer input rate exceeded").await;
                    break;
                }
                match message {
                    Message::Text(text) => {
                        if !viewer_auth_still_valid(&state, &authorized) {
                            send_protocol_error(
                                &mut socket,
                                "viewer_auth_expired",
                                "Viewer authentication expired.",
                                false,
                            ).await;
                            close_policy(&mut socket, "viewer authentication expired").await;
                            break;
                        }
                        if text.len() > MAX_INBOUND_MESSAGE_BYTES {
                            close_policy(&mut socket, "viewer message too large").await;
                            break;
                        }
                        let command = match serde_json::from_str::<ViewerCommand>(&text) {
                            Ok(command) => command,
                            Err(_) => {
                                send_protocol_error(
                                    &mut socket,
                                    "invalid_viewer_message",
                                    "Viewer message is invalid.",
                                    false,
                                ).await;
                                continue;
                            }
                        };
                        if let Err(error) = handle_viewer_command(
                            &state,
                            &authorized,
                            &mut connection,
                            command,
                            &published_frames,
                            &mut socket,
                        ).await {
                            send_platform_error(&mut socket, &error).await;
                        }
                    }
                    Message::Ping(payload) => {
                        if socket.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    Message::Pong(_) => {
                        let _ = state
                            .authority
                            .viewer_heartbeat(&authorized.user_id, &authorized.lane_id)
                            .await;
                    }
                    Message::Close(_) => break,
                    Message::Binary(_) => {
                        send_protocol_error(
                            &mut socket,
                            "invalid_viewer_message",
                            "Binary client messages are not accepted.",
                            false,
                        ).await;
                        close_policy(&mut socket, "binary client message rejected").await;
                        break;
                    }
                }
            }
        }
    }

    // Do not abort a driver future after the Hub has counted it as active.
    // Dropping the receiver makes the bounded producer exit after its current
    // (driver-deadline-bounded) capture without leaving Lane activity stuck.
    drop(frame_rx);
    drop(producer);
    if let Some(claim) = connection.control_claim.as_ref() {
        state
            .release_control_holder(&authorized.lane_id, claim)
            .await;
    }
    let _ = state
        .authority
        .viewer_disconnected(
            &authorized.user_id,
            &authorized.lane_id,
            &connection.connection_id,
        )
        .await;
}

fn viewer_auth_still_valid(state: &BrowserViewerState, authorized: &AuthorizedViewer) -> bool {
    (state.ws_auth.token_authenticator)(&authorized.auth_token)
        .as_deref()
        == Some(authorized.user_id.as_str())
}

async fn frame_producer(
    authority: Arc<dyn BrowserViewerAuthority>,
    user_id: String,
    lane_id: BrowserLaneId,
    frame_tx: watch::Sender<Option<Arc<FrameUpdate>>>,
) {
    // Page.startScreencast is the primary path. The driver acknowledges each
    // CDP frame before returning it here; this one-slot watch channel then
    // drops any application frame the socket has not yet consumed. A backend
    // that cannot start/continue screencast falls back to bounded screenshot
    // polling without taking down lane management.
    let mut mode = ViewerStreamMode::Screencast;
    let mut interval = tokio::time::interval(SCREENSHOT_POLL_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_failure: Option<(BrowserErrorCode, Instant)> = None;
    let mut next_frame_at = Instant::now();
    loop {
        if frame_tx.is_closed() {
            break;
        }
        if matches!(mode, ViewerStreamMode::ScreenshotPollingFallback) {
            interval.tick().await;
        }

        let captured =
            capture_viewer_frame(authority.as_ref(), &user_id, &lane_id, mode).await;
        let update = match captured {
            Ok(update) => {
                last_failure = None;
                update
            }
            Err(error)
                if matches!(mode, ViewerStreamMode::Screencast)
                    && matches!(
                        error.code,
                        BrowserErrorCode::OperationNotAllowed
                            | BrowserErrorCode::ViewerStreamFailed
                    ) =>
            {
                stop_viewer_screencast(authority.as_ref(), &user_id, &lane_id).await;
                mode = ViewerStreamMode::ScreenshotPollingFallback;
                continue;
            }
            Err(error) => FrameUpdate::Failed {
                code: error.code,
                message: error.message,
                retryable: error.retryable,
            },
        };

        if matches!(update, FrameUpdate::Frame { .. }) {
            let now = Instant::now();
            if now < next_frame_at {
                tokio::time::sleep_until(tokio::time::Instant::from_std(next_frame_at)).await;
            }
            next_frame_at = Instant::now() + MIN_STREAM_FRAME_INTERVAL;
        }
        if let FrameUpdate::Failed { code, .. } = &update {
            let now = Instant::now();
            if last_failure
                .is_some_and(|(previous, at)| previous == *code && now.duration_since(at) < Duration::from_secs(5))
            {
                continue;
            }
            last_failure = Some((*code, now));
        }
        if frame_tx.send(Some(Arc::new(update))).is_err() {
            break;
        }
    }
    stop_viewer_screencast(authority.as_ref(), &user_id, &lane_id).await;
}

async fn capture_viewer_frame(
    authority: &dyn BrowserViewerAuthority,
    user_id: &str,
    lane_id: &BrowserLaneId,
    mode: ViewerStreamMode,
) -> Result<FrameUpdate, BrowserPlatformError> {
    let action = match mode {
        ViewerStreamMode::Screencast => "viewer_screencast_frame",
        ViewerStreamMode::ScreenshotPollingFallback => "viewer_screenshot",
    };
    let result = authority
        .viewer_observe(
            user_id,
            lane_id,
            BrowserOperation {
                kind: BrowserOperationKind::Screenshot,
                action: action.to_owned(),
                input: json!({
                    "format": "jpeg",
                    "quality": 70,
                    "max_width": MAX_FRAME_WIDTH,
                    "max_height": MAX_FRAME_HEIGHT,
                    "max_fps": 12,
                }),
                expected_browser_epoch: None,
                target_id: None,
                frame_id: None,
                ref_generation: None,
                may_modify_identity: false,
            },
        )
        .await?;
    let (bytes, width, height, target_id) = decode_jpeg_result(&result)?;
    let snapshot = authority.viewer_snapshot(user_id, lane_id).await?;
    Ok(FrameUpdate::Frame {
        bytes: bytes.into(),
        width,
        height,
        target_id,
        mode,
        snapshot,
    })
}

async fn stop_viewer_screencast(
    authority: &dyn BrowserViewerAuthority,
    user_id: &str,
    lane_id: &BrowserLaneId,
) {
    let _ = authority
        .viewer_observe(
            user_id,
            lane_id,
            BrowserOperation {
                kind: BrowserOperationKind::View,
                action: "viewer_screencast_stop".to_owned(),
                input: json!({}),
                expected_browser_epoch: None,
                target_id: None,
                frame_id: None,
                ref_generation: None,
                may_modify_identity: false,
            },
        )
        .await;
}

fn decode_jpeg_result(
    result: &BrowserOperationResult,
) -> Result<(Vec<u8>, u32, u32, String), BrowserPlatformError> {
    let output = &result.output;
    let media_type = output
        .get("media_type")
        .and_then(Value::as_str)
        .unwrap_or("image/jpeg");
    if media_type != "image/jpeg" {
        return Err(viewer_stream_error(
            "The browser driver did not return a JPEG frame.",
        ));
    }
    let encoded = output
        .get("data")
        .or_else(|| output.get("jpeg_base64"))
        .or_else(|| output.get("base64"))
        .and_then(Value::as_str)
        .or_else(|| output.as_str())
        .ok_or_else(|| viewer_stream_error("The browser driver returned no JPEG frame."))?;
    let encoded = encoded
        .strip_prefix("data:image/jpeg;base64,")
        .unwrap_or(encoded);
    if encoded.len() > (MAX_OUTBOUND_JPEG_BYTES * 4 / 3) + 8 {
        return Err(viewer_stream_error("The browser frame exceeded the viewer limit."));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| viewer_stream_error("The browser driver returned an invalid JPEG frame."))?;
    if bytes.len() > MAX_OUTBOUND_JPEG_BYTES {
        return Err(viewer_stream_error("The browser frame exceeded the viewer limit."));
    }
    let parsed_dimensions = jpeg_dimensions(&bytes)
        .ok_or_else(|| viewer_stream_error("The browser driver returned an invalid JPEG frame."))?;
    let width = output
        .get("width")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(parsed_dimensions.0);
    let height = output
        .get("height")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(parsed_dimensions.1);
    if (width, height) != parsed_dimensions
        || width == 0
        || height == 0
        || width > MAX_FRAME_WIDTH
        || height > MAX_FRAME_HEIGHT
    {
        return Err(viewer_stream_error(
            "The browser frame dimensions exceeded the viewer limit.",
        ));
    }
    let target_id = output
        .get("target_id")
        .and_then(Value::as_str)
        .filter(|target_id| {
            !target_id.is_empty()
                && target_id.len() <= 128
                && target_id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        })
        .ok_or_else(|| {
            viewer_stream_error("The browser driver did not bind the frame to a target.")
        })?
        .to_owned();
    Ok((bytes, width, height, target_id))
}

fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 4 || !bytes.starts_with(&[0xff, 0xd8]) {
        return None;
    }
    let mut cursor = 2;
    while cursor + 4 <= bytes.len() {
        while cursor < bytes.len() && bytes[cursor] != 0xff {
            cursor += 1;
        }
        while cursor < bytes.len() && bytes[cursor] == 0xff {
            cursor += 1;
        }
        let marker = *bytes.get(cursor)?;
        cursor += 1;
        if marker == 0xd9 || marker == 0xda {
            break;
        }
        if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }
        let length = u16::from_be_bytes([*bytes.get(cursor)?, *bytes.get(cursor + 1)?]) as usize;
        if length < 2 || cursor + length > bytes.len() {
            return None;
        }
        let is_start_of_frame = matches!(
            marker,
            0xc0 | 0xc1 | 0xc2 | 0xc3 | 0xc5 | 0xc6 | 0xc7 | 0xc9 | 0xca | 0xcb | 0xcd | 0xce | 0xcf
        );
        if is_start_of_frame {
            if length < 7 {
                return None;
            }
            let height =
                u16::from_be_bytes([*bytes.get(cursor + 3)?, *bytes.get(cursor + 4)?]) as u32;
            let width =
                u16::from_be_bytes([*bytes.get(cursor + 5)?, *bytes.get(cursor + 6)?]) as u32;
            return Some((width, height));
        }
        cursor += length;
    }
    None
}

fn viewer_stream_error(message: &'static str) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::ViewerStreamFailed,
        message,
        true,
        "Retry the embedded viewer; lane management remains available.",
    )
}

fn viewer_metadata(
    event_type: &'static str,
    snapshot: &BrowserLaneSnapshot,
    dimensions: Option<(u32, u32)>,
    mode: Option<&'static str>,
    captured_target_id: Option<&str>,
    displayed_frame: Option<&DisplayedFrame>,
) -> Value {
    let active_tab = captured_target_id
        .and_then(|target_id| snapshot.tabs.iter().find(|tab| tab.target_id == target_id))
        .or_else(|| {
            snapshot
        .active_tab_id
        .as_ref()
        .and_then(|active| snapshot.tabs.iter().find(|tab| &tab.tab_id == active))
        })
        .or_else(|| snapshot.tabs.iter().find(|tab| tab.active))
        .or_else(|| snapshot.tabs.first());
    json!({
        "type": event_type,
        "frame": dimensions.map(|(width, height)| json!({"width": width, "height": height})),
        // These are deliberately opaque and connection-scoped. Never publish
        // the raw CDP target id: the renderer only needs to echo this pair on
        // coordinate input so the server can recover the exact target and
        // dimensions associated with the JPEG it actually displayed.
        "frame_id": displayed_frame.map(|frame| frame.frame_id.as_str()),
        "frame_version": displayed_frame.map(|frame| frame.frame_version),
        "title": active_tab.and_then(|tab| tab.title.as_deref()),
        "url": active_tab
            .and_then(|tab| tab.url.as_deref())
            .map(project_renderer_url),
        "active_tab_id": active_tab.map(|tab| tab.tab_id.as_str()).or(snapshot.active_tab_id.as_deref()),
        "control_state": snapshot.control_state,
        "stream_mode": mode,
    })
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum ViewerCommand {
    Observe {
        #[serde(default)]
        lane_id: Option<String>,
    },
    Takeover,
    Heartbeat {
        #[serde(default)]
        lane_id: Option<String>,
    },
    ReturnControl,
    Input {
        input: Value,
        // Compatibility: older clients can still deserialize and receive a
        // safe stale-frame command error. Coordinate input is dispatched only
        // when both fields bind it to a frame published on this socket.
        #[serde(default)]
        frame_id: Option<String>,
        #[serde(default)]
        frame_version: Option<u64>,
    },
    Navigate {
        url: String,
    },
    Back,
    Forward,
    Reload,
    SelectTab {
        tab_id: String,
    },
}

impl ViewerCommand {
    fn requires_user_control(&self) -> bool {
        matches!(
            self,
            Self::Takeover
                | Self::Input { .. }
                | Self::Navigate { .. }
                | Self::Back
                | Self::Forward
                | Self::Reload
                | Self::SelectTab { .. }
        )
    }
}

fn resolve_viewer_input_frame(
    input: &mut Value,
    command_frame_id: Option<String>,
    command_frame_version: Option<u64>,
    published_frames: &PublishedFrameBindings,
) -> Result<DisplayedFrame, BrowserPlatformError> {
    let (input_frame_id, input_frame_version) = match input.as_object_mut() {
        Some(input) => {
            let frame_id = input.remove("frame_id");
            let frame_version = input.remove("frame_version");
            let frame_id = frame_id
                .map(|value| {
                    value
                        .as_str()
                        .filter(|value| !value.is_empty() && value.len() <= 256)
                        .map(str::to_owned)
                        .ok_or_else(|| {
                            stale_viewer_frame_error(
                                "Viewer input references an invalid browser frame.",
                            )
                        })
                })
                .transpose()?;
            let frame_version = frame_version
                .map(|value| {
                    value
                        .as_u64()
                        .filter(|value| *value > 0 && *value <= MAX_SAFE_FRAME_VERSION)
                        .ok_or_else(|| {
                            stale_viewer_frame_error(
                                "Viewer input references an invalid browser frame.",
                            )
                        })
                })
                .transpose()?;
            (frame_id, frame_version)
        }
        None => (None, None),
    };
    if command_frame_id.is_some()
        && input_frame_id.is_some()
        && command_frame_id != input_frame_id
        || command_frame_version.is_some()
            && input_frame_version.is_some()
            && command_frame_version != input_frame_version
    {
        return Err(stale_viewer_frame_error(
            "Viewer input contains conflicting browser frame bindings.",
        ));
    }
    let frame_id = command_frame_id.or(input_frame_id);
    let frame_version = command_frame_version.or(input_frame_version);
    let coordinate_input = input
        .get("kind")
        .and_then(Value::as_str)
        .is_some_and(|kind| matches!(kind, "pointer" | "wheel"));
    if coordinate_input || frame_id.is_some() || frame_version.is_some() {
        published_frames.resolve(frame_id.as_deref(), frame_version)
    } else {
        // Key/text commands predate frame tokens and contain no coordinates.
        // Keeping their legacy fallback avoids breaking input-method and key
        // release flows while all coordinate input remains strictly bound.
        published_frames.frames.back().cloned().ok_or_else(|| {
            stale_viewer_frame_error("Wait for a browser frame before sending viewer input.")
        })
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ViewerInput {
    Pointer {
        action: PointerAction,
        x: f64,
        y: f64,
        button: u8,
        buttons: u16,
        #[serde(default)]
        modifiers: Modifiers,
    },
    Wheel {
        x: f64,
        y: f64,
        delta_x: f64,
        delta_y: f64,
        #[serde(default)]
        modifiers: Modifiers,
    },
    Key {
        action: KeyAction,
        key: String,
        code: String,
        #[serde(default)]
        repeat: bool,
        #[serde(default)]
        modifiers: Modifiers,
    },
    Text {
        text: String,
    },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum PointerAction {
    Move,
    Down,
    Up,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum KeyAction {
    Down,
    Up,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Modifiers {
    #[serde(default)]
    alt: bool,
    #[serde(default)]
    ctrl: bool,
    #[serde(default)]
    meta: bool,
    #[serde(default)]
    shift: bool,
}

async fn handle_viewer_command(
    state: &BrowserViewerState,
    authorized: &AuthorizedViewer,
    connection: &mut ViewerConnection,
    command: ViewerCommand,
    published_frames: &PublishedFrameBindings,
    socket: &mut WebSocket,
) -> Result<(), BrowserPlatformError> {
    if command.requires_user_control() {
        ensure_viewer_control_allowed(authorized)?;
    }
    match command {
        ViewerCommand::Observe { lane_id } => {
            validate_echoed_lane(lane_id.as_deref(), &authorized.lane_id)?;
            state
                .authority
                .viewer_heartbeat(&authorized.user_id, &authorized.lane_id)
                .await?;
        }
        ViewerCommand::Takeover => {
            ensure_control(
                state,
                authorized,
                connection,
                ControlAcquisition::ExplicitTakeover,
            )
            .await?;
            send_control_state(socket, "user", connection.control_lease.as_ref()).await;
        }
        ViewerCommand::Heartbeat { lane_id } => {
            validate_echoed_lane(lane_id.as_deref(), &authorized.lane_id)?;
            if connection.control_lease.is_some() {
                ensure_viewer_control_allowed(authorized)?;
                let gate = state.lane_control_mutation(&authorized.lane_id).await;
                let _guard = gate.lock().await;
                renew_existing_control(state, authorized, connection).await?;
                send_control_state(socket, "user", connection.control_lease.as_ref()).await;
            } else {
                state
                    .authority
                    .viewer_heartbeat(&authorized.user_id, &authorized.lane_id)
                    .await?;
            }
        }
        ViewerCommand::ReturnControl => {
            let claim = connection.control_claim.clone();
            let lease_id = connection
                .control_lease
                .as_ref()
                .map(|lease| lease.lease_id.clone());
            state
                .return_control_for_connection(
                    &authorized.user_id,
                    &authorized.lane_id,
                    claim.as_ref(),
                    lease_id.as_deref(),
                )
                .await?;
            connection.control_lease = None;
            connection.explicitly_revoked = true;
            connection.control_claim = None;
            send_control_state(socket, "agent", None).await;
        }
        ViewerCommand::Input {
            mut input,
            frame_id,
            frame_version,
        } => {
            let frame = resolve_viewer_input_frame(
                &mut input,
                frame_id,
                frame_version,
                published_frames,
            )?;
            let input = validate_input(input, (frame.width, frame.height))?;
            let include_dimensions =
                matches!(&input, ViewerInput::Pointer { .. } | ViewerInput::Wheel { .. });
            let mut input = serde_json::to_value(input)
                .map_err(|_| viewer_input_error("Viewer input could not be encoded."))?;
            if include_dimensions {
                let object = input
                    .as_object_mut()
                    .ok_or_else(|| viewer_input_error("Viewer input could not be encoded."))?;
                object.insert("frame_width".to_owned(), frame.width.into());
                object.insert("frame_height".to_owned(), frame.height.into());
            }
            ensure_control(state, authorized, connection, ControlAcquisition::Automatic).await?;
            dispatch_viewer_input(
                state,
                authorized,
                connection,
                input,
                frame.target_id,
            )
            .await?;
        }
        ViewerCommand::Navigate { url } => {
            validate_navigation_url(&url)?;
            ensure_control(state, authorized, connection, ControlAcquisition::Automatic).await?;
            dispatch_control_operation(
                state,
                authorized,
                connection,
                BrowserOperationKind::Navigate,
                "viewer_navigate",
                json!({"url": url}),
            )
            .await?;
        }
        ViewerCommand::Back => {
            ensure_control(state, authorized, connection, ControlAcquisition::Automatic).await?;
            dispatch_control_operation(
                state,
                authorized,
                connection,
                BrowserOperationKind::Navigate,
                "viewer_back",
                json!({}),
            )
            .await?;
        }
        ViewerCommand::Forward => {
            ensure_control(state, authorized, connection, ControlAcquisition::Automatic).await?;
            dispatch_control_operation(
                state,
                authorized,
                connection,
                BrowserOperationKind::Navigate,
                "viewer_forward",
                json!({}),
            )
            .await?;
        }
        ViewerCommand::Reload => {
            ensure_control(state, authorized, connection, ControlAcquisition::Automatic).await?;
            dispatch_control_operation(
                state,
                authorized,
                connection,
                BrowserOperationKind::Navigate,
                "viewer_reload",
                json!({}),
            )
            .await?;
        }
        ViewerCommand::SelectTab { tab_id } => {
            if tab_id.is_empty() || tab_id.len() > 128 {
                return Err(viewer_input_error("The selected tab is invalid."));
            }
            let snapshot = state
                .authority
                .viewer_snapshot(&authorized.user_id, &authorized.lane_id)
                .await?;
            let (selected_tab_id, selected_target_id) = snapshot
                .tabs
                .iter()
                .find(|tab| tab.tab_id == tab_id)
                .map(|tab| (tab.tab_id.clone(), tab.target_id.clone()))
                .ok_or_else(|| {
                    viewer_input_error("The selected tab is not in this browser lane.")
                })?;
            ensure_control(state, authorized, connection, ControlAcquisition::Automatic).await?;
            dispatch_control_operation(
                state,
                authorized,
                connection,
                BrowserOperationKind::Tabs,
                "viewer_select_tab",
                json!({"tab_id": selected_tab_id, "target_id": selected_target_id}),
            )
            .await?;
        }
    }
    Ok(())
}

async fn ensure_control(
    state: &BrowserViewerState,
    authorized: &AuthorizedViewer,
    connection: &mut ViewerConnection,
    acquisition: ControlAcquisition,
) -> Result<(), BrowserPlatformError> {
    ensure_viewer_control_allowed(authorized)?;

    if acquisition == ControlAcquisition::ExplicitTakeover {
        connection.explicitly_revoked = false;
    } else if connection.explicitly_revoked {
        return Err(control_returned_error(&authorized.lane_id));
    }

    let gate = state.lane_control_mutation(&authorized.lane_id).await;
    let _guard = gate.lock().await;

    if connection.control_lease.is_some() {
        match renew_existing_control(state, authorized, connection).await {
            Ok(()) => return Ok(()),
            Err(error) => {
                let claim_status = match connection.control_claim.as_ref() {
                    Some(claim) => state.claim_status(&authorized.lane_id, claim).await,
                    None => ViewerClaimStatus::Missing,
                };
                connection.control_lease = None;
                match claim_status {
                    ViewerClaimStatus::Revoked | ViewerClaimStatus::Replaced => {
                        connection.explicitly_revoked = true;
                        return Err(control_returned_error(&authorized.lane_id));
                    }
                    ViewerClaimStatus::Current
                    | ViewerClaimStatus::Expired
                    | ViewerClaimStatus::Missing => {
                        if let Some(claim) = connection.control_claim.take() {
                            state
                                .release_control_holder(&authorized.lane_id, &claim)
                                .await;
                        }
                        if acquisition == ControlAcquisition::ExplicitTakeover {
                            // Explicit takeover is allowed to recover from any
                            // stale local lease after the authoritative renew
                            // has already failed.
                        } else if claim_status == ViewerClaimStatus::Current {
                            return Err(error);
                        }
                    }
                }
            }
        }
    }

    if acquisition == ControlAcquisition::Automatic && connection.explicitly_revoked {
        return Err(control_returned_error(&authorized.lane_id));
    }

    let claim = state
        .claim_control_holder(
            &authorized.user_id,
            &authorized.lane_id,
            &connection.connection_id,
        )
        .await?;
    match state
        .authority
        .take_control(&authorized.user_id, &authorized.lane_id)
        .await
    {
        Ok(lease) => {
            if !state
                .update_control_holder(&authorized.lane_id, &claim, &lease)
                .await
            {
                return Err(control_returned_error(&authorized.lane_id));
            }
            connection.control_claim = Some(claim);
            connection.control_lease = Some(lease);
            Ok(())
        }
        Err(error) => {
            state
                .release_control_holder(&authorized.lane_id, &claim)
                .await;
            Err(error)
        }
    }
}

fn control_returned_error(lane_id: &BrowserLaneId) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::OperationNotAllowed,
        "Browser control was returned to the Agent for this viewer.",
        true,
        "Choose Take over before sending more browser input.",
    )
    .for_lane(lane_id.clone())
}

async fn renew_existing_control(
    state: &BrowserViewerState,
    authorized: &AuthorizedViewer,
    connection: &mut ViewerConnection,
) -> Result<(), BrowserPlatformError> {
    let lease_id = connection
        .control_lease
        .as_ref()
        .map(|lease| lease.lease_id.clone())
        .ok_or_else(|| control_returned_error(&authorized.lane_id))?;
    let claim = connection
        .control_claim
        .clone()
        .ok_or_else(|| control_returned_error(&authorized.lane_id))?;
    match state.claim_status(&authorized.lane_id, &claim).await {
        ViewerClaimStatus::Revoked
        | ViewerClaimStatus::Replaced
        | ViewerClaimStatus::Missing => {
            connection.control_lease = None;
            connection.explicitly_revoked = true;
            return Err(control_returned_error(&authorized.lane_id));
        }
        ViewerClaimStatus::Current | ViewerClaimStatus::Expired => {}
    }
    let renewed = state
        .authority
        .renew_control(
            &authorized.user_id,
            &authorized.lane_id,
            &lease_id,
        )
        .await?;
    if !state
        .update_control_holder(&authorized.lane_id, &claim, &renewed)
        .await
    {
        connection.control_lease = None;
        connection.explicitly_revoked = true;
        return Err(control_returned_error(&authorized.lane_id));
    }
    connection.control_lease = Some(renewed);
    Ok(())
}

fn ensure_viewer_control_allowed(
    authorized: &AuthorizedViewer,
) -> Result<(), BrowserPlatformError> {
    if !authorized.identity_mode.permits_interaction() {
        return Err(BrowserPlatformError::new(
            BrowserErrorCode::NeedsPrimaryIdentity,
            "Read-only crawl browser lanes cannot accept embedded-viewer control or input.",
            false,
            "Open a Primary or Isolated interactive lane to take control or send browser input.",
        )
        .for_lane(authorized.lane_id.clone()));
    }
    Ok(())
}

async fn dispatch_viewer_input(
    state: &BrowserViewerState,
    authorized: &AuthorizedViewer,
    connection: &ViewerConnection,
    input: Value,
    target_id: String,
) -> Result<(), BrowserPlatformError> {
    dispatch_targeted_control_operation(
        state,
        authorized,
        connection,
        BrowserOperationKind::Act,
        "viewer_input",
        input,
        target_id,
    )
    .await
}

async fn dispatch_control_operation(
    state: &BrowserViewerState,
    authorized: &AuthorizedViewer,
    connection: &ViewerConnection,
    kind: BrowserOperationKind,
    action: &'static str,
    input: Value,
) -> Result<(), BrowserPlatformError> {
    dispatch_control_operation_inner(state, authorized, connection, kind, action, input, None).await
}

async fn dispatch_targeted_control_operation(
    state: &BrowserViewerState,
    authorized: &AuthorizedViewer,
    connection: &ViewerConnection,
    kind: BrowserOperationKind,
    action: &'static str,
    input: Value,
    target_id: String,
) -> Result<(), BrowserPlatformError> {
    dispatch_control_operation_inner(
        state,
        authorized,
        connection,
        kind,
        action,
        input,
        Some(target_id),
    )
    .await
}

async fn dispatch_control_operation_inner(
    state: &BrowserViewerState,
    authorized: &AuthorizedViewer,
    connection: &ViewerConnection,
    kind: BrowserOperationKind,
    action: &'static str,
    input: Value,
    target_id: Option<String>,
) -> Result<(), BrowserPlatformError> {
    let lease = connection
        .control_lease
        .as_ref()
        .ok_or_else(|| viewer_input_error("Take control before sending browser input."))?;
    state
        .authority
        .viewer_input(
            &authorized.user_id,
            &authorized.lane_id,
            &lease.lease_id,
            BrowserOperation {
                kind,
                action: action.to_owned(),
                input,
                expected_browser_epoch: None,
                target_id,
                frame_id: None,
                ref_generation: None,
                // Read-only crawl viewer control is rejected above before
                // this operation is constructed. The Hub validates identity
                // mode and trusted viewer action shape independently of this
                // wire hint.
                may_modify_identity: false,
            },
        )
        .await?;
    Ok(())
}

fn validate_echoed_lane(
    echoed: Option<&str>,
    expected: &BrowserLaneId,
) -> Result<(), BrowserPlatformError> {
    if echoed.is_some_and(|lane| lane != expected.as_str()) {
        return Err(viewer_input_error(
            "Viewer messages cannot target a different browser lane.",
        ));
    }
    Ok(())
}

fn validate_input(
    input: Value,
    dimensions: (u32, u32),
) -> Result<ViewerInput, BrowserPlatformError> {
    let input: ViewerInput = serde_json::from_value(input)
        .map_err(|_| viewer_input_error("Viewer input is invalid."))?;
    match &input {
        ViewerInput::Pointer {
            x,
            y,
            button,
            buttons,
            ..
        } => {
            validate_point(*x, *y, dimensions)?;
            if *button > 4 || *buttons > 31 {
                return Err(viewer_input_error("Pointer button state is invalid."));
            }
        }
        ViewerInput::Wheel {
            x,
            y,
            delta_x,
            delta_y,
            ..
        } => {
            validate_point(*x, *y, dimensions)?;
            if !delta_x.is_finite()
                || !delta_y.is_finite()
                || delta_x.abs() > 10_000.0
                || delta_y.abs() > 10_000.0
            {
                return Err(viewer_input_error("Wheel delta is outside the allowed range."));
            }
        }
        ViewerInput::Key { key, code, .. } => {
            if key.is_empty()
                || code.is_empty()
                || key.len() > MAX_TEXT_INPUT_BYTES
                || code.len() > 64
            {
                return Err(viewer_input_error("Keyboard input is outside the allowed range."));
            }
        }
        ViewerInput::Text { text } => {
            if text.is_empty()
                || text.len() > MAX_TEXT_INPUT_BYTES
                || text
                    .chars()
                    .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
                || text.contains('\u{7f}')
            {
                return Err(viewer_input_error(
                    "Text input is outside the allowed range.",
                ));
            }
        }
    }
    Ok(input)
}

fn validate_point(
    x: f64,
    y: f64,
    dimensions: (u32, u32),
) -> Result<(), BrowserPlatformError> {
    if !x.is_finite()
        || !y.is_finite()
        || x < 0.0
        || y < 0.0
        || x > f64::from(dimensions.0)
        || y > f64::from(dimensions.1)
    {
        return Err(viewer_input_error(
            "Pointer coordinates are outside the current browser frame.",
        ));
    }
    Ok(())
}

fn validate_navigation_url(url: &str) -> Result<(), BrowserPlatformError> {
    if url.is_empty() || url.len() > MAX_NAVIGATION_URL_BYTES {
        return Err(viewer_input_error("The browser address is invalid."));
    }
    let uri = url
        .parse::<axum::http::Uri>()
        .map_err(|_| viewer_input_error("The browser address is invalid."))?;
    if !matches!(uri.scheme_str(), Some("http" | "https")) || uri.authority().is_none() {
        return Err(viewer_input_error(
            "Only absolute HTTP or HTTPS browser addresses are allowed.",
        ));
    }
    Ok(())
}

fn viewer_input_error(message: &'static str) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::OperationNotAllowed,
        message,
        false,
        "Use the controls in the embedded browser viewer.",
    )
}

fn stale_viewer_frame_error(message: &'static str) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::StaleLaneRef,
        message,
        true,
        "Wait for the next browser frame and retry the input.",
    )
    .with_metadata(json!({
        "reason": "viewer_frame_binding_stale",
        "requires_fresh_frame": true,
    }))
}

fn epoch_ms() -> u64 {
    SystemTime::UNIX_EPOCH
        .elapsed()
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

async fn send_json(socket: &mut WebSocket, value: Value) -> Result<(), axum::Error> {
    let text = serde_json::to_string(&value)
        .unwrap_or_else(|_| r#"{"type":"error","code":"viewer_stream_failed"}"#.to_owned());
    socket.send(Message::Text(text.into())).await
}

async fn send_platform_error(socket: &mut WebSocket, error: &BrowserPlatformError) {
    let _ = send_json(
        socket,
        json!({
            "type": "command_error",
            "code": error.code,
            "message": error.message,
            "recoverable": error.retryable,
        }),
    )
    .await;
}

async fn send_protocol_error(
    socket: &mut WebSocket,
    code: &'static str,
    message: &'static str,
    recoverable: bool,
) {
    let _ = send_json(
        socket,
        json!({
            "type": "protocol_error",
            "code": code,
            "message": message,
            "recoverable": recoverable,
        }),
    )
    .await;
}

async fn send_control_state(
    socket: &mut WebSocket,
    control_state: &'static str,
    lease: Option<&ControlLease>,
) {
    let _ = send_json(
        socket,
        json!({
            "type": "control",
            "control_state": control_state,
            "expires_at": lease.map(|lease| lease.expires_at_ms),
        }),
    )
    .await;
}

async fn close_policy(socket: &mut WebSocket, reason: &'static str) {
    let _ = socket
        .send(Message::Close(Some(CloseFrame {
            code: 1008,
            reason: reason.into(),
        })))
        .await;
}

#[async_trait]
trait BrowserViewerAuthority: Send + Sync {
    async fn consume_viewer_token(
        &self,
        user_id: &str,
        lane_id: &BrowserLaneId,
        raw_token: &str,
    ) -> Result<ViewerGrant, BrowserPlatformError>;

    async fn viewer_snapshot(
        &self,
        user_id: &str,
        lane_id: &BrowserLaneId,
    ) -> Result<BrowserLaneSnapshot, BrowserPlatformError>;

    async fn viewer_heartbeat(
        &self,
        user_id: &str,
        lane_id: &BrowserLaneId,
    ) -> Result<(), BrowserPlatformError>;

    async fn viewer_connected(
        &self,
        user_id: &str,
        lane_id: &BrowserLaneId,
        viewer_id: &str,
    ) -> Result<(), BrowserPlatformError>;

    async fn viewer_streaming(
        &self,
        user_id: &str,
        lane_id: &BrowserLaneId,
        viewer_id: &str,
    ) -> Result<(), BrowserPlatformError>;

    async fn viewer_failed(
        &self,
        user_id: &str,
        lane_id: &BrowserLaneId,
        viewer_id: &str,
    ) -> Result<(), BrowserPlatformError>;

    async fn viewer_disconnected(
        &self,
        user_id: &str,
        lane_id: &BrowserLaneId,
        viewer_id: &str,
    ) -> Result<(), BrowserPlatformError>;

    async fn viewer_observe(
        &self,
        user_id: &str,
        lane_id: &BrowserLaneId,
        operation: BrowserOperation,
    ) -> Result<BrowserOperationResult, BrowserPlatformError>;

    async fn viewer_input(
        &self,
        user_id: &str,
        lane_id: &BrowserLaneId,
        lease_id: &str,
        operation: BrowserOperation,
    ) -> Result<BrowserOperationResult, BrowserPlatformError>;

    async fn take_control(
        &self,
        user_id: &str,
        lane_id: &BrowserLaneId,
    ) -> Result<ControlLease, BrowserPlatformError>;

    async fn renew_control(
        &self,
        user_id: &str,
        lane_id: &BrowserLaneId,
        lease_id: &str,
    ) -> Result<ControlLease, BrowserPlatformError>;

    async fn return_control_for_user(
        &self,
        user_id: &str,
        lane_id: &BrowserLaneId,
    ) -> Result<bool, BrowserPlatformError>;

    async fn return_control_for_lease(
        &self,
        user_id: &str,
        lane_id: &BrowserLaneId,
        lease_id: &str,
    ) -> Result<bool, BrowserPlatformError>;
}

#[async_trait]
impl BrowserViewerAuthority for BrowserSessionHub {
    async fn consume_viewer_token(
        &self,
        user_id: &str,
        lane_id: &BrowserLaneId,
        raw_token: &str,
    ) -> Result<ViewerGrant, BrowserPlatformError> {
        BrowserSessionHub::consume_viewer_token(self, user_id, lane_id, raw_token).await
    }

    async fn viewer_snapshot(
        &self,
        user_id: &str,
        lane_id: &BrowserLaneId,
    ) -> Result<BrowserLaneSnapshot, BrowserPlatformError> {
        BrowserSessionHub::viewer_snapshot(self, user_id, lane_id).await
    }

    async fn viewer_heartbeat(
        &self,
        user_id: &str,
        lane_id: &BrowserLaneId,
    ) -> Result<(), BrowserPlatformError> {
        BrowserSessionHub::viewer_heartbeat(self, user_id, lane_id).await
    }

    async fn viewer_connected(
        &self,
        user_id: &str,
        lane_id: &BrowserLaneId,
        viewer_id: &str,
    ) -> Result<(), BrowserPlatformError> {
        BrowserSessionHub::viewer_connected(self, user_id, lane_id, viewer_id).await
    }

    async fn viewer_streaming(
        &self,
        user_id: &str,
        lane_id: &BrowserLaneId,
        viewer_id: &str,
    ) -> Result<(), BrowserPlatformError> {
        BrowserSessionHub::viewer_streaming(self, user_id, lane_id, viewer_id).await
    }

    async fn viewer_failed(
        &self,
        user_id: &str,
        lane_id: &BrowserLaneId,
        viewer_id: &str,
    ) -> Result<(), BrowserPlatformError> {
        BrowserSessionHub::viewer_failed(self, user_id, lane_id, viewer_id).await
    }

    async fn viewer_disconnected(
        &self,
        user_id: &str,
        lane_id: &BrowserLaneId,
        viewer_id: &str,
    ) -> Result<(), BrowserPlatformError> {
        BrowserSessionHub::viewer_disconnected(self, user_id, lane_id, viewer_id).await
    }

    async fn viewer_observe(
        &self,
        user_id: &str,
        lane_id: &BrowserLaneId,
        operation: BrowserOperation,
    ) -> Result<BrowserOperationResult, BrowserPlatformError> {
        BrowserSessionHub::viewer_observe(self, user_id, lane_id, operation).await
    }

    async fn viewer_input(
        &self,
        user_id: &str,
        lane_id: &BrowserLaneId,
        lease_id: &str,
        operation: BrowserOperation,
    ) -> Result<BrowserOperationResult, BrowserPlatformError> {
        BrowserSessionHub::viewer_input(self, user_id, lane_id, lease_id, operation).await
    }

    async fn take_control(
        &self,
        user_id: &str,
        lane_id: &BrowserLaneId,
    ) -> Result<ControlLease, BrowserPlatformError> {
        BrowserSessionHub::take_control(self, user_id, lane_id).await
    }

    async fn renew_control(
        &self,
        user_id: &str,
        lane_id: &BrowserLaneId,
        lease_id: &str,
    ) -> Result<ControlLease, BrowserPlatformError> {
        BrowserSessionHub::renew_control(self, user_id, lane_id, lease_id).await
    }

    async fn return_control_for_user(
        &self,
        user_id: &str,
        lane_id: &BrowserLaneId,
    ) -> Result<bool, BrowserPlatformError> {
        BrowserSessionHub::return_control_for_user(self, user_id, lane_id).await
    }

    async fn return_control_for_lease(
        &self,
        user_id: &str,
        lane_id: &BrowserLaneId,
        lease_id: &str,
    ) -> Result<bool, BrowserPlatformError> {
        Ok(BrowserSessionHub::return_control(self, user_id, lane_id, lease_id).await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;

    use nomifun_browser_platform::{
        BrowserCapacitySnapshot, BrowserLaneSnapshot, CallerIdentity, LaneControlState,
        LaneKey, LaneLifecycleState, ManualClock, OwnerLeaseId, ViewerState,
        ViewerTokenService,
    };
    use nomifun_realtime::{NoopMessageRouter, WebSocketManager};
    use tokio::net::TcpListener;
    use tokio_tungstenite::tungstenite;

    struct FakeAuthority {
        tokens: ViewerTokenService,
        lane: BrowserLaneId,
        identity_mode: BrowserIdentityMode,
        observed_actions: std::sync::Mutex<Vec<String>>,
        control_attempts: std::sync::atomic::AtomicUsize,
        input_attempts: std::sync::atomic::AtomicUsize,
    }

    impl FakeAuthority {
        fn issue(&self, user_id: &str, lane_id: BrowserLaneId) -> ViewerGrant {
            self.tokens.issue(user_id, lane_id).unwrap()
        }

        fn snapshot(&self) -> BrowserLaneSnapshot {
            BrowserLaneSnapshot {
                lane_id: self.lane.clone(),
                lane_key: LaneKey::new("runtime", Some("default")).unwrap(),
                caller: CallerIdentity {
                    user_id: "user-1".to_owned(),
                    conversation_id: None,
                    runtime_instance_id: "runtime".to_owned(),
                    agent_id: None,
                    companion_id: None,
                    execution_id: None,
                    step_id: None,
                    attempt_id: None,
                    remote_connection_id: None,
                    surface: nomifun_browser_platform::BrowserSurface::User,
                    owner_lease_id: OwnerLeaseId::new(),
                    capability_expires_at_ms: u64::MAX,
                    allowed_operations: Default::default(),
                },
                identity_mode: self.identity_mode,
                identity_generation: 0,
                lifecycle_state: LaneLifecycleState::Running,
                control_state: LaneControlState::Agent,
                browser_epoch: 1,
                tabs: Vec::new(),
                active_tab_id: None,
                active_frame_id: None,
                ref_generation: 0,
                queue: None,
                resource_estimate_bytes: 0,
                active_operation_count: 0,
                last_active_at_ms: 0,
                created_at_ms: 0,
                viewer_state: ViewerState::Streaming,
                error_code: None,
                error_message: None,
                recoverable: false,
            }
        }
    }

    #[async_trait]
    impl BrowserViewerAuthority for FakeAuthority {
        async fn consume_viewer_token(
            &self,
            user_id: &str,
            lane_id: &BrowserLaneId,
            raw_token: &str,
        ) -> Result<ViewerGrant, BrowserPlatformError> {
            self.tokens.consume(raw_token, user_id, lane_id)
        }

        async fn viewer_snapshot(
            &self,
            user_id: &str,
            lane_id: &BrowserLaneId,
        ) -> Result<BrowserLaneSnapshot, BrowserPlatformError> {
            if user_id == "user-1" && lane_id == &self.lane {
                Ok(self.snapshot())
            } else {
                Err(BrowserPlatformError::lane_not_found(lane_id.clone()))
            }
        }

        async fn viewer_heartbeat(
            &self,
            _user_id: &str,
            _lane_id: &BrowserLaneId,
        ) -> Result<(), BrowserPlatformError> {
            Ok(())
        }

        async fn viewer_connected(
            &self,
            _user_id: &str,
            _lane_id: &BrowserLaneId,
            _viewer_id: &str,
        ) -> Result<(), BrowserPlatformError> {
            Ok(())
        }

        async fn viewer_streaming(
            &self,
            _user_id: &str,
            _lane_id: &BrowserLaneId,
            _viewer_id: &str,
        ) -> Result<(), BrowserPlatformError> {
            Ok(())
        }

        async fn viewer_failed(
            &self,
            _user_id: &str,
            _lane_id: &BrowserLaneId,
            _viewer_id: &str,
        ) -> Result<(), BrowserPlatformError> {
            Ok(())
        }

        async fn viewer_disconnected(
            &self,
            _user_id: &str,
            _lane_id: &BrowserLaneId,
            _viewer_id: &str,
        ) -> Result<(), BrowserPlatformError> {
            Ok(())
        }

        async fn viewer_observe(
            &self,
            _user_id: &str,
            lane_id: &BrowserLaneId,
            operation: BrowserOperation,
        ) -> Result<BrowserOperationResult, BrowserPlatformError> {
            self.observed_actions
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(operation.action);
            Err(BrowserPlatformError::new(
                BrowserErrorCode::ViewerStreamFailed,
                "test stream unavailable",
                true,
                "retry",
            )
            .for_lane(lane_id.clone()))
        }

        async fn viewer_input(
            &self,
            _user_id: &str,
            _lane_id: &BrowserLaneId,
            _lease_id: &str,
            operation: BrowserOperation,
        ) -> Result<BrowserOperationResult, BrowserPlatformError> {
            self.input_attempts
                .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            assert!(
                !operation.may_modify_identity,
                "viewer identity authorization is mode- and action-shape-based, not caller-set"
            );
            Ok(BrowserOperationResult::default())
        }

        async fn take_control(
            &self,
            _user_id: &str,
            lane_id: &BrowserLaneId,
        ) -> Result<ControlLease, BrowserPlatformError> {
            self.control_attempts
                .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            Ok(ControlLease {
                lease_id: "lease".to_owned(),
                lane_id: lane_id.clone(),
                user_id: "user-1".to_owned(),
                issued_at_ms: 0,
                renewed_at_ms: 0,
                expires_at_ms: u64::MAX,
            })
        }

        async fn renew_control(
            &self,
            _user_id: &str,
            lane_id: &BrowserLaneId,
            _lease_id: &str,
        ) -> Result<ControlLease, BrowserPlatformError> {
            self.take_control("user-1", lane_id).await
        }

        async fn return_control_for_user(
            &self,
            _user_id: &str,
            _lane_id: &BrowserLaneId,
        ) -> Result<bool, BrowserPlatformError> {
            Ok(true)
        }

        async fn return_control_for_lease(
            &self,
            _user_id: &str,
            _lane_id: &BrowserLaneId,
            _lease_id: &str,
        ) -> Result<bool, BrowserPlatformError> {
            Ok(true)
        }
    }

    fn ws_auth() -> WsHandlerState {
        WsHandlerState {
            manager: Arc::new(WebSocketManager::new()),
            router: Arc::new(NoopMessageRouter),
            token_authenticator: Arc::new(|token| {
                (token == "app-auth").then(|| "user-1".to_owned())
            }),
            token_extractor: Arc::new(|headers| {
                headers
                    .get(header::AUTHORIZATION)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.strip_prefix("Bearer "))
                    .map(str::to_owned)
            }),
        }
    }

    async fn start_server(
        authority: Arc<FakeAuthority>,
    ) -> (SocketAddr, tokio::task::JoinHandle<()>) {
        let app = browser_viewer_routes(BrowserViewerState::with_authority(
            authority,
            ws_auth(),
            false,
        ));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (addr, task)
    }

    fn request(
        addr: SocketAddr,
        lane: &BrowserLaneId,
        token: &str,
        origin: Option<&str>,
    ) -> tungstenite::http::Request<()> {
        let mut request = tungstenite::http::Request::builder()
            .uri(format!(
                "ws://{addr}/api/browser/lanes/{lane}/view?token={token}"
            ))
            .header(header::HOST.as_str(), addr.to_string())
            .header(header::CONNECTION.as_str(), "Upgrade")
            .header(header::UPGRADE.as_str(), "websocket")
            .header(header::SEC_WEBSOCKET_VERSION.as_str(), "13")
            .header(
                header::SEC_WEBSOCKET_KEY.as_str(),
                tungstenite::handshake::client::generate_key(),
            )
            .header(header::AUTHORIZATION.as_str(), "Bearer app-auth");
        if let Some(origin) = origin {
            request = request.header(header::ORIGIN.as_str(), origin);
        }
        request.body(()).unwrap()
    }

    async fn rejected_status(request: tungstenite::http::Request<()>) -> StatusCode {
        match tokio_tungstenite::connect_async(request).await {
            Err(tungstenite::Error::Http(response)) => response.status(),
            Ok((mut socket, _)) => {
                let _ = socket.close(None).await;
                StatusCode::SWITCHING_PROTOCOLS
            }
            Err(error) => panic!("unexpected websocket error: {error}"),
        }
    }

    #[tokio::test]
    async fn invalid_expired_replayed_and_cross_lane_tokens_fail_closed() {
        let clock = ManualClock::new(100);
        let lane = BrowserLaneId::new();
        let other_lane = BrowserLaneId::new();
        let authority = Arc::new(FakeAuthority {
            tokens: ViewerTokenService::new(Arc::new(clock.clone()), 10),
            lane: lane.clone(),
            identity_mode: BrowserIdentityMode::Primary,
            observed_actions: Default::default(),
            control_attempts: Default::default(),
            input_attempts: Default::default(),
        });
        let (addr, server) = start_server(authority.clone()).await;
        let origin = format!("http://{addr}");

        assert_eq!(
            rejected_status(request(addr, &lane, &"0".repeat(64), Some(&origin))).await,
            StatusCode::FORBIDDEN
        );

        let cross_lane = authority.issue("user-1", lane.clone());
        assert_eq!(
            rejected_status(request(addr, &other_lane, &cross_lane.token, Some(&origin))).await,
            StatusCode::FORBIDDEN
        );
        // A cross-lane attempt does not consume a grant for its actual lane.
        assert_eq!(
            rejected_status(request(addr, &lane, &cross_lane.token, Some(&origin))).await,
            StatusCode::SWITCHING_PROTOCOLS
        );
        assert_eq!(
            rejected_status(request(addr, &lane, &cross_lane.token, Some(&origin))).await,
            StatusCode::FORBIDDEN
        );

        let expired = authority.issue("user-1", lane.clone());
        clock.advance(10);
        assert_eq!(
            rejected_status(request(addr, &lane, &expired.token, Some(&origin))).await,
            StatusCode::FORBIDDEN
        );

        server.abort();
    }

    #[tokio::test]
    async fn cross_user_token_attempt_fails_without_consuming_owner_grant() {
        let lane = BrowserLaneId::new();
        let authority = Arc::new(FakeAuthority {
            tokens: ViewerTokenService::new(Arc::new(ManualClock::new(100)), 1_000),
            lane: lane.clone(),
            identity_mode: BrowserIdentityMode::Primary,
            observed_actions: Default::default(),
            control_attempts: Default::default(),
            input_attempts: Default::default(),
        });
        let grant = authority.issue("user-1", lane.clone());
        let state = BrowserViewerState::with_authority(
            authority,
            WsHandlerState {
                manager: Arc::new(WebSocketManager::new()),
                router: Arc::new(NoopMessageRouter),
                token_authenticator: Arc::new(|token| match token {
                    "owner-auth" => Some("user-1".to_owned()),
                    "other-auth" => Some("user-2".to_owned()),
                    _ => None,
                }),
                token_extractor: Arc::new(|headers| {
                    headers
                        .get(header::AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        .and_then(|value| value.strip_prefix("Bearer "))
                        .map(str::to_owned)
                }),
            },
            false,
        );
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "127.0.0.1:4400".parse().unwrap());
        headers.insert(header::ORIGIN, "http://127.0.0.1:4400".parse().unwrap());
        headers.insert(
            header::AUTHORIZATION,
            "Bearer other-auth".parse().unwrap(),
        );

        assert!(
            authorize_upgrade(&state, &headers, lane.as_str(), &grant.token)
                .await
                .is_err(),
            "another authenticated user must not redeem the owner-bound viewer grant"
        );

        headers.insert(
            header::AUTHORIZATION,
            "Bearer owner-auth".parse().unwrap(),
        );
        assert!(
            authorize_upgrade(&state, &headers, lane.as_str(), &grant.token)
                .await
                .is_ok(),
            "a cross-user mismatch must not consume the owner-bound one-shot grant"
        );
        assert!(
            authorize_upgrade(&state, &headers, lane.as_str(), &grant.token)
                .await
                .is_err(),
            "the successful owner redemption must consume the grant"
        );
    }

    #[tokio::test]
    async fn origin_is_required_and_checked_before_single_use_token_consumption() {
        let lane = BrowserLaneId::new();
        let authority = Arc::new(FakeAuthority {
            tokens: ViewerTokenService::new(Arc::new(ManualClock::new(100)), 1_000),
            lane: lane.clone(),
            identity_mode: BrowserIdentityMode::Primary,
            observed_actions: Default::default(),
            control_attempts: Default::default(),
            input_attempts: Default::default(),
        });
        let grant = authority.issue("user-1", lane.clone());
        let (addr, server) = start_server(authority).await;

        assert_eq!(
            rejected_status(request(addr, &lane, &grant.token, None)).await,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            rejected_status(request(
                addr,
                &lane,
                &grant.token,
                Some("https://attacker.example")
            ))
            .await,
            StatusCode::FORBIDDEN
        );
        // Both rejected origins leave the one-shot token usable.
        let origin = format!("http://{addr}");
        assert_eq!(
            rejected_status(request(addr, &lane, &grant.token, Some(&origin))).await,
            StatusCode::SWITCHING_PROTOCOLS
        );
        server.abort();
    }

    #[tokio::test]
    async fn cross_origin_local_webview_requires_protocol_bound_app_auth() {
        let lane = BrowserLaneId::new();
        let authority = Arc::new(FakeAuthority {
            tokens: ViewerTokenService::new(Arc::new(ManualClock::new(100)), 1_000),
            lane: lane.clone(),
            identity_mode: BrowserIdentityMode::Primary,
            observed_actions: Default::default(),
            control_attempts: Default::default(),
            input_attempts: Default::default(),
        });
        let grant = authority.issue("user-1", lane.clone());
        let protocol_auth = WsHandlerState {
            manager: Arc::new(WebSocketManager::new()),
            router: Arc::new(NoopMessageRouter),
            token_authenticator: Arc::new(|token| {
                matches!(token, "app-auth" | "ambient-auth").then(|| "user-1".to_owned())
            }),
            token_extractor: Arc::new(|headers| {
                headers
                    .get(header::AUTHORIZATION)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.strip_prefix("Bearer "))
                    .map(str::to_owned)
                    .or_else(|| {
                        headers
                            .get(header::SEC_WEBSOCKET_PROTOCOL)
                            .and_then(|value| value.to_str().ok())
                            .map(str::to_owned)
                    })
            }),
        };
        let state = BrowserViewerState::with_authority(
            authority.clone(),
            protocol_auth,
            true,
        );
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "127.0.0.1:4400".parse().unwrap());
        headers.insert(header::ORIGIN, "http://localhost:5173".parse().unwrap());
        headers.insert(
            header::AUTHORIZATION,
            "Bearer ambient-auth".parse().unwrap(),
        );

        assert!(
            authorize_upgrade(&state, &headers, lane.as_str(), &grant.token)
                .await
                .is_err(),
            "a cookie/header credential must not authorize a cross-origin local page"
        );

        headers.insert(
            header::SEC_WEBSOCKET_PROTOCOL,
            "app-auth".parse().unwrap(),
        );
        assert!(
            authorize_upgrade(&state, &headers, lane.as_str(), &grant.token)
                .await
                .is_ok(),
            "the rejected cross-origin attempt must not consume the viewer token"
        );
    }

    #[test]
    fn origin_parser_rejects_ambiguous_or_non_origin_values() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "127.0.0.1:4400".parse().unwrap());
        headers.append(header::ORIGIN, "http://127.0.0.1:4400".parse().unwrap());
        headers.append(header::ORIGIN, "https://attacker.example".parse().unwrap());
        assert!(validate_origin(&headers, true).is_err());

        headers.remove(header::ORIGIN);
        headers.insert(
            header::ORIGIN,
            "http://127.0.0.1:4400/path".parse().unwrap(),
        );
        assert!(validate_origin(&headers, true).is_err());

        headers.insert(
            header::ORIGIN,
            "http://user@localhost:5173".parse().unwrap(),
        );
        assert!(validate_origin(&headers, true).is_err());

        headers.insert(header::ORIGIN, "http://[::1]:5173".parse().unwrap());
        assert_eq!(
            validate_origin(&headers, true),
            Ok(OriginDisposition::LocalWebview)
        );
    }

    #[tokio::test]
    async fn one_live_viewer_holds_control_but_expired_holder_is_replaceable() {
        let lane = BrowserLaneId::new();
        let authority = Arc::new(FakeAuthority {
            tokens: ViewerTokenService::new(Arc::new(ManualClock::new(100)), 1_000),
            lane: lane.clone(),
            identity_mode: BrowserIdentityMode::Primary,
            observed_actions: Default::default(),
            control_attempts: Default::default(),
            input_attempts: Default::default(),
        });
        let state = BrowserViewerState::with_authority(authority, ws_auth(), false);
        state
            .claim_control_holder("user-1", &lane, "viewer-1")
            .await
            .unwrap();
        assert_eq!(
            state
                .claim_control_holder("user-1", &lane, "viewer-2")
                .await
                .unwrap_err()
                .code,
            BrowserErrorCode::LaneControlledByUser
        );
        state
            .control_holders
            .lock()
            .await
            .get_mut(&lane)
            .expect("holder exists")
            .expires_at_ms = 0;
        state
            .claim_control_holder("user-1", &lane, "viewer-2")
            .await
            .unwrap();
        assert_eq!(
            state
                .control_holders
                .lock()
                .await
                .get(&lane)
                .map(|holder| holder.connection_id.as_str()),
            Some("viewer-2")
        );
    }

    #[tokio::test]
    async fn http_return_control_clear_is_user_and_lane_scoped_and_idempotent() {
        let lane = BrowserLaneId::new();
        let other_lane = BrowserLaneId::new();
        let other_user_lane = BrowserLaneId::new();
        let authority = Arc::new(FakeAuthority {
            tokens: ViewerTokenService::new(Arc::new(ManualClock::new(100)), 1_000),
            lane: lane.clone(),
            identity_mode: BrowserIdentityMode::Primary,
            observed_actions: Default::default(),
            control_attempts: Default::default(),
            input_attempts: Default::default(),
        });
        let state = BrowserViewerState::with_authority(authority, ws_auth(), false);
        state
            .claim_control_holder("user-1", &lane, "viewer-1")
            .await
            .unwrap();
        state
            .claim_control_holder("user-1", &other_lane, "viewer-other-lane")
            .await
            .unwrap();
        state
            .claim_control_holder("user-2", &other_user_lane, "viewer-other-user")
            .await
            .unwrap();

        assert!(
            !state
                .clear_control_holder_for_user("user-2", &other_lane)
                .await,
            "another user must not clear a same-user holder on another lane"
        );
        assert_eq!(
            state
                .claim_control_holder("user-1", &lane, "viewer-2")
                .await
                .unwrap_err()
                .code,
            BrowserErrorCode::LaneControlledByUser
        );

        assert!(
            state
                .clear_control_holder_for_user("user-1", &lane)
                .await,
            "the matching HTTP return-control scope should remove the stale holder"
        );
        assert!(
            !state
                .clear_control_holder_for_user("user-1", &lane)
                .await,
            "repeated clear calls must be idempotent"
        );
        state
            .claim_control_holder("user-1", &lane, "viewer-2")
            .await
            .unwrap();

        let holders = state.control_holders.lock().await;
        assert_eq!(
            holders
                .get(&lane)
                .map(|holder| (holder.user_id.as_str(), holder.connection_id.as_str())),
            Some(("user-1", "viewer-2"))
        );
        assert_eq!(
            holders.get(&other_lane).map(|holder| {
                (holder.user_id.as_str(), holder.connection_id.as_str())
            }),
            Some(("user-1", "viewer-other-lane")),
            "clearing one lane must not touch another lane for the same user"
        );
        assert_eq!(
            holders.get(&other_user_lane).map(|holder| {
                (holder.user_id.as_str(), holder.connection_id.as_str())
            }),
            Some(("user-2", "viewer-other-user")),
            "clearing one user's lane must not touch another user's lane"
        );
    }

    #[tokio::test]
    async fn concurrent_http_return_control_clears_remove_matching_holder_once() {
        let lane = BrowserLaneId::new();
        let unrelated_lane = BrowserLaneId::new();
        let authority = Arc::new(FakeAuthority {
            tokens: ViewerTokenService::new(Arc::new(ManualClock::new(100)), 1_000),
            lane: lane.clone(),
            identity_mode: BrowserIdentityMode::Primary,
            observed_actions: Default::default(),
            control_attempts: Default::default(),
            input_attempts: Default::default(),
        });
        let state = BrowserViewerState::with_authority(authority, ws_auth(), false);
        state
            .claim_control_holder("user-1", &lane, "viewer-1")
            .await
            .unwrap();
        state
            .claim_control_holder("user-2", &unrelated_lane, "viewer-2")
            .await
            .unwrap();

        let task_count = 16;
        let barrier = Arc::new(tokio::sync::Barrier::new(task_count + 1));
        let mut tasks = Vec::with_capacity(task_count);
        for index in 0..task_count {
            let state = state.clone();
            let lane = lane.clone();
            let barrier = barrier.clone();
            tasks.push(tokio::spawn(async move {
                let user_id = if index % 2 == 0 { "user-1" } else { "user-2" };
                barrier.wait().await;
                (
                    user_id,
                    state
                        .clear_control_holder_for_user(user_id, &lane)
                        .await,
                )
            }));
        }
        barrier.wait().await;

        let mut matching_removals = 0;
        for task in tasks {
            let (user_id, removed) = task.await.unwrap();
            if removed {
                assert_eq!(user_id, "user-1", "a cross-user clear must never win");
                matching_removals += 1;
            }
        }
        assert_eq!(
            matching_removals, 1,
            "concurrent idempotent clears must remove the holder exactly once"
        );

        let holders = state.control_holders.lock().await;
        assert!(!holders.contains_key(&lane));
        assert_eq!(
            holders
                .get(&unrelated_lane)
                .map(|holder| (holder.user_id.as_str(), holder.connection_id.as_str())),
            Some(("user-2", "viewer-2"))
        );
    }

    #[tokio::test]
    async fn finishing_old_return_control_revocation_preserves_replacement_generation() {
        let lane = BrowserLaneId::new();
        let authority = Arc::new(FakeAuthority {
            tokens: ViewerTokenService::new(Arc::new(ManualClock::new(100)), 1_000),
            lane: lane.clone(),
            identity_mode: BrowserIdentityMode::Primary,
            observed_actions: Default::default(),
            control_attempts: Default::default(),
            input_attempts: Default::default(),
        });
        let state = BrowserViewerState::with_authority(authority, ws_auth(), false);
        let old_claim = state
            .claim_control_holder("user-1", &lane, "old-viewer")
            .await
            .unwrap();
        let revocation = state.begin_control_revocation("user-1", &lane).await;

        {
            let mut holders = state.control_holders.lock().await;
            holders
                .get_mut(&lane)
                .expect("old holder exists")
                .expires_at_ms = 0;
        }
        let replacement = state
            .claim_control_holder("user-1", &lane, "replacement-viewer")
            .await
            .unwrap();
        assert_ne!(old_claim.generation, replacement.generation);

        assert!(
            !state.finish_control_revocation(revocation).await,
            "completion for the old generation must not revoke a replacement"
        );
        let holders = state.control_holders.lock().await;
        let current = holders.get(&lane).expect("replacement remains installed");
        assert_eq!(current.generation, replacement.generation);
        assert_eq!(current.connection_id, "replacement-viewer");
        assert!(!current.revoked);
    }

    #[tokio::test]
    async fn returned_control_blocks_automatic_reacquire_until_explicit_takeover() {
        let lane = BrowserLaneId::new();
        let authority = Arc::new(FakeAuthority {
            tokens: ViewerTokenService::new(Arc::new(ManualClock::new(100)), 1_000),
            lane: lane.clone(),
            identity_mode: BrowserIdentityMode::Primary,
            observed_actions: Default::default(),
            control_attempts: Default::default(),
            input_attempts: Default::default(),
        });
        let state = BrowserViewerState::with_authority(authority.clone(), ws_auth(), false);
        let authorized = AuthorizedViewer {
            user_id: "user-1".to_owned(),
            lane_id: lane.clone(),
            identity_mode: BrowserIdentityMode::Primary,
            selected_protocol: None,
            auth_token: "app-auth".to_owned(),
        };
        let mut connection = ViewerConnection::new();
        let claim = state
            .claim_control_holder("user-1", &lane, &connection.connection_id)
            .await
            .unwrap();
        let lease = ControlLease {
            lease_id: "lease".to_owned(),
            lane_id: lane.clone(),
            user_id: "user-1".to_owned(),
            issued_at_ms: 0,
            renewed_at_ms: 0,
            expires_at_ms: u64::MAX,
        };
        assert!(state.update_control_holder(&lane, &claim, &lease).await);
        connection.control_claim = Some(claim);
        connection.control_lease = Some(lease);

        assert!(
            state
                .return_control_and_revoke("user-1", &lane)
                .await
                .unwrap()
        );
        let automatic = ensure_control(
            &state,
            &authorized,
            &mut connection,
            ControlAcquisition::Automatic,
        )
        .await
        .unwrap_err();
        assert_eq!(automatic.code, BrowserErrorCode::OperationNotAllowed);
        assert_eq!(
            authority
                .control_attempts
                .load(std::sync::atomic::Ordering::Acquire),
            0,
            "automatic input must not silently reacquire after explicit return"
        );

        ensure_control(
            &state,
            &authorized,
            &mut connection,
            ControlAcquisition::ExplicitTakeover,
        )
        .await
        .unwrap();
        assert!(connection.control_lease.is_some());
        assert_eq!(
            authority
                .control_attempts
                .load(std::sync::atomic::Ordering::Acquire),
            1
        );
    }

    #[tokio::test]
    async fn stale_socket_disconnect_cannot_release_replacement_holder() {
        let lane = BrowserLaneId::new();
        let authority = Arc::new(FakeAuthority {
            tokens: ViewerTokenService::new(Arc::new(ManualClock::new(100)), 1_000),
            lane: lane.clone(),
            identity_mode: BrowserIdentityMode::Primary,
            observed_actions: Default::default(),
            control_attempts: Default::default(),
            input_attempts: Default::default(),
        });
        let state = BrowserViewerState::with_authority(authority, ws_auth(), false);
        let old_claim = state
            .claim_control_holder("user-1", &lane, "old-viewer")
            .await
            .unwrap();
        {
            let mut holders = state.control_holders.lock().await;
            holders
                .get_mut(&lane)
                .expect("old holder exists")
                .expires_at_ms = 0;
        }
        let replacement = state
            .claim_control_holder("user-1", &lane, "replacement-viewer")
            .await
            .unwrap();
        assert!(!state.release_control_holder(&lane, &old_claim).await);
        let holders = state.control_holders.lock().await;
        assert_eq!(
            holders.get(&lane).map(|holder| holder.generation),
            Some(replacement.generation)
        );
    }

    #[tokio::test]
    async fn crawl_viewers_are_observe_only_and_never_reach_control_authority() {
        for identity_mode in [
            BrowserIdentityMode::Anonymous,
            BrowserIdentityMode::AuthenticatedReplica,
        ] {
            let lane = BrowserLaneId::new();
            let authority = Arc::new(FakeAuthority {
                tokens: ViewerTokenService::new(
                    Arc::new(ManualClock::new(100)),
                    1_000,
                ),
                lane: lane.clone(),
                identity_mode,
                observed_actions: Default::default(),
                control_attempts: Default::default(),
                input_attempts: Default::default(),
            });
            let state =
                BrowserViewerState::with_authority(authority.clone(), ws_auth(), false);
            let authorized = AuthorizedViewer {
                user_id: "user-1".to_owned(),
                lane_id: lane.clone(),
                identity_mode,
                selected_protocol: None,
                auth_token: "app-auth".to_owned(),
            };

            for command in [
                ViewerCommand::Takeover,
                ViewerCommand::Input {
                    input: json!({"kind": "text", "text": "blocked"}),
                    frame_id: None,
                    frame_version: None,
                },
                ViewerCommand::Navigate {
                    url: "https://example.test/blocked".to_owned(),
                },
                ViewerCommand::Back,
                ViewerCommand::Forward,
                ViewerCommand::Reload,
                ViewerCommand::SelectTab {
                    tab_id: "tab-1".to_owned(),
                },
            ] {
                assert!(command.requires_user_control());
                let error = ensure_viewer_control_allowed(&authorized).unwrap_err();
                assert_eq!(error.code, BrowserErrorCode::NeedsPrimaryIdentity);
                assert_eq!(error.lane_id.as_ref(), Some(&lane));
            }

            assert!(!ViewerCommand::Observe { lane_id: None }.requires_user_control());
            assert!(!ViewerCommand::Heartbeat { lane_id: None }.requires_user_control());
            assert!(!ViewerCommand::ReturnControl.requires_user_control());
            authority
                .viewer_heartbeat(&authorized.user_id, &authorized.lane_id)
                .await
                .unwrap();

            let mut connection = ViewerConnection::new();
            let error = ensure_control(
                &state,
                &authorized,
                &mut connection,
                ControlAcquisition::ExplicitTakeover,
            )
            .await
            .unwrap_err();
            assert_eq!(error.code, BrowserErrorCode::NeedsPrimaryIdentity);
            assert!(connection.control_lease.is_none());
            assert_eq!(
                authority
                    .control_attempts
                    .load(std::sync::atomic::Ordering::Acquire),
                0,
                "{identity_mode:?} takeover must fail before calling the Hub control authority"
            );
            assert_eq!(
                authority
                    .input_attempts
                    .load(std::sync::atomic::Ordering::Acquire),
                0,
                "{identity_mode:?} viewer input must never dispatch"
            );
        }
    }

    #[tokio::test]
    async fn primary_and_isolated_viewers_can_still_take_control() {
        for identity_mode in [
            BrowserIdentityMode::Primary,
            BrowserIdentityMode::Isolated,
        ] {
            let lane = BrowserLaneId::new();
            let authority = Arc::new(FakeAuthority {
                tokens: ViewerTokenService::new(
                    Arc::new(ManualClock::new(100)),
                    1_000,
                ),
                lane: lane.clone(),
                identity_mode,
                observed_actions: Default::default(),
                control_attempts: Default::default(),
                input_attempts: Default::default(),
            });
            let state =
                BrowserViewerState::with_authority(authority.clone(), ws_auth(), false);
            let authorized = AuthorizedViewer {
                user_id: "user-1".to_owned(),
                lane_id: lane,
                identity_mode,
                selected_protocol: None,
                auth_token: "app-auth".to_owned(),
            };
            let mut connection = ViewerConnection::new();

            ensure_control(
                &state,
                &authorized,
                &mut connection,
                ControlAcquisition::ExplicitTakeover,
            )
                .await
                .unwrap();
            assert!(connection.control_lease.is_some());
            assert_eq!(
                authority
                    .control_attempts
                    .load(std::sync::atomic::Ordering::Acquire),
                1,
                "{identity_mode:?} viewer control must remain available"
            );
        }
    }

    #[test]
    fn jpeg_parser_and_input_bounds_fail_closed() {
        // Minimal SOI + baseline SOF0 segment: 16x8.
        let jpeg = [
            0xff, 0xd8, 0xff, 0xc0, 0x00, 0x0b, 0x08, 0x00, 0x08, 0x00, 0x10, 0x01,
            0x01, 0x11, 0x00, 0xff, 0xd9,
        ];
        assert_eq!(jpeg_dimensions(&jpeg), Some((16, 8)));
        assert!(jpeg_dimensions(b"not-jpeg").is_none());
        let encoded = base64::engine::general_purpose::STANDARD.encode(jpeg);
        let frame = BrowserOperationResult {
            output: json!({
                "media_type": "image/jpeg",
                "data": encoded,
                "width": 16,
                "height": 8,
                "target_id": "ABCDEF0123456789",
            }),
            ..Default::default()
        };
        let (_, width, height, target_id) = decode_jpeg_result(&frame).unwrap();
        assert_eq!((width, height), (16, 8));
        assert_eq!(target_id, "ABCDEF0123456789");
        let unbound_frame = BrowserOperationResult {
            output: json!({
                "media_type": "image/jpeg",
                "data": base64::engine::general_purpose::STANDARD.encode(jpeg),
                "width": 16,
                "height": 8,
            }),
            ..Default::default()
        };
        assert!(decode_jpeg_result(&unbound_frame).is_err());
        assert!(validate_point(16.0, 8.0, (16, 8)).is_ok());
        assert!(validate_point(16.1, 8.0, (16, 8)).is_err());
        assert!(validate_navigation_url("javascript:alert(1)").is_err());
        let pointer_down = validate_input(
            json!({
                "kind": "pointer",
                "action": "down",
                "x": 4.0,
                "y": 5.0,
                "button": 0,
                "buttons": 1,
                "modifiers": {"alt": false, "ctrl": false, "meta": false, "shift": false}
            }),
            (16, 8),
        )
        .unwrap();
        let encoded = serde_json::to_value(pointer_down).unwrap();
        assert_eq!(encoded["kind"], "pointer");
        assert_eq!(encoded["action"], "down");

        // Keep an otherwise unrelated model type referenced so schema drift in
        // the platform snapshot is caught by this module's test build.
        let _capacity = BrowserCapacitySnapshot {
            active: 0,
            queued: 0,
            max_active: 1,
            max_open_lanes: 1,
            recommended_concurrency: 1,
            reason_code: None,
        };
    }

    #[test]
    fn displayed_frame_tokens_resolve_exact_published_target_and_dimensions() {
        let mut published = PublishedFrameBindings::new("viewer-connection".to_owned());
        let first = published.reserve(800, 600, "target-old".to_owned()).unwrap();
        published.commit(first.clone());
        let newest = published
            .reserve(1600, 1200, "target-new".to_owned())
            .unwrap();
        published.commit(newest.clone());

        let resolved = published
            .resolve(Some(&first.frame_id), Some(first.frame_version))
            .unwrap();
        assert_eq!(resolved.target_id, "target-old");
        assert_eq!((resolved.width, resolved.height), (800, 600));
        assert_ne!(resolved.frame_version, newest.frame_version);

        let metadata_authority = FakeAuthority {
            tokens: ViewerTokenService::new(Arc::new(ManualClock::new(100)), 1_000),
            lane: BrowserLaneId::new(),
            identity_mode: BrowserIdentityMode::Primary,
            observed_actions: Default::default(),
            control_attempts: Default::default(),
            input_attempts: Default::default(),
        };
        let metadata = viewer_metadata(
            "frame",
            &metadata_authority.snapshot(),
            Some((first.width, first.height)),
            Some("screencast"),
            Some(&first.target_id),
            Some(&first),
        );
        assert_eq!(metadata["frame_id"], first.frame_id);
        assert_eq!(metadata["frame_version"], first.frame_version);
        assert!(
            !metadata.to_string().contains("target-old"),
            "opaque frame metadata must not expose the raw browser target"
        );
    }

    #[test]
    fn displayed_frame_tokens_fail_closed_when_missing_cross_socket_or_evicted() {
        let mut published = PublishedFrameBindings::new("viewer-a".to_owned());
        let first = published.reserve(640, 480, "target-first".to_owned()).unwrap();
        published.commit(first.clone());

        for (frame_id, frame_version) in [
            (None, Some(first.frame_version)),
            (Some(first.frame_id.as_str()), None),
            (Some("viewer-b"), Some(first.frame_version)),
            (Some(first.frame_id.as_str()), Some(first.frame_version + 1)),
        ] {
            let error = published.resolve(frame_id, frame_version).unwrap_err();
            assert_eq!(error.code, BrowserErrorCode::StaleLaneRef);
            assert!(error.retryable);
            assert_eq!(error.metadata["reason"], "viewer_frame_binding_stale");
        }

        for index in 0..MAX_PUBLISHED_FRAME_BINDINGS {
            let frame = published
                .reserve(1, 1, format!("target-{index}"))
                .unwrap();
            published.commit(frame);
        }
        let error = published
            .resolve(Some(&first.frame_id), Some(first.frame_version))
            .unwrap_err();
        assert_eq!(error.code, BrowserErrorCode::StaleLaneRef);
        assert_eq!(published.frames.len(), MAX_PUBLISHED_FRAME_BINDINGS);
    }

    #[test]
    fn reserved_frame_token_is_unresolvable_until_jpeg_send_is_committed() {
        let mut published = PublishedFrameBindings::new("viewer-connection".to_owned());
        let visible = published
            .reserve(800, 600, "target-visible".to_owned())
            .unwrap();
        published.commit(visible.clone());
        let in_flight = published
            .reserve(1600, 1200, "target-not-yet-sent".to_owned())
            .unwrap();

        // This models the precise server ordering after metadata was sent but
        // before socket.send(Binary) completed. The new token is fail-closed,
        // while input from the frame the user can still see remains routable.
        let error = published
            .resolve(Some(&in_flight.frame_id), Some(in_flight.frame_version))
            .unwrap_err();
        assert_eq!(error.code, BrowserErrorCode::StaleLaneRef);
        assert_eq!(
            published
                .resolve(Some(&visible.frame_id), Some(visible.frame_version))
                .unwrap()
                .target_id,
            "target-visible"
        );

        // Only the successful binary-send branch performs this commit.
        published.commit(in_flight.clone());
        assert_eq!(
            published
                .resolve(Some(&in_flight.frame_id), Some(in_flight.frame_version))
                .unwrap()
                .target_id,
            "target-not-yet-sent"
        );
    }

    #[test]
    fn coordinate_input_wire_contract_accepts_frame_binding_but_keeps_legacy_parseable() {
        let bound: ViewerCommand = serde_json::from_value(json!({
            "type": "input",
            "frame_id": "viewer-connection",
            "frame_version": 7,
            "input": {
                "kind": "pointer",
                "action": "down",
                "x": 12,
                "y": 34,
                "button": 0,
                "buttons": 1
            }
        }))
        .unwrap();
        assert!(matches!(
            bound,
            ViewerCommand::Input {
                frame_id: Some(ref frame_id),
                frame_version: Some(7),
                ..
            } if frame_id == "viewer-connection"
        ));

        let legacy: ViewerCommand = serde_json::from_value(json!({
            "type": "input",
            "input": {
                "kind": "pointer",
                "action": "down",
                "x": 12,
                "y": 34,
                "button": 0,
                "buttons": 1
            }
        }))
        .unwrap();
        assert!(matches!(
            legacy,
            ViewerCommand::Input {
                frame_id: None,
                frame_version: None,
                ..
            }
        ));
        let ViewerCommand::Input {
            mut input,
            frame_id,
            frame_version,
        } = legacy else {
            unreachable!()
        };
        let empty = PublishedFrameBindings::new("viewer-connection".to_owned());
        let error = resolve_viewer_input_frame(
            &mut input,
            frame_id,
            frame_version,
            &empty,
        )
        .unwrap_err();
        assert_eq!(error.code, BrowserErrorCode::StaleLaneRef);

        let mut nested = json!({
            "kind": "pointer",
            "action": "down",
            "x": 12,
            "y": 34,
            "button": 0,
            "buttons": 1,
            "frame_id": "viewer-connection",
            "frame_version": 7
        });
        let mut nested_published = PublishedFrameBindings::new("viewer-connection".to_owned());
        let nested_frame = nested_published
            .reserve(800, 600, "nested-target".to_owned())
            .unwrap();
        assert_eq!(nested_frame.frame_version, 1);
        nested["frame_version"] = 1.into();
        nested_published.commit(nested_frame);
        assert_eq!(
            resolve_viewer_input_frame(&mut nested, None, None, &nested_published)
                .unwrap()
                .target_id,
            "nested-target"
        );
        assert!(nested.get("frame_id").is_none());
        assert!(nested.get("frame_version").is_none());
    }

    #[test]
    fn inbound_rate_limit_is_bounded_per_connection() {
        let mut rate = MessageRate::new();
        for _ in 0..MAX_INBOUND_MESSAGES_PER_SECOND {
            assert!(rate.allow());
        }
        assert!(!rate.allow());
    }

    #[test]
    fn viewer_metadata_projects_url_credentials_tokens_and_fragment() {
        let authority = FakeAuthority {
            tokens: ViewerTokenService::new(Arc::new(ManualClock::new(100)), 1_000),
            lane: BrowserLaneId::new(),
            identity_mode: BrowserIdentityMode::Primary,
            observed_actions: Default::default(),
            control_attempts: Default::default(),
            input_attempts: Default::default(),
        };
        let mut snapshot = authority.snapshot();
        let exact_url = "https://viewer:password@example.test/page?safe=yes&ID-Token=id-secret&request_signature=signature-secret#private-fragment".to_owned();
        snapshot.tabs = vec![nomifun_browser_platform::BrowserTabSnapshot {
            tab_id: "tab-1".to_owned(),
            target_id: "target-1".to_owned(),
            title: Some("Viewer page".to_owned()),
            url: Some(exact_url.clone()),
            active: true,
            crashed: false,
        }];
        snapshot.active_tab_id = Some("tab-1".to_owned());

        let metadata = viewer_metadata(
            "frame",
            &snapshot,
            Some((800, 600)),
            Some("screencast"),
            Some("target-1"),
            None,
        );
        assert_eq!(
            metadata["url"],
            "https://example.test/page"
        );
        assert_eq!(
            snapshot.tabs[0].url.as_deref(),
            Some(exact_url.as_str()),
            "viewer projection must not alter the authoritative snapshot"
        );
        let encoded = metadata.to_string();
        for secret in [
            "viewer",
            "password",
            "id-secret",
            "signature-secret",
            "private-fragment",
        ] {
            assert!(!encoded.contains(secret), "viewer metadata leaked {secret}");
        }
    }

    #[test]
    fn viewer_metadata_fails_closed_for_malformed_url() {
        let authority = FakeAuthority {
            tokens: ViewerTokenService::new(Arc::new(ManualClock::new(100)), 1_000),
            lane: BrowserLaneId::new(),
            identity_mode: BrowserIdentityMode::Primary,
            observed_actions: Default::default(),
            control_attempts: Default::default(),
            input_attempts: Default::default(),
        };
        let mut snapshot = authority.snapshot();
        snapshot.tabs = vec![nomifun_browser_platform::BrowserTabSnapshot {
            tab_id: "tab-malformed".to_owned(),
            target_id: "target-malformed".to_owned(),
            title: None,
            url: Some("not a url?token=viewer-secret".to_owned()),
            active: true,
            crashed: false,
        }];
        snapshot.active_tab_id = Some("tab-malformed".to_owned());

        let metadata = viewer_metadata("ready", &snapshot, None, None, None, None);
        assert_eq!(metadata["url"], "[REDACTED_URL]");
        assert!(!metadata.to_string().contains("viewer-secret"));
    }

    #[tokio::test]
    async fn screencast_failure_stops_stream_and_uses_screenshot_fallback() {
        let lane = BrowserLaneId::new();
        let authority = Arc::new(FakeAuthority {
            tokens: ViewerTokenService::new(Arc::new(ManualClock::new(100)), 1_000),
            lane: lane.clone(),
            identity_mode: BrowserIdentityMode::Primary,
            observed_actions: Default::default(),
            control_attempts: Default::default(),
            input_attempts: Default::default(),
        });
        let (frame_tx, frame_rx) = watch::channel(None);
        let producer = tokio::spawn(frame_producer(
            authority.clone(),
            "user-1".to_owned(),
            lane,
            frame_tx,
        ));

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let saw_fallback = authority
                    .observed_actions
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .iter()
                    .any(|action| action == "viewer_screenshot");
                if saw_fallback {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        drop(frame_rx);
        tokio::time::timeout(Duration::from_secs(1), producer)
            .await
            .unwrap()
            .unwrap();

        let actions = authority
            .observed_actions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(
            actions.first().map(String::as_str),
            Some("viewer_screencast_frame")
        );
        assert!(actions.iter().any(|action| action == "viewer_screencast_stop"));
        assert!(actions.iter().any(|action| action == "viewer_screenshot"));
    }
}
