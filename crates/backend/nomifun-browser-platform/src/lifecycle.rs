//! Host failure/restart coordination primitives.
//!
//! The Hub owns policy and decides whether an error is target-local or
//! host-fatal. Once it has classified a host failure, these types provide the
//! two concurrency guarantees required by the lifecycle contract:
//!
//! - a rolling 60-second / 3-failure circuit breaker; and
//! - cancellation-safe, per-host-key restart single-flight.
//!
//! The restart primitive deliberately does not know how to launch a browser or
//! rebind lanes. The leader supplies that future. The future is spawned so an
//! aborted request cannot abandon authoritative Host recovery halfway through.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::hash::Hash;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::{Mutex, Notify};

use crate::{BrowserErrorCode, BrowserPlatformError, Clock, SystemClock};

/// Host circuit window required by FR-LIFE-011.
pub const HOST_FAILURE_WINDOW_MS: u64 = 60_000;
/// The third distinct Host failure inside the rolling window opens the circuit.
pub const HOST_FAILURE_THRESHOLD: usize = 3;

/// Rolling Host-failure circuit policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostCircuitPolicy {
    pub window_ms: u64,
    pub failure_threshold: usize,
}

impl Default for HostCircuitPolicy {
    fn default() -> Self {
        Self {
            window_ms: HOST_FAILURE_WINDOW_MS,
            failure_threshold: HOST_FAILURE_THRESHOLD,
        }
    }
}

impl HostCircuitPolicy {
    /// Construct a non-degenerate policy.
    pub fn new(window_ms: u64, failure_threshold: usize) -> Option<Self> {
        (window_ms > 0 && failure_threshold > 0).then_some(Self {
            window_ms,
            failure_threshold,
        })
    }
}

/// Point-in-time state of one Host circuit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum HostCircuitSnapshot {
    Closed {
        failures_in_window: usize,
        failures_remaining: usize,
    },
    Open {
        failures_in_window: usize,
        retry_at_ms: u64,
        retry_after_ms: u64,
    },
    HalfOpen {
        failures_in_window: usize,
        retry_at_ms: u64,
    },
}

impl HostCircuitSnapshot {
    pub fn is_open(self) -> bool {
        matches!(self, Self::Open { .. } | Self::HalfOpen { .. })
    }

    pub fn is_half_open(self) -> bool {
        matches!(self, Self::HalfOpen { .. })
    }

    /// Stable typed platform error for a recovery attempt blocked by the
    /// circuit. No process/CDP detail is exposed.
    pub fn browser_unavailable_error(self) -> BrowserPlatformError {
        let metadata = match self {
            Self::Closed {
                failures_in_window,
                failures_remaining,
            } => json!({
                "circuit_open": false,
                "failures_in_window": failures_in_window,
                "failures_remaining": failures_remaining,
            }),
            Self::Open {
                failures_in_window,
                retry_at_ms,
                retry_after_ms,
            } => json!({
                "circuit_open": true,
                "failures_in_window": failures_in_window,
                "retry_at_ms": retry_at_ms,
                "retry_after_ms": retry_after_ms,
            }),
            Self::HalfOpen {
                failures_in_window,
                retry_at_ms,
            } => json!({
                "circuit_open": true,
                "circuit_half_open": true,
                "failures_in_window": failures_in_window,
                "retry_at_ms": retry_at_ms,
                "retry_after_ms": 0,
            }),
        };
        BrowserPlatformError::new(
            BrowserErrorCode::BrowserUnavailable,
            "The managed browser restart circuit is temporarily open.",
            true,
            "Wait for the retry window, then open or observe the browser lane again.",
        )
        .with_metadata(metadata)
    }
}

#[derive(Default)]
struct HostFailureWindow {
    failures: VecDeque<u64>,
    open_retry_at_ms: Option<u64>,
    half_open_probe_in_flight: bool,
}

impl HostFailureWindow {
    fn prune(&mut self, now_ms: u64, window_ms: u64) {
        while self
            .failures
            .front()
            .is_some_and(|failure_ms| now_ms.saturating_sub(*failure_ms) >= window_ms)
        {
            self.failures.pop_front();
        }
    }

    fn snapshot(&mut self, now_ms: u64, policy: HostCircuitPolicy) -> HostCircuitSnapshot {
        self.prune(now_ms, policy.window_ms);
        let failures_in_window = self.failures.len();
        if let Some(retry_at_ms) = self.open_retry_at_ms {
            if now_ms >= retry_at_ms {
                return HostCircuitSnapshot::HalfOpen {
                    failures_in_window,
                    retry_at_ms,
                };
            }
            HostCircuitSnapshot::Open {
                failures_in_window,
                retry_at_ms,
                retry_after_ms: retry_at_ms.saturating_sub(now_ms),
            }
        } else {
            HostCircuitSnapshot::Closed {
                failures_in_window,
                failures_remaining: policy.failure_threshold.saturating_sub(failures_in_window),
            }
        }
    }
}

pub(crate) enum HostCircuitAttempt {
    Closed,
    HalfOpen(HostCircuitProbe),
}

impl HostCircuitAttempt {
    pub(crate) fn is_half_open(&self) -> bool {
        matches!(self, Self::HalfOpen(_))
    }

    pub(crate) fn succeed(self) {
        if let Self::HalfOpen(probe) = self {
            probe.succeed();
        }
    }

    pub(crate) fn fail(self) {
        if let Self::HalfOpen(probe) = self {
            probe.fail();
        }
    }
}

pub(crate) struct HostCircuitProbe {
    circuit: Arc<HostCircuitBreaker>,
    completed: bool,
}

impl HostCircuitProbe {
    pub(crate) fn succeed(mut self) {
        self.circuit.finish_half_open_probe(true);
        self.completed = true;
    }

    pub(crate) fn fail(mut self) {
        self.circuit.finish_half_open_probe(false);
        self.completed = true;
    }
}

impl Drop for HostCircuitProbe {
    fn drop(&mut self) {
        if !self.completed {
            self.circuit.finish_half_open_probe(false);
        }
    }
}

/// Thread-safe rolling circuit for one managed Host.
///
/// Call [`Self::record_failure`] once per distinct Host failure/restart attempt,
/// from the single-flight leader. Lane fan-out followers must reuse the
/// leader's decision rather than each recording the same process failure.
/// Closed-state successful relaunches do not erase earlier crashes, so three
/// crash/relaunch cycles still trip FR-LIFE-011. Once open, exactly one
/// half-open probe is admitted at the stable retry time; a successful probe
/// clears the circuit and a failed/abandoned probe opens a fresh retry window.
pub struct HostCircuitBreaker {
    clock: Arc<dyn Clock>,
    policy: HostCircuitPolicy,
    window: StdMutex<HostFailureWindow>,
}

impl Default for HostCircuitBreaker {
    fn default() -> Self {
        Self::new(Arc::new(SystemClock), HostCircuitPolicy::default())
    }
}

impl HostCircuitBreaker {
    pub fn new(clock: Arc<dyn Clock>, policy: HostCircuitPolicy) -> Self {
        let policy = HostCircuitPolicy {
            window_ms: policy.window_ms.max(1),
            failure_threshold: policy.failure_threshold.max(1),
        };
        Self {
            clock,
            policy,
            window: StdMutex::new(HostFailureWindow::default()),
        }
    }

    pub fn policy(&self) -> HostCircuitPolicy {
        self.policy
    }

    pub(crate) fn acquire_attempt(
        self: &Arc<Self>,
    ) -> Result<HostCircuitAttempt, BrowserPlatformError> {
        let now_ms = self.clock.now_ms();
        let mut window = self
            .window
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        window.prune(now_ms, self.policy.window_ms);
        let Some(retry_at_ms) = window.open_retry_at_ms else {
            return Ok(HostCircuitAttempt::Closed);
        };
        if now_ms < retry_at_ms || window.half_open_probe_in_flight {
            return Err(window
                .snapshot(now_ms, self.policy)
                .browser_unavailable_error());
        }
        window.half_open_probe_in_flight = true;
        Ok(HostCircuitAttempt::HalfOpen(HostCircuitProbe {
            circuit: Arc::clone(self),
            completed: false,
        }))
    }

    /// Record one distinct failure and return the post-record decision. The
    /// third failure in the live window returns [`HostCircuitSnapshot::Open`].
    pub fn record_failure(&self) -> HostCircuitSnapshot {
        let now_ms = self.clock.now_ms();
        let mut window = self
            .window
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        window.prune(now_ms, self.policy.window_ms);
        if window.open_retry_at_ms.is_some() {
            return window.snapshot(now_ms, self.policy);
        }
        window.failures.push_back(now_ms);
        if window.failures.len() >= self.policy.failure_threshold {
            window.open_retry_at_ms = Some(
                window
                    .failures
                    .front()
                    .copied()
                    .unwrap_or(now_ms)
                    .saturating_add(self.policy.window_ms),
            );
        }
        window.snapshot(now_ms, self.policy)
    }

    /// Read the current decision, expiring failures outside the rolling window.
    pub fn snapshot(&self) -> HostCircuitSnapshot {
        let now_ms = self.clock.now_ms();
        let mut window = self
            .window
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        window.snapshot(now_ms, self.policy)
    }

    fn finish_half_open_probe(&self, succeeded: bool) {
        let now_ms = self.clock.now_ms();
        let mut window = self
            .window
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !window.half_open_probe_in_flight {
            return;
        }
        window.half_open_probe_in_flight = false;
        if succeeded {
            window.failures.clear();
            window.open_retry_at_ms = None;
        } else {
            window.prune(now_ms, self.policy.window_ms);
            window.failures.push_back(now_ms);
            window.open_retry_at_ms = Some(now_ms.saturating_add(self.policy.window_ms));
        }
    }
}

/// Monotonic epoch transition produced by one successful Host restart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostRestartTransition {
    pub old_epoch: u64,
    pub new_epoch: u64,
}

impl HostRestartTransition {
    /// Reject a restart result which did not actually advance the epoch.
    pub fn new(
        old_epoch: u64,
        new_epoch: u64,
    ) -> Result<Self, BrowserPlatformError> {
        if new_epoch <= old_epoch {
            return Err(BrowserPlatformError::new(
                BrowserErrorCode::BrowserUnavailable,
                "The managed browser restart did not advance its epoch.",
                true,
                "Retry Host recovery; if it persists, close the affected browser lanes.",
            )
            .with_metadata(json!({
                "old_epoch": old_epoch,
                "new_epoch": new_epoch,
            })));
        }
        Ok(Self {
            old_epoch,
            new_epoch,
        })
    }

    /// Required post-restart result: typed `browser_restarted`, both epochs,
    /// and an explicit fresh-observe requirement.
    pub fn browser_restarted_error(self) -> BrowserPlatformError {
        BrowserPlatformError::new(
            BrowserErrorCode::BrowserRestarted,
            "The managed browser restarted and invalidated the previous page state.",
            true,
            "Run a fresh observe before issuing another browser action.",
        )
        .with_metadata(json!({
            "old_epoch": self.old_epoch,
            "new_epoch": self.new_epoch,
            "fresh_observe_required": true,
        }))
    }
}

/// Typed stale-epoch result for an operation prepared against an older Host.
pub fn stale_browser_epoch_error(
    operation_epoch: u64,
    current_epoch: u64,
) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::StaleBrowserEpoch,
        "The browser operation belongs to an older browser epoch.",
        true,
        "Run a fresh observe and retry with the current browser epoch.",
    )
    .with_metadata(json!({
        "operation_epoch": operation_epoch,
        "current_epoch": current_epoch,
        "fresh_observe_required": true,
    }))
}

type RestartResult = Result<HostRestartTransition, BrowserPlatformError>;

struct RestartAttempt {
    // Retained for cancellation/single-flight diagnostics; exercised by the
    // lifecycle tests even though production callers only consume the result.
    #[cfg_attr(not(test), allow(dead_code))]
    observed_epoch: u64,
    result: OnceLock<RestartResult>,
    completed: Notify,
}

impl RestartAttempt {
    fn new(observed_epoch: u64) -> Self {
        Self {
            observed_epoch,
            result: OnceLock::new(),
            completed: Notify::new(),
        }
    }

    fn complete(&self, result: RestartResult) {
        let _ = self.result.set(result);
        self.completed.notify_waiters();
    }

    async fn wait(&self) -> RestartResult {
        loop {
            // Register before checking the cell so completion cannot be lost
            // between the check and awaiting the notification.
            let notified = self.completed.notified();
            if let Some(result) = self.result.get() {
                return result.clone();
            }
            notified.await;
        }
    }
}

#[derive(Default)]
struct RestartGateState {
    in_flight: Option<Arc<RestartAttempt>>,
    last_success: Option<HostRestartTransition>,
}

struct HostRestartSingleFlightInner {
    state: Mutex<RestartGateState>,
}

enum RestartRole {
    Cached(HostRestartTransition),
    Follow(Arc<RestartAttempt>),
    Lead(Arc<RestartAttempt>),
}

/// Completion observed by one restart caller.
#[derive(Clone, Debug)]
pub struct HostRestartFlightResult {
    /// Exactly one caller is leader for a given attempt. Followers receive the
    /// same cloned result.
    pub leader: bool,
    pub result: RestartResult,
}

/// Cancellation-safe restart single-flight for one Host key.
///
/// The leader's future runs in an owned Tokio task. If that caller is aborted,
/// recovery continues and followers still receive its terminal result.
#[derive(Clone)]
pub struct HostRestartSingleFlight {
    inner: Arc<HostRestartSingleFlightInner>,
}

impl Default for HostRestartSingleFlight {
    fn default() -> Self {
        Self {
            inner: Arc::new(HostRestartSingleFlightInner {
                state: Mutex::new(RestartGateState::default()),
            }),
        }
    }
}

impl HostRestartSingleFlight {
    async fn select_role(&self, observed_epoch: u64) -> RestartRole {
        let mut state = self.inner.state.lock().await;
        if let Some(transition) = state
            .last_success
            .filter(|transition| transition.new_epoch > observed_epoch)
        {
            RestartRole::Cached(transition)
        } else if let Some(attempt) = state.in_flight.clone() {
            RestartRole::Follow(attempt)
        } else {
            let attempt = Arc::new(RestartAttempt::new(observed_epoch));
            state.in_flight = Some(attempt.clone());
            RestartRole::Lead(attempt)
        }
    }

    fn spawn_attempt<F, Fut>(
        &self,
        attempt: Arc<RestartAttempt>,
        restart: F,
        attempt_timeout: Option<Duration>,
    ) where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = RestartResult> + Send + 'static,
    {
        let inner = self.inner.clone();
        tokio::spawn(async move {
            let observed_epoch = attempt.observed_epoch;
            let restart_future = restart();
            let result = match attempt_timeout {
                Some(timeout) => {
                    let mut task = tokio::spawn(restart_future);
                    match tokio::time::timeout(timeout, &mut task).await {
                        Ok(Ok(result)) => result,
                        Ok(Err(join_error)) => {
                            Err(host_restart_task_failed_error(observed_epoch, &join_error))
                        }
                        Err(_) => {
                            task.abort();
                            let _ = task.await;
                            Err(host_restart_timeout_error(observed_epoch, timeout, false))
                        }
                    }
                }
                None => match tokio::spawn(restart_future).await {
                    Ok(result) => result,
                    Err(join_error) => {
                        Err(host_restart_task_failed_error(observed_epoch, &join_error))
                    }
                },
            };
            let mut state = inner.state.lock().await;
            if result.is_ok() {
                state.last_success = result.as_ref().ok().copied();
            }
            if state
                .in_flight
                .as_ref()
                .is_some_and(|active| Arc::ptr_eq(active, &attempt))
            {
                state.in_flight = None;
            }
            drop(state);
            // Publish completion only after the gate state is coherent. A
            // caller that observed a newer epoch may need to start a successor
            // flight instead of reusing this older transition.
            attempt.complete(result);
        });
    }

    async fn wait_for_attempt(
        attempt: Arc<RestartAttempt>,
        wait_timeout: Option<Duration>,
    ) -> RestartResult {
        let Some(wait_timeout) = wait_timeout else {
            return attempt.wait().await;
        };
        match tokio::time::timeout(wait_timeout, attempt.wait()).await {
            Ok(result) => result,
            Err(_) => Err(host_restart_timeout_error(
                attempt.observed_epoch,
                wait_timeout,
                true,
            )),
        }
    }

    async fn run_inner<F, Fut>(
        &self,
        observed_epoch: u64,
        restart: F,
        attempt_timeout: Option<Duration>,
        wait_timeout: Option<Duration>,
    ) -> HostRestartFlightResult
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = RestartResult> + Send + 'static,
    {
        let mut restart = Some(restart);
        loop {
            match self.select_role(observed_epoch).await {
                RestartRole::Cached(transition) => {
                    return HostRestartFlightResult {
                        leader: false,
                        result: Ok(transition),
                    };
                }
                RestartRole::Follow(attempt) => {
                    let result = Self::wait_for_attempt(attempt, wait_timeout).await;
                    if result
                        .as_ref()
                        .is_ok_and(|transition| transition.new_epoch <= observed_epoch)
                    {
                        continue;
                    }
                    return HostRestartFlightResult {
                        leader: false,
                        result,
                    };
                }
                RestartRole::Lead(attempt) => {
                    self.spawn_attempt(
                        Arc::clone(&attempt),
                        restart
                            .take()
                            .expect("restart closure is consumed only by the leader"),
                        attempt_timeout,
                    );
                    return HostRestartFlightResult {
                        leader: true,
                        result: Self::wait_for_attempt(attempt, wait_timeout).await,
                    };
                }
            }
        }
    }

    pub async fn run<F, Fut>(
        &self,
        observed_epoch: u64,
        restart: F,
    ) -> HostRestartFlightResult
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = RestartResult> + Send + 'static,
    {
        self.run_inner(observed_epoch, restart, None, None).await
    }

    /// Runs one restart attempt with an owned deadline. The underlying restart
    /// task is aborted when the attempt deadline expires, while every caller
    /// wait is independently bounded so a damaged flight cannot hold followers
    /// forever.
    pub async fn run_bounded<F, Fut>(
        &self,
        observed_epoch: u64,
        attempt_timeout: Duration,
        restart: F,
    ) -> HostRestartFlightResult
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = RestartResult> + Send + 'static,
    {
        let wait_timeout = attempt_timeout.saturating_add(Duration::from_secs(1));
        self.run_inner(
            observed_epoch,
            restart,
            Some(attempt_timeout),
            Some(wait_timeout),
        )
        .await
    }

    #[cfg(test)]
    async fn active_observed_epoch(&self) -> Option<u64> {
        self.inner
            .state
            .lock()
            .await
            .in_flight
            .as_ref()
            .map(|attempt| attempt.observed_epoch)
    }
}

/// Registry of independent restart gates. Calls for the same key join one
/// attempt; different Host keys never share a gate.
pub struct PerKeyHostRestartSingleFlight<K> {
    gates: Mutex<HashMap<K, HostRestartSingleFlight>>,
}

impl<K> Default for PerKeyHostRestartSingleFlight<K> {
    fn default() -> Self {
        Self {
            gates: Mutex::new(HashMap::new()),
        }
    }
}

impl<K> PerKeyHostRestartSingleFlight<K>
where
    K: Clone + Eq + Hash,
{
    async fn gate(&self, key: K) -> HostRestartSingleFlight {
        let mut gates = self.gates.lock().await;
        gates.entry(key).or_default().clone()
    }

    pub async fn run<F, Fut>(
        &self,
        key: K,
        observed_epoch: u64,
        restart: F,
    ) -> HostRestartFlightResult
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = RestartResult> + Send + 'static,
    {
        self.gate(key).await.run(observed_epoch, restart).await
    }

    pub async fn run_bounded<F, Fut>(
        &self,
        key: K,
        observed_epoch: u64,
        attempt_timeout: Duration,
        restart: F,
    ) -> HostRestartFlightResult
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = RestartResult> + Send + 'static,
    {
        self.gate(key)
            .await
            .run_bounded(observed_epoch, attempt_timeout, restart)
            .await
    }

    /// Drops the gate for a key whose Host no longer exists.
    ///
    /// The map is keyed by identifiers that can churn (isolated-lane UUIDs,
    /// replica identity generations); without eviction every failed Host on a
    /// unique key retains its gate for the process lifetime. A gate with an
    /// in-flight attempt is kept so the single-flight guarantee is never
    /// split across two gate instances.
    pub async fn evict_settled(&self, key: &K) {
        let mut gates = self.gates.lock().await;
        if let Some(gate) = gates.get(key) {
            if gate.inner.state.lock().await.in_flight.is_some() {
                return;
            }
        }
        gates.remove(key);
    }

    #[cfg(test)]
    pub(crate) async fn gate_count(&self) -> usize {
        self.gates.lock().await.len()
    }
}

fn host_restart_timeout_error(
    observed_epoch: u64,
    timeout: Duration,
    waiter_timeout: bool,
) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserUnavailable,
        "The managed browser Host restart did not finish before its deadline.",
        true,
        "Retry the browser operation so the Hub can attempt recovery again.",
    )
    .with_metadata(json!({
        "restart_timeout": true,
        "restart_wait_timeout": waiter_timeout,
        "observed_epoch": observed_epoch,
        "timeout_ms": timeout.as_millis() as u64,
    }))
}

fn host_restart_task_failed_error(
    observed_epoch: u64,
    join_error: &tokio::task::JoinError,
) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserUnavailable,
        "The managed browser Host restart task terminated unexpectedly.",
        true,
        "Retry the browser operation so the Hub can attempt recovery again.",
    )
    .with_metadata(json!({
        "restart_task_failed": true,
        "observed_epoch": observed_epoch,
        "task_cancelled": join_error.is_cancelled(),
        "task_panicked": join_error.is_panic(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ManualClock;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Semaphore;

    #[test]
    fn rolling_window_opens_on_third_failure_and_expires_at_boundary() {
        let clock = ManualClock::new(1_000);
        let circuit = HostCircuitBreaker::new(
            Arc::new(clock.clone()),
            HostCircuitPolicy::default(),
        );

        assert_eq!(
            circuit.record_failure(),
            HostCircuitSnapshot::Closed {
                failures_in_window: 1,
                failures_remaining: 2,
            }
        );
        clock.advance(10_000);
        assert_eq!(
            circuit.record_failure(),
            HostCircuitSnapshot::Closed {
                failures_in_window: 2,
                failures_remaining: 1,
            }
        );
        clock.advance(10_000);
        assert_eq!(
            circuit.record_failure(),
            HostCircuitSnapshot::Open {
                failures_in_window: 3,
                retry_at_ms: 61_000,
                retry_after_ms: 40_000,
            }
        );

        clock.advance(39_999);
        assert_eq!(
            circuit.snapshot(),
            HostCircuitSnapshot::Open {
                failures_in_window: 3,
                retry_at_ms: 61_000,
                retry_after_ms: 1,
            }
        );
        clock.advance(1);
        assert_eq!(
            circuit.snapshot(),
            HostCircuitSnapshot::HalfOpen {
                failures_in_window: 2,
                retry_at_ms: 61_000,
            },
            "the retry boundary enters half-open state; only a probe may proceed"
        );
    }

    #[test]
    fn circuit_breaker_normalizes_bypassed_zero_policy_fields() {
        let circuit = HostCircuitBreaker::new(
            Arc::new(ManualClock::new(10)),
            HostCircuitPolicy {
                window_ms: 0,
                failure_threshold: 0,
            },
        );

        assert_eq!(
            circuit.policy(),
            HostCircuitPolicy {
                window_ms: 1,
                failure_threshold: 1,
            }
        );
        assert_eq!(
            circuit.snapshot(),
            HostCircuitSnapshot::Closed {
                failures_in_window: 0,
                failures_remaining: 1,
            }
        );
        assert!(circuit.record_failure().is_open());
    }

    #[test]
    fn half_open_allows_one_probe_and_success_closes_the_circuit() {
        let clock = Arc::new(ManualClock::new(1_000));
        let circuit = Arc::new(HostCircuitBreaker::new(
            clock.clone(),
            HostCircuitPolicy::default(),
        ));
        circuit.record_failure();
        clock.advance(10_000);
        circuit.record_failure();
        clock.advance(10_000);
        circuit.record_failure();
        clock.advance(40_000);

        let probe = circuit.acquire_attempt().expect("one half-open probe");
        assert!(probe.is_half_open());
        assert!(
            circuit.acquire_attempt().is_err(),
            "a second caller must fail closed while the probe is in flight"
        );
        probe.succeed();
        assert_eq!(
            circuit.snapshot(),
            HostCircuitSnapshot::Closed {
                failures_in_window: 0,
                failures_remaining: 3,
            }
        );
    }

    #[test]
    fn failed_half_open_probe_reopens_with_a_stable_retry_window() {
        let clock = Arc::new(ManualClock::new(1_000));
        let circuit = Arc::new(HostCircuitBreaker::new(
            clock.clone(),
            HostCircuitPolicy::default(),
        ));
        circuit.record_failure();
        clock.advance(10_000);
        circuit.record_failure();
        clock.advance(10_000);
        circuit.record_failure();
        clock.advance(40_000);

        let probe = circuit.acquire_attempt().expect("one half-open probe");
        probe.fail();
        assert_eq!(
            circuit.snapshot(),
            HostCircuitSnapshot::Open {
                failures_in_window: 3,
                retry_at_ms: 121_000,
                retry_after_ms: 60_000,
            }
        );
        assert!(circuit.acquire_attempt().is_err());
    }

    #[test]
    fn typed_epoch_errors_require_fresh_observe() {
        let transition = HostRestartTransition::new(7, 8).unwrap();
        let restarted = transition.browser_restarted_error();
        assert_eq!(restarted.code, BrowserErrorCode::BrowserRestarted);
        assert_eq!(restarted.metadata["old_epoch"], 7);
        assert_eq!(restarted.metadata["new_epoch"], 8);
        assert_eq!(restarted.metadata["fresh_observe_required"], true);

        let stale = stale_browser_epoch_error(7, 8);
        assert_eq!(stale.code, BrowserErrorCode::StaleBrowserEpoch);
        assert_eq!(stale.metadata["operation_epoch"], 7);
        assert_eq!(stale.metadata["current_epoch"], 8);
        assert_eq!(stale.metadata["fresh_observe_required"], true);
        assert!(HostRestartTransition::new(8, 8).is_err());
    }

    #[tokio::test]
    async fn evict_settled_reclaims_idle_gates_but_never_an_in_flight_attempt() {
        let flights = Arc::new(PerKeyHostRestartSingleFlight::<String>::default());
        let release = Arc::new(Semaphore::new(0));

        // A settled gate for a churned key is reclaimable.
        let settled = flights
            .run("isolated-a".into(), 1, || async {
                HostRestartTransition::new(1, 2)
            })
            .await;
        assert!(settled.result.is_ok());
        assert_eq!(flights.gate_count().await, 1);
        flights.evict_settled(&"isolated-a".to_string()).await;
        assert_eq!(flights.gate_count().await, 0);

        // An in-flight attempt keeps its gate so followers still join it.
        let leader = {
            let flights = flights.clone();
            let release = release.clone();
            tokio::spawn(async move {
                flights
                    .run("isolated-b".into(), 5, move || async move {
                        let permit = release.acquire().await.unwrap();
                        permit.forget();
                        HostRestartTransition::new(5, 6)
                    })
                    .await
            })
        };
        while flights
            .gate("isolated-b".into())
            .await
            .active_observed_epoch()
            .await
            != Some(5)
        {
            tokio::task::yield_now().await;
        }
        flights.evict_settled(&"isolated-b".to_string()).await;
        assert_eq!(
            flights.gate_count().await,
            1,
            "an in-flight gate must survive eviction"
        );
        let follower = {
            let flights = flights.clone();
            tokio::spawn(async move {
                flights
                    .run("isolated-b".into(), 5, || async {
                        panic!("follower must join the retained in-flight attempt")
                    })
                    .await
            })
        };
        release.add_permits(1);
        assert!(leader.await.unwrap().result.is_ok());
        assert_eq!(
            follower.await.unwrap().result.unwrap(),
            HostRestartTransition {
                old_epoch: 5,
                new_epoch: 6,
            }
        );
    }

    #[tokio::test]
    async fn same_key_restart_is_single_flight_and_stale_callers_reuse_success() {
        let flights = Arc::new(PerKeyHostRestartSingleFlight::<String>::default());
        let calls = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(Semaphore::new(0));

        let first = {
            let flights = flights.clone();
            let calls = calls.clone();
            let release = release.clone();
            tokio::spawn(async move {
                flights
                    .run("primary".into(), 7, move || async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        let permit = release.acquire().await.unwrap();
                        permit.forget();
                        HostRestartTransition::new(7, 8)
                    })
                    .await
            })
        };

        while calls.load(Ordering::SeqCst) != 1 {
            tokio::task::yield_now().await;
        }

        let second_calls = calls.clone();
        let second = {
            let flights = flights.clone();
            tokio::spawn(async move {
                flights
                    .run("primary".into(), 7, move || async move {
                        second_calls.fetch_add(1, Ordering::SeqCst);
                        HostRestartTransition::new(7, 9)
                    })
                    .await
            })
        };
        while flights
            .gate("primary".into())
            .await
            .active_observed_epoch()
            .await
            != Some(7)
        {
            tokio::task::yield_now().await;
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        release.add_permits(1);

        let a = first.await.unwrap();
        let b = second.await.unwrap();
        assert_ne!(a.leader, b.leader);
        assert_eq!(a.result.unwrap(), HostRestartTransition { old_epoch: 7, new_epoch: 8 });
        assert_eq!(b.result.unwrap(), HostRestartTransition { old_epoch: 7, new_epoch: 8 });
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let cached_calls = calls.clone();
        let cached = flights
            .run("primary".into(), 7, move || async move {
                cached_calls.fetch_add(1, Ordering::SeqCst);
                HostRestartTransition::new(7, 10)
            })
            .await;
        assert!(!cached.leader);
        assert_eq!(
            cached.result.unwrap(),
            HostRestartTransition { old_epoch: 7, new_epoch: 8 }
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn newer_epoch_caller_starts_a_successor_after_following_older_flight() {
        let flights = Arc::new(PerKeyHostRestartSingleFlight::<String>::default());
        let calls = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(Semaphore::new(0));

        let first = {
            let flights = flights.clone();
            let calls = calls.clone();
            let release = release.clone();
            tokio::spawn(async move {
                flights
                    .run("primary".into(), 7, move || async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        let permit = release.acquire().await.unwrap();
                        permit.forget();
                        HostRestartTransition::new(7, 8)
                    })
                    .await
            })
        };
        while calls.load(Ordering::SeqCst) != 1 {
            tokio::task::yield_now().await;
        }

        let second = {
            let flights = flights.clone();
            let calls = calls.clone();
            tokio::spawn(async move {
                flights
                    .run("primary".into(), 8, move || async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        HostRestartTransition::new(8, 9)
                    })
                    .await
            })
        };
        tokio::task::yield_now().await;
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "the newer caller must wait for the older in-flight restart"
        );
        release.add_permits(1);

        let first = first.await.unwrap();
        let second = second.await.unwrap();
        assert_eq!(
            first.result.unwrap(),
            HostRestartTransition {
                old_epoch: 7,
                new_epoch: 8,
            }
        );
        assert!(
            second.leader,
            "the newer observed epoch requires a successor flight"
        );
        assert_eq!(
            second.result.unwrap(),
            HostRestartTransition {
                old_epoch: 8,
                new_epoch: 9,
            }
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn different_host_keys_restart_in_parallel() {
        let flights = Arc::new(PerKeyHostRestartSingleFlight::<String>::default());
        let entered = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(Semaphore::new(0));
        let mut tasks = Vec::new();

        for (key, old_epoch, new_epoch) in [("primary", 3, 4), ("crawl", 9, 10)] {
            let flights = flights.clone();
            let entered = entered.clone();
            let release = release.clone();
            tasks.push(tokio::spawn(async move {
                flights
                    .run(key.to_string(), old_epoch, move || async move {
                        entered.fetch_add(1, Ordering::SeqCst);
                        let permit = release.acquire().await.unwrap();
                        permit.forget();
                        HostRestartTransition::new(old_epoch, new_epoch)
                    })
                    .await
            }));
        }

        while entered.load(Ordering::SeqCst) != 2 {
            tokio::task::yield_now().await;
        }
        release.add_permits(2);
        for task in tasks {
            assert!(task.await.unwrap().result.is_ok());
        }
    }

    #[tokio::test]
    async fn leader_cancellation_does_not_abandon_restart() {
        let flights = Arc::new(PerKeyHostRestartSingleFlight::<String>::default());
        let calls = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(Semaphore::new(0));

        let leader = {
            let flights = flights.clone();
            let calls = calls.clone();
            let release = release.clone();
            tokio::spawn(async move {
                flights
                    .run("primary".into(), 11, move || async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        let permit = release.acquire().await.unwrap();
                        permit.forget();
                        HostRestartTransition::new(11, 12)
                    })
                    .await
            })
        };
        while calls.load(Ordering::SeqCst) != 1 {
            tokio::task::yield_now().await;
        }

        let follower = {
            let flights = flights.clone();
            tokio::spawn(async move {
                flights
                    .run("primary".into(), 11, || async {
                        panic!("follower must not run a second restart")
                    })
                    .await
            })
        };
        leader.abort();
        assert!(leader.await.unwrap_err().is_cancelled());
        release.add_permits(1);

        let result = follower.await.unwrap();
        assert!(!result.leader);
        assert_eq!(
            result.result.unwrap(),
            HostRestartTransition {
                old_epoch: 11,
                new_epoch: 12,
            }
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
