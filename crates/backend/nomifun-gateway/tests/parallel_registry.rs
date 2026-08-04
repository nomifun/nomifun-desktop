//! Gateway concurrency regression tests over the shared BrowserSessionHub.
//!
//! These tests are hermetic: the fake host never launches Chromium. They prove
//! that the Gateway no longer owns per-companion engines or a global operation
//! mutex, while retaining input-order results for batched calls.

#![cfg(feature = "browser-use")]

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use nomifun_browser_platform::{
    BrowserErrorCode,
    BrowserHostDriver, BrowserHostFactory, BrowserHostId, BrowserLaneDriver,
    BrowserProfileFootprint,
    BrowserOperation, BrowserOperationKind, BrowserOperationResult,
    BrowserPlatformError, BrowserSessionHub, BrowserSurface, CallerIdentity,
    DriverOperationContext, HostLaunchRequest, HostLifecycleState, HubConfig,
    LaneLaunchRequest,
};
use nomifun_gateway::CallerCtx;
use nomifun_gateway::browser_registry::{
    BrowserRegistry, GatewayBrowserCall,
};
use serde_json::{Value, json};
use tokio::sync::Semaphore;

struct Probe {
    active: AtomicUsize,
    maximum: AtomicUsize,
    entered: Semaphore,
    releases: Semaphore,
}

impl Probe {
    fn record_enter(&self) {
        let active = self.active.fetch_add(1, Ordering::AcqRel) + 1;
        self.maximum.fetch_max(active, Ordering::AcqRel);
    }

    fn record_exit(&self) {
        self.active.fetch_sub(1, Ordering::AcqRel);
    }

    async fn wait_for_entries(&self, expected: u32) {
        self.entered
            .acquire_many(expected)
            .await
            .expect("entry semaphore closed")
            .forget();
    }

    fn release(&self, count: usize) {
        self.releases.add_permits(count);
    }
}

struct FakeLane {
    probe: Arc<Probe>,
}

#[async_trait]
impl BrowserLaneDriver for FakeLane {
    async fn execute(
        &self,
        operation: BrowserOperation,
        _context: DriverOperationContext,
    ) -> Result<BrowserOperationResult, BrowserPlatformError> {
        self.probe.record_enter();
        if operation
            .input
            .get("block")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            self.probe.entered.add_permits(1);
            self.probe
                .releases
                .acquire()
                .await
                .expect("release semaphore closed")
                .forget();
        } else {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        self.probe.record_exit();
        Ok(BrowserOperationResult {
            output: json!({
                "marker": operation
                    .input
                    .get("marker")
                    .and_then(Value::as_str),
            }),
            ..Default::default()
        })
    }

    async fn close(&self) -> Result<(), BrowserPlatformError> {
        Ok(())
    }
}

struct FakeHost {
    host_id: BrowserHostId,
    probe: Arc<Probe>,
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
        Ok(Arc::new(FakeLane {
            probe: Arc::clone(&self.probe),
        }))
    }

    async fn shutdown(&self) -> Result<(), BrowserPlatformError> {
        Ok(())
    }
}

struct FakeFactory {
    probe: Arc<Probe>,
}

#[async_trait]
impl BrowserHostFactory for FakeFactory {
    async fn launch(
        &self,
        request: HostLaunchRequest,
    ) -> Result<Arc<dyn BrowserHostDriver>, BrowserPlatformError> {
        Ok(Arc::new(FakeHost {
            host_id: request.host_id,
            probe: Arc::clone(&self.probe),
        }))
    }
}

fn fixture() -> (BrowserRegistry, CallerCtx, Arc<Probe>) {
    let probe = Arc::new(Probe {
        active: AtomicUsize::new(0),
        maximum: AtomicUsize::new(0),
        entered: Semaphore::new(0),
        releases: Semaphore::new(0),
    });
    let hub = BrowserSessionHub::new(
        Arc::new(FakeFactory {
            probe: Arc::clone(&probe),
        }),
        HubConfig::default(),
    );
    let user_id = nomifun_common::UserId::parse(
        "0190f5fe-7c00-7a00-8000-000000000001",
    )
    .unwrap();
    let conversation_id = nomifun_common::ConversationId::parse(
        "0190f5fe-7c00-7a00-8abc-012345678901",
    )
    .unwrap();
    let runtime_id = "gateway-runtime-parallel";
    let owner = hub
        .issue_owner_lease(
            user_id.as_str(),
            Some(conversation_id.as_str().to_owned()),
            runtime_id,
        )
        .unwrap();
    let caller = CallerCtx {
        conversation_id: Some(conversation_id.clone()),
        user_id: user_id.clone(),
        browser_identity: Some(CallerIdentity {
            user_id: user_id.as_str().to_owned(),
            conversation_id: Some(conversation_id.as_str().to_owned()),
            runtime_instance_id: runtime_id.to_owned(),
            agent_id: Some("gateway-agent".to_owned()),
            companion_id: None,
            execution_id: None,
            step_id: None,
            attempt_id: Some("attempt-1".to_owned()),
            remote_connection_id: None,
            surface: BrowserSurface::Gateway,
            owner_lease_id: owner.lease_id,
            capability_expires_at_ms: u64::MAX,
            allowed_operations: BTreeSet::from([
                BrowserOperationKind::Manage,
                BrowserOperationKind::Navigate,
            ]),
        }),
        ..Default::default()
    };
    (BrowserRegistry::from_hub(hub), caller, probe)
}

fn gateway_caller_without_browser_identity(
    conversation_id: &nomifun_common::ConversationId,
    user_id: &nomifun_common::UserId,
    companion_id: &nomifun_common::CompanionId,
) -> CallerCtx {
    CallerCtx {
        conversation_id: Some(conversation_id.clone()),
        user_id: user_id.clone(),
        companion_id: Some(companion_id.clone()),
        remote: true,
        ..Default::default()
    }
}

fn call(caller: &CallerCtx, lane_name: &str, marker: &str) -> GatewayBrowserCall {
    GatewayBrowserCall {
        caller: caller.clone(),
        lane_name: lane_name.to_owned(),
        input: json!({
            "action": "navigate",
            "url": format!("https://example.test/{marker}"),
            "marker": marker,
        }),
    }
}

fn blocking_call(
    caller: &CallerCtx,
    lane_name: &str,
    marker: &str,
) -> GatewayBrowserCall {
    let mut call = call(caller, lane_name, marker);
    call.input["block"] = Value::Bool(true);
    call
}

#[tokio::test]
async fn execute_parallel_distinct_lanes_overlap_and_keep_input_order() {
    let (registry, caller, probe) = fixture();
    let results = registry
        .execute_parallel(vec![
            call(&caller, "alpha", "first"),
            call(&caller, "beta", "second"),
        ])
        .await;
    assert_eq!(results.len(), 2);
    assert_eq!(
        results[0]
            .as_ref()
            .unwrap()
            .output
            .get("marker")
            .and_then(Value::as_str),
        Some("first")
    );
    assert_eq!(
        results[1]
            .as_ref()
            .unwrap()
            .output
            .get("marker")
            .and_then(Value::as_str),
        Some("second")
    );
    assert_eq!(
        probe.maximum.load(Ordering::Acquire),
        2,
        "different lanes must not be globally serialized"
    );
}

#[tokio::test]
async fn execute_parallel_same_lane_remains_serialized() {
    let (registry, caller, probe) = fixture();
    let results = registry
        .execute_parallel(vec![
            call(&caller, "default", "first"),
            call(&caller, "default", "second"),
        ])
        .await;
    assert!(results.iter().all(Result::is_ok));
    assert_eq!(
        probe.maximum.load(Ordering::Acquire),
        1,
        "the Hub must serialize operations in one lane"
    );
}

#[tokio::test]
async fn same_companion_attempts_get_distinct_default_lanes_and_revoke_exact_owner() {
    let probe = Arc::new(Probe {
        active: AtomicUsize::new(0),
        maximum: AtomicUsize::new(0),
        entered: Semaphore::new(0),
        releases: Semaphore::new(0),
    });
    let hub = BrowserSessionHub::new(
        Arc::new(FakeFactory {
            probe: Arc::clone(&probe),
        }),
        HubConfig::default(),
    );
    let registry = BrowserRegistry::from_hub(hub.clone());
    let user_id = nomifun_common::UserId::parse(
        "0190f5fe-7c00-7a00-8000-000000000001",
    )
    .unwrap();
    let conversation_id = nomifun_common::ConversationId::parse(
        "0190f5fe-7c00-7a00-8abc-012345678901",
    )
    .unwrap();
    let companion_id = nomifun_common::CompanionId::parse(
        "0190f5fe-7c00-7a00-8abc-012345678902",
    )
    .unwrap();
    let mut first = gateway_caller_without_browser_identity(
        &conversation_id,
        &user_id,
        &companion_id,
    );
    let mut second = gateway_caller_without_browser_identity(
        &conversation_id,
        &user_id,
        &companion_id,
    );
    registry
        .attach_trusted_identity(
            &mut first,
            "signed-child-attempt-a",
            Some("attempt-a"),
            u64::MAX,
        )
        .await
        .unwrap();
    registry
        .attach_trusted_identity(
            &mut second,
            "signed-child-attempt-b",
            Some("attempt-b"),
            u64::MAX,
        )
        .await
        .unwrap();

    let first_lane = registry.open(&first, None).await.unwrap();
    let second_lane = registry.open(&second, None).await.unwrap();
    assert_ne!(first_lane.lane_id, second_lane.lane_id);
    assert_ne!(first_lane.lane_key, second_lane.lane_key);
    assert_eq!(first_lane.lane_key.lane_name, "default");
    assert_eq!(second_lane.lane_key.lane_name, "default");
    assert_eq!(
        first_lane.caller.companion_id,
        second_lane.caller.companion_id,
        "companion attribution must not merge attempt-owned lanes"
    );
    assert_eq!(
        first_lane.caller.conversation_id,
        second_lane.caller.conversation_id,
        "conversation attribution must not merge attempt-owned lanes"
    );

    let parallel_registry = registry.clone();
    let parallel_first = first.clone();
    let parallel_second = second.clone();
    let parallel = tokio::spawn(async move {
        parallel_registry
            .execute_parallel(vec![
                blocking_call(
                    &parallel_first,
                    "default",
                    "attempt-a",
                ),
                blocking_call(
                    &parallel_second,
                    "default",
                    "attempt-b",
                ),
            ])
            .await
    });
    tokio::time::timeout(
        Duration::from_secs(1),
        probe.wait_for_entries(2),
    )
    .await
    .expect(
        "attempt-owned default lanes were serialized before both drivers entered",
    );
    assert_eq!(
        probe.maximum.load(Ordering::Acquire),
        2,
        "attempt-owned default lanes must overlap despite identical companion/conversation"
    );
    probe.release(2);
    let results = parallel.await.unwrap();
    assert!(results.iter().all(Result::is_ok));

    let stale_first = first.clone();
    let revoked = registry
        .revoke_signed_child_lease("signed-child-attempt-a")
        .await
        .unwrap();
    assert_eq!(revoked.closed, 1);
    assert!(!revoked.already_closed);

    let lanes = hub.list_lanes().await;
    assert_eq!(lanes.len(), 1);
    assert_eq!(lanes[0].lane_id, second_lane.lane_id);
    assert_eq!(
        lanes[0].caller.runtime_instance_id,
        "signed-child-attempt-b"
    );
    assert_eq!(
        registry.open(&stale_first, None).await.unwrap_err().code,
        BrowserErrorCode::OwnerLeaseExpired,
        "a detached caller must not reopen the revoked owner's lane"
    );
    assert_eq!(
        registry
            .execute(
                &stale_first,
                None,
                json!({
                    "action": "navigate",
                    "url": "https://example.test/revoked",
                }),
            )
            .await
            .unwrap_err()
            .code,
        BrowserErrorCode::OwnerLeaseExpired
    );
    assert_eq!(
        registry.open(&second, None).await.unwrap().lane_id,
        second_lane.lane_id,
        "revoking one remote attempt must not disturb its sibling owner"
    );
}
