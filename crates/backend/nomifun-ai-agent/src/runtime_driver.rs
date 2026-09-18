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
        turn_cleanup_count: AtomicUsize,
        terminal_count: AtomicUsize,
        session_cleanup_count: AtomicUsize,
        fail_turn_cleanup: AtomicBool,
    }

    impl FakeDriver {
        fn new(fail_turn_cleanup: bool) -> Self {
            Self {
                started: Notify::new(),
                turn_cleanup_count: AtomicUsize::new(0),
                terminal_count: AtomicUsize::new(0),
                session_cleanup_count: AtomicUsize::new(0),
                fail_turn_cleanup: AtomicBool::new(fail_turn_cleanup),
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
            self.started.notify_waiters();
            cancellation.cancelled().await;
            Ok(NomiRuntimeTurnOutcome::cancelled(0))
        }

        async fn cleanup_turn(&self, _message: &SendMessageData) -> Result<(), AppError> {
            self.turn_cleanup_count.fetch_add(1, Ordering::AcqRel);
            if self.fail_turn_cleanup.load(Ordering::Acquire) {
                return Err(AppError::Conflict("fake cleanup failed".into()));
            }
            Ok(())
        }

        async fn record_terminal(
            &self,
            _message: &SendMessageData,
            outcome: &NomiRuntimeTurnOutcome,
        ) -> Result<(), AppError> {
            assert!(matches!(outcome.terminal, EngineTurnTerminal::Cancelled));
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
            device_mcp_servers: Vec::new(),
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
    }
}
