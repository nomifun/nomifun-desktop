use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};
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
use tokio::sync::Mutex;

use super::{BrowserFetcher, rendered_to_page};

#[derive(Default)]
struct Probe {
    launches: StdMutex<Vec<HostLaunchRequest>>,
    operations: StdMutex<Vec<RecordedOperation>>,
    lane_closes: AtomicUsize,
    close_failures_remaining: AtomicUsize,
    host_shutdowns: AtomicUsize,
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
        lock_unpoisoned(&self.probe.launches).push(request.clone());
        Ok(Arc::new(FakeHost {
            host_id: request.host_id,
            probe: Arc::clone(&self.probe),
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

    let lanes = harness.hub.list_lanes().await;
    assert_eq!(lanes.len(), 1);
    assert_eq!(lanes[0].identity_mode, BrowserIdentityMode::Anonymous);
    assert_eq!(lanes[0].caller.surface, BrowserSurface::System);
    assert!(lanes[0].caller.conversation_id.is_none());
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
    assert!(
        harness.probe.lane_closes.load(Ordering::Acquire) >= 1,
        "the expired owner's stale Lane was not closed"
    );
    let lanes = harness.hub.list_lanes().await;
    assert_eq!(lanes.len(), 1);
    assert_eq!(lanes[0].caller.owner_lease_id, second_lease);
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
    assert_eq!(harness.hub.list_lanes().await.len(), 1);

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
async fn failed_last_lane_cleanup_is_resolved_by_authoritative_host_shutdown() {
    let harness = Harness::new(HubConfig::default());
    let fetcher = harness.fetcher();
    fetcher
        .fetch_page("https://spa.example.test/retry-cleanup")
        .await
        .expect("initial fetch");
    let lease_id = fetcher.owner_lease_id().expect("owner lease");
    harness
        .probe
        .close_failures_remaining
        .store(1, Ordering::Release);

    fetcher
        .shutdown()
        .await
        .expect("Host shutdown is authoritative after the last target close fails");
    assert_eq!(harness.probe.lane_closes.load(Ordering::Acquire), 1);
    assert!(
        harness.hub.list_lanes().await.is_empty(),
        "the exact owner Lane must be detached"
    );
    assert!(fetcher.owner_lease_id().is_none());
    let overview = harness.hub.overview().await;
    assert_eq!(overview.total_lanes, 0);
    assert_eq!(overview.pending_cleanup_count, 0);
    assert_eq!(overview.managed_host_count, 0);
    assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 1);
    assert!(
        harness.hub.sweep().await.is_ok(),
        "authoritative Host shutdown must drain retained target authority"
    );
    assert_eq!(
        harness.hub.renew_owner_lease(&lease_id).unwrap_err().code,
        BrowserErrorCode::OwnerLeaseExpired
    );
    fetcher
        .shutdown()
        .await
        .expect("repeated shutdown must remain idempotent");
    assert_eq!(harness.probe.lane_closes.load(Ordering::Acquire), 1);
    assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 1);
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
    assert_eq!(runtime.block_on(harness.hub.list_lanes()).len(), 1);

    drop(runtime);
    drop(fetcher);

    // F49: Drop itself is non-blocking — it hands the bounded revoke to a
    // detached cleanup thread rather than block_on-driving Hub teardown from
    // whatever thread happens to run Drop. Wait for that thread's effect.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while harness.probe.lane_closes.load(Ordering::Acquire) == 0 {
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
