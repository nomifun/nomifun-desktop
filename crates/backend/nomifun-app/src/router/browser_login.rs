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
    BrowserSurface, CallerIdentity, OpenLaneOutcome, OwnerLeaseId,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

#[derive(Clone)]
pub(crate) struct BrowserLoginState {
    inner: Arc<BrowserLoginInner>,
}

struct BrowserLoginInner {
    hub: Option<Arc<BrowserSessionHub>>,
    user_id: Arc<str>,
    session: Mutex<Option<LoginLane>>,
}

struct LoginLane {
    lane_id: BrowserLaneId,
    owner_lease_id: OwnerLeaseId,
    hub: Arc<BrowserSessionHub>,
    renewal_task: tokio::task::JoinHandle<()>,
}

impl Drop for LoginLane {
    fn drop(&mut self) {
        self.renewal_task.abort();
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

impl BrowserLoginState {
    pub(crate) fn new(hub: Option<Arc<BrowserSessionHub>>, user_id: Arc<str>) -> Self {
        Self {
            inner: Arc::new(BrowserLoginInner {
                hub,
                user_id,
                session: Mutex::new(None),
            }),
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct OpenLoginBody {
    // Kept for wire compatibility. The process composition root resolves the
    // trusted Chrome source; a request body cannot replace host policy.
    #[serde(default, rename = "source")]
    _source: String,
}

#[derive(Serialize)]
pub(crate) struct LoginStatus {
    active: bool,
    message: Option<String>,
    saved: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    lane_id: Option<BrowserLaneId>,
}

pub(crate) async fn open_browser_login(
    State(state): State<BrowserLoginState>,
    Json(_body): Json<OpenLoginBody>,
) -> Json<ApiResponse<LoginStatus>> {
    let Some(hub) = state.inner.hub.clone() else {
        return login_response(false, "launch_failed:browser_not_supported", false, None);
    };

    let mut session = state.inner.session.lock().await;
    if let Some(existing) = session.as_ref() {
        let lane_present = hub
            .list_lanes()
            .await
            .iter()
            .any(|lane| lane.lane_id == existing.lane_id);
        if lane_present && hub.renew_owner_lease(&existing.owner_lease_id).is_ok() {
            return login_response(
                true,
                "already_open",
                false,
                Some(existing.lane_id.clone()),
            );
        }
    }
    if let Some(stale) = session.take() {
        stale.renewal_task.abort();
        // `renew` may already have removed an expired lease. Hub revocation is
        // still authoritative for any Lane that survived that expiry.
        let _ = hub.revoke_owner_lease(&stale.owner_lease_id).await;
    }

    let runtime_instance_id = format!("browser-login-{}", uuid::Uuid::now_v7());
    let lease = match hub.issue_owner_lease(
        state.inner.user_id.to_string(),
        None,
        runtime_instance_id.clone(),
    ) {
        Ok(lease) => lease,
        Err(_) => {
            return login_response(false, "launch_failed:owner_lease", false, None);
        }
    };
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
            let _ = hub.revoke_owner_lease(&lease.lease_id).await;
            return login_response(false, "launch_failed:invalid_capability", false, None);
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
            let _ = hub.revoke_owner_lease(&lease.lease_id).await;
            return login_response(false, "launch_failed:browser_unavailable", false, None);
        }
    };
    let lane_id = outcome.lane().lane_id.clone();
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
    *session = Some(LoginLane {
        lane_id: lane_id.clone(),
        owner_lease_id: lease.lease_id,
        hub: Arc::clone(&hub),
        renewal_task,
    });
    let message = match outcome {
        OpenLaneOutcome::Running { .. } => "opened",
        OpenLaneOutcome::Queued { .. } => "queued",
    };
    login_response(true, message, false, Some(lane_id))
}

pub(crate) async fn close_browser_login(
    State(state): State<BrowserLoginState>,
) -> Json<ApiResponse<LoginStatus>> {
    let Some(hub) = state.inner.hub.clone() else {
        return login_response(false, "not_open", false, None);
    };
    let Some(session) = state.inner.session.lock().await.take() else {
        return login_response(false, "not_open", false, None);
    };
    session.renewal_task.abort();
    let saved = hub
        .revoke_owner_lease(&session.owner_lease_id)
        .await
        .is_ok();
    login_response(false, "closed", saved, None)
}

pub(crate) async fn browser_login_status(
    State(state): State<BrowserLoginState>,
) -> Json<ApiResponse<LoginStatus>> {
    let Some(hub) = state.inner.hub.as_ref() else {
        return login_response(false, "browser_not_supported", false, None);
    };
    let mut session = state.inner.session.lock().await;
    let lane_id = session.as_ref().map(|value| value.lane_id.clone());
    let lanes = hub.list_lanes().await;
    let lane_present = lane_id
        .as_ref()
        .is_some_and(|lane_id| lanes.iter().any(|lane| &lane.lane_id == lane_id));
    let active = lane_present
        && session
            .as_ref()
            .is_some_and(|value| hub.renew_owner_lease(&value.owner_lease_id).is_ok());
    if !active {
        if let Some(stale) = session.take() {
            stale.renewal_task.abort();
            let _ = hub.revoke_owner_lease(&stale.owner_lease_id).await;
        }
    }
    login_response(active, "managed_primary_lane", false, lane_id.filter(|_| active))
}

fn login_response(
    active: bool,
    message: &str,
    saved: bool,
    lane_id: Option<BrowserLaneId>,
) -> Json<ApiResponse<LoginStatus>> {
    Json(ApiResponse::ok(LoginStatus {
        active,
        message: Some(message.to_owned()),
        saved,
        lane_id,
    }))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use nomifun_browser_platform::{
        BrowserHostDriver, BrowserHostFactory, BrowserHostId, BrowserLaneDriver,
        BrowserOperation, BrowserOperationResult, BrowserPlatformError,
        DriverOperationContext, HostLaunchRequest, HostLifecycleState, HubConfig,
        LaneLaunchRequest,
    };

    use super::*;

    struct FakeFactory;
    struct FakeHost {
        id: BrowserHostId,
    }
    struct FakeLane;

    #[async_trait]
    impl BrowserHostFactory for FakeFactory {
        async fn launch(
            &self,
            request: HostLaunchRequest,
        ) -> Result<Arc<dyn BrowserHostDriver>, BrowserPlatformError> {
            Ok(Arc::new(FakeHost {
                id: request.host_id,
            }))
        }
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
            _request: LaneLaunchRequest,
        ) -> Result<Arc<dyn BrowserLaneDriver>, BrowserPlatformError> {
            Ok(Arc::new(FakeLane))
        }

        async fn shutdown(&self) -> Result<(), BrowserPlatformError> {
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
            Ok(())
        }
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
        let hub = Arc::new(BrowserSessionHub::new(Arc::new(FakeFactory), config));
        let state = BrowserLoginState::new(Some(Arc::clone(&hub)), Arc::from("user-1"));

        let Json(opened) = open_browser_login(
            State(state.clone()),
            Json(open_body()),
        )
        .await;
        let opened = opened.data.expect("login response data");
        assert!(opened.active);
        assert!(opened.lane_id.is_some());

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
    async fn externally_closed_login_lane_stops_reporting_active() {
        let mut config = HubConfig::default();
        config.owner_lease_ttl_ms = 90;
        let hub = Arc::new(BrowserSessionHub::new(Arc::new(FakeFactory), config));
        let state = BrowserLoginState::new(Some(Arc::clone(&hub)), Arc::from("user-1"));

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
        let hub = Arc::new(BrowserSessionHub::new(
            Arc::new(FakeFactory),
            HubConfig::default(),
        ));
        let state = BrowserLoginState::new(Some(Arc::clone(&hub)), Arc::from("user-1"));

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
}
