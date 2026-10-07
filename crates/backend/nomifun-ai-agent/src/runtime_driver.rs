//! Internal seam for the one official Nomi Runtime.
//!
//! This is deliberately not a runtime-registration SDK. The application
//! installs one source-integrated factory for `nomifun.nomi`; future Runtime
//! replacements implement the same driver contract and consume the same typed
//! host ports without adding a family, selector, or handle variant.

pub const OFFICIAL_NOMI_RUNTIME_FAMILY_ID: &str = "nomifun.nomi";

pub use crate::engine_sdk::{
    EngineDriverFactory as NomiRuntimeDriverFactory,
    EngineSessionDriver as NomiRuntimeDriver,
    EngineTurnOutcome as NomiRuntimeTurnOutcome,
    EngineTurnOutput as NomiRuntimeTurnOutput,
    EngineTurnTerminal as NomiRuntimeTurnTerminal,
    HostedAgentRuntime as HostedNomiRuntime,
};

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use async_trait::async_trait;
    use nomifun_common::{AgentType, AppError};
    use tokio::sync::Notify;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::engine_sdk::EngineTurnTerminal;
    use crate::types::{AgentRuntimeBuildOptions, SendMessageData};
    use crate::{AgentRuntimeControl, OfficialAgentRuntime};

    const OWNER: &str = "0190f5fe-7c00-7a00-8000-000000000001";
    const SESSION: &str = "0190f5fe-7c00-7a00-8000-000000000002";

    struct FakeDriver {
        started: Notify,
        run_count: AtomicUsize,
        turn_cleanup_count: AtomicUsize,
        terminal_count: AtomicUsize,
        session_cleanup_count: AtomicUsize,
        fail_turn_cleanup: AtomicBool,
        fail_terminal: AtomicBool,
        complete_without_cancel: AtomicBool,
        cleanup_error: String,
    }

    impl FakeDriver {
        fn new(fail_turn_cleanup: bool) -> Self {
            Self {
                started: Notify::new(),
                run_count: AtomicUsize::new(0),
                turn_cleanup_count: AtomicUsize::new(0),
                terminal_count: AtomicUsize::new(0),
                session_cleanup_count: AtomicUsize::new(0),
                fail_turn_cleanup: AtomicBool::new(fail_turn_cleanup),
                fail_terminal: AtomicBool::new(false),
                complete_without_cancel: AtomicBool::new(false),
                cleanup_error: "fake cleanup failed".into(),
            }
        }
    }

    #[async_trait]
    impl NomiRuntimeDriver for FakeDriver {
        async fn run_turn(
            &self,
            _message: &SendMessageData,
            cancellation: CancellationToken,
            _output: NomiRuntimeTurnOutput,
        ) -> Result<NomiRuntimeTurnOutcome, AppError> {
            self.run_count.fetch_add(1, Ordering::AcqRel);
            self.started.notify_waiters();
            if self.complete_without_cancel.load(Ordering::Acquire) {
                return Ok(NomiRuntimeTurnOutcome {model_steps:3,terminal:EngineTurnTerminal::Completed {
                    finish_reason:nomifun_chat_model_broker::ChatFinishReason::Completed}});
            }
            cancellation.cancelled().await;
            Ok(NomiRuntimeTurnOutcome::cancelled(0))
        }

        async fn cleanup_turn(&self, _message: &SendMessageData) -> Result<(), AppError> {
            self.turn_cleanup_count.fetch_add(1, Ordering::AcqRel);
            if self.fail_turn_cleanup.load(Ordering::Acquire) {
                return Err(AppError::Conflict(self.cleanup_error.clone()));
            }
            Ok(())
        }

        async fn record_terminal(
            &self,
            _message: &SendMessageData,
            outcome: &NomiRuntimeTurnOutcome,
        ) -> Result<(), AppError> {
            assert!(matches!(outcome.terminal, EngineTurnTerminal::Cancelled));
            if self.fail_terminal.load(Ordering::Acquire) {
                return Err(AppError::Conflict("fixture terminal receipt unavailable".into()));
            }
            self.terminal_count.fetch_add(1, Ordering::AcqRel);
            Ok(())
        }

        async fn cleanup_session(&self) -> Result<(), AppError> {
            self.session_cleanup_count.fetch_add(1, Ordering::AcqRel);
            Ok(())
        }
    }

    fn options() -> AgentRuntimeBuildOptions {
        AgentRuntimeBuildOptions {
            user_id: OWNER.into(),
            agent_type: AgentType::Nomi,
            workspace: std::env::temp_dir().to_string_lossy().into_owned(),
            model: None,
            conversation_id: SESSION.into(),
            delegation_policy: Default::default(),
            extra: serde_json::json!({}),
            conversation_created_at: None,
            workspace_binding_lease: None,
        }
    }

    fn message(id: &str) -> SendMessageData {
        SendMessageData {
            content: "hello".into(),
            msg_id: id.into(),
            source_message_id: Some(id.into()),
            files: Vec::new(),
            inject_skills: Vec::new(),
            origin: None,
        }
    }

    #[tokio::test]
    async fn official_driver_enforces_one_turn_cancel_cleanup_and_teardown() {
        let driver = Arc::new(FakeDriver::new(false));
        let started = driver.started.notified();
        let runtime = HostedNomiRuntime::new(&options(), driver.clone()).unwrap();
        runtime.send_message(message("message-1")).await.unwrap();
        started.await;

        assert!(runtime.send_message(message("message-2")).await.is_err());
        runtime.cancel().await.unwrap();
        assert_eq!(driver.turn_cleanup_count.load(Ordering::Acquire), 1);
        assert_eq!(driver.terminal_count.load(Ordering::Acquire), 1);

        runtime.kill_and_wait(None).await.unwrap();
        assert_eq!(driver.session_cleanup_count.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn failed_turn_cleanup_quarantines_the_official_runtime() {
        let driver = Arc::new(FakeDriver::new(true));
        let started = driver.started.notified();
        let runtime = HostedNomiRuntime::new(&options(), driver.clone()).unwrap();
        runtime.send_message(message("message-1")).await.unwrap();
        started.await;

        assert!(runtime.cancel().await.is_err());
        assert!(!runtime.is_transport_healthy());
        assert_eq!(driver.turn_cleanup_count.load(Ordering::Acquire), 1);
        assert_eq!(driver.terminal_count.load(Ordering::Acquire), 0);
        assert!(runtime.kill_and_wait(None).await.is_err());
        assert_eq!(driver.session_cleanup_count.load(Ordering::Acquire),0);
        driver.fail_turn_cleanup.store(false,Ordering::Release);
        runtime.kill_and_wait(None).await.unwrap();
        assert_eq!(driver.terminal_count.load(Ordering::Acquire),1);
        assert_eq!(driver.session_cleanup_count.load(Ordering::Acquire),1);
        assert!(!runtime.is_transport_healthy());
    }

    #[tokio::test]
    async fn failed_terminal_receipt_stays_owned_until_explicit_teardown_retry() {
        let driver=Arc::new(FakeDriver::new(false));
        driver.fail_terminal.store(true,Ordering::Release);
        let started=driver.started.notified();
        let runtime=HostedNomiRuntime::new(&options(),driver.clone()).unwrap();
        runtime.send_message(message("failed-receipt")).await.unwrap();
        started.await;
        let cancellation = runtime.cancel().await.unwrap_err().to_string();
        assert!(cancellation.contains("fixture terminal receipt unavailable"),
            "cancel must preserve the exact terminal settlement condition: {cancellation}");
        let shutdown = runtime.kill_and_wait(None).await.unwrap_err().to_string();
        assert!(shutdown.contains("fixture terminal receipt unavailable"),
            "teardown must preserve the exact terminal settlement condition: {shutdown}");
        assert_eq!(driver.session_cleanup_count.load(Ordering::Acquire),0);
        assert_eq!(driver.turn_cleanup_count.load(Ordering::Acquire),1);
        assert!(runtime.send_message(message("must-not-restart")).await.is_err());
        driver.fail_terminal.store(false,Ordering::Release);
        runtime.kill_and_wait(None).await.unwrap();
        runtime.kill_and_wait(None).await.unwrap();
        assert_eq!(driver.terminal_count.load(Ordering::Acquire),1);
        assert_eq!(driver.turn_cleanup_count.load(Ordering::Acquire),1);
        assert_eq!(driver.session_cleanup_count.load(Ordering::Acquire),1);
    }

    #[tokio::test]
    async fn shutdown_joining_first_failed_cleanup_keeps_its_cause_until_explicit_retry() {
        let mut driver = FakeDriver::new(true);
        driver.cleanup_error = format!("database is locked; Bearer fixture-hidden-secret; {}", "界".repeat(2_000));
        let driver = Arc::new(driver);
        let started = driver.started.notified();
        let runtime = HostedNomiRuntime::new(&options(), driver.clone()).unwrap();
        runtime.send_message(message("shutdown-during-first-cleanup")).await.unwrap();
        started.await;
        // This explicit shutdown starts while the Turn is still running. Its
        // flight joins the first failing cleanup and must report that cause;
        // it cannot count joining as permission to retry cleanup immediately.
        let failure = runtime.kill_and_wait(None).await.unwrap_err().to_string();
        assert!(failure.contains("Engine turn cleanup failed"));
        assert!(failure.contains("database is locked"), "first condition disappeared: {failure}");
        assert!(!failure.contains("fixture-hidden-secret"));
        assert!(failure.chars().count() <= 1_100);
        assert_eq!(driver.run_count.load(Ordering::Acquire), 1);
        assert_eq!(driver.turn_cleanup_count.load(Ordering::Acquire), 1);
        assert_eq!(driver.terminal_count.load(Ordering::Acquire), 0);
        assert_eq!(driver.session_cleanup_count.load(Ordering::Acquire), 0);
        assert!(runtime.send_message(message("cannot-replay-while-unproven")).await.is_err());
        driver.fail_turn_cleanup.store(false, Ordering::Release);
        runtime.kill_and_wait(None).await.unwrap();
        runtime.kill_and_wait(None).await.unwrap();
        assert_eq!(driver.run_count.load(Ordering::Acquire), 1, "teardown must never replay the driver");
        assert_eq!(driver.turn_cleanup_count.load(Ordering::Acquire), 2);
        assert_eq!(driver.terminal_count.load(Ordering::Acquire), 1);
        assert_eq!(driver.session_cleanup_count.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn cancellation_during_failed_cleanup_wins_before_the_first_terminal_commit() {
        let driver=Arc::new(FakeDriver::new(true));
        driver.complete_without_cancel.store(true,Ordering::Release);
        let runtime=HostedNomiRuntime::new(&options(),driver.clone()).unwrap();
        runtime.send_message(message("completed-before-cleanup-fault")).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2),async {
            while runtime.is_transport_healthy() {tokio::task::yield_now().await;}
        }).await.unwrap();
        driver.fail_turn_cleanup.store(false,Ordering::Release);
        runtime.kill_and_wait(None).await.unwrap();
        assert_eq!(driver.terminal_count.load(Ordering::Acquire),1);
        assert_eq!(driver.session_cleanup_count.load(Ordering::Acquire),1);
    }
}
