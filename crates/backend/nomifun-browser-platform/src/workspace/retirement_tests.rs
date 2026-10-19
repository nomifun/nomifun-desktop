//! Domain/native-owner fixtures; no user browser or persistent user data.
use super::*;

#[derive(Default)]
struct WatchedFactory {
    creates: AtomicUsize,
    closes: Arc<AtomicUsize>,
    last: std::sync::Mutex<Option<std::sync::Weak<WatchedRuntime>>>,
    close_gate: std::sync::Mutex<Option<Arc<CloseGate>>>,
    hide_gate: std::sync::Mutex<Option<Arc<CloseGate>>>,
    show_gate: std::sync::Mutex<Option<Arc<CloseGate>>>,
    lock_gate: std::sync::Mutex<Option<Arc<CloseGate>>>,
    unlock_gate: std::sync::Mutex<Option<Arc<CloseGate>>>,
    fail_lock_once: Arc<AtomicBool>,
    fail_unlock_once: Arc<AtomicBool>,
    panic_operation_once: Arc<AtomicBool>,
    lock_calls: Arc<AtomicUsize>,
}

struct CloseGate {
    first: AtomicBool,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

struct WatchedRuntime {
    base: Runtime,
    changes: tokio::sync::watch::Sender<u64>,
    closes: Arc<AtomicUsize>,
    close_gate: Option<Arc<CloseGate>>,
    hide_gate: Option<Arc<CloseGate>>,
    show_gate: Option<Arc<CloseGate>>,
    lock_gate: Option<Arc<CloseGate>>,
    unlock_gate: Option<Arc<CloseGate>>,
    fail_lock_once: Arc<AtomicBool>,
    fail_unlock_once: Arc<AtomicBool>,
    panic_operation_once: Arc<AtomicBool>,
    lock_calls: Arc<AtomicUsize>,
}

#[async_trait]
impl BrowserRuntimeFactory for WatchedFactory {
    async fn create(&self, request: CreateBrowserRuntime) -> Result<Arc<dyn BrowserRuntime>, WorkspaceError> {
        self.creates.fetch_add(1, Ordering::SeqCst);
        let runtime = Arc::new(WatchedRuntime {
            base: Runtime {
                snapshot: Mutex::new(BrowserRuntimeSnapshot { runtime_generation: request.runtime_generation,
                    revision: 0, active_tab_id: None, tabs: vec![], downloads: vec![] }),
                locked: AtomicBool::new(!request.user_input_enabled), fail_close_once: Arc::new(AtomicBool::new(false)), clear: None,
            },
            changes: tokio::sync::watch::channel(0).0,
            closes: self.closes.clone(),
            close_gate: self.close_gate.lock().unwrap().clone(),
            hide_gate: self.hide_gate.lock().unwrap().clone(),
            show_gate: self.show_gate.lock().unwrap().clone(),
            lock_gate: self.lock_gate.lock().unwrap().clone(),
            unlock_gate: self.unlock_gate.lock().unwrap().clone(),
            fail_lock_once: self.fail_lock_once.clone(),
            fail_unlock_once: self.fail_unlock_once.clone(),
            panic_operation_once: self.panic_operation_once.clone(),
            lock_calls: self.lock_calls.clone(),
        });
        *self.last.lock().unwrap() = Some(Arc::downgrade(&runtime));
        Ok(runtime)
    }
}

#[async_trait]
impl NativeInputGate for WatchedRuntime {
    async fn lock_user_input(&self) -> Result<(), RunAdmissionError> {
        self.lock_calls.fetch_add(1, Ordering::SeqCst);
        if let Some(gate) = &self.lock_gate {
            if gate.first.swap(false, Ordering::SeqCst) {
                gate.entered.notify_one();
                gate.release.notified().await;
            }
        }
        if self.fail_lock_once.swap(false, Ordering::SeqCst) { return Err(RunAdmissionError::InputGateFailed); }
        self.base.lock_user_input().await
    }
    async fn release_pressed_input(&self) -> Result<(), RunAdmissionError> { self.base.release_pressed_input().await }
    async fn unlock_user_input(&self) -> Result<(), RunAdmissionError> {
        if let Some(gate) = &self.unlock_gate {
            if gate.first.swap(false, Ordering::SeqCst) {
                gate.entered.notify_one();
                gate.release.notified().await;
            }
        }
        self.base.unlock_user_input().await?;
        if self.fail_unlock_once.swap(false, Ordering::SeqCst) { return Err(RunAdmissionError::InputGateFailed); }
        Ok(())
    }
}

#[async_trait]
impl BrowserNativeSurfacePort for WatchedRuntime {
    async fn set_surface(&self, _: BrowserSurfaceBounds, visible: bool, _: CancellationToken) -> Result<(), WorkspaceError> {
        if visible {
            if let Some(gate) = &self.show_gate {
                if gate.first.swap(false, Ordering::SeqCst) {
                    gate.entered.notify_one();
                    gate.release.notified().await;
                }
            }
        }
        if !visible {
            if let Some(gate) = &self.hide_gate {
                if gate.first.swap(false, Ordering::SeqCst) {
                    gate.entered.notify_one();
                    gate.release.notified().await;
                }
            }
        }
        Ok(())
    }
}

#[async_trait]
impl BrowserRuntime for WatchedRuntime {
    fn changes(&self) -> Option<tokio::sync::watch::Receiver<u64>> { Some(self.changes.subscribe()) }
    fn surface(&self) -> Option<&dyn BrowserNativeSurfacePort> { Some(self) }
    async fn snapshot(&self) -> Result<BrowserRuntimeSnapshot, WorkspaceError> { self.base.snapshot().await }
    async fn execute(&self, command: BrowserTabCommand, cancel: CancellationToken) -> Result<BrowserRuntimeSnapshot, WorkspaceError> {
        assert!(!self.panic_operation_once.swap(false, Ordering::SeqCst), "fixture native worker failed");
        let result = self.base.execute(command, cancel).await;
        self.changes.send_modify(|revision| *revision += 1);
        result
    }
    async fn close(&self) -> Result<(), WorkspaceError> {
        self.closes.fetch_add(1, Ordering::SeqCst);
        if let Some(gate) = &self.close_gate {
            if gate.first.swap(false, Ordering::SeqCst) {
                gate.entered.notify_one();
                gate.release.notified().await;
            }
        }
        let result = self.base.close().await;
        self.changes.send_modify(|revision| *revision += 1);
        result
    }
}

async fn agent_page(service: &BrowserResourceService, session: &str, task: &str, profile: BrowserProfile) -> Arc<BrowserResource> {
    let resource = service.ensure(authority("alice", session, "managed", all_actions()), profile).await.unwrap();
    resource.retain_task(task).await.unwrap();
    let run = resource.begin_run().await.unwrap();
    resource.agent_command(&run, create()).await.unwrap();
    resource.finish_run(&run).await.unwrap();
    resource
}

async fn retired(service: &BrowserResourceService, session: &str) {
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while service.get_for_agent_session("alice", session).await.unwrap().is_some() { tokio::task::yield_now().await; }
    }).await.expect("the native owner must retire without closing the historical Session");
}

async fn retention(service: &BrowserResourceService, session: &str) -> BrowserRetentionReason {
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let reason = service.observe_workspace("alice", session).await.unwrap().retention;
            if reason != BrowserRetentionReason::ObservationBusy { return reason; }
            tokio::task::yield_now().await;
        }
    }).await.expect("the original owner must expose its short-held use state")
}

#[tokio::test]
async fn deleting_a_slow_session_fences_only_its_own_admission_and_retains_native_owner() {
    let gate = Arc::new(CloseGate { first: AtomicBool::new(true), entered: Default::default(), release: Default::default() });
    let factory = Arc::new(WatchedFactory::default());
    *factory.close_gate.lock().unwrap() = Some(gate.clone());
    let service = Arc::new(BrowserResourceService::new(factory.clone()));
    let original = service.ensure_user("alice", "removed", BrowserProfile::Ephemeral).await.unwrap();
    original.user_command(create()).await.unwrap();
    let entered = gate.entered.notified();
    let deletion = tokio::spawn({ let service = service.clone(); async move { service.delete_agent_session("alice", "removed").await }});
    entered.await;
    deletion.abort();
    let next = tokio::time::timeout(std::time::Duration::from_secs(1),
        service.ensure_user("alice", "successor", BrowserProfile::Ephemeral)).await
        .expect("slow native close must not hold service admission").unwrap();
    next.user_command(create()).await.unwrap();
    assert!(matches!(service.ensure_user("alice", "removed", BrowserProfile::Ephemeral).await, Err(WorkspaceError::WorkspaceClosed)));
    assert!(original.user_command(create()).await.is_err());
    assert!(!original.native_close_proven().await, "a closing flag cannot replace the native acknowledgement");
    gate.release.notify_one();
    service.delete_agent_session("alice", "removed").await.unwrap();
    assert!(original.native_close_proven().await);
    assert_eq!(factory.closes.load(Ordering::SeqCst), 1, "retry joins the original native close rather than creating a replacement");
    assert_eq!(factory.creates.load(Ordering::SeqCst), 2);
    assert!(matches!(service.ensure_user("alice", "removed", BrowserProfile::Ephemeral).await, Err(WorkspaceError::WorkspaceClosed)));
}

#[tokio::test]
async fn completed_agent_only_carrier_retires_and_reopens_lazily_without_deleting_profile() {
    let factory = Arc::new(WatchedFactory::default());
    let service = BrowserResourceService::new(factory.clone());
    let directory = tempfile::tempdir().unwrap();
    let profile = directory.path().join("profile");
    std::fs::create_dir(&profile).unwrap();
    std::fs::write(profile.join("login-fixture"), "retained profile").unwrap();
    let old = agent_page(&service, "history", "task", BrowserProfile::Persistent(profile.clone())).await;
    old.workspace().try_retire_after_task("task").await.unwrap();
    retired(&service, "history").await;
    assert!(old.snapshot().await.unwrap().runtime.is_none());
    assert_eq!(factory.closes.load(Ordering::SeqCst), 1);
    assert_eq!(std::fs::read_to_string(profile.join("login-fixture")).unwrap(), "retained profile");
    let fresh = service.ensure(authority("alice", "history", "managed", all_actions()), BrowserProfile::Persistent(profile)).await.unwrap();
    assert!(fresh.snapshot().await.unwrap().runtime.is_none());
    assert_eq!(factory.creates.load(Ordering::SeqCst), 1, "reading/reopening history must not start native work");
    assert!(old.begin_run().await.is_err(), "a retained old handle must not reopen its native generation");
    fresh.retain_task("explicit-new-task").await.unwrap();
    let run = fresh.begin_run().await.unwrap();
    fresh.agent_command(&run, create()).await.unwrap();
    fresh.finish_run(&run).await.unwrap();
    assert_eq!(factory.creates.load(Ordering::SeqCst), 2, "explicit new use creates one fresh carrier");
    service.shutdown().await.unwrap();
}

fn native_barrier() -> Arc<CloseGate> {
    Arc::new(CloseGate { first: AtomicBool::new(true), entered: Default::default(), release: Default::default() })
}

fn desktop_bounds() -> BrowserSurfaceBounds {
    BrowserSurfaceBounds { x: 0., y: 0., width: 880., height: 600. }
}

async fn running_agent_page(service: &BrowserResourceService, session: &str) -> (Arc<BrowserResource>, BrowserRunGuard) {
    let resource = service.ensure(authority("alice", session, "managed", all_actions()), BrowserProfile::Ephemeral).await.unwrap();
    resource.retain_task("task").await.unwrap();
    let run = resource.begin_run().await.unwrap();
    resource.agent_command(&run, create()).await.unwrap();
    (resource, run)
}

#[tokio::test]
async fn agent_only_locked_presentation_retires_nonempty_after_exact_hide_and_detach() {
    let factory = Arc::new(WatchedFactory::default());
    let service = BrowserResourceService::new(factory.clone());
    let (resource, run) = running_agent_page(&service, "locked-presentation").await;
    assert_eq!(factory.lock_calls.load(Ordering::SeqCst), 1, "first locked creation must acquire a real native ACK");
    resource.retain_surface_attachment(41).await.unwrap();
    resource.set_agent_surface(desktop_bounds(), true, CancellationToken::new()).await.unwrap();
    resource.set_agent_surface(desktop_bounds(), false, CancellationToken::new()).await.unwrap();
    resource.release_surface_attachment(41).await;
    resource.finish_interaction(&run).await.unwrap();
    assert_eq!(resource.snapshot().await.unwrap().runtime.unwrap().tabs.len(), 1);
    resource.workspace().try_retire_after_task("task").await.unwrap();
    retired(&service, "locked-presentation").await;
    assert!(resource.native_close_proven().await);
    assert_eq!(factory.closes.load(Ordering::SeqCst), 1);
    assert!(matches!(resource.retain_user_page_use().await, Err(WorkspaceError::WorkspaceClosed)), "a late human request must not reopen a closing generation");
}

#[tokio::test]
async fn agent_auto_source_without_current_native_lock_cannot_erase_human_exposure() {
    let factory = Arc::new(WatchedFactory::default());
    let service = BrowserResourceService::new(factory.clone());
    let resource = agent_page(&service, "auto-without-lock", "task", BrowserProfile::Ephemeral).await;
    resource.set_agent_surface(desktop_bounds(), true, CancellationToken::new()).await.unwrap();
    resource.set_agent_surface(desktop_bounds(), false, CancellationToken::new()).await.unwrap();
    assert!(!resource.workspace().try_retire_after_task("task").await.unwrap());
    assert_eq!(retention(&service, "auto-without-lock").await, BrowserRetentionReason::UserPageStateUnknown);
    assert_eq!(factory.closes.load(Ordering::SeqCst), 0);
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn manual_open_or_default_surface_under_agent_lock_remains_a_human_purpose() {
    for explicit_open in [false, true] {
        let factory = Arc::new(WatchedFactory::default());
        let service = BrowserResourceService::new(factory.clone());
        let (resource, run) = running_agent_page(&service, "human-under-lock").await;
        if explicit_open {
            resource.retain_user_page_use().await.unwrap();
            resource.set_agent_surface(desktop_bounds(), true, CancellationToken::new()).await.unwrap();
        } else {
            resource.set_surface(desktop_bounds(), true, CancellationToken::new()).await.unwrap();
        }
        resource.set_agent_surface(desktop_bounds(), false, CancellationToken::new()).await.unwrap();
        resource.finish_interaction(&run).await.unwrap();
        assert!(!resource.workspace().try_retire_after_task("task").await.unwrap());
        assert_eq!(retention(&service, "human-under-lock").await, BrowserRetentionReason::UserPageStateUnknown);
        assert_eq!(factory.closes.load(Ordering::SeqCst), 0);
        service.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn visible_handoff_pins_before_native_enable_even_when_enable_partially_fails() {
    let factory = Arc::new(WatchedFactory::default());
    let service = BrowserResourceService::new(factory.clone());
    let (resource, run) = running_agent_page(&service, "partial-enable").await;
    resource.set_agent_surface(desktop_bounds(), true, CancellationToken::new()).await.unwrap();
    factory.fail_unlock_once.store(true, Ordering::SeqCst);
    assert!(resource.finish_interaction(&run).await.is_err());
    let native = factory.last.lock().unwrap().as_ref().unwrap().upgrade().unwrap();
    assert!(!native.base.locked.load(Ordering::SeqCst), "the native fixture applied enable before reporting its failure");
    resource.set_agent_surface(desktop_bounds(), false, CancellationToken::new()).await.unwrap();
    resource.finish_run(&run).await.unwrap();
    assert!(!resource.workspace().try_retire_after_task("task").await.unwrap());
    assert_eq!(retention(&service, "partial-enable").await, BrowserRetentionReason::UserPageStateUnknown);
    assert_eq!(factory.closes.load(Ordering::SeqCst), 0);
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn retained_attachment_pins_handoff_even_after_native_hide_acknowledges() {
    let factory = Arc::new(WatchedFactory::default());
    let service = BrowserResourceService::new(factory.clone());
    let (resource, run) = running_agent_page(&service, "retained-attachment").await;
    resource.retain_surface_attachment(41).await.unwrap();
    resource.set_agent_surface(desktop_bounds(), true, CancellationToken::new()).await.unwrap();
    resource.set_agent_surface(desktop_bounds(), false, CancellationToken::new()).await.unwrap();
    resource.finish_interaction(&run).await.unwrap();
    resource.release_surface_attachment(41).await;
    assert!(!resource.workspace().try_retire_after_task("task").await.unwrap());
    assert_eq!(retention(&service, "retained-attachment").await, BrowserRetentionReason::UserPageStateUnknown);
    assert_eq!(factory.closes.load(Ordering::SeqCst), 0);
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn cancelled_unconfirmed_agent_show_remains_possible_exposure_at_handoff() {
    let shown = native_barrier();
    let factory = Arc::new(WatchedFactory::default());
    *factory.show_gate.lock().unwrap() = Some(shown.clone());
    let service = BrowserResourceService::new(factory.clone());
    let (resource, run) = running_agent_page(&service, "cancelled-show").await;
    let cancel = CancellationToken::new();
    let show = tokio::spawn({ let resource = resource.clone(); let cancel = cancel.clone(); async move {
        resource.set_agent_surface(desktop_bounds(), true, cancel).await
    }});
    shown.entered.notified().await;
    cancel.cancel();
    resource.finish_interaction(&run).await.unwrap();
    shown.release.notify_one();
    show.await.unwrap().unwrap();
    resource.set_agent_surface(desktop_bounds(), false, CancellationToken::new()).await.unwrap();
    assert!(!resource.workspace().try_retire_after_task("task").await.unwrap());
    assert_eq!(retention(&service, "cancelled-show").await, BrowserRetentionReason::UserPageStateUnknown);
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn hide_posted_before_handoff_does_not_replace_its_original_native_ack() {
    let hidden = native_barrier();
    let factory = Arc::new(WatchedFactory::default());
    *factory.hide_gate.lock().unwrap() = Some(hidden.clone());
    let service = BrowserResourceService::new(factory.clone());
    let (resource, run) = running_agent_page(&service, "pending-hide").await;
    resource.set_agent_surface(desktop_bounds(), true, CancellationToken::new()).await.unwrap();
    let hide = tokio::spawn({ let resource = resource.clone(); async move {
        resource.set_agent_surface(desktop_bounds(), false, CancellationToken::new()).await
    }});
    hidden.entered.notified().await;
    resource.finish_interaction(&run).await.unwrap();
    hidden.release.notify_one();
    hide.await.unwrap().unwrap();
    assert!(!resource.workspace().try_retire_after_task("task").await.unwrap());
    assert_eq!(retention(&service, "pending-hide").await, BrowserRetentionReason::UserPageStateUnknown);
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn native_lock_failure_cannot_turn_auto_source_into_a_task_only_proof() {
    let factory = Arc::new(WatchedFactory::default());
    let service = BrowserResourceService::new(factory.clone());
    let resource = agent_page(&service, "failed-lock", "task", BrowserProfile::Ephemeral).await;
    factory.fail_lock_once.store(true, Ordering::SeqCst);
    assert!(resource.begin_run().await.is_err());
    resource.set_agent_surface(desktop_bounds(), true, CancellationToken::new()).await.unwrap();
    resource.set_agent_surface(desktop_bounds(), false, CancellationToken::new()).await.unwrap();
    resource.workspace().recover_failed_start().await.unwrap();
    assert!(!resource.workspace().try_retire_after_task("task").await.unwrap());
    assert_eq!(retention(&service, "failed-lock").await, BrowserRetentionReason::UserPageStateUnknown);
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn locked_creation_waits_for_original_native_ack_before_auto_presentation() {
    let locked = native_barrier();
    let factory = Arc::new(WatchedFactory::default());
    *factory.lock_gate.lock().unwrap() = Some(locked.clone());
    let service = BrowserResourceService::new(factory.clone());
    let resource = service.ensure(authority("alice", "creating-ack", "managed", all_actions()), BrowserProfile::Ephemeral).await.unwrap();
    resource.retain_task("task").await.unwrap();
    let run = resource.begin_run().await.unwrap();
    let creating = tokio::spawn({ let resource = resource.clone(); let run = run.clone(); async move { resource.agent_command(&run, create()).await }});
    locked.entered.notified().await;
    assert!(!creating.is_finished());
    let show = tokio::spawn({ let resource = resource.clone(); async move { resource.set_agent_surface(desktop_bounds(), true, CancellationToken::new()).await }});
    locked.release.notify_one();
    creating.await.unwrap().unwrap();
    show.await.unwrap().unwrap();
    resource.set_agent_surface(desktop_bounds(), false, CancellationToken::new()).await.unwrap();
    resource.finish_interaction(&run).await.unwrap();
    resource.workspace().try_retire_after_task("task").await.unwrap();
    retired(&service, "creating-ack").await;
    assert_eq!(factory.lock_calls.load(Ordering::SeqCst), 1);
    assert_eq!(factory.closes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_show_during_native_enable_pins_without_waiting_for_that_native_owner() {
    let enabling = native_barrier();
    let factory = Arc::new(WatchedFactory::default());
    *factory.unlock_gate.lock().unwrap() = Some(enabling.clone());
    let service = BrowserResourceService::new(factory.clone());
    let (resource, run) = running_agent_page(&service, "show-during-enable").await;
    let finish = tokio::spawn({ let resource = resource.clone(); async move { resource.finish_interaction(&run).await }});
    enabling.entered.notified().await;
    tokio::time::timeout(std::time::Duration::from_secs(3), resource.set_agent_surface(desktop_bounds(), true, CancellationToken::new()))
        .await.expect("layout must not wait for an in-flight native input handoff").unwrap();
    resource.set_agent_surface(desktop_bounds(), false, CancellationToken::new()).await.unwrap();
    enabling.release.notify_one();
    finish.await.unwrap().unwrap();
    assert!(!resource.workspace().try_retire_after_task("task").await.unwrap());
    assert_eq!(retention(&service, "show-during-enable").await, BrowserRetentionReason::UserPageStateUnknown);
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn late_agent_hide_cannot_erase_a_successor_show_before_user_handoff() {
    let hidden = native_barrier();
    let factory = Arc::new(WatchedFactory::default());
    *factory.hide_gate.lock().unwrap() = Some(hidden.clone());
    let service = BrowserResourceService::new(factory.clone());
    let (resource, run) = running_agent_page(&service, "agent-late-hide").await;
    resource.set_agent_surface(desktop_bounds(), true, CancellationToken::new()).await.unwrap();
    let hide = tokio::spawn({ let resource = resource.clone(); async move { resource.set_agent_surface(desktop_bounds(), false, CancellationToken::new()).await }});
    hidden.entered.notified().await;
    resource.set_agent_surface(desktop_bounds(), true, CancellationToken::new()).await.unwrap();
    hidden.release.notify_one();
    hide.await.unwrap().unwrap();
    resource.finish_interaction(&run).await.unwrap();
    resource.set_agent_surface(desktop_bounds(), false, CancellationToken::new()).await.unwrap();
    assert!(!resource.workspace().try_retire_after_task("task").await.unwrap());
    assert_eq!(retention(&service, "agent-late-hide").await, BrowserRetentionReason::UserPageStateUnknown);
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn first_native_lock_failure_keeps_the_created_owner_for_exact_shutdown() {
    let factory = Arc::new(WatchedFactory::default());
    factory.fail_lock_once.store(true, Ordering::SeqCst);
    let service = BrowserResourceService::new(factory.clone());
    let resource = service.ensure(authority("alice", "first-lock-failure", "managed", all_actions()), BrowserProfile::Ephemeral).await.unwrap();
    resource.retain_task("task").await.unwrap();
    let run = resource.begin_run().await.unwrap();
    assert!(resource.agent_command(&run, create()).await.is_err());
    assert!(matches!(resource.agent_command(&run, create()).await,
        Err(WorkspaceError::Admission(RunAdmissionError::InputGateFailed))), "a retained handle is not a successful native lock ACK");
    assert!(resource.snapshot().await.unwrap().runtime.is_some(), "an acquired native owner cannot disappear when its lock ACK fails");
    assert!(resource.snapshot().await.unwrap().runtime.unwrap().tabs.is_empty(), "failed admission must not dispatch later native page creation");
    assert_eq!(factory.creates.load(Ordering::SeqCst), 1);
    assert_eq!(factory.lock_calls.load(Ordering::SeqCst), 1);
    resource.set_agent_surface(desktop_bounds(), true, CancellationToken::new()).await.unwrap();
    resource.set_agent_surface(desktop_bounds(), false, CancellationToken::new()).await.unwrap();
    resource.finish_run(&run).await.unwrap();
    service.shutdown().await.unwrap();
    assert!(resource.native_close_proven().await);
    assert_eq!(factory.closes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn failed_native_worker_cannot_reuse_an_earlier_lock_ack_as_human_use_proof() {
    let factory = Arc::new(WatchedFactory::default());
    let service = BrowserResourceService::new(factory.clone());
    let (resource, run) = running_agent_page(&service, "failed-worker").await;
    factory.panic_operation_once.store(true, Ordering::SeqCst);
    assert!(matches!(resource.agent_command(&run, create()).await,
        Err(WorkspaceError::Admission(RunAdmissionError::WorkerFailed))));
    resource.set_agent_surface(desktop_bounds(), true, CancellationToken::new()).await.unwrap();
    resource.set_agent_surface(desktop_bounds(), false, CancellationToken::new()).await.unwrap();
    resource.finish_run(&run).await.unwrap();
    assert!(!resource.workspace().try_retire_after_task("task").await.unwrap());
    assert_eq!(retention(&service, "failed-worker").await, BrowserRetentionReason::UserPageStateUnknown);
    assert_eq!(resource.snapshot().await.unwrap().runtime.unwrap().tabs.len(), 1, "no native command was replayed after the failed worker");
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn user_exposure_and_hidden_nonempty_pages_retain_site_state_until_last_page_closes() {
    let factory = Arc::new(WatchedFactory::default());
    let service = BrowserResourceService::new(factory.clone());
    let resource = agent_page(&service, "human", "task", BrowserProfile::Ephemeral).await;
    let bounds = BrowserSurfaceBounds { x: 0., y: 0., width: 880., height: 600. };
    resource.set_surface(bounds, true, CancellationToken::new()).await.unwrap();
    resource.set_surface(bounds, false, CancellationToken::new()).await.unwrap();
    assert!(!resource.workspace().try_retire_after_task("task").await.unwrap());
    assert_eq!(resource.snapshot().await.unwrap().runtime.unwrap().tabs.len(), 1);
    assert_eq!(retention(&service, "human").await, BrowserRetentionReason::UserPageStateUnknown);
    assert_eq!(factory.closes.load(Ordering::SeqCst), 0, "hiding or ending a Turn must not discard a human page");
    let target = resource.snapshot().await.unwrap().runtime.unwrap().tabs[0].target.clone();
    resource.user_command(BrowserTabCommand::Close { target }).await.unwrap();
    retired(&service, "human").await;
    assert_eq!(factory.closes.load(Ordering::SeqCst), 1, "past exposure must not permanently pin an empty carrier");
}

#[tokio::test]
async fn in_use_empty_browser_keeps_its_runtime_until_the_current_native_hide_acknowledges() {
    let factory = Arc::new(WatchedFactory::default());
    let hidden = Arc::new(CloseGate { first: AtomicBool::new(true), entered: Default::default(), release: Default::default() });
    *factory.hide_gate.lock().unwrap() = Some(hidden.clone());
    let service = BrowserResourceService::new(factory.clone());
    let resource = agent_page(&service, "same-runtime", "task", BrowserProfile::Ephemeral).await;
    let generation = resource.runtime_generation();
    let bounds = BrowserSurfaceBounds { x: 0., y: 0., width: 880., height: 600. };
    resource.set_surface(bounds, true, CancellationToken::new()).await.unwrap();
    let empty = resource.user_command(BrowserTabCommand::CloseAll { runtime_generation: generation }).await.unwrap();
    assert!(empty.tabs.is_empty());
    assert!(!resource.workspace().try_retire_after_task("task").await.unwrap());
    let reopened = resource.user_command(create()).await.unwrap();
    assert_eq!(reopened.runtime_generation, generation, "a currently used browser must preserve close-all/reopen semantics");
    assert_eq!(factory.creates.load(Ordering::SeqCst), 1);
    resource.user_command(BrowserTabCommand::CloseAll { runtime_generation: generation }).await.unwrap();
    let workspace = resource.workspace().clone();
    let hide = tokio::spawn(async move { workspace.set_surface(bounds, false, CancellationToken::new()).await });
    hidden.entered.notified().await;
    assert!(!resource.workspace().try_retire_after_task("task").await.unwrap());
    assert_eq!(retention(&service, "same-runtime").await, BrowserRetentionReason::CurrentUserUse);
    assert_eq!(factory.closes.load(Ordering::SeqCst), 0, "posted hide alone is not native release evidence");
    hidden.release.notify_one();
    hide.await.unwrap().unwrap();
    retired(&service, "same-runtime").await;
    assert_eq!(factory.closes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_late_hide_cannot_retire_a_successor_native_presentation() {
    let factory = Arc::new(WatchedFactory::default());
    let hidden = Arc::new(CloseGate { first: AtomicBool::new(true), entered: Default::default(), release: Default::default() });
    *factory.hide_gate.lock().unwrap() = Some(hidden.clone());
    let service = BrowserResourceService::new(factory.clone());
    let resource = agent_page(&service, "late-hide", "task", BrowserProfile::Ephemeral).await;
    let bounds = BrowserSurfaceBounds { x: 0., y: 0., width: 880., height: 600. };
    resource.set_surface(bounds, true, CancellationToken::new()).await.unwrap();
    resource.user_command(BrowserTabCommand::CloseAll { runtime_generation: resource.runtime_generation() }).await.unwrap();
    let workspace = resource.workspace().clone();
    let hide = tokio::spawn(async move { workspace.set_surface(bounds, false, CancellationToken::new()).await });
    hidden.entered.notified().await;
    resource.set_surface(bounds, true, CancellationToken::new()).await.unwrap();
    hidden.release.notify_one();
    hide.await.unwrap().unwrap();
    assert!(!resource.workspace().try_retire_after_task("task").await.unwrap());
    assert_eq!(retention(&service, "late-hide").await, BrowserRetentionReason::CurrentUserUse);
    assert_eq!(factory.closes.load(Ordering::SeqCst), 0);
    resource.set_surface(bounds, false, CancellationToken::new()).await.unwrap();
    retired(&service, "late-hide").await;
}

#[tokio::test]
async fn temporary_overlay_keeps_the_exact_attachment_until_native_detach_releases_it() {
    let factory = Arc::new(WatchedFactory::default());
    let service = BrowserResourceService::new(factory.clone());
    let resource = agent_page(&service, "overlay", "task", BrowserProfile::Ephemeral).await;
    let bounds = BrowserSurfaceBounds { x: 0., y: 0., width: 880., height: 600. };
    resource.retain_surface_attachment(41).await.unwrap();
    resource.set_surface(bounds, true, CancellationToken::new()).await.unwrap();
    resource.user_command(BrowserTabCommand::CloseAll { runtime_generation: resource.runtime_generation() }).await.unwrap();
    resource.set_surface(bounds, false, CancellationToken::new()).await.unwrap();
    assert!(!resource.workspace().try_retire_after_task("task").await.unwrap());
    assert_eq!(retention(&service, "overlay").await, BrowserRetentionReason::CurrentUserUse);
    assert_eq!(factory.closes.load(Ordering::SeqCst), 0, "a menu/overlay hide must not dispose the still-attached user workspace");
    resource.retain_surface_attachment(42).await.unwrap();
    resource.release_surface_attachment(41).await;
    resource.release_surface_attachment(41).await;
    assert!(!resource.workspace().try_retire_after_task("task").await.unwrap(), "a late old detach cannot release the successor");
    resource.release_surface_attachment(42).await;
    retired(&service, "overlay").await;
    assert_eq!(factory.closes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn pending_download_retires_on_its_native_completion_notification_without_cancelling_transfer() {
    let factory = Arc::new(WatchedFactory::default());
    let service = BrowserResourceService::new(factory.clone());
    let resource = agent_page(&service, "download", "task", BrowserProfile::Ephemeral).await;
    let native = factory.last.lock().unwrap().as_ref().unwrap().upgrade().unwrap();
    native.base.snapshot.lock().await.downloads.push(BrowserDownloadSnapshot {
        id: "original-download".into(), tab_id: "tab-0".into(), filename: "output.bin".into(),
        state: BrowserDownloadState::InProgress, received_bytes: 1, total_bytes: Some(2), can_cancel: true,
    });
    assert!(!resource.workspace().try_retire_after_task("task").await.unwrap());
    assert_eq!(factory.closes.load(Ordering::SeqCst), 0, "legitimate transfer must retain its native owner");
    {
        let mut snapshot = native.base.snapshot.lock().await;
        snapshot.downloads[0].state = BrowserDownloadState::Completed;
        snapshot.downloads[0].received_bytes = 2;
        snapshot.downloads[0].can_cancel = false;
    }
    native.changes.send_modify(|revision| *revision += 1);
    retired(&service, "download").await;
    assert_eq!(native.base.snapshot.lock().await.downloads[0].state, BrowserDownloadState::Completed);
    assert_eq!(factory.closes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn successor_task_and_active_native_run_prevent_stale_automatic_retirement() {
    let factory = Arc::new(WatchedFactory::default());
    let service = BrowserResourceService::new(factory.clone());
    let resource = agent_page(&service, "successor", "old", BrowserProfile::Ephemeral).await;
    resource.retain_task("successor").await.unwrap();
    assert!(!resource.workspace().try_retire_after_task("old").await.unwrap());
    let run = resource.begin_run().await.unwrap();
    assert!(!resource.workspace().try_retire_after_task("successor").await.unwrap());
    assert_eq!(factory.closes.load(Ordering::SeqCst), 0);
    resource.finish_run(&run).await.unwrap();
    resource.workspace().try_retire_after_task("successor").await.unwrap();
    retired(&service, "successor").await;
    assert_eq!(factory.closes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn failed_automatic_close_retries_the_same_native_owner_without_recreating_business() {
    let factory = Arc::new(WatchedFactory::default());
    let service = BrowserResourceService::new(factory.clone());
    let resource = agent_page(&service, "failed-close", "task", BrowserProfile::Ephemeral).await;
    let native = factory.last.lock().unwrap().as_ref().unwrap().upgrade().unwrap();
    native.base.fail_close_once.store(true, Ordering::SeqCst);
    let _first = resource.workspace().try_retire_after_task("task").await;
    retired(&service, "failed-close").await;
    assert_eq!(factory.creates.load(Ordering::SeqCst), 1);
    assert_eq!(factory.closes.load(Ordering::SeqCst), 2, "the failed original close must retain retry responsibility");
    assert!(resource.snapshot().await.unwrap().runtime.is_none());
}

#[tokio::test]
async fn an_ordinary_answer_does_not_prove_nonempty_browser_mission_finished() {
    let factory = Arc::new(WatchedFactory::default());
    let service = BrowserResourceService::new(factory.clone());
    let resource = agent_page(&service, "ongoing-page", "ordinary-turn", BrowserProfile::Ephemeral).await;
    assert!(!resource.workspace().try_retire_empty_after_turn("ordinary-turn").await.unwrap());
    assert!(service.get_for_agent_session("alice", "ongoing-page").await.unwrap().is_some());
    assert_eq!(resource.snapshot().await.unwrap().runtime.unwrap().tabs.len(), 1);
    assert_eq!(factory.closes.load(Ordering::SeqCst), 0, "page scripts/background responsibility remains unknown");
    assert_eq!(retention(&service, "ongoing-page").await, BrowserRetentionReason::TaskCompletionUnknown);
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn slow_retirement_retains_native_owner_while_requests_and_other_sessions_progress() {
    let factory = Arc::new(WatchedFactory::default());
    let gate = Arc::new(CloseGate { first: AtomicBool::new(true), entered: Default::default(), release: Default::default() });
    *factory.close_gate.lock().unwrap() = Some(gate.clone());
    let service = BrowserResourceService::new(factory.clone());
    let resource = agent_page(&service, "slow-close", "completed", BrowserProfile::Ephemeral).await;
    let workspace = resource.workspace().clone();
    let caller = tokio::spawn(async move { workspace.try_retire_after_task("completed").await });
    tokio::time::timeout(std::time::Duration::from_secs(3), gate.entered.notified()).await.unwrap();
    caller.abort();
    let _ = caller.await;
    assert!(tokio::time::timeout(std::time::Duration::from_secs(1),
        resource.workspace().request_retirement_after_turn("completed", true)).await
        .expect("host acceptance must not wait behind native destruction").unwrap());
    assert_eq!(tokio::time::timeout(std::time::Duration::from_secs(1), resource.retain_task("successor")).await
        .expect("the exact closing carrier must be observable without waiting on its native close"), Err(WorkspaceError::WorkspaceClosed));
    assert!(service.get_for_agent_session("alice", "slow-close").await.unwrap().is_some(),
        "request/caller drop cannot manufacture a native close receipt");
    let observed = tokio::time::timeout(std::time::Duration::from_secs(1), service.observe_workspace("alice", "slow-close"))
        .await.expect("activity observation must not wait on a native close").unwrap();
    assert!(observed.closing && observed.cleanup_unconfirmed);
    assert_eq!(observed.retention, BrowserRetentionReason::NativeCleanup);
    assert_eq!(observed.close_confirmed, None, "an in-flight close is not positive native destruction evidence");
    assert_eq!(observed.operation_in_flight, Some(true));
    let other = agent_page(&service, "independent", "live", BrowserProfile::Ephemeral).await;
    assert_eq!(other.snapshot().await.unwrap().runtime.unwrap().tabs.len(), 1);
    assert_eq!(factory.closes.load(Ordering::SeqCst), 1);
    gate.release.notify_one();
    retired(&service, "slow-close").await;
    assert!(resource.snapshot().await.unwrap().runtime.is_none());
    assert_eq!(factory.closes.load(Ordering::SeqCst), 1, "the original retained close must finish exactly once");
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn resource_observation_is_read_only_and_absence_is_not_a_close_receipt() {
    let factory = Arc::new(WatchedFactory::default());
    let service = BrowserResourceService::new(factory.clone());
    let absent = service.observe_workspace("alice", "history-only").await.unwrap();
    assert!(!absent.registered);
    assert_eq!(absent.native_registered, None);
    assert_eq!(absent.close_confirmed, None);
    assert_eq!(factory.creates.load(Ordering::SeqCst), 0);
    assert!(service.get_for_agent_session("alice", "history-only").await.unwrap().is_none());
}

#[tokio::test]
async fn repeated_completed_carriers_retire_while_a_legitimate_long_task_and_history_handles_remain() {
    let factory = Arc::new(WatchedFactory::default());
    let service = BrowserResourceService::new(factory.clone());
    let long = agent_page(&service, "long", "still-working", BrowserProfile::Ephemeral).await;
    let mut history = Vec::new();
    for index in 0..4 {
        let session = format!("completed-{index}");
        let resource = agent_page(&service, &session, "completed-task", BrowserProfile::Ephemeral).await;
        resource.workspace().try_retire_after_task("completed-task").await.unwrap();
        retired(&service, &session).await;
        history.push(resource);
        assert_eq!(factory.closes.load(Ordering::SeqCst), index + 1);
        assert!(long.snapshot().await.unwrap().runtime.is_some());
    }
    for old in history { assert!(old.snapshot().await.unwrap().runtime.is_none()); }
    assert_eq!(factory.creates.load(Ordering::SeqCst), 5);
    service.shutdown().await.unwrap();
    assert_eq!(factory.closes.load(Ordering::SeqCst), 5);
}
