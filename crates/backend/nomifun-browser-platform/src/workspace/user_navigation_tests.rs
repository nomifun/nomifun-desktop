//! The Stop HTTP caller interrupts the exact admitted human navigation without
//! waiting behind it, while the original operation still owns native cleanup.
use super::*;
use crate::runtime::*;
use std::sync::atomic::AtomicUsize;
use tokio::sync::Notify;

struct BlockingRuntime {
    state: Mutex<BrowserRuntimeSnapshot>,
    entered: Notify,
    cancellation_seen: Notify,
    release_cleanup: Notify,
    native_stops: AtomicUsize,
    native_pauses: AtomicUsize,
}

struct BlockingFactory(Arc<BlockingRuntime>);

impl BlockingRuntime {
    async fn await_navigation_cleanup(&self, cancel: CancellationToken, tab_id: &str) -> Result<BrowserRuntimeSnapshot, WorkspaceError> {
        self.entered.notify_one();
        cancel.cancelled().await;
        self.cancellation_seen.notify_one();
        self.release_cleanup.notified().await;
        let mut state = self.state.lock().await;
        state.tabs.iter_mut().find(|tab| tab.target.tab_id == tab_id).unwrap().lifecycle = BrowserTabLifecycle::Stopped;
        state.revision += 1;
        Err(RunAdmissionError::Cancelled.into())
    }
}

#[async_trait]
impl BrowserRuntimeFactory for BlockingFactory {
    async fn create(&self, _: CreateBrowserRuntime) -> Result<Arc<dyn BrowserRuntime>, WorkspaceError> {
        Ok(self.0.clone())
    }
}

#[async_trait]
impl NativeInputGate for BlockingRuntime {
    async fn lock_user_input(&self) -> Result<(), RunAdmissionError> { Ok(()) }
    async fn release_pressed_input(&self) -> Result<(), RunAdmissionError> { Ok(()) }
    async fn unlock_user_input(&self) -> Result<(), RunAdmissionError> { Ok(()) }
}

#[async_trait]
impl BrowserRuntime for BlockingRuntime {
    fn surface(&self) -> Option<&dyn BrowserNativeSurfacePort> { None }
    async fn snapshot(&self) -> Result<BrowserRuntimeSnapshot, WorkspaceError> { Ok(self.state.lock().await.clone()) }
    async fn close(&self) -> Result<(), WorkspaceError> { Ok(()) }
    async fn execute(&self, command: BrowserTabCommand, cancel: CancellationToken) -> Result<BrowserRuntimeSnapshot, WorkspaceError> {
        match command {
            BrowserTabCommand::Navigate { target, .. } | BrowserTabCommand::Reload { target, .. } => {
                {
                    let mut state = self.state.lock().await;
                    let tab = state.tabs.iter_mut().find(|tab| tab.target == target).ok_or(WorkspaceError::StaleTarget)?;
                    // A newer provisional attempt may retain the same document.
                    tab.load.as_mut().unwrap().navigation_sequence += 1;
                    tab.lifecycle = BrowserTabLifecycle::Loading;
                    state.revision += 1;
                }
                self.await_navigation_cleanup(cancel, &target.tab_id).await
            }
            BrowserTabCommand::Create { .. } => {
                {
                    let mut state = self.state.lock().await;
                    let mut created = state.tabs[0].clone();
                    created.target.tab_id = "created".into();
                    created.load.as_mut().unwrap().navigation_sequence = 1;
                    created.lifecycle = BrowserTabLifecycle::Loading;
                    state.tabs.push(created);
                    state.active_tab_id = Some("created".into());
                    state.revision += 1;
                }
                self.await_navigation_cleanup(cancel, "created").await
            }
            BrowserTabCommand::StopLoading { target, expected_navigation_sequence } => {
                if !self.state.lock().await.tabs.iter().any(|tab| tab.target == target
                    && tab.load.as_ref().is_some_and(|load| load.navigation_sequence == expected_navigation_sequence)) {
                    return Err(WorkspaceError::StaleTarget);
                }
                self.native_stops.fetch_add(1, Ordering::SeqCst);
                self.snapshot().await
            }
            BrowserTabCommand::PauseNavigation { target, expected_navigation_sequence } => {
                let mut state = self.state.lock().await;
                let tab = state.tabs.iter_mut().find(|tab| tab.target == target
                    && tab.load.as_ref().is_some_and(|load| load.navigation_sequence == expected_navigation_sequence)).ok_or(WorkspaceError::StaleTarget)?;
                tab.load.get_or_insert_with(Default::default).navigation_control = Some(crate::navigation::BrowserNavigationControl {
                    paused: true, ..Default::default()
                });
                self.native_pauses.fetch_add(1, Ordering::SeqCst);
                state.revision += 1;
                Ok(state.clone())
            }
            _ => self.snapshot().await,
        }
    }
}

async fn fixture() -> (Arc<BrowserWorkspace>, Arc<BlockingRuntime>, BrowserTabTarget) {
    let target = BrowserTabTarget { tab_id: "page".into(), runtime_generation: 1, document_generation: 1 };
    let runtime = Arc::new(BlockingRuntime {
        state: Mutex::new(BrowserRuntimeSnapshot { runtime_generation: 1, revision: 1, active_tab_id: Some("page".into()), downloads: vec![],
            tabs: vec![BrowserTabSnapshot { target: target.clone(), title: "fixture".into(), url: "https://fixture.test/".into(),
                lifecycle: BrowserTabLifecycle::Ready, load: Some(Default::default()), can_go_back: false, can_go_forward: false,
                zoom_percent: 100, blocked_permissions: vec![], permission_requests: vec![], script_dialog: None, diagnostics: Default::default() }] }),
        entered: Notify::new(), cancellation_seen: Notify::new(), release_cleanup: Notify::new(),
        native_stops: AtomicUsize::new(0), native_pauses: AtomicUsize::new(0),
    });
    let service = BrowserResourceService::new(Arc::new(BlockingFactory(runtime.clone())));
    let workspace = service.ensure_user("alice", "session", BrowserProfile::Ephemeral).await.unwrap();
    workspace.slot.ensure().await.unwrap();
    (workspace, runtime, target)
}

#[tokio::test]
async fn stop_interrupts_navigation_but_waits_for_native_settlement_and_does_not_replay_stop() {
    let (workspace, runtime, original) = fixture().await;
    let navigation = tokio::spawn({ let workspace = workspace.clone(); async move {
        workspace.user_command(BrowserTabCommand::Navigate { target: original, url: "https://fixture.test/slow".into() }).await
    }});
    runtime.entered.notified().await;
    let target = runtime.snapshot().await.unwrap().tabs[0].target.clone();
    let stop = tokio::spawn({ let workspace = workspace.clone(); async move { workspace.user_command(BrowserTabCommand::StopLoading { target, expected_navigation_sequence: 1 }).await }});
    tokio::time::timeout(std::time::Duration::from_secs(1), runtime.cancellation_seen.notified()).await.unwrap();
    assert!(!stop.is_finished(), "Stop returned before native settlement");
    navigation.abort();
    runtime.release_cleanup.notify_one();
    let stopped = tokio::time::timeout(std::time::Duration::from_secs(1), stop).await.unwrap().unwrap().unwrap();
    assert_eq!(stopped.tabs[0].lifecycle, BrowserTabLifecycle::Stopped);
    assert_eq!(runtime.native_stops.load(Ordering::SeqCst), 1, "The current user Stop must not be replayed after settlement");
    assert!(workspace.user_navigation.lock().unwrap().is_none());
}

#[tokio::test]
async fn stale_target_cannot_cancel_the_current_navigation() {
    let (workspace, runtime, original) = fixture().await;
    let navigation = tokio::spawn({ let (workspace, target) = (workspace.clone(), original.clone()); async move {
        workspace.user_command(BrowserTabCommand::Reload { target, expected_navigation_sequence: 0 }).await
    }});
    runtime.entered.notified().await;
    assert_eq!(workspace.user_command(BrowserTabCommand::StopLoading { target: original, expected_navigation_sequence: 0 }).await, Err(WorkspaceError::StaleTarget));
    let pending = workspace.user_navigation.lock().unwrap().clone().unwrap();
    assert!(!pending.cancel.is_cancelled());
    let current = runtime.snapshot().await.unwrap().tabs[0].target.clone();
    let stop = tokio::spawn({ let workspace = workspace.clone(); async move { workspace.user_command(BrowserTabCommand::StopLoading { target: current, expected_navigation_sequence: 1 }).await }});
    runtime.cancellation_seen.notified().await;
    runtime.release_cleanup.notify_one();
    stop.await.unwrap().unwrap();
    assert_eq!(navigation.await.unwrap(), Err(WorkspaceError::Admission(RunAdmissionError::Cancelled)));
}

#[tokio::test]
async fn human_stop_cannot_cancel_or_bypass_an_agent_run() {
    let (workspace, runtime, target) = fixture().await;
    let run = workspace.begin_run().await.unwrap();
    assert_eq!(workspace.user_command(BrowserTabCommand::StopLoading { target, expected_navigation_sequence: 0 }).await,
        Err(WorkspaceError::Admission(RunAdmissionError::UserInputLocked)));
    assert_eq!(runtime.native_stops.load(Ordering::SeqCst), 0);
    workspace.finish_run(&run).await.unwrap();
}

#[tokio::test]
async fn stop_during_create_cancels_only_the_newly_published_page() {
    let (workspace, runtime, _) = fixture().await;
    let create = tokio::spawn({ let workspace = workspace.clone(); async move {
        workspace.user_command(BrowserTabCommand::Create { url: "https://fixture.test/new".into() }).await
    }});
    runtime.entered.notified().await;
    let target = runtime.snapshot().await.unwrap().tabs[1].target.clone();
    let stop = tokio::spawn({ let workspace = workspace.clone(); async move { workspace.user_command(BrowserTabCommand::StopLoading { target, expected_navigation_sequence: 1 }).await }});
    tokio::time::timeout(std::time::Duration::from_secs(1), runtime.cancellation_seen.notified()).await.unwrap();
    runtime.release_cleanup.notify_one();
    let result = stop.await.unwrap().unwrap();
    assert_eq!(result.tabs[0].lifecycle, BrowserTabLifecycle::Ready);
    assert_eq!(result.tabs[1].lifecycle, BrowserTabLifecycle::Stopped);
    assert_eq!(create.await.unwrap(), Err(WorkspaceError::Admission(RunAdmissionError::Cancelled)));
    assert_eq!(runtime.native_stops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn pause_applies_hold_and_interrupts_navigation_but_waits_for_cleanup() {
    let (workspace, runtime, target) = fixture().await;
    let navigation = tokio::spawn({ let workspace = workspace.clone(); async move {
        workspace.user_command(BrowserTabCommand::Reload { target, expected_navigation_sequence: 0 }).await
    }});
    runtime.entered.notified().await;
    let target = runtime.snapshot().await.unwrap().tabs[0].target.clone();
    let pause = tokio::spawn({ let workspace = workspace.clone(); async move { workspace.user_command(BrowserTabCommand::PauseNavigation { target, expected_navigation_sequence: 1 }).await }});
    tokio::time::timeout(std::time::Duration::from_secs(1), runtime.cancellation_seen.notified()).await.unwrap();
    assert_eq!(runtime.native_pauses.load(Ordering::SeqCst), 1);
    assert!(!pause.is_finished(), "Pause returned before pending native cleanup settled");
    runtime.release_cleanup.notify_one();
    let result = pause.await.unwrap().unwrap();
    assert!(result.tabs[0].load.as_ref().unwrap().navigation_control.as_ref().unwrap().paused);
    assert_eq!(runtime.native_pauses.load(Ordering::SeqCst), 1);
    assert_eq!(navigation.await.unwrap(), Err(WorkspaceError::Admission(RunAdmissionError::Cancelled)));
}
