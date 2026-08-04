use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex, MutexGuard};

use async_trait::async_trait;
use nomifun_browser_platform::{
    BrowserErrorCode, BrowserHostDriver, BrowserHostFactory, BrowserHostId,
    BrowserIdentityMode, BrowserLaneClient, BrowserLaneDriver, BrowserLaneId,
    BrowserOperation, BrowserOperationKind, BrowserOperationResult, BrowserPlatformError,
    BrowserSessionHub, BrowserSurface, CallerIdentity, DriverOperationContext, HostLaunchRequest,
    HostLifecycleState, HubConfig, LaneLaunchRequest, ManualClock, OpenLaneOutcome,
};
use nomifun_common::AppError;
use nomifun_knowledge::PageFetcher;
use serde_json::{Value, json};
use tokio::sync::{Mutex, Notify};

use super::{
    BrowserFetcher, rendered_to_page, rendered_to_page_with_source_truncation,
};

#[derive(Default)]
struct Probe {
    launches: StdMutex<Vec<RecordedLaunch>>,
    operations: StdMutex<Vec<RecordedOperation>>,
    lane_closes: AtomicUsize,
    closed_lane_ids: StdMutex<Vec<BrowserLaneId>>,
    close_failures_remaining: AtomicUsize,
    block_lane_close: AtomicBool,
    lane_close_started: Notify,
    lane_close_release: Notify,
    execute_failures_remaining: AtomicUsize,
    block_navigation: AtomicBool,
    navigation_started: Notify,
    navigation_release: Notify,
    host_shutdowns: AtomicUsize,
    host_shutdown_failures_remaining: AtomicUsize,
    active_anonymous_profiles: AtomicUsize,
    peak_anonymous_profiles: AtomicUsize,
}

#[derive(Clone, Copy, Debug)]
struct RecordedLaunch {
    identity_mode: BrowserIdentityMode,
    headful: bool,
}

#[derive(Clone, Debug)]
struct RecordedOperation {
    lane_id: BrowserLaneId,
    kind: BrowserOperationKind,
    action: String,
    url: Option<String>,
}

struct FakeLane {
    lane_id: BrowserLaneId,
    current_url: Mutex<Option<String>>,
    probe: Arc<Probe>,
}

#[async_trait]
impl BrowserLaneDriver for FakeLane {
    async fn execute(
        &self,
        operation: BrowserOperation,
        _context: DriverOperationContext,
    ) -> Result<BrowserOperationResult, BrowserPlatformError> {
        let input_url = operation
            .input
            .get("url")
            .and_then(Value::as_str)
            .map(str::to_owned);
        lock_unpoisoned(&self.probe.operations).push(RecordedOperation {
            lane_id: self.lane_id.clone(),
            kind: operation.kind,
            action: operation.action.clone(),
            url: input_url.clone(),
        });
        if self
            .probe
            .execute_failures_remaining
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                remaining.checked_sub(1)
            })
            .is_ok()
        {
            return Err(BrowserPlatformError::new(
                BrowserErrorCode::BrowserUnavailable,
                "The fake knowledge operation failed.",
                true,
                "Retry the knowledge source.",
            ));
        }

        match operation.action.as_str() {
            "navigate" => {
                let url = input_url.ok_or_else(|| {
                    BrowserPlatformError::new(
                        BrowserErrorCode::BrowserUnavailable,
                        "The fake navigate operation is missing a URL.",
                        false,
                        "Fix the test operation.",
                    )
                })?;
                *self.current_url.lock().await = Some(url.clone());
                if self.probe.block_navigation.load(Ordering::Acquire) {
                    self.probe.navigation_started.notify_waiters();
                    self.probe.navigation_release.notified().await;
                }
                // Make concurrent fetch tasks contend at the transaction seam.
                tokio::task::yield_now().await;
                Ok(BrowserOperationResult {
                    output: json!({ "final_url": url }),
                    ..BrowserOperationResult::default()
                })
            }
            "rendered_html" => {
                let url = self
                    .current_url
                    .lock()
                    .await
                    .clone()
                    .unwrap_or_else(|| "about:blank".to_owned());
                tokio::task::yield_now().await;
                Ok(BrowserOperationResult {
                    output: json!({
                        "html": format!(
                            "<html><head><title>{url}</title></head><body><h1>{url}</h1></body></html>"
                        )
                    }),
                    ..BrowserOperationResult::default()
                })
            }
            action => Err(BrowserPlatformError::new(
                BrowserErrorCode::OperationNotAllowed,
                format!("Unexpected fake browser action: {action}"),
                false,
                "Fix the test operation.",
            )),
        }
    }

    async fn close(&self) -> Result<(), BrowserPlatformError> {
        self.probe.lane_closes.fetch_add(1, Ordering::AcqRel);
        lock_unpoisoned(&self.probe.closed_lane_ids).push(self.lane_id.clone());
        if self.probe.block_lane_close.load(Ordering::Acquire) {
            self.probe.lane_close_started.notify_waiters();
            self.probe.lane_close_release.notified().await;
        }
        if self
            .probe
            .close_failures_remaining
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                remaining.checked_sub(1)
            })
            .is_ok()
        {
            return Err(BrowserPlatformError::new(
                BrowserErrorCode::BrowserUnavailable,
                "The fake knowledge lane cleanup failed.",
                true,
                "Retry the retained Hub cleanup.",
            ));
        }
        Ok(())
    }
}

struct FakeHost {
    host_id: BrowserHostId,
    probe: Arc<Probe>,
    shutdown_complete: AtomicBool,
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
        request: LaneLaunchRequest,
    ) -> Result<Arc<dyn BrowserLaneDriver>, BrowserPlatformError> {
        Ok(Arc::new(FakeLane {
            lane_id: request.lane_id,
            current_url: Mutex::new(None),
            probe: Arc::clone(&self.probe),
        }))
    }

    async fn shutdown(&self) -> Result<(), BrowserPlatformError> {
        self.probe.host_shutdowns.fetch_add(1, Ordering::AcqRel);
        if self
            .probe
            .host_shutdown_failures_remaining
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                remaining.checked_sub(1)
            })
            .is_ok()
        {
            return Err(BrowserPlatformError::new(
                BrowserErrorCode::BrowserUnavailable,
                "The fake Anonymous Host shutdown failed.",
                true,
                "Retry retained Host cleanup.",
            ));
        }
        if !self.shutdown_complete.swap(true, Ordering::AcqRel) {
            self.probe
                .active_anonymous_profiles
                .fetch_sub(1, Ordering::AcqRel);
        }
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
        // The fake has no out-of-process Host cleanup to retain. Recording the
        // complete request would clone and pin its launch-cleanup lease forever,
        // making Hub shutdown proof fail for a resource that never existed.
        let HostLaunchRequest {
            host_id,
            identity_mode,
            headful,
            cleanup_lease,
            ..
        } = request;
        lock_unpoisoned(&self.probe.launches).push(RecordedLaunch {
            identity_mode,
            headful,
        });
        drop(cleanup_lease);
        let active = self
            .probe
            .active_anonymous_profiles
            .fetch_add(1, Ordering::AcqRel)
            + 1;
        self.probe
            .peak_anonymous_profiles
            .fetch_max(active, Ordering::AcqRel);
        Ok(Arc::new(FakeHost {
            host_id,
            probe: Arc::clone(&self.probe),
            shutdown_complete: AtomicBool::new(false),
        }))
    }
}

struct Harness {
    hub: Arc<BrowserSessionHub>,
    clock: Arc<ManualClock>,
    probe: Arc<Probe>,
}

impl Harness {
    fn new(mut config: HubConfig) -> Self {
        config.headful = false;
        let clock = Arc::new(ManualClock::new(1_000));
        let probe = Arc::new(Probe::default());
        let factory = Arc::new(FakeFactory {
            probe: Arc::clone(&probe),
        });
        let hub = Arc::new(BrowserSessionHub::with_clock(
            factory,
            config,
            clock.clone(),
        ));
        Self { hub, clock, probe }
    }

    fn fetcher(&self) -> BrowserFetcher {
        BrowserFetcher::with_runtime_instance_id(
            Arc::clone(&self.hub),
            "installation-owner".to_owned(),
            "knowledge-test-runtime".to_owned(),
        )
    }
}

fn lock_unpoisoned<T>(mutex: &StdMutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

async fn wait_for_recorded_operations(probe: &Probe, expected: usize) {
    let recorded = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if lock_unpoisoned(&probe.operations).len() >= expected {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert!(
        recorded.is_ok(),
        "browser operation was never recorded: expected {expected}, observed {}",
        lock_unpoisoned(&probe.operations).len()
    );
}

fn caller_for(lease: &nomifun_browser_platform::OwnerLease) -> CallerIdentity {
    CallerIdentity {
        user_id: lease.user_id.clone(),
        conversation_id: lease.conversation_id.clone(),
        runtime_instance_id: lease.runtime_instance_id.clone(),
        agent_id: None,
        companion_id: None,
        execution_id: None,
        step_id: None,
        attempt_id: None,
        remote_connection_id: None,
        surface: BrowserSurface::System,
        owner_lease_id: lease.lease_id.clone(),
        capability_expires_at_ms: lease.expires_at_ms,
        allowed_operations: BTreeSet::from([
            BrowserOperationKind::Crawl,
            BrowserOperationKind::Manage,
        ]),
    }
}

fn bind_test_client(
    hub: &BrowserSessionHub,
    user_id: &str,
    runtime_id: &str,
) -> BrowserLaneClient {
    let lease = hub
        .issue_owner_lease(user_id, None, runtime_id)
        .expect("issue owner lease");
    hub.bind(caller_for(&lease)).expect("bind test client")
}

#[test]
fn rendered_to_page_converts_html_like_the_http_pipeline() {
    let html = "<html><head><title>Rendered title</title><script>noise()</script></head>\
                <body><h1>Dynamic body</h1><p>Only Chromium sees this</p></body></html>";
    let page = rendered_to_page("https://spa.example.test/app", html);
    assert_eq!(page.title.as_deref(), Some("Rendered title"));
    assert!(page.markdown.contains("# Dynamic body"), "got: {}", page.markdown);
    assert!(page.markdown.contains("Only Chromium sees this"));
    assert!(!page.markdown.contains("<data"));
    assert!(!page.markdown.contains("[REDACTED"));
    assert!(!page.markdown.contains("noise()"));
    assert_eq!(page.final_url, "https://spa.example.test/app");
    assert!(!page.truncated);
}

#[test]
fn renderer_side_html_truncation_is_preserved_when_markdown_is_small() {
    let page = rendered_to_page_with_source_truncation(
        "https://example.test",
        "<html><body><p>bounded prefix</p></body></html>",
        true,
    );
    assert!(page.truncated);
    assert!(page.markdown.contains("bounded prefix"));
}

#[test]
fn rendered_to_page_truncates_oversized_markdown() {
    let big = "x".repeat(nomifun_knowledge::source_url::FETCH_MAX_BYTES + 10_000);
    let html = format!("<html><body><p>{big}</p></body></html>");
    let page = rendered_to_page("https://example.test", &html);
    assert!(page.truncated);
    assert!(page.markdown.len() <= nomifun_knowledge::source_url::FETCH_MAX_BYTES);
}

#[test]
fn rendered_to_page_handles_missing_title() {
    let page = rendered_to_page(
        "https://example.test",
        "<html><body><p>no title here</p></body></html>",
    );
    assert!(page.title.is_none());
    assert!(page.markdown.contains("no title here"));
}

#[tokio::test]
async fn fetch_uses_anonymous_hub_lane_and_crawl_operations() {
    let harness = Harness::new(HubConfig::default());
    let fetcher = harness.fetcher();

    let page = fetcher
        .fetch_page("https://spa.example.test/a")
        .await
        .expect("render through the fake Hub");

    assert_eq!(page.final_url, "https://spa.example.test/a");
    assert_eq!(page.title.as_deref(), Some("https://spa.example.test/a"));
    let launches = lock_unpoisoned(&harness.probe.launches);
    assert_eq!(launches.len(), 1);
    assert_eq!(launches[0].identity_mode, BrowserIdentityMode::Anonymous);
    assert!(!launches[0].headful);
    drop(launches);

    let operations = lock_unpoisoned(&harness.probe.operations);
    assert_eq!(operations.len(), 2);
    assert!(operations.iter().all(|op| op.kind == BrowserOperationKind::Crawl));
    assert_eq!(operations[0].action, "navigate");
    assert_eq!(operations[0].url.as_deref(), Some("https://spa.example.test/a"));
    assert_eq!(operations[1].action, "rendered_html");
    assert_eq!(operations[0].lane_id, operations[1].lane_id);
    drop(operations);

    assert!(
        harness.hub.list_lanes().await.is_empty(),
        "a successful fetch must not pin its Anonymous Lane"
    );
    let overview = harness.hub.overview().await;
    assert_eq!(overview.total_lanes, 0);
    assert_eq!(overview.pending_cleanup_count, 0);
    assert_eq!(overview.managed_host_count, 0);
    assert_eq!(harness.probe.lane_closes.load(Ordering::Acquire), 1);
    assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 1);
    assert_eq!(
        harness
            .probe
            .active_anonymous_profiles
            .load(Ordering::Acquire),
        0
    );
    fetcher.shutdown().await.expect("shutdown fetcher");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_fetches_remain_two_operation_transactions() {
    let harness = Harness::new(HubConfig::default());
    let fetcher = Arc::new(harness.fetcher());
    let mut tasks = Vec::new();
    for index in 0..8 {
        let fetcher = Arc::clone(&fetcher);
        tasks.push(tokio::spawn(async move {
            let url = format!("https://spa.example.test/{index}");
            let page = fetcher.fetch_page(&url).await.expect("fetch page");
            assert_eq!(page.final_url, url);
            assert_eq!(page.title.as_deref(), Some(page.final_url.as_str()));
        }));
    }
    for task in tasks {
        task.await.expect("fetch task");
    }

    let operations = lock_unpoisoned(&harness.probe.operations);
    assert_eq!(operations.len(), 16);
    for transaction in operations.chunks_exact(2) {
        assert_eq!(transaction[0].action, "navigate");
        assert_eq!(transaction[1].action, "rendered_html");
        assert_eq!(transaction[0].lane_id, transaction[1].lane_id);
    }
    drop(operations);
    assert!(harness.hub.list_lanes().await.is_empty());
    assert_eq!(harness.probe.lane_closes.load(Ordering::Acquire), 8);
    assert_eq!(
        harness
            .probe
            .active_anonymous_profiles
            .load(Ordering::Acquire),
        0
    );
    assert_eq!(
        harness
            .probe
            .peak_anonymous_profiles
            .load(Ordering::Acquire),
        1,
        "serialized fetches must never retain overlapping Anonymous profiles"
    );
    fetcher.shutdown().await.expect("shutdown fetcher");
}

#[tokio::test]
async fn lease_is_renewed_then_reissued_after_expiry() {
    let mut config = HubConfig::default();
    config.owner_lease_ttl_ms = 100;
    let harness = Harness::new(config);
    let fetcher = harness.fetcher();

    fetcher
        .fetch_page("https://spa.example.test/first")
        .await
        .expect("initial fetch");
    let first_lease = fetcher.owner_lease_id().expect("first lease");

    harness.clock.advance(50);
    fetcher
        .fetch_page("https://spa.example.test/renewed")
        .await
        .expect("fetch after renewal");
    assert_eq!(fetcher.owner_lease_id().as_ref(), Some(&first_lease));

    harness.clock.advance(101);
    fetcher
        .fetch_page("https://spa.example.test/reissued")
        .await
        .expect("fetch after expiry");
    let second_lease = fetcher.owner_lease_id().expect("replacement lease");
    assert_ne!(first_lease, second_lease);
    assert_eq!(harness.probe.lane_closes.load(Ordering::Acquire), 3);
    assert!(harness.hub.list_lanes().await.is_empty());
    fetcher.shutdown().await.expect("shutdown fetcher");
}

#[tokio::test]
async fn queued_fetch_waits_for_promotion_and_completes() {
    let mut config = HubConfig::default();
    config.resource_policy.max_open_lanes = 1;
    let harness = Harness::new(config);
    let blocker = bind_test_client(&harness.hub, "installation-owner", "blocker-runtime");
    let blocker_lane = blocker
        .open(Some("blocker"), BrowserIdentityMode::Anonymous, None)
        .await
        .expect("open blocker");
    assert!(matches!(blocker_lane, OpenLaneOutcome::Running { .. }));

    // F40: while the knowledge Lane is queued for capacity, the fetch must
    // wait for scheduler promotion instead of failing the knowledge source.
    let fetcher = Arc::new(harness.fetcher());
    let fetch_fetcher = Arc::clone(&fetcher);
    let fetch = tokio::spawn(async move {
        fetch_fetcher
            .fetch_page("https://spa.example.test/queued-then-promoted")
            .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(!fetch.is_finished(), "fetch must wait while queued");

    blocker.close_all().await.expect("close blocker");
    let page = tokio::time::timeout(std::time::Duration::from_secs(10), fetch)
        .await
        .expect("queued fetch must complete after promotion")
        .expect("fetch task")
        .expect("promoted fetch succeeds");
    assert_eq!(page.final_url, "https://spa.example.test/queued-then-promoted");
    fetcher.shutdown().await.expect("shutdown fetcher");
}

#[tokio::test]
async fn exhausted_queue_wait_is_cancelled_and_reported_without_private_fallback() {
    let mut config = HubConfig::default();
    config.resource_policy.max_open_lanes = 1;
    let harness = Harness::new(config);
    let blocker = bind_test_client(&harness.hub, "installation-owner", "blocker-runtime");
    let blocker_lane = blocker
        .open(
            Some("blocker"),
            BrowserIdentityMode::Anonymous,
            None,
        )
        .await
        .expect("open blocker");
    assert!(matches!(blocker_lane, OpenLaneOutcome::Running { .. }));

    let mut fetcher = harness.fetcher();
    fetcher.set_queue_wait_timeout(std::time::Duration::from_millis(300));
    let error = fetcher
        .fetch_page("https://spa.example.test/queued")
        .await
        .expect_err("an exhausted queue wait must surface the capacity error");
    let message = match error {
        AppError::BadGateway(message) => message,
        other => panic!("unexpected error: {other:?}"),
    };
    assert!(message.contains("browser_capacity_queued"), "{message}");
    assert!(message.contains("metadata="), "{message}");
    assert!(message.contains("retry_delay_ms"), "{message}");

    let lanes = harness.hub.list_lanes().await;
    assert_eq!(lanes.len(), 1, "queued knowledge Lane was left orphaned");
    assert_eq!(lanes[0].caller.runtime_instance_id, "blocker-runtime");
    assert_eq!(harness.probe.launches.lock().unwrap().len(), 1);
    fetcher.shutdown().await.expect("shutdown fetcher");
    blocker.close_all().await.expect("close blocker");
}

#[tokio::test]
async fn shutdown_closes_runtime_and_revokes_owner_lease() {
    let harness = Harness::new(HubConfig::default());
    let fetcher = harness.fetcher();
    fetcher
        .fetch_page("https://spa.example.test/a")
        .await
        .expect("initial fetch");
    let lease_id = fetcher.owner_lease_id().expect("owner lease");
    assert!(harness.hub.list_lanes().await.is_empty());
    assert_eq!(harness.probe.lane_closes.load(Ordering::Acquire), 1);

    fetcher.shutdown().await.expect("shutdown fetcher");
    fetcher.shutdown().await.expect("idempotent shutdown");

    assert!(harness.hub.list_lanes().await.is_empty());
    assert_eq!(
        harness.hub.renew_owner_lease(&lease_id).unwrap_err().code,
        BrowserErrorCode::OwnerLeaseExpired
    );
    assert_eq!(harness.probe.lane_closes.load(Ordering::Acquire), 1);
    let closed_error = fetcher
        .fetch_page("https://spa.example.test/after-shutdown")
        .await
        .unwrap_err();
    assert!(closed_error.to_string().contains("shutting down"));
}

#[tokio::test]
async fn failed_transaction_cleanup_is_handed_to_the_hub_and_converges() {
    let harness = Harness::new(HubConfig::default());
    let fetcher = harness.fetcher();
    harness
        .probe
        .close_failures_remaining
        .store(1, Ordering::Release);
    harness
        .probe
        .host_shutdown_failures_remaining
        .store(1, Ordering::Release);

    let error = fetcher
        .fetch_page("https://spa.example.test/retry-cleanup")
        .await
        .expect_err("uncertain physical cleanup must be surfaced");
    assert!(
        error.to_string().contains("closing the Anonymous knowledge lane"),
        "{error}"
    );

    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let overview = harness.hub.overview().await;
            if overview.total_lanes == 0
                && overview.pending_cleanup_count == 0
                && overview.managed_host_count == 0
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("Hub cleanup handoff did not converge");

    let overview = harness.hub.overview().await;
    assert_eq!(overview.total_lanes, 0);
    assert_eq!(overview.pending_cleanup_count, 0);
    assert_eq!(overview.managed_host_count, 0);
    assert!(harness.probe.lane_closes.load(Ordering::Acquire) >= 2);
    assert!(harness.probe.host_shutdowns.load(Ordering::Acquire) >= 2);
    assert_eq!(
        harness
            .probe
            .active_anonymous_profiles
            .load(Ordering::Acquire),
        0
    );
    fetcher.shutdown().await.expect("shutdown fetcher");
}

#[tokio::test]
async fn operation_error_still_closes_the_exact_transaction_lane() {
    let harness = Harness::new(HubConfig::default());
    let fetcher = harness.fetcher();
    harness
        .probe
        .execute_failures_remaining
        .store(1, Ordering::Release);

    fetcher
        .fetch_page("https://spa.example.test/operation-error")
        .await
        .expect_err("the injected operation failure must surface");

    let overview = harness.hub.overview().await;
    assert_eq!(overview.total_lanes, 0);
    assert_eq!(overview.pending_cleanup_count, 0);
    assert_eq!(overview.managed_host_count, 0);
    assert_eq!(harness.probe.lane_closes.load(Ordering::Acquire), 1);
    assert_eq!(
        harness
            .probe
            .active_anonymous_profiles
            .load(Ordering::Acquire),
        0
    );
    fetcher.shutdown().await.expect("shutdown fetcher");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_fetch_hands_exact_cleanup_to_the_hub() {
    let harness = Harness::new(HubConfig::default());
    let fetcher = Arc::new(harness.fetcher());
    harness
        .probe
        .block_navigation
        .store(true, Ordering::Release);

    let task_fetcher = Arc::clone(&fetcher);
    let task = tokio::spawn(async move {
        task_fetcher
            .fetch_page("https://spa.example.test/cancelled")
            .await
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        harness.probe.navigation_started.notified(),
    )
    .await
    .expect("fetch never entered the blocked navigation");
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());

    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let overview = harness.hub.overview().await;
            if overview.total_lanes == 0
                && overview.pending_cleanup_count == 0
                && overview.managed_host_count == 0
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("cancelled fetch cleanup did not converge");
    assert_eq!(harness.probe.lane_closes.load(Ordering::Acquire), 1);
    assert_eq!(
        harness
            .probe
            .active_anonymous_profiles
            .load(Ordering::Acquire),
        0
    );
    fetcher.shutdown().await.expect("shutdown fetcher");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn delayed_cancel_cleanup_cannot_kill_the_immediate_replacement_transaction() {
    let harness = Harness::new(HubConfig::default());
    let fetcher = Arc::new(harness.fetcher());
    harness
        .probe
        .block_navigation
        .store(true, Ordering::Release);
    harness
        .probe
        .block_lane_close
        .store(true, Ordering::Release);

    let old_fetcher = Arc::clone(&fetcher);
    let old = tokio::spawn(async move {
        old_fetcher
            .fetch_page("https://spa.example.test/old-cancelled")
            .await
    });
    wait_for_recorded_operations(&harness.probe, 1).await;
    let old_lane_id = lock_unpoisoned(&harness.probe.operations)[0].lane_id.clone();
    old.abort();
    assert!(old.await.unwrap_err().is_cancelled());

    let replacement_fetcher = Arc::clone(&fetcher);
    let replacement = tokio::spawn(async move {
        replacement_fetcher
            .fetch_page("https://spa.example.test/replacement")
            .await
    });
    wait_for_recorded_operations(&harness.probe, 2).await;
    let replacement_lane_id = lock_unpoisoned(&harness.probe.operations)[1]
        .lane_id
        .clone();
    assert_ne!(old_lane_id, replacement_lane_id);

    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if harness.probe.lane_closes.load(Ordering::Acquire) >= 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("old exact cleanup never reached the delayed driver close");

    let live = harness.hub.list_lanes().await;
    assert!(live.iter().any(|lane| lane.lane_id == replacement_lane_id));
    assert!(!live.iter().any(|lane| lane.lane_id == old_lane_id));
    assert!(!replacement.is_finished());
    assert!(
        lock_unpoisoned(&harness.probe.closed_lane_ids)
            .iter()
            .all(|lane_id| lane_id == &old_lane_id),
        "the delayed old handoff must not close the replacement Lane"
    );

    harness
        .probe
        .block_lane_close
        .store(false, Ordering::Release);
    harness.probe.lane_close_release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let overview = harness.hub.overview().await;
            if overview.pending_cleanup_count == 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("old exact cleanup authority did not converge");
    assert!(!replacement.is_finished());
    assert!(
        harness
            .hub
            .list_lanes()
            .await
            .iter()
            .any(|lane| lane.lane_id == replacement_lane_id)
    );

    harness.probe.navigation_release.notify_one();
    let page = replacement
        .await
        .expect("replacement task panicked")
        .expect("replacement was killed by stale cleanup");
    assert_eq!(page.final_url, "https://spa.example.test/replacement");
    let closed = lock_unpoisoned(&harness.probe.closed_lane_ids);
    assert!(closed.contains(&old_lane_id));
    assert!(closed.contains(&replacement_lane_id));
    drop(closed);
    assert!(harness.hub.list_lanes().await.is_empty());
    fetcher.shutdown().await.expect("shutdown fetcher");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancellation_storm_keeps_exact_handoff_ledger_bounded_and_drains_to_zero() {
    // A cancelled Hub execute keeps its driver-operation authority until the
    // exact driver future acknowledges cancellation. Keep this storm below the
    // independently enforced task-operation ceiling while still accumulating
    // several simultaneous exact Lane cleanup handoffs.
    const CANCELLED_FETCHES: usize = 6;
    let mut config = HubConfig::default();
    config.resource_policy.max_open_lanes = 32;
    config.resource_policy.max_task_open_lanes = 32;
    config.resource_policy.max_active_operations =
        nomifun_browser_platform::MAX_TASK_ACTIVE_OPERATIONS;
    config.resource_policy.max_task_active_operations =
        nomifun_browser_platform::MAX_TASK_ACTIVE_OPERATIONS;
    config.resource_policy.max_task_memory_bytes =
        nomifun_browser_platform::MAX_TASK_MEMORY_BYTES;
    config.resource_policy.max_task_tabs = nomifun_browser_platform::MAX_TASK_TABS;
    let harness = Harness::new(config);
    let capacity = harness.hub.overview().await.capacity;
    assert_eq!(capacity.max_task_open_lanes, 32);
    assert_eq!(
        capacity.max_task_active_operations,
        nomifun_browser_platform::MAX_TASK_ACTIVE_OPERATIONS
    );
    let fetcher = Arc::new(harness.fetcher());
    let foreign = bind_test_client(
        &harness.hub,
        "installation-owner",
        "healthy-foreign-runtime",
    );
    let foreign_lane = foreign
        .open(
            Some("healthy-foreign"),
            BrowserIdentityMode::Anonymous,
            None,
        )
        .await
        .expect("open foreign healthy Lane")
        .lane()
        .lane_id
        .clone();

    harness
        .probe
        .block_navigation
        .store(true, Ordering::Release);
    harness
        .probe
        .block_lane_close
        .store(true, Ordering::Release);
    let mut peak_cleanup_count = 0;
    for index in 0..CANCELLED_FETCHES {
        let task_fetcher = Arc::clone(&fetcher);
        let task = tokio::spawn(async move {
            task_fetcher
                .fetch_page(&format!("https://spa.example.test/storm-{index}"))
                .await
        });
        wait_for_recorded_operations(&harness.probe, index + 1).await;
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        peak_cleanup_count = peak_cleanup_count.max(harness.hub.overview().await.pending_cleanup_count);
    }
    assert!(peak_cleanup_count > 1);
    assert!(
        peak_cleanup_count
            <= 2 * (nomifun_browser_platform::MAX_TASK_OPEN_LANES
                + nomifun_browser_platform::MAX_OWNER_QUEUE),
        "exact cleanup ledger exceeded the scheduler-derived per-task bound"
    );
    assert!(
        harness
            .hub
            .list_lanes()
            .await
            .iter()
            .any(|lane| lane.lane_id == foreign_lane),
        "exact handoff storm must not broaden to the foreign runtime"
    );

    harness
        .probe
        .block_lane_close
        .store(false, Ordering::Release);
    harness
        .probe
        .block_navigation
        .store(false, Ordering::Release);
    harness.probe.lane_close_release.notify_one();
    harness.probe.navigation_release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let overview = harness.hub.overview().await;
            let lanes = harness.hub.list_lanes().await;
            if overview.pending_cleanup_count == 0
                && lanes.len() == 1
                && lanes[0].lane_id == foreign_lane
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("exact cancellation storm did not drain to the healthy foreign Lane");

    foreign.close(&foreign_lane).await.expect("close foreign Lane");
    assert!(harness.hub.list_lanes().await.is_empty());
    assert_eq!(harness.hub.overview().await.pending_cleanup_count, 0);
    fetcher.shutdown().await.expect("shutdown fetcher");
}

#[tokio::test]
async fn many_sequential_fetches_leave_no_lane_host_or_profile_growth() {
    const FETCHES: usize = 256;

    let harness = Harness::new(HubConfig::default());
    let fetcher = harness.fetcher();
    for index in 0..FETCHES {
        let url = format!("https://site-{index}.example.test/page");
        let page = fetcher.fetch_page(&url).await.expect("sequential fetch");
        assert_eq!(page.final_url, url);
        let overview = harness.hub.overview().await;
        assert_eq!(overview.total_lanes, 0, "fetch {index} retained a Lane");
        assert_eq!(
            overview.pending_cleanup_count, 0,
            "fetch {index} retained cleanup debt"
        );
        assert_eq!(
            overview.managed_host_count, 0,
            "fetch {index} retained an Anonymous Host/profile"
        );
    }

    assert_eq!(harness.probe.lane_closes.load(Ordering::Acquire), FETCHES);
    assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), FETCHES);
    assert_eq!(
        harness
            .probe
            .active_anonymous_profiles
            .load(Ordering::Acquire),
        0
    );
    assert_eq!(
        harness
            .probe
            .peak_anonymous_profiles
            .load(Ordering::Acquire),
        1
    );
    fetcher.shutdown().await.expect("shutdown fetcher");
}

#[test]
fn drop_without_tokio_runtime_revokes_exact_owner() {
    let harness = Harness::new(HubConfig::default());
    let fetcher = harness.fetcher();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime");
    runtime
        .block_on(fetcher.fetch_page("https://spa.example.test/drop-cleanup"))
        .expect("initial fetch");
    let lease_id = fetcher.owner_lease_id().expect("owner lease");
    assert!(runtime.block_on(harness.hub.list_lanes()).is_empty());
    assert_eq!(harness.probe.lane_closes.load(Ordering::Acquire), 1);

    drop(runtime);
    drop(fetcher);

    // F49: Drop itself is non-blocking — it hands the bounded revoke to a
    // detached cleanup thread rather than block_on-driving Hub teardown from
    // whatever thread happens to run Drop. The transaction already closed its
    // Lane, so wait for the remaining exact lease revocation.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while harness.hub.renew_owner_lease(&lease_id).is_ok() {
        assert!(
            std::time::Instant::now() < deadline,
            "detached Drop cleanup never revoked the exact owner"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(harness.probe.lane_closes.load(Ordering::Acquire), 1);
    let verifier = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("verification runtime");
    assert!(verifier.block_on(harness.hub.list_lanes()).is_empty());
    assert_eq!(
        harness.hub.renew_owner_lease(&lease_id).unwrap_err().code,
        BrowserErrorCode::OwnerLeaseExpired
    );
}
