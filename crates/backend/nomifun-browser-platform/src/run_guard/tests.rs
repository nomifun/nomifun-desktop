use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Notify;

#[derive(Default)]
struct Gate {
    locked: AtomicBool,
    fail_release: AtomicBool,
    log: Mutex<Vec<&'static str>>,
    unlocked: Notify,
}

#[tokio::test]
async fn dropping_human_close_caller_does_not_release_its_transition() {
    let gate=Arc::new(Gate::default());
    let coordinator=BrowserRunCoordinator::new(gate.clone());
    let entered=Arc::new(Notify::new());
    let release=Arc::new(Notify::new());
    let finished=Arc::new(Notify::new());
    let caller=tokio::spawn({
        let (coordinator,entered,release,finished)=(coordinator.clone(),entered.clone(),release.clone(),finished.clone());
        async move {coordinator.close_idle_runtime(move ||async move {
            entered.notify_one();release.notified().await;finished.notify_one();
        }).await}
    });
    entered.notified().await;
    assert!(gate.locked.load(Ordering::SeqCst));
    caller.abort();assert!(caller.await.unwrap_err().is_cancelled());
    let next=tokio::spawn({let coordinator=coordinator.clone();async move {coordinator.begin().await}});
    assert!(!next.is_finished());
    release.notify_one();finished.notified().await;
    assert!(matches!(next.await.unwrap(),Err(RunAdmissionError::Busy)));
    coordinator.close_idle_runtime(||async {}).await.unwrap();
}

#[async_trait]
impl NativeInputGate for Gate {
    async fn lock_user_input(&self) -> Result<(), RunAdmissionError> {
        self.locked.store(true, Ordering::SeqCst);
        self.log.lock().await.push("lock");
        Ok(())
    }
    async fn release_pressed_input(&self) -> Result<(), RunAdmissionError> {
        self.log.lock().await.push("release");
        if self.fail_release.load(Ordering::SeqCst) {
            Err(RunAdmissionError::InputGateFailed)
        } else {
            Ok(())
        }
    }
    async fn unlock_user_input(&self) -> Result<(), RunAdmissionError> {
        self.log.lock().await.push("unlock");
        self.locked.store(false, Ordering::SeqCst);
        self.unlocked.notify_one();
        Ok(())
    }
}

#[tokio::test]
async fn stop_waits_for_native_operation_cleanup_and_rejects_queued_work() {
    let gate = Arc::new(Gate::default());
    let coordinator = BrowserRunCoordinator::new(gate.clone());
    let run = coordinator.begin().await.unwrap();
    assert!(gate.locked.load(Ordering::SeqCst));
    assert_eq!(
        coordinator.user_operation(|| async { Ok(()) }).await,
        Err(RunAdmissionError::UserInputLocked)
    );
    let entered = Arc::new(Notify::new());
    let saw_cancel = Arc::new(Notify::new());
    let settled = Arc::new(Notify::new());
    let op = tokio::spawn({
        let coordinator = coordinator.clone();
        let run = run.clone();
        let entered = entered.clone();
        let saw_cancel = saw_cancel.clone();
        let settled = settled.clone();
        async move {
            coordinator
                .agent_operation(&run, |cancel| async move {
                    entered.notify_one();
                    cancel.cancelled().await;
                    saw_cancel.notify_one();
                    settled.notified().await; // Native release callback is still pending.
                    Ok(())
                })
                .await
        }
    });
    entered.notified().await;
    let finishing = tokio::spawn({
        let coordinator = coordinator.clone();
        let run = run.clone();
        async move { coordinator.finish(&run).await }
    });
    saw_cancel.notified().await;
    assert!(gate.locked.load(Ordering::SeqCst));
    assert_eq!(
        coordinator.snapshot().await.input_state,
        BrowserInputState::AgentRunning
    );
    assert!(!finishing.is_finished());
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_millis(100),coordinator.user_operation(||async {
            panic!("a user command during native cleanup must never be replayed");
            #[allow(unreachable_code)] Ok(())
        })).await,
        Ok(Err(RunAdmissionError::UserInputLocked)),
    );
    assert_eq!(
        coordinator
            .agent_operation(&run, |_| async {
                panic!("cancelled work must not execute");
                #[allow(unreachable_code)]
                Ok(())
            })
            .await,
        Err(RunAdmissionError::Cancelled)
    );
    settled.notify_one();
    op.await.unwrap().unwrap();
    finishing.await.unwrap().unwrap();
    assert!(!gate.locked.load(Ordering::SeqCst));
    assert_eq!(
        *gate.log.lock().await,
        ["lock", "release", "release", "unlock"]
    );
    assert_eq!(
        coordinator.snapshot().await.input_state,
        BrowserInputState::UserReady
    );
    coordinator
        .user_operation(|| async { Ok(()) })
        .await
        .unwrap();
}

#[tokio::test]
async fn queued_user_intent_cannot_cross_an_agent_run_revision() {
    let coordinator=BrowserRunCoordinator::new(Arc::new(Gate::default()));
    let operation=coordinator.operation.lock().await;
    let mut user=Box::pin(coordinator.user_operation(||async {Ok("must not execute")}));
    tokio::select! {biased;
        _=&mut user=>panic!("operation guard must still be held"),
        _=tokio::task::yield_now()=>{},
    }
    // Isolate the revision fence: even a transition that has already returned
    // to UserReady cannot inherit this earlier intent.
    coordinator.state.lock().await.revision+=2;
    drop(operation);
    assert_eq!(user.await,Err(RunAdmissionError::UserInputLocked));
}

#[tokio::test]
async fn dropping_tool_caller_does_not_release_inflight_native_work() {
    let gate = Arc::new(Gate::default());
    let coordinator = BrowserRunCoordinator::new(gate.clone());
    let run = coordinator.begin().await.unwrap();
    let entered = Arc::new(Notify::new());
    let settle = Arc::new(Notify::new());
    let caller = tokio::spawn({
        let coordinator = coordinator.clone();
        let run = run.clone();
        let entered = entered.clone();
        let settle = settle.clone();
        async move {
            coordinator
                .agent_operation(&run, |_| async move {
                    entered.notify_one();
                    settle.notified().await;
                    Ok(())
                })
                .await
        }
    });
    entered.notified().await;
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    run.cancel();
    let finishing = tokio::spawn({
        let coordinator = coordinator.clone();
        let run = run.clone();
        async move { coordinator.finish(&run).await }
    });
    // Completion is controlled by the native callback, not by dropping the caller.
    assert!(gate.locked.load(Ordering::SeqCst));
    assert!(!finishing.is_finished());
    settle.notify_one();
    finishing.await.unwrap().unwrap();
    assert!(!gate.locked.load(Ordering::SeqCst));
}

#[tokio::test]
async fn failed_cleanup_stays_locked_until_same_run_successfully_finishes() {
    let gate = Arc::new(Gate::default());
    let coordinator = BrowserRunCoordinator::new(gate.clone());
    let run = coordinator.begin().await.unwrap();
    gate.fail_release.store(true, Ordering::SeqCst);
    assert_eq!(
        coordinator.finish(&run).await,
        Err(RunAdmissionError::InputGateFailed)
    );
    assert!(gate.locked.load(Ordering::SeqCst));
    assert!(coordinator.snapshot().await.input_gate_failed);
    assert!(matches!(
        coordinator.begin().await,
        Err(RunAdmissionError::Busy)
    ));
    gate.fail_release.store(false, Ordering::SeqCst);
    coordinator.finish(&run).await.unwrap();
    let next_run = coordinator.begin().await.unwrap();
    assert_eq!(
        coordinator
            .agent_operation(&run, |_| async { Ok(()) })
            .await,
        Err(RunAdmissionError::StaleRun)
    );
    assert_eq!(
        coordinator.finish(&run).await,
        Err(RunAdmissionError::StaleRun)
    );
    assert!(gate.locked.load(Ordering::SeqCst));
    coordinator.finish(&next_run).await.unwrap();
}

#[tokio::test]
async fn guards_cannot_cross_resources_and_failed_starts_can_be_cleaned_up() {
    let gate = Arc::new(Gate::default());
    let coordinator = BrowserRunCoordinator::new(gate.clone());
    gate.fail_release.store(true, Ordering::SeqCst);
    assert!(matches!(
        coordinator.begin().await,
        Err(RunAdmissionError::InputGateFailed)
    ));
    assert_eq!(
        coordinator.user_operation(|| async { Ok(()) }).await,
        Err(RunAdmissionError::UserInputLocked)
    );
    gate.fail_release.store(false, Ordering::SeqCst);
    coordinator.recover_failed_start().await.unwrap();
    let run = coordinator.begin().await.unwrap();
    let other = BrowserRunCoordinator::new(Arc::new(Gate::default()));
    let other_run = other.begin().await.unwrap();
    assert_eq!(
        other.agent_operation(&run, |_| async { Ok(()) }).await,
        Err(RunAdmissionError::StaleRun)
    );
    coordinator.finish(&run).await.unwrap();
    other.finish(&other_run).await.unwrap();
}

#[tokio::test]
async fn dropping_run_owner_cancels_and_settles_before_unlocking() {
    let gate = Arc::new(Gate::default());
    let coordinator = BrowserRunCoordinator::new(gate.clone());
    let run = coordinator.begin().await.unwrap();
    let entered = Arc::new(Notify::new());
    let cancelled = Arc::new(Notify::new());
    let settle = Arc::new(Notify::new());
    let caller = tokio::spawn({
        let coordinator = coordinator.clone();
        let run = run.clone();
        let entered = entered.clone();
        let cancelled = cancelled.clone();
        let settle = settle.clone();
        async move {
            coordinator
                .agent_operation(&run, |token| async move {
                    entered.notify_one();
                    token.cancelled().await;
                    cancelled.notify_one();
                    settle.notified().await;
                    Ok(())
                })
                .await
        }
    });
    entered.notified().await;
    caller.abort();
    let _ = caller.await;
    drop(run);
    cancelled.notified().await;
    assert!(gate.locked.load(Ordering::SeqCst));
    settle.notify_one();
    gate.unlocked.notified().await;
    // A new begin serializes behind RAII finish even if the unlock callback
    // wakes the test before finish publishes its final state.
    let next = coordinator.begin().await.unwrap();
    coordinator.finish(&next).await.unwrap();
}

#[tokio::test]
async fn panicking_action_closes_admission_until_the_run_is_settled() {
    let gate = Arc::new(Gate::default());
    let coordinator = BrowserRunCoordinator::new(gate.clone());
    let run = coordinator.begin().await.unwrap();
    let result: Result<(), _> = coordinator
        .agent_operation(&run, |_| async { panic!("test platform operation panic") })
        .await;
    assert_eq!(result, Err(RunAdmissionError::WorkerFailed));
    assert!(coordinator.snapshot().await.input_gate_failed);
    assert!(gate.locked.load(Ordering::SeqCst));
    assert_eq!(
        coordinator
            .agent_operation(&run, |_| async { Ok(()) })
            .await,
        Err(RunAdmissionError::Cancelled)
    );
    coordinator.finish(&run).await.unwrap();
    assert!(!gate.locked.load(Ordering::SeqCst));
}

#[tokio::test]
async fn shutdown_cancels_a_run_that_was_still_locking_native_input() {
    #[derive(Default)]
    struct StartingGate {
        entered: Notify,
        release: Notify,
    }
    #[async_trait]
    impl NativeInputGate for StartingGate {
        async fn lock_user_input(&self) -> Result<(), RunAdmissionError> {
            self.entered.notify_one();
            self.release.notified().await;
            Ok(())
        }
        async fn release_pressed_input(&self) -> Result<(), RunAdmissionError> {
            Ok(())
        }
        async fn unlock_user_input(&self) -> Result<(), RunAdmissionError> {
            Ok(())
        }
    }
    let gate = Arc::new(StartingGate::default());
    let coordinator = BrowserRunCoordinator::new(gate.clone());
    let starting = tokio::spawn({
        let coordinator = coordinator.clone();
        async move { coordinator.begin().await }
    });
    gate.entered.notified().await;
    let closing_entered = Arc::new(Notify::new());
    let closing = tokio::spawn({
        let coordinator = coordinator.clone();
        let entered = closing_entered.clone();
        async move {
            entered.notify_one();
            coordinator.close_runtime(|| async {}).await
        }
    });
    closing_entered.notified().await;
    gate.release.notify_one();
    let run = starting.await.unwrap().unwrap();
    closing.await.unwrap().unwrap();
    assert!(run.0.run.cancelled.is_cancelled());
    assert_eq!(
        coordinator
            .agent_operation(&run, |_| async { Ok(()) })
            .await,
        Err(RunAdmissionError::Cancelled)
    );
}

#[tokio::test]
async fn explicit_terminal_owner_does_not_unlock_on_an_unproven_drop() {
    let gate = Arc::new(Gate::default());
    let coordinator = BrowserRunCoordinator::new(gate.clone());
    let run = coordinator.begin().await.unwrap();
    run.require_explicit_finish();
    coordinator.settle(&run).await.unwrap();
    assert!(gate.locked.load(Ordering::SeqCst));
    drop(run);
    assert_eq!(
        coordinator.snapshot().await.input_state,
        BrowserInputState::AgentRunning
    );
    assert!(gate.locked.load(Ordering::SeqCst));
    assert_eq!(
        coordinator.user_operation(|| async { Ok(()) }).await,
        Err(RunAdmissionError::UserInputLocked)
    );
    // Destruction is still possible without declaring the Agent terminal proven.
    coordinator.close_runtime(|| async {}).await.unwrap();
}
