//! Compatibility endpoints for opening a Primary identity-setup Browser Lane.
//!
//! The legacy implementation launched a second private Chromium instance and
//! owned its profile in this route. That violated the process-wide browser
//! authority. These endpoints now allocate a normal Hub owner lease and a
//! Primary Lane in the external managed Chromium window. This flow is only for
//! one-time authentication setup; `/browser` remains status-only and does not
//! expose page input or Agent takeover controls.

#![cfg(feature = "browser-use")]

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::State;
use nomifun_api_types::ApiResponse;
use nomifun_browser_platform::{
    BrowserIdentityMode, BrowserLaneId, BrowserOperationKind, BrowserSessionHub,
    BrowserSurface, CallerIdentity, LaneLifecycleState, OpenLaneOutcome, OwnerLeaseId,
};
use nomifun_db::IClientPreferenceRepository;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, OnceCell};

/// How often the queued-login watcher checks whether the Lane reached Running
/// so it can foreground the (otherwise headless) window (F29).
const QUEUED_FOREGROUND_POLL: Duration = Duration::from_millis(250);

/// A failed exact-owner cleanup is already retained by the Hub's single
/// autonomous cleanup supervisor.  Throttle request-driven confirmation so a
/// burst of repeated login clicks cannot turn one stuck driver into thousands
/// of concurrent/repeated cleanup calls.
const LOGIN_CLEANUP_RETRY_BACKOFF: Duration = Duration::from_secs(1);

#[derive(Clone)]
pub(crate) struct BrowserLoginState {
    inner: Arc<BrowserLoginInner>,
}

struct BrowserLoginInner {
    hub: Option<Arc<BrowserSessionHub>>,
    user_id: Arc<str>,
    /// The trusted Chrome source is host policy resolved by the process
    /// composition root at startup and frozen into the Hub's engine template
    /// (`load_browser_startup_preferences` in services.rs). This endpoint
    /// reads the same preference once (boot snapshot) so responses can REPORT
    /// the effective source (F67) — the request body `source` stays ignored
    /// by design, and a live preference toggle only takes effect after an app
    /// restart.
    source_prefs: Option<Arc<dyn IClientPreferenceRepository>>,
    effective_source: OnceCell<Arc<str>>,
    session: Mutex<Option<LoginLane>>,
}

struct LoginLane {
    /// `None` while an issued owner is being bound/opened, or when opening was
    /// cancelled before a Lane id was published.  The owner lease itself is
    /// still cleanup authority and must remain fenced until exact revocation
    /// succeeds.
    lane_id: Option<BrowserLaneId>,
    owner_lease_id: OwnerLeaseId,
    hub: Arc<BrowserSessionHub>,
    renewal_task: Option<tokio::task::JoinHandle<()>>,
    /// Present while the Lane was admitted Queued: foregrounds the window
    /// once the scheduler promotes it to Running (F29).
    foreground_task: Option<tokio::task::JoinHandle<()>>,
    /// Canonical Primary identity generation at open time. `saved` on close
    /// reports whether a FRESH capture was committed during this session, not
    /// whether lease revocation succeeded (F60).
    identity_generation_at_open: u64,
    /// Explicit cleanup disarms this flag only after the Hub proves the exact
    /// owner is gone.  Until then Drop remains the final best-effort fallback.
    cleanup_armed: bool,
    /// Fail-closed replacement fence.  A pending authority is never replaced
    /// by a fresh runtime/lease, even when its Lane has already disappeared
    /// from the public inventory.
    cleanup_pending: bool,
    cleanup_retry_not_before: Option<std::time::Instant>,
}

impl LoginLane {
    fn pending_authority(
        owner_lease_id: OwnerLeaseId,
        hub: Arc<BrowserSessionHub>,
        identity_generation_at_open: u64,
    ) -> Self {
        Self {
            lane_id: None,
            owner_lease_id,
            hub,
            renewal_task: None,
            foreground_task: None,
            identity_generation_at_open,
            cleanup_armed: true,
            // The authority is installed before bind/open awaits.  If that
            // request is cancelled, the next request must clean this owner,
            // not assume the slot is free and mint another runtime.
            cleanup_pending: true,
            cleanup_retry_not_before: None,
        }
    }

    fn stop_background_tasks(&mut self) {
        if let Some(task) = self.renewal_task.take() {
            task.abort();
        }
        if let Some(task) = self.foreground_task.take() {
            task.abort();
        }
    }

    fn cleanup_retry_due(&self) -> bool {
        self.cleanup_retry_not_before
            .is_none_or(|deadline| std::time::Instant::now() >= deadline)
    }

    fn mark_active(&mut self) {
        self.cleanup_pending = false;
        self.cleanup_retry_not_before = None;
    }

    fn disarm_cleanup(&mut self) {
        self.stop_background_tasks();
        self.cleanup_armed = false;
        self.cleanup_pending = false;
        self.cleanup_retry_not_before = None;
    }
}

impl Drop for LoginLane {
    fn drop(&mut self) {
        self.stop_background_tasks();
        if !self.cleanup_armed {
            return;
        }
        // `BrowserLoginState` is router state and is normally dropped only
        // during application teardown.  Do not rely on the next status call
        // to revoke the owner lease: arrange a best-effort asynchronous
        // cleanup when the state disappears, while keeping Drop itself
        // non-blocking.
        let hub = Arc::clone(&self.hub);
        let lease_id = self.owner_lease_id.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let _ = hub.revoke_owner_lease(&lease_id).await;
            });
        }
    }
}

/// Revoke the exact tracked login owner without ever exposing an empty router
/// slot before cleanup succeeds.  The Hub owns durable retry authority; this
/// helper only confirms completion and maintains the route-level replacement
/// fence.
async fn clear_tracked_login(session: &mut Option<LoginLane>) -> bool {
    let Some(authority) = session.as_mut() else {
        return true;
    };
    authority.stop_background_tasks();
    authority.cleanup_pending = true;
    if !authority.cleanup_retry_due() {
        return false;
    }

    let result = authority
        .hub
        .revoke_owner_lease(&authority.owner_lease_id)
        .await;
    match result {
        Ok(_) => {
            authority.disarm_cleanup();
            // Drop only after disarming; otherwise LoginLane::drop would spawn
            // a redundant second revoke and could multiply cleanup workers.
            session.take();
            true
        }
        Err(error) => {
            authority.cleanup_retry_not_before =
                Some(std::time::Instant::now() + LOGIN_CLEANUP_RETRY_BACKOFF);
            tracing::warn!(
                code = ?error.code,
                lease_id = %authority.owner_lease_id,
                "login browser owner cleanup remains pending; replacement is fenced"
            );
            false
        }
    }
}

impl BrowserLoginState {
    pub(crate) fn new(
        hub: Option<Arc<BrowserSessionHub>>,
        user_id: Arc<str>,
        source_prefs: Option<Arc<dyn IClientPreferenceRepository>>,
    ) -> Self {
        let state = Self {
            inner: Arc::new(BrowserLoginInner {
                hub,
                user_id,
                source_prefs,
                effective_source: OnceCell::new(),
                session: Mutex::new(None),
            }),
        };
        // Snapshot the effective source at construction (process boot) so it
        // mirrors the source frozen into the Hub's engine template, not a
        // later live toggle.
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let prime = state.clone();
            handle.spawn(async move {
                let _ = prime.effective_source().await;
            });
        }
        state
    }

    async fn effective_source(&self) -> Arc<str> {
        self.inner
            .effective_source
            .get_or_init(|| async {
                let raw = match self.inner.source_prefs.as_deref() {
                    Some(repo) => repo
                        .get_by_keys(&["agent.browserUse.source"])
                        .await
                        .ok()
                        .and_then(|rows| rows.into_iter().next())
                        .map(|row| row.value),
                    None => None,
                };
                Arc::from(normalize_chrome_source(raw.as_deref()))
            })
            .await
            .clone()
    }
}

/// Mirror the boot-time reader (`load_browser_startup_preferences`: missing or
/// blank rows mean "system") composed with `ChromeSource::from_source_str`
/// ("system", case-insensitive, selects the system Chrome/Edge; every other
/// value resolves to the managed Chrome-for-Testing).
fn normalize_chrome_source(value: Option<&str>) -> &'static str {
    let value = value.map(|value| value.trim().trim_matches('"')).unwrap_or("");
    if value.is_empty() || value.eq_ignore_ascii_case("system") {
        "system"
    } else {
        "managed"
    }
}

#[derive(Deserialize)]
pub(crate) struct OpenLoginBody {
    // Kept for wire compatibility. The process composition root resolves the
    // trusted Chrome source; a request body cannot replace host policy. The
    // response instead reports the EFFECTIVE source (F67).
    #[serde(default, rename = "source")]
    _source: String,
}

#[derive(Serialize)]
pub(crate) struct LoginStatus {
    active: bool,
    message: Option<String>,
    /// Whether a fresh Primary identity capture was committed to the vault
    /// during this login session. NOT lease-revocation success (F60): a
    /// manual login that triggered no capture honestly reports `false` (the
    /// persistent on-disk profile still retains the login for reuse).
    saved: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    lane_id: Option<BrowserLaneId>,
    /// The effective host-policy Chrome source ("managed" | "system") the
    /// login browser actually uses; the request body cannot change it.
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<String>,
}

/// How an open request must treat the Lane of an already-tracked login
/// session, based on its current Hub lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReopenDisposition {
    /// Genuinely pending (queued for capacity, or its start is in flight):
    /// keep the session and report `queued` — the open-time watcher
    /// foregrounds the window once the Lane runs (F29).
    ReportQueued,
    /// Running: reveal the existing managed window.
    Foreground,
    /// Dead or dying. A Failed/Frozen/Stopping Lane will never be promoted to
    /// Running, so reporting it `queued` would livelock the login feature
    /// forever (the queued watcher has already given up on these states).
    /// Close the dead Lane with its owner lease and open a fresh one instead
    /// (the pre-F29 self-heal).
    ReplaceDeadLane,
}

fn reopen_disposition(state: LaneLifecycleState) -> ReopenDisposition {
    match state {
        LaneLifecycleState::Queued | LaneLifecycleState::Starting => {
            ReopenDisposition::ReportQueued
        }
        LaneLifecycleState::Running => ReopenDisposition::Foreground,
        LaneLifecycleState::Frozen
        | LaneLifecycleState::Stopping
        | LaneLifecycleState::Failed => ReopenDisposition::ReplaceDeadLane,
    }
}

pub(crate) async fn open_browser_login(
    State(state): State<BrowserLoginState>,
    Json(_body): Json<OpenLoginBody>,
) -> Json<ApiResponse<LoginStatus>> {
    let Some(hub) = state.inner.hub.clone() else {
        return login_response(false, "launch_failed:browser_not_supported", false, None, None);
    };
    let source = Some(state.effective_source().await.to_string());

    let mut session = state.inner.session.lock().await;
    let reusable = session
        .as_ref()
        .filter(|existing| !existing.cleanup_pending)
        .and_then(|existing| {
            existing
                .lane_id
                .clone()
                .map(|lane_id| (lane_id, existing.owner_lease_id.clone()))
        });
    if let Some((existing_lane_id, existing_lease_id)) = reusable {
        let existing_snapshot = hub
            .list_lanes()
            .await
            .into_iter()
            .find(|lane| lane.lane_id == existing_lane_id);
        if let Some(snapshot) = existing_snapshot
            && hub.renew_owner_lease(&existing_lease_id).is_ok()
        {
            match reopen_disposition(snapshot.lifecycle_state) {
                // A Lane that is still queued (or starting) cannot be
                // foregrounded — the Hub only foregrounds Running lanes — and
                // a failed foreground here used to revoke the pending login
                // (F29). Report the queue state instead; the watcher spawned
                // at open time foregrounds the window once the Lane runs.
                ReopenDisposition::ReportQueued => {
                    return login_response(
                        true,
                        "queued",
                        false,
                        Some(existing_lane_id),
                        source,
                    );
                }
                ReopenDisposition::Foreground => {
                    if hub
                        .foreground_lane_for_user(
                            state.inner.user_id.as_ref(),
                            &existing_lane_id,
                        )
                        .await
                        .is_err()
                    {
                        let cleaned = clear_tracked_login(&mut session).await;
                        return login_response(
                            false,
                            if cleaned {
                                "launch_failed:browser_unavailable"
                            } else {
                                "cleanup_pending"
                            },
                            false,
                            None,
                            source,
                        );
                    }
                    return login_response(
                        true,
                        "already_open",
                        false,
                        Some(existing_lane_id),
                        source,
                    );
                }
                // Fall through to the stale-session cleanup below: revoking
                // the owner lease closes the dead Lane, and a fresh Lane is
                // opened in this same request (pre-F29 self-heal).
                ReopenDisposition::ReplaceDeadLane => {}
            }
        }
    }
    if session.is_some() {
        // `renew` may already have removed an expired lease. Hub revocation is
        // still authoritative for any Lane that survived that expiry.
        if !clear_tracked_login(&mut session).await {
            return login_response(false, "cleanup_pending", false, None, source);
        }
    }

    let runtime_instance_id = format!("browser-login-{}", uuid::Uuid::now_v7());
    let lease = match hub.issue_owner_lease(
        state.inner.user_id.to_string(),
        None,
        runtime_instance_id.clone(),
    ) {
        Ok(lease) => lease,
        Err(_) => {
            return login_response(false, "launch_failed:owner_lease", false, None, source);
        }
    };
    let identity_generation_at_open = current_identity_generation(&hub);
    // Publish cleanup authority before any bind/open await.  Cancellation of
    // this request can therefore never make the router look idle while this
    // exact owner may still be allocating or cleaning browser resources.
    *session = Some(LoginLane::pending_authority(
        lease.lease_id.clone(),
        Arc::clone(&hub),
        identity_generation_at_open,
    ));
    let caller = CallerIdentity {
        user_id: state.inner.user_id.to_string(),
        conversation_id: None,
        runtime_instance_id,
        agent_id: None,
        companion_id: None,
        execution_id: None,
        step_id: None,
        attempt_id: None,
        remote_connection_id: None,
        surface: BrowserSurface::User,
        owner_lease_id: lease.lease_id.clone(),
        // The owner lease is renewable for the duration of the user login
        // flow. The caller snapshot must therefore not retain the original
        // short deadline; Hub validation remains authoritative.
        capability_expires_at_ms: u64::MAX,
        allowed_operations: BTreeSet::from([
            BrowserOperationKind::Manage,
            BrowserOperationKind::Navigate,
            BrowserOperationKind::Observe,
            BrowserOperationKind::Act,
            BrowserOperationKind::Screenshot,
            BrowserOperationKind::Tabs,
        ]),
    };
    let client = match hub.bind(caller) {
        Ok(client) => client,
        Err(_) => {
            let cleaned = clear_tracked_login(&mut session).await;
            return login_response(
                false,
                if cleaned {
                    "launch_failed:invalid_capability"
                } else {
                    "cleanup_pending"
                },
                false,
                None,
                source,
            );
        }
    };
    let outcome = match client
        .open(
            Some("login"),
            BrowserIdentityMode::Primary,
            None,
        )
        .await
    {
        Ok(outcome) => outcome,
        Err(_) => {
            let cleaned = clear_tracked_login(&mut session).await;
            return login_response(
                false,
                if cleaned {
                    "launch_failed:browser_unavailable"
                } else {
                    "cleanup_pending"
                },
                false,
                None,
                source,
            );
        }
    };
    let lane_id = outcome.lane().lane_id.clone();
    session
        .as_mut()
        .expect("login cleanup authority must remain installed")
        .lane_id = Some(lane_id.clone());
    let renewal_period = Duration::from_millis(
        (lease
            .expires_at_ms
            .saturating_sub(lease.issued_at_ms)
            .max(1)
            / 3)
            .max(1),
    );
    let renewal_hub = Arc::clone(&hub);
    let renewal_lease_id = lease.lease_id.clone();
    let renewal_lane_id = lane_id.clone();
    let renewal_task = tokio::spawn(async move {
        loop {
            tokio::time::sleep(renewal_period).await;
            // A user or another management surface may have closed the Lane.
            // Stop renewing an owner lease that no longer owns any resource.
            let still_present = renewal_hub
                .list_lanes()
                .await
                .iter()
                .any(|lane| lane.lane_id == renewal_lane_id);
            if !still_present {
                let _ = renewal_hub
                    .revoke_owner_lease(&renewal_lease_id)
                    .await;
                break;
            }
            if renewal_hub
                .renew_owner_lease(&renewal_lease_id)
                .is_err()
            {
                let _ = renewal_hub
                    .revoke_owner_lease(&renewal_lease_id)
                    .await;
                break;
            }
        }
    });
    session
        .as_mut()
        .expect("login cleanup authority must remain installed")
        .renewal_task = Some(renewal_task);
    let message = match outcome {
        OpenLaneOutcome::Running { .. } => {
            if hub
                .foreground_lane_for_user(state.inner.user_id.as_ref(), &lane_id)
                .await
                .is_err()
            {
                // Opening the sign-in Lane without making its real managed
                // Chromium window visible leaves the explicit login request
                // unusable. Tear down the owner capability so failure cannot
                // strand a live background Lane.
                let cleaned = clear_tracked_login(&mut session).await;
                return login_response(
                    false,
                    if cleaned {
                        "launch_failed:browser_unavailable"
                    } else {
                        "cleanup_pending"
                    },
                    false,
                    None,
                    source,
                );
            }
            "opened"
        }
        // F29: the login Lane was admitted into the scheduler queue. Once it
        // dequeues it would run on the (default headless) Primary Host with
        // no visible window; watch for the Running transition and foreground
        // it so the explicit login request eventually becomes visible.
        OpenLaneOutcome::Queued { .. } => {
            session
                .as_mut()
                .expect("login cleanup authority must remain installed")
                .foreground_task = Some(spawn_foreground_once_running(
                Arc::clone(&hub),
                Arc::clone(&state.inner.user_id),
                lane_id.clone(),
            ));
            "queued"
        }
    };
    session
        .as_mut()
        .expect("login cleanup authority must remain installed")
        .mark_active();
    login_response(true, message, false, Some(lane_id), source)
}

fn spawn_foreground_once_running(
    hub: Arc<BrowserSessionHub>,
    user_id: Arc<str>,
    lane_id: BrowserLaneId,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(QUEUED_FOREGROUND_POLL).await;
            let snapshot = hub
                .list_lanes()
                .await
                .into_iter()
                .find(|lane| lane.lane_id == lane_id);
            let Some(snapshot) = snapshot else {
                // Closed or cancelled while queued: nothing to reveal.
                return;
            };
            match snapshot.lifecycle_state {
                LaneLifecycleState::Running => {
                    if let Err(error) = hub
                        .foreground_lane_for_user(user_id.as_ref(), &lane_id)
                        .await
                    {
                        // Keep the session: the user can click the login
                        // button again (the Running path retries foreground).
                        tracing::warn!(
                            code = ?error.code,
                            %lane_id,
                            "queued login lane started but could not be foregrounded"
                        );
                    }
                    return;
                }
                LaneLifecycleState::Queued | LaneLifecycleState::Starting => {}
                LaneLifecycleState::Frozen
                | LaneLifecycleState::Stopping
                | LaneLifecycleState::Failed => return,
            }
        }
    })
}

fn current_identity_generation(hub: &BrowserSessionHub) -> u64 {
    hub.current_identity_snapshot()
        .ok()
        .flatten()
        .map(|snapshot| snapshot.generation)
        .unwrap_or(0)
}

pub(crate) async fn close_browser_login(
    State(state): State<BrowserLoginState>,
) -> Json<ApiResponse<LoginStatus>> {
    let Some(hub) = state.inner.hub.clone() else {
        return login_response(false, "not_open", false, None, None);
    };
    let source = Some(state.effective_source().await.to_string());
    let mut session = state.inner.session.lock().await;
    let Some(existing) = session.as_ref() else {
        return login_response(false, "not_open", false, None, source);
    };
    // F60/F44 contract: `saved` reports actual vault capture — whether the
    // canonical Primary identity generation advanced (the Hub publishes a
    // generation only after a successful capture + vault persist) during this
    // session — never the lease-revocation result below.
    let saved = hub
        .current_identity_snapshot()
        .ok()
        .flatten()
        .is_some_and(|snapshot| snapshot.generation > existing.identity_generation_at_open);
    if !clear_tracked_login(&mut session).await {
        return login_response(false, "cleanup_pending", saved, None, source);
    }
    login_response(false, "closed", saved, None, source)
}

/// GET /api/browser/login/status — a PURE read (F30).
///
/// This handler must never renew or revoke the owner lease: renewing would
/// make a safe GET extend the lease for as long as the UI polls, and tearing
/// the session down on a single failed renewal closed the user's visible
/// login window mid-login after a laptop sleep. Lease renewal is owned by the
/// session's renewal task; stale-session cleanup is owned by that task and by
/// the next open request.
pub(crate) async fn browser_login_status(
    State(state): State<BrowserLoginState>,
) -> Json<ApiResponse<LoginStatus>> {
    let Some(hub) = state.inner.hub.as_ref() else {
        return login_response(false, "browser_not_supported", false, None, None);
    };
    let source = Some(state.effective_source().await.to_string());
    let session = state.inner.session.lock().await;
    let cleanup_pending = session
        .as_ref()
        .is_some_and(|value| value.cleanup_pending);
    let lane_id = session.as_ref().and_then(|value| value.lane_id.clone());
    drop(session);
    let lanes = hub.list_lanes().await;
    let active = lane_id
        .as_ref()
        .is_some_and(|lane_id| lanes.iter().any(|lane| &lane.lane_id == lane_id));
    login_response(
        active,
        if cleanup_pending {
            "cleanup_pending"
        } else {
            "managed_primary_lane"
        },
        false,
        lane_id.filter(|_| active),
        source,
    )
}

fn login_response(
    active: bool,
    message: &str,
    saved: bool,
    lane_id: Option<BrowserLaneId>,
    source: Option<String>,
) -> Json<ApiResponse<LoginStatus>> {
    Json(ApiResponse::ok(LoginStatus {
        active,
        message: Some(message.to_owned()),
        saved,
        lane_id,
        source,
    }))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;

    use async_trait::async_trait;
    use nomifun_browser_platform::{
        BrowserHostDriver, BrowserHostFactory, BrowserHostId, BrowserLaneDriver,
        BrowserOperation, BrowserOperationResult, BrowserPlatformError,
        DriverOperationContext, HostLaunchRequest, HostLifecycleState, HubConfig,
        LaneLaunchRequest,
    };

    use super::*;

    #[derive(Default)]
    struct FakeProbe {
        foregrounds: AtomicUsize,
        fail_foreground: AtomicBool,
        /// While set, `open_lane` fails on the fake Host: a QUEUED login Lane
        /// promoted by the scheduler then transitions to `Failed` (the Hub
        /// marks a failed start instead of discarding the Lane).
        fail_open_lane: AtomicBool,
        block_open_lane: AtomicBool,
        open_lane_entered: tokio::sync::Notify,
        allow_open_lane: tokio::sync::Notify,
        lane_opens: AtomicUsize,
        lane_close_attempts: AtomicUsize,
        fail_close: AtomicBool,
        block_close: AtomicBool,
        close_entered: tokio::sync::Notify,
        allow_close: tokio::sync::Notify,
        fail_shutdown: AtomicBool,
        shutdown_attempts: AtomicUsize,
    }

    struct FakeFactory {
        probe: Arc<FakeProbe>,
    }
    struct FakeHost {
        id: BrowserHostId,
        epoch: u64,
        headful: bool,
        probe: Arc<FakeProbe>,
    }
    struct FakeLane {
        probe: Arc<FakeProbe>,
    }

    #[async_trait]
    impl BrowserHostFactory for FakeFactory {
        async fn launch(
            &self,
            request: HostLaunchRequest,
        ) -> Result<Arc<dyn BrowserHostDriver>, BrowserPlatformError> {
            Ok(Arc::new(FakeHost {
                id: request.host_id,
                epoch: request.browser_epoch,
                headful: request.headful,
                probe: Arc::clone(&self.probe),
            }))
        }
    }

    #[async_trait]
    impl BrowserHostDriver for FakeHost {
        fn host_id(&self) -> BrowserHostId {
            self.id.clone()
        }

        fn epoch(&self) -> u64 {
            self.epoch
        }

        fn state(&self) -> HostLifecycleState {
            HostLifecycleState::Running
        }

        fn is_headful(&self) -> bool {
            self.headful
        }

        async fn open_lane(
            &self,
            _request: LaneLaunchRequest,
        ) -> Result<Arc<dyn BrowserLaneDriver>, BrowserPlatformError> {
            self.probe.lane_opens.fetch_add(1, Ordering::AcqRel);
            if self.probe.block_open_lane.load(Ordering::Acquire) {
                self.probe.open_lane_entered.notify_one();
                self.probe.allow_open_lane.notified().await;
            }
            if self.probe.fail_open_lane.load(Ordering::Acquire) {
                return Err(BrowserPlatformError::new(
                    nomifun_browser_platform::BrowserErrorCode::BrowserUnavailable,
                    "Synthetic login lane start failure.",
                    true,
                    "Retry the login request.",
                ));
            }
            Ok(Arc::new(FakeLane {
                probe: Arc::clone(&self.probe),
            }))
        }

        async fn reconcile_task_tab_limit(
            &self,
            _task_resource_key: &str,
            _max_task_tabs: usize,
        ) -> Result<(), BrowserPlatformError> {
            // The fake has no tab inventory. Production Hosts implement the
            // same seam with real target reconciliation.
            Ok(())
        }

        async fn shutdown(&self) -> Result<(), BrowserPlatformError> {
            self.probe
                .shutdown_attempts
                .fetch_add(1, Ordering::AcqRel);
            if self.probe.fail_shutdown.load(Ordering::Acquire) {
                return Err(BrowserPlatformError::new(
                    nomifun_browser_platform::BrowserErrorCode::BrowserUnavailable,
                    "Synthetic login host shutdown failure.",
                    true,
                    "Retry the exact login Host shutdown.",
                ));
            }
            Ok(())
        }
    }

    #[async_trait]
    impl BrowserLaneDriver for FakeLane {
        async fn execute(
            &self,
            _operation: BrowserOperation,
            _context: DriverOperationContext,
        ) -> Result<BrowserOperationResult, BrowserPlatformError> {
            Ok(BrowserOperationResult::default())
        }

        async fn close(&self) -> Result<(), BrowserPlatformError> {
            self.probe
                .lane_close_attempts
                .fetch_add(1, Ordering::AcqRel);
            if self.probe.block_close.load(Ordering::Acquire) {
                self.probe.close_entered.notify_one();
                self.probe.allow_close.notified().await;
            }
            if self.probe.fail_close.load(Ordering::Acquire) {
                return Err(BrowserPlatformError::new(
                    nomifun_browser_platform::BrowserErrorCode::BrowserUnavailable,
                    "Synthetic login lane close failure.",
                    true,
                    "Retry the exact login owner cleanup.",
                ));
            }
            Ok(())
        }

        async fn bring_to_front(&self) -> Result<(), BrowserPlatformError> {
            self.probe.foregrounds.fetch_add(1, Ordering::AcqRel);
            if self.probe.fail_foreground.load(Ordering::Acquire) {
                return Err(BrowserPlatformError::new(
                    nomifun_browser_platform::BrowserErrorCode::BrowserUnavailable,
                    "Synthetic login foreground failure.",
                    true,
                    "Retry the login request.",
                ));
            }
            Ok(())
        }
    }

    fn login_hub(config: HubConfig) -> (Arc<BrowserSessionHub>, Arc<FakeProbe>) {
        let probe = Arc::new(FakeProbe::default());
        let hub = Arc::new(BrowserSessionHub::new(
            Arc::new(FakeFactory {
                probe: Arc::clone(&probe),
            }),
            config,
        ));
        (hub, probe)
    }

    fn login_state(hub: &Arc<BrowserSessionHub>) -> BrowserLoginState {
        BrowserLoginState::new(Some(Arc::clone(hub)), Arc::from("user-1"), None)
    }

    fn open_body() -> OpenLoginBody {
        OpenLoginBody {
            _source: "system".to_owned(),
        }
    }

    #[tokio::test]
    async fn login_lane_renews_its_owner_lease_until_explicit_close() {
        let mut config = HubConfig::default();
        config.owner_lease_ttl_ms = 90;
        let (hub, probe) = login_hub(config);
        let state = login_state(&hub);

        let Json(opened) = open_browser_login(
            State(state.clone()),
            Json(open_body()),
        )
        .await;
        let opened = opened.data.expect("login response data");
        assert!(opened.active);
        assert!(opened.lane_id.is_some());
        assert_eq!(probe.foregrounds.load(Ordering::Acquire), 1);

        tokio::time::sleep(Duration::from_millis(220)).await;
        assert_eq!(
            hub.sweep().await.expect("sweep renewed login Lane").closed,
            0
        );
        assert_eq!(hub.list_lanes().await.len(), 1);

        let Json(closed) = close_browser_login(State(state)).await;
        assert!(
            !closed
                .data
                .expect("close response data")
                .active
        );
        assert!(hub.list_lanes().await.is_empty());
        hub.shutdown().await.expect("shutdown fake browser Hub");
    }

    #[tokio::test]
    async fn failed_close_retains_exact_authority_and_fences_a_thousand_reopens() {
        let (hub, probe) = login_hub(HubConfig::default());
        let state = login_state(&hub);

        let Json(opened) = open_browser_login(State(state.clone()), Json(open_body())).await;
        assert!(opened.data.expect("open data").active);
        let original_lease = state
            .inner
            .session
            .lock()
            .await
            .as_ref()
            .expect("tracked login")
            .owner_lease_id
            .clone();
        let opens_before_cleanup = probe.lane_opens.load(Ordering::Acquire);
        let shutdowns_before_cleanup = probe.shutdown_attempts.load(Ordering::Acquire);
        probe.fail_close.store(true, Ordering::Release);
        probe.fail_shutdown.store(true, Ordering::Release);

        let Json(closed) = close_browser_login(State(state.clone())).await;
        let closed = closed.data.expect("close data");
        assert!(!closed.active);
        assert_eq!(closed.message.as_deref(), Some("cleanup_pending"));
        {
            let session = state.inner.session.lock().await;
            let retained = session.as_ref().expect("failed cleanup authority retained");
            assert_eq!(retained.owner_lease_id, original_lease);
            assert!(retained.cleanup_armed);
        assert!(retained.cleanup_pending);
        }
        assert!(
            probe.shutdown_attempts.load(Ordering::Acquire) > shutdowns_before_cleanup,
            "the test must inject a real retained Host-cleanup failure"
        );

        // The first failed revoke installs a retry backoff. A request storm
        // must observe the same fence instead of invoking cleanup (or minting
        // a new runtime/Lane) once per click.
        for _ in 0..1_000 {
            let Json(reopened) =
                open_browser_login(State(state.clone()), Json(open_body())).await;
            let reopened = reopened.data.expect("fenced reopen data");
            assert!(!reopened.active);
            assert_eq!(reopened.message.as_deref(), Some("cleanup_pending"));
        }
        assert_eq!(
            probe.lane_opens.load(Ordering::Acquire),
            opens_before_cleanup,
            "the storm must not open any replacement Lane"
        );
        assert_eq!(
            state
                .inner
                .session
                .lock()
                .await
                .as_ref()
                .expect("authority remains fenced")
                .owner_lease_id,
            original_lease
        );

        // Let the retained Hub cleanup finish, then confirm the exact old
        // owner before opening a replacement. A stale old-owner revoke must
        // not affect the replacement (ABA/exact-owner regression).
        probe.fail_close.store(false, Ordering::Release);
        probe.fail_shutdown.store(false, Ordering::Release);
        tokio::time::sleep(LOGIN_CLEANUP_RETRY_BACKOFF + Duration::from_millis(50)).await;
        let Json(reopened) =
            open_browser_login(State(state.clone()), Json(open_body())).await;
        let reopened = reopened.data.expect("reopen after cleanup");
        assert!(reopened.active);
        let replacement_lane = reopened.lane_id.expect("replacement login Lane");
        let replacement_lease = state
            .inner
            .session
            .lock()
            .await
            .as_ref()
            .expect("replacement tracked")
            .owner_lease_id
            .clone();
        assert_ne!(replacement_lease, original_lease);
        assert!(probe.lane_opens.load(Ordering::Acquire) > opens_before_cleanup);

        hub.revoke_owner_lease(&original_lease)
            .await
            .expect("stale exact-owner revoke is idempotent");
        assert!(
            hub.list_lanes()
                .await
                .iter()
                .any(|lane| lane.lane_id == replacement_lane),
            "a stale cleanup completion must not clear the replacement"
        );

        let _ = close_browser_login(State(state)).await;
        hub.shutdown().await.expect("shutdown fake browser Hub");
    }

    #[tokio::test]
    async fn blocked_close_never_exposes_an_idle_slot_to_concurrent_open() {
        let (hub, probe) = login_hub(HubConfig::default());
        let state = login_state(&hub);
        let Json(opened) = open_browser_login(State(state.clone()), Json(open_body())).await;
        assert!(opened.data.expect("open data").active);
        let opens_before_cleanup = probe.lane_opens.load(Ordering::Acquire);

        probe.block_close.store(true, Ordering::Release);
        let closing_state = state.clone();
        let closing = tokio::spawn(async move { close_browser_login(State(closing_state)).await });
        tokio::time::timeout(Duration::from_secs(1), probe.close_entered.notified())
            .await
            .expect("login close never reached the driver");

        let reopening_state = state.clone();
        let reopening = tokio::spawn(async move {
            open_browser_login(State(reopening_state), Json(open_body())).await
        });
        tokio::task::yield_now().await;
        assert!(!reopening.is_finished(), "open must wait behind exact-owner cleanup");
        assert_eq!(
            probe.lane_opens.load(Ordering::Acquire),
            opens_before_cleanup,
            "no replacement Lane may open while old cleanup is in flight"
        );

        probe.block_close.store(false, Ordering::Release);
        probe.allow_close.notify_waiters();
        let Json(closed) = closing.await.expect("close task");
        assert_eq!(
            closed.data.expect("close data").message.as_deref(),
            Some("closed")
        );
        let Json(reopened) = reopening.await.expect("reopen task");
        assert!(reopened.data.expect("reopen data").active);
        assert!(probe.lane_opens.load(Ordering::Acquire) > opens_before_cleanup);

        let _ = close_browser_login(State(state)).await;
        hub.shutdown().await.expect("shutdown fake browser Hub");
    }

    #[tokio::test]
    async fn cancelled_initial_open_keeps_cleanup_authority_before_lane_publication() {
        let (hub, probe) = login_hub(HubConfig::default());
        let state = login_state(&hub);
        probe.block_open_lane.store(true, Ordering::Release);

        let opening_state = state.clone();
        let opening = tokio::spawn(async move {
            open_browser_login(State(opening_state), Json(open_body())).await
        });
        tokio::time::timeout(Duration::from_secs(1), probe.open_lane_entered.notified())
            .await
            .expect("login open never reached the driver");
        opening.abort();
        let _ = opening.await;

        let original_lease = {
            let session = state.inner.session.lock().await;
            let retained = session
                .as_ref()
                .expect("cancelled open must retain cleanup authority");
            assert!(retained.cleanup_armed);
            assert!(retained.cleanup_pending);
            assert!(retained.lane_id.is_none());
            retained.owner_lease_id.clone()
        };
        assert_eq!(probe.lane_opens.load(Ordering::Acquire), 1);

        // Settle the Hub-owned late start. The next open first confirms exact
        // cleanup of the retained lease and only then issues a replacement.
        probe.block_open_lane.store(false, Ordering::Release);
        probe.allow_open_lane.notify_waiters();
        let Json(reopened) =
            open_browser_login(State(state.clone()), Json(open_body())).await;
        assert!(reopened.data.expect("reopen data").active);
        let replacement_lease = state
            .inner
            .session
            .lock()
            .await
            .as_ref()
            .expect("replacement tracked")
            .owner_lease_id
            .clone();
        assert_ne!(replacement_lease, original_lease);
        assert!(probe.lane_opens.load(Ordering::Acquire) > 1);

        let _ = close_browser_login(State(state)).await;
        hub.shutdown().await.expect("shutdown fake browser Hub");
    }

    #[tokio::test]
    async fn externally_closed_login_lane_stops_reporting_active() {
        let mut config = HubConfig::default();
        config.owner_lease_ttl_ms = 90;
        let (hub, _probe) = login_hub(config);
        let state = login_state(&hub);

        let Json(opened) = open_browser_login(
            State(state.clone()),
            Json(open_body()),
        )
        .await;
        let lane_id = opened
            .data
            .expect("login response data")
            .lane_id
            .expect("opened login Lane");
        hub.close_lane(&lane_id)
            .await
            .expect("external management close");

        // The renewal worker must revoke the now-resource-less owner without
        // requiring a status request to perform cleanup.
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(
            state
                .inner
                .session
                .lock()
                .await
                .as_ref()
                .is_some_and(|session| {
                    hub.renew_owner_lease(&session.owner_lease_id).is_err()
                })
        );

        let Json(status) = browser_login_status(State(state)).await;
        assert!(
            !status
                .data
                .expect("status response data")
                .active
        );
        hub.shutdown().await.expect("shutdown fake browser Hub");
    }

    #[tokio::test]
    async fn dropping_login_state_revokes_the_owner_without_a_status_call() {
        let (hub, _probe) = login_hub(HubConfig::default());
        let state = login_state(&hub);

        let Json(opened) = open_browser_login(
            State(state.clone()),
            Json(open_body()),
        )
        .await;
        assert!(
            opened
                .data
                .expect("login response data")
                .active
        );
        drop(state);

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if hub.list_lanes().await.is_empty() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("dropping BrowserLoginState did not revoke its owner");
        hub.shutdown().await.expect("shutdown fake browser Hub");
    }

    #[tokio::test]
    async fn login_open_foregrounds_an_existing_running_primary_lane_again() {
        let (hub, probe) = login_hub(HubConfig::default());
        let state = login_state(&hub);

        let Json(first) = open_browser_login(
            State(state.clone()),
            Json(open_body()),
        )
        .await;
        assert!(first.data.expect("first login response").active);
        let Json(second) = open_browser_login(
            State(state.clone()),
            Json(open_body()),
        )
        .await;
        assert!(second.data.expect("second login response").active);

        assert_eq!(
            probe.foregrounds.load(Ordering::Acquire),
            2,
            "each explicit login-open request should reveal the managed Primary Lane"
        );
        let _ = close_browser_login(State(state)).await;
        hub.shutdown().await.expect("shutdown fake browser Hub");
    }

    #[tokio::test]
    async fn failed_login_foreground_revokes_the_new_owner_and_lane() {
        let (hub, probe) = login_hub(HubConfig::default());
        probe.fail_foreground.store(true, Ordering::Release);
        let state = login_state(&hub);

        let Json(response) = open_browser_login(
            State(state.clone()),
            Json(open_body()),
        )
        .await;
        let response = response.data.expect("login response data");

        assert!(!response.active);
        assert_eq!(response.message.as_deref(), Some("launch_failed:browser_unavailable"));
        assert!(response.lane_id.is_none());
        assert_eq!(probe.foregrounds.load(Ordering::Acquire), 1);
        assert!(hub.list_lanes().await.is_empty());
        assert!(state.inner.session.lock().await.is_none());
        hub.shutdown().await.expect("shutdown fake browser Hub");
    }

    #[tokio::test]
    async fn failed_reopen_foreground_revokes_the_existing_login_session() {
        let (hub, probe) = login_hub(HubConfig::default());
        let state = login_state(&hub);
        let Json(first) = open_browser_login(
            State(state.clone()),
            Json(open_body()),
        )
        .await;
        assert!(first.data.expect("first login response").active);
        probe.fail_foreground.store(true, Ordering::Release);

        let Json(second) = open_browser_login(
            State(state.clone()),
            Json(open_body()),
        )
        .await;
        let second = second.data.expect("second login response");

        assert!(!second.active);
        assert_eq!(second.message.as_deref(), Some("launch_failed:browser_unavailable"));
        assert_eq!(probe.foregrounds.load(Ordering::Acquire), 2);
        assert!(hub.list_lanes().await.is_empty());
        assert!(state.inner.session.lock().await.is_none());
        hub.shutdown().await.expect("shutdown fake browser Hub");
    }

    #[test]
    fn chrome_source_normalization_mirrors_the_boot_reader() {
        assert_eq!(normalize_chrome_source(None), "system");
        assert_eq!(normalize_chrome_source(Some("")), "system");
        assert_eq!(normalize_chrome_source(Some("  \"system\"  ")), "system");
        assert_eq!(normalize_chrome_source(Some("SYSTEM")), "system");
        assert_eq!(normalize_chrome_source(Some("managed")), "managed");
        assert_eq!(normalize_chrome_source(Some("\"managed\"")), "managed");
        // ChromeSource::from_source_str resolves every non-"system" value to
        // the managed Chrome-for-Testing.
        assert_eq!(normalize_chrome_source(Some("garbage")), "managed");
    }

    struct FakePrefRepo {
        source: String,
    }

    #[async_trait]
    impl nomifun_db::IClientPreferenceRepository for FakePrefRepo {
        async fn get_all(
            &self,
        ) -> Result<Vec<nomifun_db::models::ClientPreference>, nomifun_db::DbError> {
            self.get_by_keys(&["agent.browserUse.source"]).await
        }

        async fn get_by_keys(
            &self,
            keys: &[&str],
        ) -> Result<Vec<nomifun_db::models::ClientPreference>, nomifun_db::DbError> {
            Ok(keys
                .iter()
                .filter(|key| **key == "agent.browserUse.source")
                .map(|key| nomifun_db::models::ClientPreference {
                    id: 1,
                    key: (*key).to_owned(),
                    value: self.source.clone(),
                    updated_at: 0,
                })
                .collect())
        }

        async fn upsert_batch(
            &self,
            _entries: &[(&str, &str)],
        ) -> Result<(), nomifun_db::DbError> {
            Ok(())
        }

        async fn delete_keys(&self, _keys: &[&str]) -> Result<(), nomifun_db::DbError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn open_reports_the_effective_host_source_and_ignores_the_body_source() {
        let (hub, _probe) = login_hub(HubConfig::default());
        let state = BrowserLoginState::new(
            Some(Arc::clone(&hub)),
            Arc::from("user-1"),
            Some(Arc::new(FakePrefRepo {
                source: "\"managed\"".to_owned(),
            })),
        );

        // The body asks for "system"; host policy (the persisted preference
        // snapshot) says managed and must win in the reported source.
        let Json(opened) = open_browser_login(
            State(state.clone()),
            Json(OpenLoginBody {
                _source: "system".to_owned(),
            }),
        )
        .await;
        let opened = opened.data.expect("login response data");
        assert!(opened.active);
        assert_eq!(opened.source.as_deref(), Some("managed"));

        let Json(status) = browser_login_status(State(state.clone())).await;
        assert_eq!(
            status.data.expect("status data").source.as_deref(),
            Some("managed")
        );
        let _ = close_browser_login(State(state)).await;
        hub.shutdown().await.expect("shutdown fake browser Hub");
    }

    #[tokio::test]
    async fn status_poll_is_a_pure_read_and_never_tears_down_the_session() {
        let (hub, _probe) = login_hub(HubConfig::default());
        let state = login_state(&hub);
        let Json(opened) = open_browser_login(State(state.clone()), Json(open_body())).await;
        let lane_id = opened
            .data
            .expect("login response data")
            .lane_id
            .expect("opened login Lane");

        // An external close makes the next (old-code) status renew+revoke and
        // silently destroy the session. The pure read must only REPORT.
        hub.close_lane(&lane_id).await.expect("external close");

        let Json(status) = browser_login_status(State(state.clone())).await;
        assert!(!status.data.expect("status data").active);
        assert!(
            state.inner.session.lock().await.is_some(),
            "a GET must not take or revoke the login session"
        );

        // Recovery path: the next open cleans the stale session and succeeds.
        let Json(reopened) = open_browser_login(State(state.clone()), Json(open_body())).await;
        assert!(reopened.data.expect("reopen data").active);
        let _ = close_browser_login(State(state)).await;
        hub.shutdown().await.expect("shutdown fake browser Hub");
    }

    #[tokio::test]
    async fn close_reports_saved_only_after_a_fresh_identity_capture() {
        use nomifun_browser_platform::{IdentitySnapshotPayload, SnapshotCoverage};

        let (hub, _probe) = login_hub(HubConfig::default());
        let state = login_state(&hub);

        // No capture happened during the session: close must be honest.
        let Json(opened) = open_browser_login(State(state.clone()), Json(open_body())).await;
        assert!(opened.data.expect("open data").active);
        let Json(closed) = close_browser_login(State(state.clone())).await;
        assert!(
            !closed.data.expect("close data").saved,
            "lease revocation success must not masquerade as a vault capture"
        );

        // A capture committed while the session was open advances the
        // canonical generation; only then does close report saved.
        let Json(reopened) = open_browser_login(State(state.clone()), Json(open_body())).await;
        assert!(reopened.data.expect("reopen data").active);
        hub.publish_identity_snapshot(
            IdentitySnapshotPayload::from_json(serde_json::json!({ "cookies": [] })),
            SnapshotCoverage::cookies_only(),
        )
        .expect("publish captured identity");
        let Json(closed) = close_browser_login(State(state)).await;
        assert!(closed.data.expect("close data").saved);
        hub.shutdown().await.expect("shutdown fake browser Hub");
    }

    #[tokio::test]
    async fn queued_login_survives_reopen_and_foregrounds_once_running() {
        let mut config = HubConfig::default();
        config.resource_policy.max_open_lanes = 1;
        let (hub, probe) = login_hub(config);

        // Fill capacity so the login Lane is admitted Queued.
        let blocker_lease = hub
            .issue_owner_lease("user-1", None, "blocker-runtime")
            .expect("issue blocker lease");
        let blocker = hub
            .bind(CallerIdentity {
                user_id: "user-1".to_owned(),
                conversation_id: None,
                runtime_instance_id: "blocker-runtime".to_owned(),
                agent_id: None,
                companion_id: None,
                execution_id: None,
                step_id: None,
                attempt_id: None,
                remote_connection_id: None,
                surface: BrowserSurface::System,
                owner_lease_id: blocker_lease.lease_id.clone(),
                capability_expires_at_ms: blocker_lease.expires_at_ms,
                allowed_operations: BTreeSet::from([
                    BrowserOperationKind::Crawl,
                    BrowserOperationKind::Manage,
                ]),
            })
            .expect("bind blocker");
        assert!(matches!(
            blocker
                .open(Some("blocker"), BrowserIdentityMode::Anonymous, None)
                .await
                .expect("open blocker"),
            OpenLaneOutcome::Running { .. }
        ));

        let state = login_state(&hub);
        let Json(first) = open_browser_login(State(state.clone()), Json(open_body())).await;
        let first = first.data.expect("first login response");
        assert!(first.active);
        assert_eq!(first.message.as_deref(), Some("queued"));
        assert_eq!(probe.foregrounds.load(Ordering::Acquire), 0);

        // F29 regression: a repeat click while still queued must keep the
        // pending login (the old code revoked it via a failed foreground).
        let Json(second) = open_browser_login(State(state.clone()), Json(open_body())).await;
        let second = second.data.expect("second login response");
        assert!(second.active);
        assert_eq!(second.message.as_deref(), Some("queued"));
        assert!(state.inner.session.lock().await.is_some());
        assert_eq!(hub.list_lanes().await.len(), 2, "queued Lane must survive");
        assert_eq!(probe.foregrounds.load(Ordering::Acquire), 0);

        // Capacity frees -> the scheduler promotes the login Lane -> the
        // watcher foregrounds the now-Running window without another click.
        blocker.close_all().await.expect("close blocker");
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if probe.foregrounds.load(Ordering::Acquire) >= 1 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("promoted login lane was never foregrounded");

        let _ = close_browser_login(State(state)).await;
        hub.shutdown().await.expect("shutdown fake browser Hub");
    }

    /// The livelock regression behind the reopen self-heal: the reopen path
    /// must never classify a dead/dying Lane as pending. The old code
    /// reported EVERY non-Running lifecycle state as "queued", so a
    /// Failed/Frozen Lane was reported pending forever and the login feature
    /// never recovered (the open-time watcher gives up on exactly these
    /// states and can never foreground them).
    #[test]
    fn reopen_disposition_never_reports_a_dead_lane_as_queued() {
        assert_eq!(
            reopen_disposition(LaneLifecycleState::Queued),
            ReopenDisposition::ReportQueued,
            "a queued login must survive a repeat click (F29)"
        );
        assert_eq!(
            reopen_disposition(LaneLifecycleState::Starting),
            ReopenDisposition::ReportQueued,
            "an in-flight start is genuinely pending"
        );
        assert_eq!(
            reopen_disposition(LaneLifecycleState::Running),
            ReopenDisposition::Foreground
        );
        for dead in [
            LaneLifecycleState::Failed,
            LaneLifecycleState::Frozen,
            LaneLifecycleState::Stopping,
        ] {
            assert_eq!(
                reopen_disposition(dead),
                ReopenDisposition::ReplaceDeadLane,
                "{dead:?} can never reach Running; reporting it queued livelocks the login"
            );
        }
    }

    #[tokio::test]
    async fn login_lane_that_failed_to_start_recovers_with_a_fresh_lane_on_reopen() {
        let mut config = HubConfig::default();
        config.resource_policy.max_open_lanes = 1;
        let (hub, probe) = login_hub(config);

        // Fill capacity so the login Lane is admitted Queued.
        let blocker_lease = hub
            .issue_owner_lease("user-1", None, "blocker-runtime")
            .expect("issue blocker lease");
        let blocker = hub
            .bind(CallerIdentity {
                user_id: "user-1".to_owned(),
                conversation_id: None,
                runtime_instance_id: "blocker-runtime".to_owned(),
                agent_id: None,
                companion_id: None,
                execution_id: None,
                step_id: None,
                attempt_id: None,
                remote_connection_id: None,
                surface: BrowserSurface::System,
                owner_lease_id: blocker_lease.lease_id.clone(),
                capability_expires_at_ms: blocker_lease.expires_at_ms,
                allowed_operations: BTreeSet::from([
                    BrowserOperationKind::Crawl,
                    BrowserOperationKind::Manage,
                ]),
            })
            .expect("bind blocker");
        assert!(matches!(
            blocker
                .open(Some("blocker"), BrowserIdentityMode::Anonymous, None)
                .await
                .expect("open blocker"),
            OpenLaneOutcome::Running { .. }
        ));

        let state = login_state(&hub);
        probe.fail_open_lane.store(true, Ordering::Release);
        let Json(first) = open_browser_login(State(state.clone()), Json(open_body())).await;
        let first = first.data.expect("first login response");
        assert!(first.active);
        assert_eq!(first.message.as_deref(), Some("queued"));
        let failed_lane_id = first.lane_id.expect("queued login Lane");

        // Capacity frees -> the scheduler promotes the login Lane -> its
        // start fails -> the Hub discards the dead Lane.
        blocker.close_all().await.expect("close blocker");
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let gone = hub
                    .list_lanes()
                    .await
                    .iter()
                    .all(|lane| lane.lane_id != failed_lane_id);
                if gone {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("failed login lane start was never discarded");

        // The reopen must self-heal within ONE request: clean the stale
        // session, then open (and foreground) a fresh Running Lane — never
        // report the dead login as still pending.
        probe.fail_open_lane.store(false, Ordering::Release);
        let Json(second) = open_browser_login(State(state.clone()), Json(open_body())).await;
        let second = second.data.expect("second login response");
        assert!(second.active, "self-heal must produce a usable login lane");
        assert_eq!(second.message.as_deref(), Some("opened"));
        let fresh_lane_id = second.lane_id.expect("fresh login Lane");
        assert_ne!(
            fresh_lane_id, failed_lane_id,
            "the dead lane must be replaced, not re-reported"
        );
        assert!(
            hub.list_lanes()
                .await
                .iter()
                .any(|lane| lane.lane_id == fresh_lane_id
                    && lane.lifecycle_state == LaneLifecycleState::Running),
            "the replacement login lane must be Running"
        );
        assert!(probe.foregrounds.load(Ordering::Acquire) >= 1);

        let _ = close_browser_login(State(state)).await;
        hub.shutdown().await.expect("shutdown fake browser Hub");
    }
}
