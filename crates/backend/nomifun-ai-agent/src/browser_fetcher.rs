//! Hub-backed rendering [`PageFetcher`] for knowledge URL sources.
//!
//! The application composition root supplies the one process-wide
//! [`BrowserSessionHub`]. This adapter owns an application-issued lease and one
//! transaction-scoped Anonymous lane at a time; it never constructs an engine,
//! chooses a profile, or launches Chromium itself.
//!
//! A rendered fetch is a two-operation transaction (`navigate`, then
//! `rendered_html`). Knowledge fetch batching is concurrent, so a local mutex
//! protects the complete transaction from overlapping with a second fetch. The
//! Hub remains authoritative for lane serialization, admission, browser
//! lifecycle, and cleanup, and every transaction releases its Lane before
//! returning.
//!
//! The raw rendered HTML is converted with the same [`html_to_markdown`]
//! pipeline as the HTTP fetcher. It intentionally bypasses LLM-facing browser
//! extraction products, which may redact or wrap content.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, MutexGuard};
use std::time::Duration;

use async_trait::async_trait;
use nomifun_browser_platform::{
    BrowserErrorCode, BrowserIdentityMode, BrowserLaneClient, BrowserLaneId,
    BrowserLaneSnapshot, BrowserOperation, BrowserOperationKind, BrowserPlatformError,
    BrowserSessionHub, BrowserSurface, CallerIdentity, LaneLifecycleState, OpenLaneOutcome,
    OwnerLease, OwnerLeaseId,
};
use nomifun_common::{AppError, generate_id};
use nomifun_knowledge::PageFetcher;
use nomifun_knowledge::source_url::{
    FETCH_MAX_BYTES, FetchedPage, html_to_markdown, truncate_to_bytes,
};
use serde_json::{Value, json};
use tokio::sync::Mutex;

const KNOWLEDGE_LANE_PREFIX: &str = "knowledge";
const KNOWLEDGE_RUNTIME_PREFIX: &str = "knowledge-renderer";
/// Bounded wait for scheduler promotion of a queued knowledge Lane (F40). The
/// pre-Hub fetcher owned a serialized private engine where contention could
/// only delay a fetch, never fail it; a queued admission therefore waits for
/// capacity instead of failing the knowledge source outright. Only an
/// exhausted wait cancels the owned queue entry and surfaces the capacity
/// error.
const KNOWLEDGE_QUEUE_WAIT_TIMEOUT: Duration = Duration::from_secs(180);
const KNOWLEDGE_QUEUE_POLL_INTERVAL: Duration = Duration::from_millis(250);
/// Upper bound for the detached Drop-time owner revoke (F49).
const DROP_CLEANUP_TIMEOUT: Duration = Duration::from_secs(10);

/// Cancellation-safe cleanup authority for one knowledge-render transaction.
///
/// Normal completion closes the exact Lane. If the fetch future is cancelled
/// after admission, Drop synchronously transfers the exact Lane id and sealed
/// owner generation to the Hub's bounded supervisor. Each transaction also
/// has a unique Lane name, so a replacement fetch cannot reuse the old Lane
/// while its physical cleanup is still pending.
struct FetchLaneCleanup {
    client: BrowserLaneClient,
    lane_id: BrowserLaneId,
    armed: bool,
}

impl FetchLaneCleanup {
    fn new(client: BrowserLaneClient, lane_id: BrowserLaneId) -> Self {
        Self {
            client,
            lane_id,
            armed: true,
        }
    }

    async fn finish(mut self) -> Result<(), BrowserPlatformError> {
        match self.client.close(&self.lane_id).await {
            Ok(_) => {
                self.armed = false;
                Ok(())
            }
            Err(error) => {
                // `close` has already retained exact driver/Host authority
                // when physical teardown is uncertain. This handoff also
                // covers pre-driver failures such as a capability expiring
                // between the transaction and cleanup.
                self.client
                    .handoff_bound_lane_cleanup(self.lane_id.clone())?;
                self.armed = false;
                Err(error)
            }
        }
    }
}

impl Drop for FetchLaneCleanup {
    fn drop(&mut self) {
        if self.armed {
            if let Err(error) = self
                .client
                .handoff_bound_lane_cleanup(self.lane_id.clone())
            {
                tracing::error!(
                    lane_id = %self.lane_id,
                    code = ?error.code,
                    "failed to retain exact knowledge Lane cleanup authority"
                );
            }
            self.armed = false;
        }
    }
}

/// Hub-backed rendering fetcher pinned to Anonymous identity.
pub struct BrowserFetcher {
    hub: Arc<BrowserSessionHub>,
    user_id: String,
    runtime_instance_id: String,
    owner_lease_id: StdMutex<Option<OwnerLeaseId>>,
    fetch_gate: Mutex<()>,
    lane_sequence: AtomicU64,
    closed: AtomicBool,
    cleanup_retry_pending: AtomicBool,
    queue_wait_timeout: Duration,
}

impl BrowserFetcher {
    /// Bind the fetcher to the process-wide Hub and authoritative installation
    /// owner. Construction is side-effect free: no lane or host is opened.
    pub fn new(hub: Arc<BrowserSessionHub>, user_id: impl Into<String>) -> Self {
        Self::with_runtime_instance_id(
            hub,
            user_id.into(),
            format!("{KNOWLEDGE_RUNTIME_PREFIX}-{}", generate_id()),
        )
    }

    fn with_runtime_instance_id(
        hub: Arc<BrowserSessionHub>,
        user_id: String,
        runtime_instance_id: String,
    ) -> Self {
        Self {
            hub,
            user_id,
            runtime_instance_id,
            owner_lease_id: StdMutex::new(None),
            fetch_gate: Mutex::new(()),
            lane_sequence: AtomicU64::new(1),
            closed: AtomicBool::new(false),
            cleanup_retry_pending: AtomicBool::new(false),
            queue_wait_timeout: KNOWLEDGE_QUEUE_WAIT_TIMEOUT,
        }
    }

    #[cfg(test)]
    fn set_queue_wait_timeout(&mut self, timeout: Duration) {
        self.queue_wait_timeout = timeout;
    }

    fn next_lane_name(&self) -> String {
        let sequence = self.lane_sequence.fetch_add(1, Ordering::AcqRel);
        // 26 ASCII bytes, safely below the platform's 32-byte Lane-name cap.
        format!("{KNOWLEDGE_LANE_PREFIX}-{sequence:016x}")
    }

    fn owner_lease_id(&self) -> Option<OwnerLeaseId> {
        lock_unpoisoned(&self.owner_lease_id).clone()
    }

    fn set_owner_lease_id(&self, lease_id: Option<OwnerLeaseId>) {
        *lock_unpoisoned(&self.owner_lease_id) = lease_id;
    }

    fn caller_for(&self, lease: &OwnerLease) -> CallerIdentity {
        CallerIdentity {
            user_id: self.user_id.clone(),
            conversation_id: None,
            runtime_instance_id: self.runtime_instance_id.clone(),
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

    async fn issue_and_bind(&self) -> Result<BrowserLaneClient, BrowserPlatformError> {
        let lease = self.hub.issue_owner_lease(
            self.user_id.clone(),
            None,
            self.runtime_instance_id.clone(),
        )?;
        // Record cleanup authority before binding. If bind fails and revocation
        // also fails, retaining the exact lease id lets shutdown retry instead
        // of orphaning an owner that this fetcher can no longer address.
        self.set_owner_lease_id(Some(lease.lease_id.clone()));
        let caller = self.caller_for(&lease);
        match self.hub.bind(caller) {
            Ok(client) => Ok(client),
            Err(error) => {
                match self.hub.revoke_owner_lease(&lease.lease_id).await {
                    Ok(_) => {
                        self.clear_owner_lease_if(&lease.lease_id);
                        Err(error)
                    }
                    Err(revoke_error) => {
                        self.cleanup_retry_pending.store(true, Ordering::Release);
                        Err(revoke_error)
                    }
                }
            }
        }
    }

    /// Renew both the owner lease and the bound capability before every fetch.
    /// If the owner expired while idle, close its stale runtime before issuing
    /// a replacement lease so the old lane can never be reused under a new
    /// owner capability.
    async fn client_for_fetch(&self) -> Result<BrowserLaneClient, BrowserPlatformError> {
        if let Some(lease_id) = self.owner_lease_id() {
            match self.hub.renew_owner_lease(&lease_id) {
                Ok(lease) => match self.hub.bind(self.caller_for(&lease)) {
                    Ok(client) => return Ok(client),
                    Err(error) if error.code == BrowserErrorCode::OwnerLeaseExpired => {
                        self.retire_owner().await?;
                    }
                    Err(error) => return Err(error),
                },
                Err(error) if error.code == BrowserErrorCode::OwnerLeaseExpired => {
                    self.retire_owner().await?;
                }
                Err(error) => return Err(error),
            }
        }
        self.issue_and_bind().await
    }

    /// Revoke the known lease and close only the resources bound to that exact
    /// owner. Hub revocation also cleans already-expired leases, so a broad
    /// runtime close is neither needed nor safe if a replacement capability
    /// later reuses the runtime identifier.
    async fn retire_owner(&self) -> Result<(), BrowserPlatformError> {
        let lease_id = self.owner_lease_id();
        if let Some(lease_id) = lease_id {
            // A failed exact-owner revoke leaves the detached driver in the
            // Hub's retained cleanup queue. Retry that authoritative queue
            // before attempting to retire the already-revoked lease again;
            // otherwise a second shutdown would see no inventory lane and
            // could report success while the driver was still pending.
            if self.cleanup_retry_pending.load(Ordering::Acquire) {
                if let Err(error) = self.hub.sweep().await {
                    return Err(error);
                }
            }

            match self.hub.revoke_owner_lease(&lease_id).await {
                Ok(_) => {
                    self.cleanup_retry_pending.store(false, Ordering::Release);
                    self.clear_owner_lease_if(&lease_id);
                }
                Err(error) => {
                    self.cleanup_retry_pending.store(true, Ordering::Release);
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    fn clear_owner_lease_if(&self, expected: &OwnerLeaseId) {
        let mut stored = lock_unpoisoned(&self.owner_lease_id);
        if stored.as_ref() == Some(expected) {
            *stored = None;
        }
    }

    /// Wait for a queued knowledge Lane to be promoted to Running, bounded by
    /// [`Self::queue_wait_timeout`]. On timeout (or a Lane that leaves the
    /// queue without ever running) the owned queue entry is cancelled so it
    /// cannot start later as an orphan, and the capacity error surfaces with
    /// the freshest queue metadata.
    async fn wait_for_queued_lane(
        &self,
        client: &BrowserLaneClient,
        mut lane: BrowserLaneSnapshot,
    ) -> Result<BrowserLaneSnapshot, FetchFailure> {
        let deadline = tokio::time::Instant::now() + self.queue_wait_timeout;
        loop {
            match lane.lifecycle_state {
                LaneLifecycleState::Running => return Ok(lane),
                LaneLifecycleState::Queued | LaneLifecycleState::Starting => {}
                LaneLifecycleState::Frozen
                | LaneLifecycleState::Stopping
                | LaneLifecycleState::Failed => {
                    return Err(FetchFailure::platform(
                        "waiting for the queued knowledge lane",
                        BrowserPlatformError::new(
                            BrowserErrorCode::BrowserUnavailable,
                            "The queued knowledge browser lane stopped before it could run.",
                            true,
                            "Retry the knowledge source.",
                        )
                        .for_lane(lane.lane_id),
                    ));
                }
            }
            if tokio::time::Instant::now() >= deadline {
                let queue = lane
                    .queue
                    .as_ref()
                    .and_then(|value| serde_json::to_value(value).ok())
                    .unwrap_or(Value::Null);
                return Err(FetchFailure::platform(
                    "opening Anonymous knowledge lane",
                    BrowserPlatformError::new(
                        BrowserErrorCode::BrowserCapacityQueued,
                        "The knowledge browser lane is queued for capacity.",
                        true,
                        "Retry the knowledge source after the reported queue delay.",
                    )
                    .for_lane(lane.lane_id)
                    .with_metadata(json!({ "queue": queue })),
                ));
            }
            tokio::time::sleep(KNOWLEDGE_QUEUE_POLL_INTERVAL).await;
            lane = client.status(&lane.lane_id).await.map_err(|error| {
                FetchFailure::platform("waiting for the queued knowledge lane", error)
            })?;
        }
    }

    async fn fetch_once(
        &self,
        client: &BrowserLaneClient,
        raw_url: &str,
    ) -> Result<(String, String, bool), FetchFailure> {
        let lane_name = self.next_lane_name();
        let outcome = client
            .open(
                Some(&lane_name),
                BrowserIdentityMode::Anonymous,
                None,
            )
            .await
            .map_err(|error| {
                FetchFailure::platform("opening Anonymous knowledge lane", error)
            })?;

        // Arm cleanup immediately after admission and before another await.
        // Cancellation during `open` is owned by the Hub's admission guards;
        // cancellation after `open` is owned by this transaction guard.
        let initial_lane = outcome.lane().clone();
        let cleanup = FetchLaneCleanup::new(client.clone(), initial_lane.lane_id.clone());

        let lane = match outcome {
            OpenLaneOutcome::Running { lane } => lane,
            // F40: a queued admission waits (bounded) for scheduler promotion
            // so knowledge ingestion degrades to "slower" under contention,
            // not "failed" — matching the pre-Hub serialized-engine behavior.
            OpenLaneOutcome::Queued { lane } => {
                match self.wait_for_queued_lane(client, lane).await {
                    Ok(lane) => lane,
                    Err(error) => {
                        return match cleanup.finish().await {
                            Ok(()) => Err(error),
                            Err(cleanup_error) => Err(FetchFailure::platform(
                                "closing the Anonymous knowledge lane after a queue failure",
                                cleanup_error,
                            )),
                        };
                    }
                }
            }
        };

        let result = async {
            if lane.identity_mode != BrowserIdentityMode::Anonymous {
                return Err(FetchFailure::platform(
                    "validating knowledge lane identity",
                    BrowserPlatformError::new(
                        BrowserErrorCode::InvalidCallerIdentity,
                        "Knowledge rendering requires an Anonymous browser lane.",
                        false,
                        "Recreate the renderer through the application Browser Session Hub.",
                    )
                    .for_lane(lane.lane_id.clone()),
                ));
            }

            let navigation = client
                .execute(
                    &lane.lane_id,
                    crawl_operation("navigate", json!({ "url": raw_url, "new_tab": false })),
                )
                .await
                .map_err(|error| {
                    FetchFailure::platform("navigating the knowledge URL", error)
                })?;
            let final_url = navigation
                .output
                .get("final_url")
                .and_then(Value::as_str)
                .unwrap_or(raw_url)
                .to_owned();

            let rendered = client
                .execute(
                    &lane.lane_id,
                    crawl_operation("rendered_html", json!({})),
                )
                .await
                .map_err(|error| {
                    FetchFailure::platform("reading rendered knowledge HTML", error)
                })?;
            let html = rendered
                .output
                .get("html")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    FetchFailure::invalid_response(
                        "The Browser Platform returned no rendered HTML for the knowledge URL.",
                    )
                })?
                .to_owned();
            let html_truncated = rendered
                .output
                .get("html_truncated")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            Ok((final_url, html, html_truncated))
        }
        .await;

        match cleanup.finish().await {
            Ok(()) => result,
            Err(error) => Err(FetchFailure::platform(
                "closing the Anonymous knowledge lane after rendering",
                error,
            )),
        }
    }

    /// Deterministically revoke the capability and close all lanes owned by
    /// this renderer. Repeated calls retry idempotent platform cleanup.
    pub async fn shutdown(&self) -> Result<(), BrowserPlatformError> {
        let _gate = self.fetch_gate.lock().await;
        self.closed.store(true, Ordering::Release);
        self.retire_owner().await
    }
}

#[async_trait]
impl PageFetcher for BrowserFetcher {
    async fn fetch_page(&self, raw_url: &str) -> Result<FetchedPage, AppError> {
        let _gate = self.fetch_gate.lock().await;
        if self.closed.load(Ordering::Acquire) {
            return Err(AppError::BadGateway(
                "knowledge Browser Platform client is shutting down".to_owned(),
            ));
        }

        // Retry only a lease that expires between preflight renewal and an
        // operation. Every other platform/capacity failure is surfaced without
        // falling back to a private browser.
        for attempt in 0..2 {
            let client = self.client_for_fetch().await.map_err(|error| {
                platform_app_error(
                    "acquiring knowledge browser capability",
                    raw_url,
                    error,
                )
            })?;
            match self.fetch_once(&client, raw_url).await {
                Ok((final_url, html, html_truncated)) => {
                    return Ok(rendered_to_page_with_source_truncation(
                        &final_url,
                        &html,
                        html_truncated,
                    ));
                }
                Err(FetchFailure::Platform { error, .. })
                    if attempt == 0 && error.code == BrowserErrorCode::OwnerLeaseExpired =>
                {
                    self.retire_owner().await.map_err(|retire_error| {
                        platform_app_error(
                            "retiring expired knowledge browser capability",
                            raw_url,
                            retire_error,
                        )
                    })?;
                }
                Err(error) => return Err(error.into_app_error(raw_url)),
            }
        }

        Err(AppError::BadGateway(format!(
            "knowledge browser lease retry exhausted for {raw_url}"
        )))
    }
}

impl Drop for BrowserFetcher {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Release);
        let lease_id = lock_unpoisoned(&self.owner_lease_id).take();
        let Some(lease_id) = lease_id else {
            return;
        };
        let hub = Arc::clone(&self.hub);
        let retry_pending = self.cleanup_retry_pending.load(Ordering::Acquire);
        let revoke = async move {
            if retry_pending {
                let _ = hub.sweep().await;
            }
            let _ = hub.revoke_owner_lease(&lease_id).await;
        };
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(revoke);
            return;
        }

        // Drop may run after the application's Tokio runtime has already
        // stopped. Never drive Hub cleanup with a block_on here (F49): the
        // Hub's lane/host teardown awaits CDP sockets and child-process
        // handles registered on the (dead) main runtime, so blocking this
        // Drop could stall or wedge the exit path. Hand the bounded revoke to
        // a detached thread instead — best-effort by design, with global Hub
        // shutdown and kill-on-drop children as the authoritative backstops.
        let spawned = std::thread::Builder::new()
            .name("nomi-knowledge-fetcher-cleanup".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                match runtime {
                    Ok(runtime) => {
                        let _ = runtime.block_on(async {
                            tokio::time::timeout(DROP_CLEANUP_TIMEOUT, revoke).await
                        });
                    }
                    Err(error) => {
                        tracing::error!(
                            %error,
                            "failed to build cleanup runtime for knowledge browser owner"
                        );
                    }
                }
            });
        if let Err(error) = spawned {
            tracing::error!(
                %error,
                "failed to spawn cleanup thread for knowledge browser owner"
            );
        }
    }
}

enum FetchFailure {
    Platform {
        context: &'static str,
        error: BrowserPlatformError,
    },
    InvalidResponse(String),
}

impl FetchFailure {
    fn platform(context: &'static str, error: BrowserPlatformError) -> Self {
        Self::Platform { context, error }
    }

    fn invalid_response(message: impl Into<String>) -> Self {
        Self::InvalidResponse(message.into())
    }

    fn into_app_error(self, raw_url: &str) -> AppError {
        match self {
            Self::Platform { context, error } => {
                platform_app_error(context, raw_url, error)
            }
            Self::InvalidResponse(message) => {
                AppError::BadGateway(format!("{message} URL: {raw_url}"))
            }
        }
    }
}

fn crawl_operation(action: &str, input: Value) -> BrowserOperation {
    BrowserOperation {
        kind: BrowserOperationKind::Crawl,
        action: action.to_owned(),
        input,
        expected_browser_epoch: None,
        target_id: None,
        frame_id: None,
        ref_generation: None,
        may_modify_identity: false,
    }
}

fn platform_app_error(
    context: &str,
    raw_url: &str,
    error: BrowserPlatformError,
) -> AppError {
    let code = serde_json::to_value(error.code)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("{:?}", error.code));
    let metadata = if error.metadata.is_null() {
        String::new()
    } else {
        format!("; metadata={}", error.metadata)
    };
    AppError::BadGateway(format!(
        "{context} failed for {raw_url}: code={code}; {}; retryable={}; next_action={}{}",
        error.message, error.retryable, error.next_action, metadata
    ))
}

fn lock_unpoisoned<T>(mutex: &StdMutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn rendered_to_page(final_url: &str, html: &str) -> FetchedPage {
    rendered_to_page_with_source_truncation(final_url, html, false)
}

fn rendered_to_page_with_source_truncation(
    final_url: &str,
    html: &str,
    source_truncated: bool,
) -> FetchedPage {
    let (title, markdown) = html_to_markdown(html);
    let markdown_truncated = markdown.len() > FETCH_MAX_BYTES;
    let markdown = if markdown_truncated {
        truncate_to_bytes(&markdown, FETCH_MAX_BYTES).to_owned()
    } else {
        markdown
    };
    FetchedPage {
        final_url: final_url.to_owned(),
        title,
        markdown,
        truncated: source_truncated || markdown_truncated,
    }
}

#[cfg(test)]
#[path = "browser_fetcher_tests.rs"]
mod tests;
