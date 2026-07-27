//! Native Agent Browser Platform capability provider.
//!
//! The provider lives at the application composition boundary because it needs
//! both the process-wide Browser Session Hub and the authoritative
//! Conversation/AgentExecution relation. The Agent factory supplies only
//! first-class runtime facts; this adapter enriches execution/attempt ownership
//! from persisted links before issuing a renewable owner lease.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nomifun_ai_agent::{
    BrowserLaneBinding, BrowserLaneClientProvider, BrowserOwnerLeaseGuard,
    TrustedBrowserRuntimeContext,
};
use nomifun_browser_platform::{
    BrowserOperationKind, BrowserSessionHub, BrowserSurface, CallerIdentity,
    OwnerLeaseId,
};
use nomifun_common::AppError;
use nomifun_conversation::{
    ConversationExecutionProjection, ExecutionConversationBoundary,
};

const OWNER_REVOKE_WAITER_TIMEOUT: Duration = Duration::from_secs(6);

const ALL_NATIVE_BROWSER_OPERATIONS: [BrowserOperationKind; 9] = [
    BrowserOperationKind::Navigate,
    BrowserOperationKind::Observe,
    BrowserOperationKind::Act,
    BrowserOperationKind::Screenshot,
    BrowserOperationKind::Tabs,
    BrowserOperationKind::Download,
    BrowserOperationKind::Debug,
    BrowserOperationKind::Manage,
    BrowserOperationKind::Crawl,
];

/// The Hub deliberately keeps the real Lane close task alive after a caller
/// timeout.  Provider cleanup must therefore run on an executor whose lifetime
/// is not tied to an Agent request, an HTTP request, or the Tokio runtime that
/// happened to create the binding.
///
/// In particular, do not replace this with `Handle::try_current()` or a
/// per-revoke runtime.  A request runtime may be torn down immediately after
/// `revoke_and_wait` times out, and dropping that runtime would abort the Hub's
/// detached close task while its `LaneCleanupFlight` remains pending forever.
struct NativeBrowserCleanupAuthority {
    handle: tokio::runtime::Handle,
}

static NATIVE_BROWSER_CLEANUP_AUTHORITY:
    std::sync::OnceLock<Result<NativeBrowserCleanupAuthority, Arc<str>>> =
    std::sync::OnceLock::new();

fn native_browser_cleanup_authority(
) -> Result<&'static NativeBrowserCleanupAuthority, Arc<str>> {
    NATIVE_BROWSER_CLEANUP_AUTHORITY
        .get_or_init(|| {
            let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel::<
                Result<tokio::runtime::Handle, Arc<str>>,
            >(1);

            let worker = std::thread::Builder::new()
                .name("nomifun-native-browser-cleanup-authority".to_owned())
                .spawn(move || {
                    let runtime = match tokio::runtime::Builder::new_multi_thread()
                        .enable_all()
                        .worker_threads(2)
                        .thread_name("nomifun-native-browser-cleanup")
                        .build()
                    {
                        Ok(runtime) => runtime,
                        Err(error) => {
                            let _ = ready_tx.send(Err(Arc::from(format!(
                                "failed to start native browser cleanup authority: {error}"
                            ))));
                            return;
                        }
                    };

                    let handle = runtime.handle().clone();
                    if ready_tx.send(Ok(handle)).is_err() {
                        return;
                    }

                    // This executor is intentionally process-lifetime.  The
                    // detached Hub cleanup tasks must remain runnable after a
                    // request/runtime caller has timed out or been dropped.
                    runtime.block_on(std::future::pending::<()>());
                });

            if let Err(error) = worker {
                return Err(Arc::from(format!(
                    "failed to spawn native browser cleanup authority: {error}"
                )));
            }

            // Dropping the JoinHandle intentionally detaches the authority
            // thread; the process owns its lifetime and terminates it at
            // process exit.
            ready_rx.recv().unwrap_or_else(|_| {
                Err(Arc::from(
                    "native browser cleanup authority exited before readiness",
                ))
            })
            .map(|handle| NativeBrowserCleanupAuthority { handle })
        })
        .as_ref()
        .map_err(Arc::clone)
}

fn spawn_native_browser_cleanup_task<F>(
    task: F,
) -> Result<tokio::task::JoinHandle<()>, Arc<str>>
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    native_browser_cleanup_authority().map(|authority| authority.handle.spawn(task))
}

pub(crate) struct HubBrowserLaneClientProvider {
    hub: Arc<BrowserSessionHub>,
    execution_boundary: Arc<dyn ExecutionConversationBoundary>,
    authoritative_user_id: Arc<str>,
    renewal_period: Duration,
}

impl HubBrowserLaneClientProvider {
    pub(crate) fn new(
        hub: Arc<BrowserSessionHub>,
        execution_boundary: Arc<dyn ExecutionConversationBoundary>,
        renewal_period: Duration,
        authoritative_user_id: Arc<str>,
    ) -> Self {
        Self {
            hub,
            execution_boundary,
            authoritative_user_id,
            renewal_period: renewal_period.max(Duration::from_millis(1)),
        }
    }

    async fn authoritative_execution_projection(
        &self,
        context: &TrustedBrowserRuntimeContext,
    ) -> Result<ConversationExecutionProjection, AppError> {
        let Some(conversation_id) = context.conversation_id.as_deref() else {
            return Ok(ConversationExecutionProjection::default());
        };
        self.execution_boundary
            .projection(&context.user_id, conversation_id)
            .await
    }

    fn new_owner_guard(
        &self,
        lease_id: OwnerLeaseId,
    ) -> Result<Arc<HubOwnerLeaseGuard>, AppError> {
        HubOwnerLeaseGuard::new(
            Arc::clone(&self.hub),
            lease_id,
            self.renewal_period,
        )
        .map(Arc::new)
        .map_err(|error| {
            AppError::Internal(format!(
                "failed to start native browser lease authority: {error}"
            ))
        })
    }

    fn require_installation_owner_authority(
        &self,
        context: &TrustedBrowserRuntimeContext,
    ) -> Result<(), AppError> {
        let authoritative_user_id = self.authoritative_user_id.as_ref();
        if authoritative_user_id.trim().is_empty()
            || authoritative_user_id.trim() != authoritative_user_id
        {
            return Err(AppError::Internal(
                "native browser provider is missing a canonical installation-owner authority"
                    .to_owned(),
            ));
        }

        // `TrustedBrowserRuntimeContext` carries the runtime principal, but it
        // does not carry an independently authoritative installation-owner
        // source. The Native provider therefore owns the installation owner
        // captured at app composition and treats any mismatch as a hard denial
        // before issuing a lease that can open the shared Primary identity.
        if context.user_id != authoritative_user_id {
            return Err(AppError::Forbidden(
                "native browser Primary identity requires the installation owner"
                    .to_owned(),
            ));
        }
        Ok(())
    }
}

fn merge_authoritative_id(
    field: &'static str,
    supplied: Option<String>,
    persisted: Option<String>,
) -> Result<Option<String>, AppError> {
    match (supplied, persisted) {
        (Some(supplied), Some(persisted)) if supplied != persisted => {
            Err(AppError::Conflict(format!(
                "trusted browser {field} conflicts with the authoritative conversation link"
            )))
        }
        (Some(supplied), _) => Ok(Some(supplied)),
        (None, persisted) => Ok(persisted),
    }
}

#[async_trait::async_trait]
impl BrowserLaneClientProvider for HubBrowserLaneClientProvider {
    async fn issue(
        &self,
        context: TrustedBrowserRuntimeContext,
    ) -> Result<BrowserLaneBinding, AppError> {
        if context.surface != BrowserSurface::Native {
            return Err(AppError::Forbidden(
                "the native browser lane provider only accepts the Native surface"
                    .to_owned(),
            ));
        }
        if context.user_id.trim().is_empty()
            || context.runtime_instance_id.trim().is_empty()
        {
            return Err(AppError::Internal(
                "trusted native browser runtime identity is incomplete".to_owned(),
            ));
        }
        self.require_installation_owner_authority(&context)?;

        let projection = self
            .authoritative_execution_projection(&context)
            .await?;
        let execution_id = merge_authoritative_id(
            "execution_id",
            context.execution_id,
            projection.linked_execution_id,
        )?;
        let step_id = merge_authoritative_id(
            "step_id",
            context.step_id,
            projection.execution_step_id,
        )?;
        let attempt_id = merge_authoritative_id(
            "attempt_id",
            context.attempt_id,
            projection.execution_attempt_id,
        )?;

        let owner_lease = self
            .hub
            .issue_owner_lease(
                context.user_id.clone(),
                context.conversation_id.clone(),
                context.runtime_instance_id.clone(),
            )
            .map_err(|error| {
                AppError::Internal(format!(
                    "failed to issue native browser owner lease: {error}"
                ))
            })?;

        // This client never leaves the process. Its renewable owner lease is
        // the actual short-lived expiry and revocation boundary validated on
        // every Hub call. A non-expiring outer timestamp avoids an independent,
        // immutable client deadline racing a successfully renewed lease.
        let caller = CallerIdentity {
            user_id: context.user_id,
            conversation_id: context.conversation_id,
            runtime_instance_id: context.runtime_instance_id,
            agent_id: context.agent_id,
            companion_id: None,
            execution_id,
            step_id,
            attempt_id,
            remote_connection_id: None,
            surface: context.surface,
            owner_lease_id: owner_lease.lease_id.clone(),
            capability_expires_at_ms: u64::MAX,
            allowed_operations: BTreeSet::from(ALL_NATIVE_BROWSER_OPERATIONS),
        };

        let client = match self.hub.bind(caller) {
            Ok(client) => client,
            Err(error) => {
                let _ = self
                    .hub
                    .revoke_owner_lease(&owner_lease.lease_id)
                    .await;
                return Err(AppError::Internal(format!(
                    "failed to bind native browser capability: {error}"
                )));
            }
        };
        let guard = match self.new_owner_guard(owner_lease.lease_id.clone()) {
            Ok(guard) => guard,
            Err(error) => {
                // No binding can safely escape without a process-lifetime
                // renewal/cleanup authority. Revoke the exact lease before
                // reporting construction failure.
                let cleanup = Arc::new(HubOwnerRevocation::new(
                    Arc::clone(&self.hub),
                    owner_lease.lease_id,
                ));
                if let Err(cleanup_error) = cleanup.revoke_and_wait().await {
                    tracing::warn!(
                        error = %cleanup_error,
                        "native browser lease cleanup after guard construction failure remains pending"
                    );
                }
                return Err(error);
            }
        };
        Ok(BrowserLaneBinding::new(client, guard))
    }
}

struct HubOwnerLeaseGuard {
    revocation: Arc<HubOwnerRevocation>,
    renew_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl HubOwnerLeaseGuard {
    fn new(
        hub: Arc<BrowserSessionHub>,
        lease_id: OwnerLeaseId,
        renewal_period: Duration,
    ) -> Result<Self, Arc<str>> {
        let revocation = Arc::new(HubOwnerRevocation::new(Arc::clone(&hub), lease_id));
        let renew_hub = Arc::clone(&revocation.hub);
        let renew_lease_id = revocation.lease_id.clone();
        let renew_revocation = Arc::clone(&revocation);
        let renew_task = spawn_native_browser_cleanup_task(async move {
            loop {
                tokio::time::sleep(renewal_period).await;
                if let Err(error) = renew_hub.renew_owner_lease(&renew_lease_id) {
                    tracing::warn!(
                        code = ?error.code,
                        retryable = error.retryable,
                        "native browser owner lease renewal failed; revoking its lanes"
                    );
                    // The cleanup attempt is exact-owner scoped and remains
                    // Hub-owned if this waiter is cancelled or times out.
                    if let Err(error) = renew_revocation.revoke_and_wait().await {
                        tracing::warn!(
                            error = %error,
                            "native browser owner lease cleanup after renewal failure remains pending"
                        );
                        renew_revocation.revoke();
                    }
                    break;
                }
            }
        })?;
        Ok(Self {
            revocation,
            renew_task: Mutex::new(Some(renew_task)),
        })
    }

    fn stop_renewal(&self) {
        if let Some(task) = self
            .renew_task
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            task.abort();
        }
    }
}

#[async_trait::async_trait]
impl BrowserOwnerLeaseGuard for HubOwnerLeaseGuard {
    fn revoke(&self) {
        self.stop_renewal();
        self.revocation.revoke();
    }

    async fn revoke_and_wait(&self) -> Result<(), AppError> {
        self.stop_renewal();
        self.revocation.revoke_and_wait().await
    }
}

impl Drop for HubOwnerLeaseGuard {
    fn drop(&mut self) {
        self.revoke();
    }
}

type OwnerRevokeResult = Result<(), Arc<str>>;

struct OwnerRevokeFlight {
    id: u64,
    result: std::sync::OnceLock<OwnerRevokeResult>,
    completed: tokio::sync::Notify,
}

impl OwnerRevokeFlight {
    fn new(id: u64) -> Arc<Self> {
        Arc::new(Self {
            id,
            result: std::sync::OnceLock::new(),
            completed: tokio::sync::Notify::new(),
        })
    }

    fn completed(result: OwnerRevokeResult) -> Arc<Self> {
        let flight = Arc::new(Self {
            id: 0,
            result: std::sync::OnceLock::new(),
            completed: tokio::sync::Notify::new(),
        });
        flight.complete(result);
        flight
    }

    fn complete(&self, result: OwnerRevokeResult) {
        let _ = self.result.set(result);
        self.completed.notify_waiters();
    }

    async fn wait(&self) -> OwnerRevokeResult {
        loop {
            let completed = self.completed.notified();
            if let Some(result) = self.result.get() {
                return result.clone();
            }
            completed.await;
        }
    }
}

#[derive(Default)]
struct OwnerRevokeState {
    next_id: u64,
    current: Option<Arc<OwnerRevokeFlight>>,
    retry_pending: bool,
    succeeded: bool,
}

/// Exact-owner cleanup coordinator.
///
/// The Hub owns detached Lane drivers in `pending_lane_cleanups`; this
/// coordinator only serializes the owner-level revoke request and gives
/// synchronous lifecycle hooks a durable background waiter. A failed attempt
/// clears the flight but leaves `retry_pending` set, so the next revoke joins
/// no stale "completed" bit and first retries the Hub's retained cleanup.
struct HubOwnerRevocation {
    hub: Arc<BrowserSessionHub>,
    lease_id: OwnerLeaseId,
    state: Mutex<OwnerRevokeState>,
}

impl HubOwnerRevocation {
    fn new(hub: Arc<BrowserSessionHub>, lease_id: OwnerLeaseId) -> Self {
        Self {
            hub,
            lease_id,
            state: Mutex::new(OwnerRevokeState::default()),
        }
    }

    fn revoke(self: &Arc<Self>) {
        let _ = self.start_or_join();
    }

    async fn revoke_and_wait(self: &Arc<Self>) -> Result<(), AppError> {
        let flight = self.start_or_join();
        let result = tokio::time::timeout(OWNER_REVOKE_WAITER_TIMEOUT, flight.wait())
            .await
            .map_err(|_| {
                AppError::Timeout(format!(
                    "native browser owner cleanup did not complete within {} ms",
                    OWNER_REVOKE_WAITER_TIMEOUT.as_millis()
                ))
            })?;
        result.map_err(|error| AppError::Internal(error.to_string()))
    }

    fn start_or_join(self: &Arc<Self>) -> Arc<OwnerRevokeFlight> {
        let (flight, is_new, retry_pending) = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if state.succeeded {
                (
                    OwnerRevokeFlight::completed(Ok(())),
                    false,
                    false,
                )
            } else if let Some(flight) = state.current.clone() {
                (flight, false, false)
            } else {
                state.next_id = state.next_id.wrapping_add(1);
                let flight = OwnerRevokeFlight::new(state.next_id);
                let retry_pending = state.retry_pending;
                state.retry_pending = false;
                state.current = Some(Arc::clone(&flight));
                (flight, true, retry_pending)
            }
        };

        if is_new {
            let coordinator = Arc::clone(self);
            let worker_flight = Arc::clone(&flight);
            if let Err(error) = spawn_native_browser_cleanup_task(async move {
                let result = coordinator.run_attempt(retry_pending).await;
                coordinator.finish(&worker_flight, result);
            }) {
                self.finish(&flight, Err(error));
            }
        }

        flight
    }

    async fn run_attempt(&self, retry_pending: bool) -> OwnerRevokeResult {
        if retry_pending
            && let Err(error) = self.hub.sweep().await
        {
            // `close_owner_lease` detaches a failed Lane into the Hub's
            // pending-cleanup queue. There is intentionally no private browser
            // or private retry queue here: sweep is the Hub's authoritative
            // retry entry point. Do not return early, though: a process-wide
            // sweep may also report an unrelated owner's failed cleanup.
            // Exact-owner revoke still has to run so this owner cannot be held
            // hostage by another retained target.
            tracing::warn!(
                code = ?error.code,
                retryable = error.retryable,
                owner_lease_id = %self.lease_id,
                "browser lifecycle sweep reported pending cleanup before exact-owner revoke"
            );
        }

        self.hub
            .revoke_owner_lease(&self.lease_id)
            .await
            .map(|_| ())
            .map_err(|error| format_browser_error("owner revoke failed", error))
    }

    fn finish(&self, flight: &Arc<OwnerRevokeFlight>, result: OwnerRevokeResult) {
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if state
                .current
                .as_ref()
                .is_some_and(|current| current.id == flight.id)
            {
                state.current = None;
                if result.is_ok() {
                    state.succeeded = true;
                    state.retry_pending = false;
                } else {
                    // A failed owner revoke is not a completed revocation. The
                    // Hub retains any detached driver, and the next exact-owner
                    // attempt must retry it before declaring success.
                    state.retry_pending = true;
                }
            }
        }
        // Followers retain this exact flight even if a retry starts
        // immediately after the state transition above.
        flight.complete(result);
    }
}

fn format_browser_error(
    context: &str,
    error: nomifun_browser_platform::BrowserPlatformError,
) -> Arc<str> {
    Arc::from(format!("{context} ({:?}): {}", error.code, error.message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use nomifun_browser_platform::{
        BrowserHostDriver, BrowserHostFactory, BrowserHostId,
        BrowserIdentityMode, BrowserLaneDriver,
        BrowserOperation, BrowserOperationResult, BrowserPlatformError,
        BrowserSurface, DriverOperationContext, HostLaunchRequest,
        HostLifecycleState, HubConfig, LaneLaunchRequest, ManualClock,
    };
    use nomifun_conversation::NoExecutionConversationBoundary;
    use std::collections::BTreeSet;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use tokio::sync::Barrier;

    const CONCURRENT_BINDINGS: usize = 16;

    struct FakeLane {
        closes: Arc<AtomicUsize>,
        close_failures_remaining: Arc<AtomicUsize>,
        close_started: Arc<tokio::sync::Semaphore>,
        close_release: Arc<tokio::sync::Semaphore>,
        block_close: Arc<AtomicBool>,
        close_drop_signals: Arc<Mutex<Vec<std::sync::mpsc::Sender<()>>>>,
    }

    struct CloseFutureDropSignal {
        signals: Arc<Mutex<Vec<std::sync::mpsc::Sender<()>>>>,
    }

    impl Drop for CloseFutureDropSignal {
        fn drop(&mut self) {
            let signals = {
                let mut signals = self
                    .signals
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                std::mem::take(&mut *signals)
            };
            for signal in signals {
                let _ = signal.send(());
            }
        }
    }

    #[async_trait]
    impl BrowserLaneDriver for FakeLane {
        async fn execute(
            &self,
            _operation: BrowserOperation,
            _context: DriverOperationContext,
        ) -> Result<BrowserOperationResult, BrowserPlatformError> {
            Ok(BrowserOperationResult {
                output: serde_json::json!({"ok": true}),
                tabs: Vec::new(),
                active_tab_id: None,
                active_frame_id: None,
                ref_generation: None,
            })
        }

        async fn close(&self) -> Result<(), BrowserPlatformError> {
            let _drop_signal = CloseFutureDropSignal {
                signals: Arc::clone(&self.close_drop_signals),
            };
            self.closes.fetch_add(1, Ordering::AcqRel);
            self.close_started.add_permits(1);
            if self.block_close.load(Ordering::Acquire) {
                self.close_release.acquire().await.unwrap().forget();
            }
            if self
                .close_failures_remaining
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                return Err(BrowserPlatformError::new(
                    nomifun_browser_platform::BrowserErrorCode::BrowserUnavailable,
                    "Synthetic native owner cleanup failure.",
                    true,
                    "Retry exact-owner cleanup.",
                ));
            }
            Ok(())
        }
    }

    struct FakeHost {
        id: BrowserHostId,
        lane_closes: Arc<AtomicUsize>,
        close_failures_remaining: Arc<AtomicUsize>,
        close_started: Arc<tokio::sync::Semaphore>,
        close_release: Arc<tokio::sync::Semaphore>,
        block_close: Arc<AtomicBool>,
        close_drop_signals: Arc<Mutex<Vec<std::sync::mpsc::Sender<()>>>>,
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
            Ok(Arc::new(FakeLane {
                closes: Arc::clone(&self.lane_closes),
                close_failures_remaining: Arc::clone(
                    &self.close_failures_remaining,
                ),
                close_started: Arc::clone(&self.close_started),
                close_release: Arc::clone(&self.close_release),
                block_close: Arc::clone(&self.block_close),
                close_drop_signals: Arc::clone(&self.close_drop_signals),
            }))
        }

        async fn shutdown(&self) -> Result<(), BrowserPlatformError> {
            Ok(())
        }
    }

    struct FakeFactory {
        launches: AtomicUsize,
        lane_closes: Arc<AtomicUsize>,
        host_ids: Mutex<Vec<BrowserHostId>>,
        close_failures_remaining: Arc<AtomicUsize>,
        close_started: Arc<tokio::sync::Semaphore>,
        close_release: Arc<tokio::sync::Semaphore>,
        block_close: Arc<AtomicBool>,
        close_drop_signals: Arc<Mutex<Vec<std::sync::mpsc::Sender<()>>>>,
    }

    impl Default for FakeFactory {
        fn default() -> Self {
            Self {
                launches: AtomicUsize::new(0),
                lane_closes: Arc::new(AtomicUsize::new(0)),
                host_ids: Mutex::new(Vec::new()),
                close_failures_remaining: Arc::new(AtomicUsize::new(0)),
                close_started: Arc::new(tokio::sync::Semaphore::new(0)),
                close_release: Arc::new(tokio::sync::Semaphore::new(0)),
                block_close: Arc::new(AtomicBool::new(false)),
                close_drop_signals: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl FakeFactory {
        fn signal_on_close_future_drop(&self) -> std::sync::mpsc::Receiver<()> {
            let (sender, receiver) = std::sync::mpsc::channel();
            self.close_drop_signals
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(sender);
            receiver
        }
    }

    #[async_trait]
    impl BrowserHostFactory for FakeFactory {
        async fn launch(
            &self,
            request: HostLaunchRequest,
        ) -> Result<Arc<dyn BrowserHostDriver>, BrowserPlatformError> {
            self.launches.fetch_add(1, Ordering::AcqRel);
            self.host_ids
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(request.host_id.clone());
            Ok(Arc::new(FakeHost {
                id: request.host_id,
                lane_closes: Arc::clone(&self.lane_closes),
                close_failures_remaining: Arc::clone(
                    &self.close_failures_remaining,
                ),
                close_started: Arc::clone(&self.close_started),
                close_release: Arc::clone(&self.close_release),
                block_close: Arc::clone(&self.block_close),
                close_drop_signals: Arc::clone(&self.close_drop_signals),
            }))
        }
    }

    struct ProjectionBoundary {
        projection: ConversationExecutionProjection,
    }

    #[async_trait]
    impl ExecutionConversationBoundary for ProjectionBoundary {
        async fn projection(
            &self,
            _owner_id: &str,
            _conversation_id: &str,
        ) -> Result<ConversationExecutionProjection, AppError> {
            Ok(self.projection.clone())
        }

        async fn is_active_attempt(
            &self,
            _owner_id: &str,
            _conversation_id: &str,
        ) -> Result<bool, AppError> {
            Ok(true)
        }

        async fn is_retained_attempt(
            &self,
            _owner_id: &str,
            _conversation_id: &str,
        ) -> Result<bool, AppError> {
            Ok(true)
        }
    }

    fn context(runtime: &str, attempt: &str) -> TrustedBrowserRuntimeContext {
        TrustedBrowserRuntimeContext {
            user_id: "owner".to_owned(),
            conversation_id: Some("conversation".to_owned()),
            runtime_instance_id: runtime.to_owned(),
            agent_id: Some("nomi".to_owned()),
            execution_id: Some("execution".to_owned()),
            step_id: Some(format!("step-{attempt}")),
            attempt_id: Some(attempt.to_owned()),
            surface: BrowserSurface::Native,
        }
    }

    #[tokio::test]
    async fn concurrent_runtime_attempt_bindings_own_unique_lanes_on_one_primary_host() {
        let factory = Arc::new(FakeFactory::default());
        let hub = Arc::new(BrowserSessionHub::new(
            factory.clone(),
            HubConfig::default(),
        ));
        let provider = Arc::new(HubBrowserLaneClientProvider::new(
            Arc::clone(&hub),
            Arc::new(NoExecutionConversationBoundary),
            Duration::from_secs(60),
            Arc::from("owner"),
        ));
        let start = Arc::new(Barrier::new(CONCURRENT_BINDINGS));
        let mut opening = Vec::with_capacity(CONCURRENT_BINDINGS);
        for index in 0..CONCURRENT_BINDINGS {
            let provider = Arc::clone(&provider);
            let start = Arc::clone(&start);
            let runtime = format!("runtime-{index:02}");
            let attempt = format!("attempt-{index:02}");
            opening.push(tokio::spawn(async move {
                let binding = provider
                    .issue(context(&runtime, &attempt))
                    .await
                    .expect("trusted runtime binding is issued");
                start.wait().await;
                let lane = binding
                    .client()
                    .open(None, BrowserIdentityMode::Primary, None)
                    .await
                    .expect("default Primary Lane opens")
                    .lane()
                    .clone();
                (runtime, attempt, binding, lane)
            }));
        }

        let opened = tokio::time::timeout(Duration::from_secs(5), async move {
            let mut opened = Vec::with_capacity(CONCURRENT_BINDINGS);
            for task in opening {
                opened.push(task.await.expect("binding task completes"));
            }
            opened
        })
        .await
        .expect("all concurrent default Primary opens complete");

        assert_eq!(opened.len(), CONCURRENT_BINDINGS);
        let lane_ids = opened
            .iter()
            .map(|(_, _, _, lane)| lane.lane_id.clone())
            .collect::<BTreeSet<_>>();
        let lane_keys = opened
            .iter()
            .map(|(_, _, _, lane)| lane.lane_key.clone())
            .collect::<BTreeSet<_>>();
        let owner_lease_ids = opened
            .iter()
            .map(|(_, _, _, lane)| lane.caller.owner_lease_id.clone())
            .collect::<BTreeSet<_>>();
        assert_eq!(lane_ids.len(), CONCURRENT_BINDINGS);
        assert_eq!(lane_keys.len(), CONCURRENT_BINDINGS);
        assert_eq!(owner_lease_ids.len(), CONCURRENT_BINDINGS);

        for (runtime, attempt, binding, lane) in &opened {
            let client = binding.client();
            assert_eq!(client.caller(), &lane.caller);
            let owned = client.list().await.expect("binding can list its Lane");
            assert_eq!(owned.len(), 1);
            assert_eq!(owned[0].lane_id, lane.lane_id);
            assert_eq!(lane.lane_key.runtime_instance_id, *runtime);
            assert_eq!(lane.lane_key.lane_name, "default");
            assert_eq!(lane.caller.user_id, "owner");
            assert_eq!(
                lane.caller.conversation_id.as_deref(),
                Some("conversation")
            );
            assert_eq!(lane.caller.runtime_instance_id, *runtime);
            assert_eq!(lane.caller.agent_id.as_deref(), Some("nomi"));
            assert_eq!(lane.caller.companion_id, None);
            assert_eq!(lane.caller.execution_id.as_deref(), Some("execution"));
            let expected_step = format!("step-{attempt}");
            assert_eq!(
                lane.caller.step_id.as_deref(),
                Some(expected_step.as_str())
            );
            assert_eq!(lane.caller.attempt_id.as_deref(), Some(attempt.as_str()));
            assert_eq!(lane.caller.remote_connection_id, None);
            assert_eq!(lane.caller.surface, BrowserSurface::Native);
            assert_eq!(lane.caller.capability_expires_at_ms, u64::MAX);
            assert_eq!(
                lane.caller.allowed_operations,
                BTreeSet::from(ALL_NATIVE_BROWSER_OPERATIONS)
            );
            assert_eq!(lane.identity_mode, BrowserIdentityMode::Primary);
            assert_eq!(lane.identity_generation, 0);
        }

        assert_eq!(factory.launches.load(Ordering::Acquire), 1);
        let launched_host_ids = factory
            .host_ids
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        assert_eq!(launched_host_ids.len(), 1);
        let overview = hub.overview().await;
        assert_eq!(overview.hosts.len(), 1);
        assert_eq!(overview.hosts[0].host_id, launched_host_ids[0]);
        assert_eq!(overview.hosts[0].identity_mode, BrowserIdentityMode::Primary);
        assert_eq!(overview.hosts[0].lane_count, CONCURRENT_BINDINGS);

        let revoked_lane_id = opened[7].3.lane_id.clone();
        let survivor_binding = opened[8].2.clone();
        let survivor_lane_ids = opened
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != 7)
            .map(|(_, (_, _, _, lane))| lane.lane_id.clone())
            .collect::<BTreeSet<_>>();
        opened[7].2.revoke();

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let current = hub
                    .list_lanes()
                    .await
                    .into_iter()
                    .map(|lane| lane.lane_id)
                    .collect::<BTreeSet<_>>();
                if current == survivor_lane_ids
                    && factory.lane_closes.load(Ordering::Acquire) == 1
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("revocation closes only its owned Lane");
        assert!(
            hub.list_lanes()
                .await
                .iter()
                .all(|lane| lane.lane_id != revoked_lane_id)
        );
        assert_eq!(factory.lane_closes.load(Ordering::Acquire), 1);
        assert_eq!(factory.launches.load(Ordering::Acquire), 1);
        let overview = hub.overview().await;
        assert_eq!(overview.hosts.len(), 1);
        assert_eq!(overview.hosts[0].host_id, launched_host_ids[0]);
        assert_eq!(overview.hosts[0].lane_count, CONCURRENT_BINDINGS - 1);
        let survivor_owned = survivor_binding
            .client()
            .list()
            .await
            .expect("sibling binding remains usable after revocation");
        assert_eq!(survivor_owned.len(), 1);
        assert_eq!(survivor_owned[0].lane_id, opened[8].3.lane_id);

        for (index, (_, _, binding, _)) in opened.iter().enumerate() {
            if index != 7 {
                binding.revoke();
            }
        }
    }

    #[tokio::test]
    async fn execution_identity_comes_from_authoritative_conversation_link() {
        let hub = Arc::new(BrowserSessionHub::new(
            Arc::new(FakeFactory::default()),
            HubConfig::default(),
        ));
        let provider = HubBrowserLaneClientProvider::new(
            Arc::clone(&hub),
            Arc::new(ProjectionBoundary {
                projection: ConversationExecutionProjection {
                    linked_execution_id: Some("persisted-execution".to_owned()),
                    execution_step_id: Some("persisted-step".to_owned()),
                    execution_attempt_id: Some("persisted-attempt".to_owned()),
                },
            }),
            Duration::from_secs(60),
            Arc::from("owner"),
        );
        let mut trusted = context("runtime-linked", "ignored");
        trusted.execution_id = None;
        trusted.step_id = None;
        trusted.attempt_id = None;

        let binding = provider.issue(trusted).await.unwrap();
        let client = binding.client();
        assert_eq!(
            client.caller().execution_id.as_deref(),
            Some("persisted-execution")
        );
        assert_eq!(
            client.caller().step_id.as_deref(),
            Some("persisted-step")
        );
        assert_eq!(
            client.caller().attempt_id.as_deref(),
            Some("persisted-attempt")
        );
        binding.revoke();
    }

    #[tokio::test]
    async fn non_installation_owner_cannot_receive_native_primary_identity_capability() {
        let factory = Arc::new(FakeFactory::default());
        let hub = Arc::new(BrowserSessionHub::new(
            factory.clone(),
            HubConfig::default(),
        ));
        let provider = HubBrowserLaneClientProvider::new(
            Arc::clone(&hub),
            Arc::new(NoExecutionConversationBoundary),
            Duration::from_secs(60),
            Arc::from("installation-owner"),
        );
        let mut trusted = context("runtime-non-owner", "attempt-non-owner");
        trusted.user_id = "secondary-user".to_owned();

        let error = provider.issue(trusted).await.unwrap_err();
        assert!(
            matches!(error, AppError::Forbidden(_)),
            "a non-installation owner must fail closed before receiving a Native browser capability: {error}"
        );
        assert!(
            hub.list_lanes().await.is_empty(),
            "rejected non-owner must not bind or open a shared Primary identity Lane"
        );
        assert_eq!(
            factory.launches.load(Ordering::Acquire),
            0,
            "rejected non-owner must not launch a browser host"
        );
    }

    #[tokio::test]
    async fn missing_installation_owner_authority_fails_closed_before_native_capability() {
        let factory = Arc::new(FakeFactory::default());
        let hub = Arc::new(BrowserSessionHub::new(
            factory.clone(),
            HubConfig::default(),
        ));
        let provider = HubBrowserLaneClientProvider::new(
            Arc::clone(&hub),
            Arc::new(NoExecutionConversationBoundary),
            Duration::from_secs(60),
            Arc::from(""),
        );

        let error = provider
            .issue(context("runtime-missing-owner", "attempt-missing-owner"))
            .await
            .unwrap_err();
        assert!(
            matches!(error, AppError::Internal(_)),
            "missing installation-owner source must fail closed: {error}"
        );
        assert!(hub.list_lanes().await.is_empty());
        assert_eq!(factory.launches.load(Ordering::Acquire), 0);
    }

    #[tokio::test]
    async fn installation_owner_keeps_native_primary_identity_compatibility() {
        let factory = Arc::new(FakeFactory::default());
        let hub = Arc::new(BrowserSessionHub::new(
            factory.clone(),
            HubConfig::default(),
        ));
        let provider = HubBrowserLaneClientProvider::new(
            Arc::clone(&hub),
            Arc::new(NoExecutionConversationBoundary),
            Duration::from_secs(60),
            Arc::from("owner"),
        );

        let binding = provider
            .issue(context("runtime-owner-primary", "attempt-owner-primary"))
            .await
            .expect("installation owner should retain Native browser capability");
        let lane = binding
            .client()
            .open(
                Some("owner-primary"),
                BrowserIdentityMode::Primary,
                None,
            )
            .await
            .expect("installation owner can open the shared Primary identity")
            .lane()
            .clone();

        assert_eq!(lane.identity_mode, BrowserIdentityMode::Primary);
        assert_eq!(lane.caller.user_id, "owner");
        assert_eq!(factory.launches.load(Ordering::Acquire), 1);
        binding.revoke_and_wait().await.unwrap();
    }

    #[tokio::test]
    async fn non_native_surfaces_are_rejected_before_issuing_native_capability() {
        let factory = Arc::new(FakeFactory::default());
        let hub = Arc::new(BrowserSessionHub::new(
            factory.clone(),
            HubConfig::default(),
        ));
        let provider = HubBrowserLaneClientProvider::new(
            Arc::clone(&hub),
            Arc::new(NoExecutionConversationBoundary),
            Duration::from_secs(60),
            Arc::from("owner"),
        );

        for surface in [
            BrowserSurface::Gateway,
            BrowserSurface::Acp,
            BrowserSurface::Remote,
            BrowserSurface::Cluster,
            BrowserSurface::User,
            BrowserSurface::System,
        ] {
            let mut trusted = context("runtime-non-native-surface", "attempt-surface");
            trusted.surface = surface;
            let error = provider.issue(trusted).await.unwrap_err();
            assert!(
                matches!(error, AppError::Forbidden(_)),
                "{surface:?} must be rejected by the Native provider: {error}"
            );
        }

        assert!(
            hub.list_lanes().await.is_empty(),
            "rejecting a non-Native surface must not bind a browser Lane"
        );
        assert_eq!(
            factory.launches.load(Ordering::Acquire),
            0,
            "rejecting a non-Native surface must happen before host launch"
        );
    }

    #[tokio::test]
    async fn owner_lease_is_renewed_for_a_live_runtime() {
        let clock = Arc::new(ManualClock::new(1_000));
        let mut config = HubConfig::default();
        config.owner_lease_ttl_ms = 100;
        let hub = Arc::new(BrowserSessionHub::with_clock(
            Arc::new(FakeFactory::default()),
            config,
            clock.clone(),
        ));
        let provider = HubBrowserLaneClientProvider::new(
            Arc::clone(&hub),
            Arc::new(NoExecutionConversationBoundary),
            Duration::from_millis(10),
            Arc::from("owner"),
        );
        let binding = provider
            .issue(context("runtime-renewed", "attempt-renewed"))
            .await
            .unwrap();

        clock.set(1_050);
        // This crate deliberately does not enable Tokio's `test-util`
        // feature. Give the 10 ms renewal worker ample real time to run
        // instead of depending on `time::advance`.
        tokio::time::sleep(Duration::from_millis(50)).await;
        // Past the original 1_100 expiry, but before the renewed 1_150 expiry.
        clock.set(1_125);
        binding
            .client()
            .list()
            .await
            .expect("renewed owner lease remains valid");
        binding.revoke();
    }

    #[tokio::test]
    async fn renewal_failure_retains_exact_owner_cleanup_authority() {
        let clock = Arc::new(ManualClock::new(1_000));
        let factory = Arc::new(FakeFactory::default());
        let mut config = HubConfig::default();
        config.owner_lease_ttl_ms = 50;
        let hub = Arc::new(BrowserSessionHub::with_clock(
            factory.clone(),
            config,
            clock.clone(),
        ));
        let provider = HubBrowserLaneClientProvider::new(
            Arc::clone(&hub),
            Arc::new(NoExecutionConversationBoundary),
            Duration::from_millis(10),
            Arc::from("owner"),
        );
        let binding = provider
            .issue(context("runtime-renewal-failure", "attempt-renewal-failure"))
            .await
            .unwrap();
        binding
            .client()
            .open(None, BrowserIdentityMode::Primary, None)
            .await
            .unwrap();
        factory
            .close_failures_remaining
            .store(1, Ordering::Release);

        clock.set(1_100);
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if hub.list_lanes().await.is_empty()
                    && factory.lane_closes.load(Ordering::Acquire) >= 2
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("renewal failure must retry retained exact-owner cleanup");
        assert_eq!(factory.lane_closes.load(Ordering::Acquire), 2);
        assert_eq!(factory.launches.load(Ordering::Acquire), 1);

        binding
            .revoke_and_wait()
            .await
            .expect("later lifecycle wait must observe successful renewal-failure cleanup");
        assert_eq!(factory.lane_closes.load(Ordering::Acquire), 2);
    }

    #[tokio::test]
    async fn failed_owner_revoke_is_retryable_and_reuses_hub_pending_cleanup() {
        let factory = Arc::new(FakeFactory::default());
        let hub = Arc::new(BrowserSessionHub::new(
            factory.clone(),
            HubConfig::default(),
        ));
        let provider = HubBrowserLaneClientProvider::new(
            Arc::clone(&hub),
            Arc::new(NoExecutionConversationBoundary),
            Duration::from_secs(60),
            Arc::from("owner"),
        );
        let binding = provider
            .issue(context("runtime-retry", "attempt-retry"))
            .await
            .unwrap();
        binding
            .client()
            .open(None, BrowserIdentityMode::Primary, None)
            .await
            .unwrap();
        factory
            .close_failures_remaining
            .store(1, Ordering::Release);

        let first = binding.revoke_and_wait().await.unwrap_err();
        assert!(
            first
                .to_string()
                .contains("pending cleanup retry failed")
                || first.to_string().contains("owner revoke failed"),
            "unexpected first cleanup error: {first}"
        );
        assert!(hub.list_lanes().await.is_empty());
        assert_eq!(factory.lane_closes.load(Ordering::Acquire), 1);
        assert_eq!(
            factory.launches.load(Ordering::Acquire),
            1,
            "retry must reuse the process-wide Hub, not launch a private browser"
        );

        binding
            .revoke_and_wait()
            .await
            .expect("a later exact-owner revoke must retry retained Hub cleanup");
        assert_eq!(factory.lane_closes.load(Ordering::Acquire), 2);
        assert_eq!(factory.launches.load(Ordering::Acquire), 1);
        assert!(
            hub.sweep().await.is_ok(),
            "successful retry must drain the Hub pending cleanup"
        );
    }

    #[tokio::test]
    async fn concurrent_owner_revoke_waiters_share_one_cleanup_flight() {
        let factory = Arc::new(FakeFactory::default());
        let hub = Arc::new(BrowserSessionHub::new(
            factory.clone(),
            HubConfig::default(),
        ));
        let provider = HubBrowserLaneClientProvider::new(
            Arc::clone(&hub),
            Arc::new(NoExecutionConversationBoundary),
            Duration::from_secs(60),
            Arc::from("owner"),
        );
        let binding = provider
            .issue(context("runtime-concurrent-revoke", "attempt-concurrent"))
            .await
            .unwrap();
        binding
            .client()
            .open(None, BrowserIdentityMode::Primary, None)
            .await
            .unwrap();
        factory.block_close.store(true, Ordering::Release);

        let first_binding = binding.clone();
        let first = tokio::spawn(async move {
            first_binding.revoke_and_wait().await
        });
        factory.close_started.acquire().await.unwrap().forget();
        let second_binding = binding.clone();
        let second = tokio::spawn(async move {
            second_binding.revoke_and_wait().await
        });
        tokio::time::sleep(Duration::from_millis(25)).await;
        assert_eq!(
            factory.lane_closes.load(Ordering::Acquire),
            1,
            "concurrent waiters must join the exact cleanup attempt"
        );

        factory.block_close.store(false, Ordering::Release);
        factory.close_release.add_permits(1);
        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();
        assert_eq!(factory.lane_closes.load(Ordering::Acquire), 1);
    }

    #[test]
    fn cleanup_flight_survives_creator_runtime_exit_and_later_waiter_completes() {
        let factory = Arc::new(FakeFactory::default());
        let hub = Arc::new(BrowserSessionHub::new(
            factory.clone(),
            HubConfig::default(),
        ));
        let drop_signal = factory.signal_on_close_future_drop();
        factory.block_close.store(true, Ordering::Release);

        let creator_runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let binding = creator_runtime.block_on(async {
            let provider = HubBrowserLaneClientProvider::new(
                Arc::clone(&hub),
                Arc::new(NoExecutionConversationBoundary),
                Duration::from_secs(60),
                Arc::from("owner"),
            );
            let binding = provider
                .issue(context("runtime-stable-authority", "attempt-stable-authority"))
                .await
                .unwrap();
            binding
                .client()
                .open(None, BrowserIdentityMode::Primary, None)
                .await
                .unwrap();
            binding.revoke();
            binding
        });

        creator_runtime.block_on(async {
            factory.close_started.acquire().await.unwrap().forget();
        });
        drop(creator_runtime);

        assert!(
            drop_signal
                .recv_timeout(Duration::from_secs(6))
                .is_err(),
            "the Hub timeout and creator-runtime exit must not abort the real cleanup future"
        );
        assert_eq!(
            factory.lane_closes.load(Ordering::Acquire),
            1,
            "the original exact-owner cleanup flight must remain in progress"
        );

        factory.block_close.store(false, Ordering::Release);
        factory.close_release.add_permits(1);
        let waiter_runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        waiter_runtime
            .block_on(binding.revoke_and_wait())
            .expect("a later runtime must join and observe the retained cleanup flight");
        assert!(waiter_runtime.block_on(hub.list_lanes()).is_empty());
        assert_eq!(factory.lane_closes.load(Ordering::Acquire), 1);
        drop_signal
            .recv_timeout(Duration::from_secs(1))
            .expect("the close future must finish normally after release");
        drop(binding);
    }

    #[tokio::test]
    async fn drop_starts_exact_owner_cleanup_and_retains_retry_authority() {
        let factory = Arc::new(FakeFactory::default());
        let hub = Arc::new(BrowserSessionHub::new(
            factory.clone(),
            HubConfig::default(),
        ));
        let provider = HubBrowserLaneClientProvider::new(
            Arc::clone(&hub),
            Arc::new(NoExecutionConversationBoundary),
            Duration::from_secs(60),
            Arc::from("owner"),
        );
        let binding = provider
            .issue(context("runtime-drop-cleanup", "attempt-drop-cleanup"))
            .await
            .unwrap();
        binding
            .client()
            .open(None, BrowserIdentityMode::Primary, None)
            .await
            .unwrap();
        factory
            .close_failures_remaining
            .store(1, Ordering::Release);
        let lease_id = binding.client().caller().owner_lease_id.clone();

        drop(binding);
        factory.close_started.acquire().await.unwrap().forget();
        assert!(
            hub.list_lanes().await.is_empty(),
            "final binding Drop must detach only its exact owner's Lane"
        );
        assert_eq!(factory.lane_closes.load(Ordering::Acquire), 1);

        if hub.sweep().await.is_err() {
            // The first sweep may still be joining Drop's just-finished failed
            // cleanup flight. A later lifecycle pass must start the retained
            // Hub retry without needing another Binding clone to stay alive.
            hub.sweep()
                .await
                .expect("Hub lifecycle retry must retain cleanup authority after Drop");
        }
        hub.revoke_owner_lease(&lease_id)
            .await
            .expect("the exact owner remains idempotently revoked");
        assert_eq!(factory.lane_closes.load(Ordering::Acquire), 2);
        assert_eq!(factory.launches.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn stale_runtime_owner_revoke_does_not_close_replacement_owner() {
        let factory = Arc::new(FakeFactory::default());
        let hub = Arc::new(BrowserSessionHub::new(
            factory.clone(),
            HubConfig::default(),
        ));
        let provider = HubBrowserLaneClientProvider::new(
            Arc::clone(&hub),
            Arc::new(NoExecutionConversationBoundary),
            Duration::from_secs(60),
            Arc::from("owner"),
        );
        let old = provider
            .issue(context("runtime-replacement", "attempt-old"))
            .await
            .unwrap();
        let old_lane = old
            .client()
            .open(
                Some("old-owner"),
                BrowserIdentityMode::Primary,
                None,
            )
            .await
            .unwrap()
            .lane()
            .lane_id
            .clone();
        let mut replacement_context =
            context("runtime-replacement", "attempt-old");
        replacement_context.step_id = Some("step-attempt-old".to_owned());
        let replacement = provider.issue(replacement_context).await.unwrap();
        let replacement_lane = replacement
            .client()
            .open(
                Some("replacement-owner"),
                BrowserIdentityMode::Primary,
                None,
            )
            .await
            .unwrap()
            .lane()
            .lane_id
            .clone();

        old.revoke_and_wait().await.unwrap();
        let remaining = hub.list_lanes().await;
        assert!(remaining.iter().all(|lane| lane.lane_id != old_lane));
        assert!(
            remaining
                .iter()
                .any(|lane| lane.lane_id == replacement_lane),
            "old exact owner must not close its same-runtime replacement"
        );
        replacement
            .client()
            .list()
            .await
            .expect("replacement owner remains usable");
        replacement.revoke_and_wait().await.unwrap();
    }

    #[tokio::test]
    async fn retrying_owner_isolated_from_an_unrelated_failed_pending_cleanup() {
        let factory = Arc::new(FakeFactory::default());
        let hub = Arc::new(BrowserSessionHub::new(
            factory.clone(),
            HubConfig::default(),
        ));
        let provider = HubBrowserLaneClientProvider::new(
            Arc::clone(&hub),
            Arc::new(NoExecutionConversationBoundary),
            Duration::from_secs(60),
            Arc::from("owner"),
        );

        let unrelated = provider
            .issue(context("runtime-unrelated-failure", "attempt-unrelated"))
            .await
            .unwrap();
        unrelated
            .client()
            .open(
                Some("unrelated-owner"),
                BrowserIdentityMode::Primary,
                None,
            )
            .await
            .unwrap();
        factory
            .close_failures_remaining
            .store(3, Ordering::Release);
        unrelated
            .revoke_and_wait()
            .await
            .expect_err("the unrelated owner leaves one retained failing cleanup");

        let exact = provider
            .issue(context("runtime-exact-retry", "attempt-exact"))
            .await
            .unwrap();
        exact
            .client()
            .open(
                Some("exact-owner"),
                BrowserIdentityMode::Primary,
                None,
            )
            .await
            .unwrap();
        exact
            .revoke_and_wait()
            .await
            .expect_err("the exact owner first close also fails");
        assert_eq!(factory.lane_closes.load(Ordering::Acquire), 2);

        exact
            .revoke_and_wait()
            .await
            .expect(
                "another owner's still-failing pending cleanup must not block exact-owner retry",
            );
        assert_eq!(
            factory.lane_closes.load(Ordering::Acquire),
            5,
            "Hub sweep retries retained targets before exact-owner revoke"
        );

        unrelated
            .revoke_and_wait()
            .await
            .expect("the unrelated retained cleanup remains independently retryable");
        assert_eq!(factory.lane_closes.load(Ordering::Acquire), 5);
    }

    #[test]
    fn merge_rejects_conflicting_persisted_attempt_identity() {
        let error = merge_authoritative_id(
            "attempt_id",
            Some("attempt-a".to_owned()),
            Some("attempt-b".to_owned()),
        )
        .unwrap_err();
        assert!(matches!(error, AppError::Conflict(_)));
    }

    #[test]
    fn revocation_outside_tokio_does_not_panic() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let guard = runtime.block_on(async {
            let hub = Arc::new(BrowserSessionHub::new(
                Arc::new(FakeFactory::default()),
                HubConfig::default(),
            ));
            let lease = hub
                .issue_owner_lease("owner", Some("conversation".to_owned()), "runtime")
                .unwrap();
            Arc::new(HubOwnerLeaseGuard::new(
                hub,
                lease.lease_id,
                Duration::from_secs(60),
            )
            .unwrap())
        });
        drop(runtime);
        guard.revoke();
    }
}
