//! Request control is host-owned runtime state, never persisted authorization.
use std::future::{Future, pending};
use std::time::Duration;

use serde::Serialize;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::McpOwnerError;

/// Shared by discovery and execution. These limits stop at business dispatch.
#[derive(Clone, Debug)]
pub struct McpSetupPolicy {
    pub startup_timeout: Duration,
    pub runtime_preparation_timeout: Duration,
    pub process_start_timeout: Duration,
    pub protocol_timeout: Duration,
    pub package_bootstrap_timeout: Duration,
}

impl Default for McpSetupPolicy {
    fn default() -> Self {
        Self {
            startup_timeout: Duration::from_secs(13 * 60),
            runtime_preparation_timeout: Duration::from_secs(11 * 60),
            process_start_timeout: Duration::from_secs(30),
            protocol_timeout: Duration::from_secs(30),
            package_bootstrap_timeout: Duration::from_secs(120),
        }
    }
}

impl McpSetupPolicy {
    /// A short readiness probe or deterministic test may supply one ceiling.
    pub fn uniform(timeout: Duration) -> Self {
        Self {
            startup_timeout: timeout,
            runtime_preparation_timeout: timeout,
            process_start_timeout: timeout,
            protocol_timeout: timeout,
            package_bootstrap_timeout: timeout,
        }
    }

    pub(super) fn start(&self) -> SetupBudget {
        SetupBudget { policy: self.clone(), deadline: Instant::now() + self.startup_timeout }
    }
}

pub(super) struct SetupBudget {
    pub(super) policy: McpSetupPolicy,
    deadline: Instant,
}

impl SetupBudget {
    pub(super) fn phase(&self, phase: &'static str, cap: Duration) -> PhaseBudget {
        let now = Instant::now();
        let deadline = self.deadline.min(now + cap);
        PhaseBudget { phase, deadline, timeout: deadline.saturating_duration_since(now) }
    }
}

#[derive(Clone, Copy)]
pub(super) struct PhaseBudget {
    pub(super) phase: &'static str,
    pub(super) deadline: Instant,
    pub(super) timeout: Duration,
}

impl PhaseBudget {
    pub(super) fn error(&self) -> McpOwnerError {
        let mut error = McpOwnerError::new("MCP_TIMEOUT", format!("MCP {} timed out at its setup deadline", self.phase));
        error.setup_timeout = Some((self.phase, self.timeout));
        error
    }
}

/// A cancellation token and an optional explicitly imposed business deadline.
/// No elapsed-time or silence limit is installed by default.
#[derive(Clone, Debug, Default)]
pub struct McpInvocationControl {
    pub cancellation: CancellationToken,
    pub execution_deadline: Option<Instant>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct McpExecutionFacts {
    /// Business RPC may have reached transport; not proof of remote acceptance.
    pub request_dispatched: bool,
    /// Includes a native launch that may return late after its waiter stopped.
    pub local_process_started: bool,
    /// Host cancellation/explicit deadline, never proof the server stopped.
    pub cancellation_requested: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpCleanupStatus {
    #[default]
    NotRequired,
    Confirmed,
    Unsupported,
    Unproven,
}

#[derive(Clone, Debug, Default)]
pub struct McpCleanupFacts {
    /// Local I/O release or exact managed process-tree cleanup.
    pub local: McpCleanupStatus,
    /// Protocol session termination only; not business cancellation or rollback.
    pub remote: McpCleanupStatus,
    /// Present only when at least one cleanup status is Unproven.
    pub error: Option<McpOwnerError>,
}

/// Business observation and cleanup are independent; cleanup never erases it.
#[derive(Debug)]
pub struct McpInvocationOutcome<T> {
    pub result: Result<T, McpOwnerError>,
    pub execution: McpExecutionFacts,
    pub cleanup: McpCleanupFacts,
}

impl<T> McpInvocationOutcome<T> {
    /// Callers that cannot retain independent facts fail closed on unproven
    /// cleanup. Effect hosts retain this full outcome instead.
    pub fn into_result(self) -> Result<T, McpOwnerError> {
        if self.cleanup.local == McpCleanupStatus::Unproven || self.cleanup.remote == McpCleanupStatus::Unproven {
            return Err(self.cleanup.error.unwrap_or_else(|| McpOwnerError::new(
                "MCP_SESSION_CLEANUP_FAILED", "MCP cleanup remains unproven")));
        }
        self.result
    }

    pub fn map<U>(self, map: impl FnOnce(T) -> U) -> McpInvocationOutcome<U> {
        McpInvocationOutcome { result: self.result.map(map), execution: self.execution, cleanup: self.cleanup }
    }

    pub fn try_map<U>(self, map: impl FnOnce(T) -> Result<U, McpOwnerError>) -> McpInvocationOutcome<U> {
        McpInvocationOutcome { result: self.result.and_then(map), execution: self.execution, cleanup: self.cleanup }
    }

    pub(super) fn rejected(error: McpOwnerError, control: &McpInvocationControl) -> Self {
        let cancellation_requested = control.cancellation.is_cancelled()
            || control.execution_deadline.is_some_and(|deadline| deadline <= Instant::now())
            || error.code() == "MCP_EXECUTION_DEADLINE_EXCEEDED";
        Self { result: Err(error), execution: McpExecutionFacts {
            cancellation_requested, ..Default::default()
        }, cleanup: McpCleanupFacts::default() }
    }
}

pub(super) async fn await_deadline(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => pending::<()>().await,
    }
}

pub(super) async fn controlled<T>(
    future: impl Future<Output = Result<T, McpOwnerError>>,
    control: &McpInvocationControl,
    deadline: Option<Instant>,
    deadline_error: impl FnOnce() -> McpOwnerError,
) -> Result<T, McpOwnerError> {
    let deadline = match (deadline, control.execution_deadline) {
        (Some(phase), Some(explicit)) => Some(phase.min(explicit)),
        (phase, explicit) => phase.or(explicit),
    };
    tokio::select! {
        biased;
        _ = control.cancellation.cancelled() => Err(cancelled_error()),
        _ = await_deadline(deadline) => Err(if control.execution_deadline.is_some_and(|explicit| Some(explicit) == deadline) {
            execution_deadline_error()
        } else { deadline_error() }),
        result = future => result,
    }
}

pub(super) fn cancelled_error() -> McpOwnerError {
    McpOwnerError::new("MCP_REQUEST_CANCELLED", "MCP cancellation was requested; this is not proof of remote stop or rollback")
}

pub(super) fn execution_deadline_error() -> McpOwnerError {
    McpOwnerError::new("MCP_EXECUTION_DEADLINE_EXCEEDED", "The explicitly supplied MCP execution deadline expired; remote outcome may be unknown")
}

pub(super) fn is_interruption(error: &McpOwnerError) -> bool {
    matches!(error.code(), "MCP_REQUEST_CANCELLED" | "MCP_EXECUTION_DEADLINE_EXCEEDED" | "MCP_TIMEOUT")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn silent_business_wait_has_no_default_total_or_idle_deadline() {
        let control = McpInvocationControl::default();
        let task = tokio::spawn(async move {
            controlled(async { tokio::time::sleep(Duration::from_secs(24 * 60 * 60)).await; Ok(42) },
                &control, None, execution_deadline_error).await
        });
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(12 * 60 * 60)).await;
        assert!(!task.is_finished());
        tokio::time::advance(Duration::from_secs(12 * 60 * 60)).await;
        assert_eq!(task.await.unwrap().unwrap(), 42);
    }

    #[tokio::test(start_paused = true)]
    async fn setup_phase_cannot_reset_the_cumulative_startup_ceiling() {
        let setup = McpSetupPolicy::uniform(Duration::from_secs(10)).start();
        tokio::time::advance(Duration::from_secs(7)).await;
        let next = setup.phase("protocol_setup", Duration::from_secs(10));
        assert_eq!(next.timeout, Duration::from_secs(3));
        assert_eq!(next.deadline, setup.deadline);
        let error = controlled(pending::<Result<(), McpOwnerError>>(), &McpInvocationControl::default(),
            Some(next.deadline), || next.error()).await.unwrap_err();
        assert_eq!(error.code(), "MCP_TIMEOUT");
        assert_eq!(error.setup_timeout, Some(("protocol_setup", Duration::from_secs(3))));
    }

    #[tokio::test]
    async fn cancellation_before_dispatch_does_not_poll_the_business_future() {
        let control = McpInvocationControl::default();
        control.cancellation.cancel();
        let error = controlled(async { panic!("cancelled business future was polled"); #[allow(unreachable_code)] Ok(()) },
            &control, None, execution_deadline_error).await.unwrap_err();
        assert_eq!(error.code(), "MCP_REQUEST_CANCELLED");
    }
}
