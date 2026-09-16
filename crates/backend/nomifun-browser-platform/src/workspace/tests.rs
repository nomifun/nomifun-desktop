use super::*;
use crate::run_guard::BrowserInputState;
use crate::runtime::*;
use std::sync::atomic::AtomicUsize;

#[derive(Default)]
struct Factory {
    creates: AtomicUsize,
    initial_locked: AtomicBool,
    fail_close_once: Arc<AtomicBool>,
    clear: Option<Arc<DelayedClear>>,
}

#[derive(Default)]
struct DelayedClear {
    started: tokio::sync::Notify,
    release: tokio::sync::Notify,
    calls: AtomicUsize,
    completed: AtomicBool,
    closed_before_completion: AtomicBool,
}

struct Runtime {
    snapshot: Mutex<BrowserRuntimeSnapshot>,
    locked: AtomicBool,
    fail_close_once: Arc<AtomicBool>,
    clear: Option<Arc<DelayedClear>>,
}

#[async_trait]
impl BrowserRuntimeFactory for Factory {
    async fn create(
        &self,
        request: CreateBrowserRuntime,
    ) -> Result<Arc<dyn BrowserRuntime>, WorkspaceError> {
        self.creates.fetch_add(1, Ordering::SeqCst);
        self.initial_locked
            .store(!request.user_input_enabled, Ordering::SeqCst);
        Ok(Arc::new(Runtime {
            snapshot: Mutex::new(BrowserRuntimeSnapshot {
                downloads: vec![],
                runtime_generation: request.runtime_generation,
                revision: 1,
                active_tab_id: None,
                tabs: vec![],
            }),
            locked: AtomicBool::new(!request.user_input_enabled),
            fail_close_once: self.fail_close_once.clone(),
            clear: self.clear.clone(),
        }))
    }
}

#[async_trait]
impl NativeInputGate for Runtime {
    async fn lock_user_input(&self) -> Result<(), RunAdmissionError> {
        self.locked.store(true, Ordering::SeqCst);
        Ok(())
    }
    async fn release_pressed_input(&self) -> Result<(), RunAdmissionError> {
        Ok(())
    }
    async fn unlock_user_input(&self) -> Result<(), RunAdmissionError> {
        self.locked.store(false, Ordering::SeqCst);
        Ok(())
    }
}

#[async_trait]
impl BrowserRuntime for Runtime {
    fn surface(&self) -> Option<&dyn BrowserNativeSurfacePort> {
        None
    }
    async fn snapshot(&self) -> Result<BrowserRuntimeSnapshot, WorkspaceError> {
        Ok(self.snapshot.lock().await.clone())
    }
    async fn execute(
        &self,
        command: BrowserTabCommand,
        _: CancellationToken,
    ) -> Result<BrowserRuntimeSnapshot, WorkspaceError> {
        if matches!(command, BrowserTabCommand::ClearSiteData { .. }) {
            if let Some(clear) = &self.clear {
                clear.calls.fetch_add(1, Ordering::SeqCst);
                clear.started.notify_one();
                clear.release.notified().await;
                clear.completed.store(true, Ordering::SeqCst);
            }
        }
        let mut snapshot = self.snapshot.lock().await;
        if matches!(command, BrowserTabCommand::ClearSiteData { .. }) {
            snapshot.tabs.clear(); snapshot.active_tab_id=None;
        }
        if let BrowserTabCommand::CloseAll { runtime_generation } = &command {
            if *runtime_generation!=snapshot.runtime_generation { return Err(WorkspaceError::StaleTarget); }
            if self.fail_close_once.swap(false,Ordering::SeqCst) {
                snapshot.tabs.pop();
                snapshot.active_tab_id=snapshot.tabs.first().map(|tab|tab.target.tab_id.clone());
                snapshot.revision+=1;
                return Err(WorkspaceError::NativeCommandFailed);
            }
            snapshot.tabs.clear();snapshot.active_tab_id=None;
        }
        if let BrowserTabCommand::Create { url } = command {
            let id = format!("tab-{}", snapshot.tabs.len());
            let runtime_generation = snapshot.runtime_generation;
            snapshot.tabs.push(BrowserTabSnapshot {
                target: BrowserTabTarget {
                    tab_id: id.clone(),
                    runtime_generation,
                    document_generation: 1,
                },
                title: "fixture".into(),
                url,
                lifecycle: BrowserTabLifecycle::Ready,
                can_go_back: false,
                can_go_forward: false,
                blocked_permissions: vec![],
                permission_requests: vec![],
                script_dialog: None,
                diagnostics: Default::default(),
            });
            snapshot.active_tab_id = Some(id);
        }
        snapshot.revision += 1;
        Ok(snapshot.clone())
    }
    async fn close(&self) -> Result<(), WorkspaceError> {
        if let Some(clear) = &self.clear {
            if clear.calls.load(Ordering::SeqCst)>0 && !clear.completed.load(Ordering::SeqCst) {
                clear.closed_before_completion.store(true, Ordering::SeqCst);
            }
        }
        if self.fail_close_once.swap(false, Ordering::SeqCst) {
            return Err(WorkspaceError::NativeCommandFailed);
        }
        self.snapshot.lock().await.tabs.clear();
        Ok(())
    }
}

#[tokio::test]
async fn close_all_is_user_only_and_never_creates_or_replaces_a_runtime() {
    let factory=Arc::new(Factory::default());
    let service=BrowserWorkspaceService::new(factory.clone());
    let workspace=service.ensure(key("u","close-all"),"provider".into(),BrowserProfile::Ephemeral).await.unwrap();
    let generation=workspace.slot.request.runtime_generation;
    let command=|| BrowserTabCommand::CloseAll {runtime_generation:generation};
    assert_eq!(workspace.user_command(command()).await,Err(WorkspaceError::TabNotFound));
    assert_eq!(factory.creates.load(Ordering::SeqCst),0);
    let before=workspace.user_command(create()).await.unwrap();
    workspace.user_command(create()).await.unwrap();
    assert_eq!(workspace.user_command(BrowserTabCommand::CloseAll {runtime_generation:generation+1}).await,Err(WorkspaceError::StaleTarget));
    let run=workspace.begin_run().await.unwrap();
    assert_eq!(workspace.agent_command(&run,command()).await,Err(WorkspaceError::UnsupportedAction));
    assert_eq!(workspace.user_command(command()).await,Err(WorkspaceError::Admission(RunAdmissionError::UserInputLocked)));
    workspace.finish_run(&run).await.unwrap();
    let cleared=workspace.user_command(command()).await.unwrap();
    assert!(cleared.tabs.is_empty());assert!(cleared.active_tab_id.is_none());
    assert_eq!(cleared.runtime_generation,before.runtime_generation);
    assert!(workspace.user_command(command()).await.unwrap().tabs.is_empty());
    workspace.user_command(create()).await.unwrap();
    assert_eq!(factory.creates.load(Ordering::SeqCst),1);
}

#[tokio::test]
async fn partially_failed_close_all_keeps_the_same_runtime_for_explicit_retry() {
    let factory=Arc::new(Factory::default());
    let service=BrowserWorkspaceService::new(factory.clone());
    let workspace=service.ensure(key("u","close-all-retry"),"provider".into(),BrowserProfile::Ephemeral).await.unwrap();
    let original=workspace.user_command(create()).await.unwrap();
    workspace.user_command(create()).await.unwrap();
    let command=|| BrowserTabCommand::CloseAll {runtime_generation:original.runtime_generation};
    factory.fail_close_once.store(true,Ordering::SeqCst);
    assert_eq!(workspace.user_command(command()).await,Err(WorkspaceError::NativeCommandFailed));
    let retained=workspace.snapshot().await.unwrap().runtime.unwrap();
    assert_eq!(retained.tabs.len(),1);assert_eq!(retained.runtime_generation,original.runtime_generation);
    assert!(workspace.user_command(command()).await.unwrap().tabs.is_empty());
    assert_eq!(factory.creates.load(Ordering::SeqCst),1);
}

#[tokio::test]
async fn downloads_folder_is_user_only_and_bound_to_existing_runtime() {
    let factory = Arc::new(Factory::default());
    let service = BrowserWorkspaceService::new(factory.clone());
    let workspace = service.ensure(key("u", "downloads-folder"), "provider".into(), BrowserProfile::Ephemeral).await.unwrap();
    let generation = workspace.runtime_generation();
    let command = || BrowserTabCommand::OpenDownloads { runtime_generation: generation };
    assert_eq!(workspace.user_command(command()).await, Err(WorkspaceError::TabNotFound));
    assert_eq!(factory.creates.load(Ordering::SeqCst), 0);
    let before = workspace.user_command(create()).await.unwrap();
    assert_eq!(workspace.user_command(BrowserTabCommand::OpenDownloads { runtime_generation: generation + 1 }).await, Err(WorkspaceError::StaleTarget));
    let run = workspace.begin_run().await.unwrap();
    assert_eq!(workspace.user_command(command()).await, Err(WorkspaceError::Admission(RunAdmissionError::UserInputLocked)));
    assert_eq!(workspace.agent_command(&run, command()).await, Err(WorkspaceError::UnsupportedAction));
    workspace.finish_run(&run).await.unwrap();
    let after = workspace.user_command(command()).await.unwrap();
    assert_eq!(after.tabs, before.tabs);
    workspace.user_command(BrowserTabCommand::CloseAll { runtime_generation: generation }).await.unwrap();
    assert!(workspace.user_command(command()).await.unwrap().tabs.is_empty());
    assert_eq!(factory.creates.load(Ordering::SeqCst), 1);
    service.shutdown().await.unwrap();
}

#[test]
fn downloads_folder_command_never_accepts_a_path_or_url() {
    for extra in ["path", "url", "directory"] {
        let mut value = serde_json::json!({"command":"open_downloads","runtime_generation":1});
        value[extra] = serde_json::json!("C:/Windows/notepad.exe");
        assert!(serde_json::from_value::<BrowserTabCommand>(value).is_err());
    }
}

#[tokio::test]
async fn site_data_clear_requires_idle_user_and_existing_exact_runtime() {
    let factory=Arc::new(Factory::default());
    let service=BrowserWorkspaceService::new(factory.clone());
    let workspace=service.ensure(key("u","site-data"),"provider".into(),BrowserProfile::Ephemeral).await.unwrap();
    let generation=workspace.runtime_generation();
    let command=||BrowserTabCommand::ClearSiteData{runtime_generation:generation};
    assert_eq!(workspace.user_command(command()).await,Err(WorkspaceError::TabNotFound));
    assert_eq!(factory.creates.load(Ordering::SeqCst),0);
    workspace.user_command(create()).await.unwrap();
    assert_eq!(workspace.user_command(BrowserTabCommand::ClearSiteData{runtime_generation:generation+1}).await,Err(WorkspaceError::StaleTarget));
    let run=workspace.begin_run().await.unwrap();
    assert_eq!(workspace.agent_command(&run,command()).await,Err(WorkspaceError::UnsupportedAction));
    assert_eq!(workspace.user_command(command()).await,Err(WorkspaceError::Admission(RunAdmissionError::UserInputLocked)));
    workspace.finish_run(&run).await.unwrap();
    workspace.user_command(command()).await.unwrap();
    assert_eq!(factory.creates.load(Ordering::SeqCst),1);
    service.shutdown().await.unwrap();
}

// This exercises the real Workspace/coordinator with a controlled asynchronous
// Runtime; it is scheduler ownership evidence, not a WebView2 callback mock.
async fn dropped_clear_request_still_owns_work(close_after: bool) {
    let clear=Arc::new(DelayedClear::default());
    let factory=Arc::new(Factory {clear:Some(clear.clone()),..Default::default()});
    let service=BrowserWorkspaceService::new(factory.clone());
    let workspace=service.ensure(key("u","detached-clear"),"provider".into(),BrowserProfile::Ephemeral).await.unwrap();
    workspace.user_command(create()).await.unwrap();
    let owned=workspace.clone();
    let caller=tokio::spawn(async move {owned.user_command(BrowserTabCommand::ClearSiteData {runtime_generation:owned.runtime_generation()}).await});
    clear.started.notified().await;
    caller.abort();assert!(caller.await.unwrap_err().is_cancelled());
    let owned=workspace.clone();
    let mut next=tokio::spawn(async move {
        if close_after {owned.close().await}
        else {let run=owned.begin_run().await?;owned.finish_run(&run).await.map_err(WorkspaceError::from)}
    });
    assert!(tokio::time::timeout(std::time::Duration::from_millis(50),&mut next).await.is_err(),"the next lifecycle operation must wait for native settlement");
    assert!(!clear.completed.load(Ordering::SeqCst));
    assert!(!clear.closed_before_completion.load(Ordering::SeqCst));
    clear.release.notify_one();
    next.await.unwrap().unwrap();
    assert!(clear.completed.load(Ordering::SeqCst));
    assert_eq!(clear.calls.load(Ordering::SeqCst),1,"losing an HTTP waiter must not replay the clear");
    assert!(!clear.closed_before_completion.load(Ordering::SeqCst));
    assert_eq!(factory.creates.load(Ordering::SeqCst),1);
    service.shutdown().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn site_data_clear_survives_dropped_request_before_agent_start() {
    dropped_clear_request_still_owns_work(false).await;
}

#[tokio::test(start_paused = true)]
async fn site_data_clear_survives_dropped_request_before_workspace_close() {
    dropped_clear_request_still_owns_work(true).await;
}

#[test]
fn site_data_clear_cannot_select_another_profile_or_directory() {
    for extra in ["path","profile","directory","user_id","conversation_id","url"] {
        let mut payload=serde_json::json!({"command":"clear_site_data","runtime_generation":1});
        payload[extra]=serde_json::json!("another-owner");
        assert!(serde_json::from_value::<BrowserTabCommand>(payload).is_err());
    }
}

#[tokio::test]
async fn failed_native_close_retains_authority_and_blocks_replacement_until_retry() {
    let factory = Arc::new(Factory::default());
    let service = BrowserWorkspaceService::new(factory.clone());
    let workspace = service
        .ensure(key("u", "c"), "a".into(), BrowserProfile::Ephemeral)
        .await
        .unwrap();
    workspace.user_command(create()).await.unwrap();
    factory.fail_close_once.store(true, Ordering::SeqCst);
    assert_eq!(
        service.close(&key("u", "c")).await,
        Err(WorkspaceError::NativeCommandFailed)
    );
    assert!(matches!(
        service
            .ensure(key("u", "c"), "a".into(), BrowserProfile::Ephemeral)
            .await,
        Err(WorkspaceError::WorkspaceClosed)
    ));
    assert_eq!(
        workspace.user_command(create()).await,
        Err(WorkspaceError::WorkspaceClosed)
    );
    service.close(&key("u", "c")).await.unwrap();
    let next = service
        .ensure(key("u", "c"), "a".into(), BrowserProfile::Ephemeral)
        .await
        .unwrap();
    assert!(!Arc::ptr_eq(&next, &workspace));
    service.shutdown().await.unwrap();
}

fn key(user: &str, conversation: &str) -> BrowserWorkspaceKey {
    BrowserWorkspaceKey {
        user_id: user.into(),
        conversation_id: conversation.into(),
    }
}
fn create() -> BrowserTabCommand {
    BrowserTabCommand::Create {
        url: "http://localhost:3000".into(),
    }
}

#[tokio::test]
async fn website_dialog_user_command_cannot_bypass_agent_run_authority() {
    let service = BrowserWorkspaceService::new(Arc::new(Factory::default()));
    let workspace = service.ensure(key("dialog-user", "dialog-thread"), "a".into(), BrowserProfile::Ephemeral).await.unwrap();
    let snapshot = workspace.user_command(create()).await.unwrap();
    let command = BrowserTabCommand::Dialog { target: snapshot.tabs[0].target.clone(), request_id: "dialog".into(), accept: true, text: None };
    let run = workspace.begin_run().await.unwrap();
    assert_eq!(workspace.user_command(command.clone()).await, Err(WorkspaceError::Admission(RunAdmissionError::UserInputLocked)));
    assert_eq!(workspace.agent_command(&run, command).await, Err(WorkspaceError::UnsupportedAction));
    let cancel = BrowserTabCommand::CancelDownload { target: snapshot.tabs[0].target.clone(), download_id: "download".into() };
    assert_eq!(workspace.user_command(cancel.clone()).await, Err(WorkspaceError::Admission(RunAdmissionError::UserInputLocked)));
    assert_eq!(workspace.agent_command(&run, cancel).await, Err(WorkspaceError::UnsupportedAction));
    let external = BrowserTabCommand::OpenExternal { target: snapshot.tabs[0].target.clone() };
    assert_eq!(workspace.user_command(external.clone()).await, Err(WorkspaceError::Admission(RunAdmissionError::UserInputLocked)));
    assert_eq!(workspace.agent_command(&run, external).await, Err(WorkspaceError::UnsupportedAction));
    workspace.finish_run(&run).await.unwrap();
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn user_workspace_binds_once_and_user_reopen_cannot_clear_exact_provider() {
    let factory = Arc::new(Factory::default());
    let service = BrowserWorkspaceService::new(factory.clone());
    let user = service
        .ensure_user(key("u", "c"), BrowserProfile::Ephemeral)
        .await
        .unwrap();
    let page = user.user_command(create()).await.unwrap();
    let agent = service
        .ensure(key("u", "c"), "exact-a".into(), BrowserProfile::Ephemeral)
        .await
        .unwrap();
    assert!(Arc::ptr_eq(&user, &agent));
    let reopened = service
        .ensure_user(key("u", "c"), BrowserProfile::Ephemeral)
        .await
        .unwrap();
    assert!(Arc::ptr_eq(&user, &reopened));
    assert!(matches!(
        service
            .ensure(key("u", "c"), "exact-b".into(), BrowserProfile::Ephemeral)
            .await,
        Err(WorkspaceError::ProviderChanged)
    ));
    assert_eq!(reopened.snapshot().await.unwrap().runtime, Some(page));
    assert_eq!(factory.creates.load(Ordering::SeqCst), 1);
    service.close(&key("u", "c")).await.unwrap();
    let replacement = service
        .ensure(key("u", "c"), "exact-b".into(), BrowserProfile::Ephemeral)
        .await
        .unwrap();
    assert!(!Arc::ptr_eq(&user, &replacement));
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn first_tab_created_during_run_is_locked_and_survives_following_turns() {
    let factory = Arc::new(Factory::default());
    let service = BrowserWorkspaceService::new(factory.clone());
    let workspace = service
        .ensure(
            key("u", "c"),
            "provider-a".into(),
            BrowserProfile::Ephemeral,
        )
        .await
        .unwrap();
    assert_eq!(factory.creates.load(Ordering::SeqCst), 0);
    workspace
        .set_surface(
            BrowserSurfaceBounds {
                x: 0.0,
                y: 0.0,
                width: 640.0,
                height: 480.0,
            },
            false,
            Default::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        factory.creates.load(Ordering::SeqCst),
        0,
        "unmounting an unopened pane must not create a browser"
    );
    let first_run = workspace.begin_run().await.unwrap();
    let tabs = workspace.agent_command(&first_run, create()).await.unwrap();
    assert!(factory.initial_locked.load(Ordering::SeqCst));
    assert_eq!(
        workspace.user_command(create()).await,
        Err(WorkspaceError::Admission(
            RunAdmissionError::UserInputLocked
        ))
    );
    workspace.finish_run(&first_run).await.unwrap();
    assert_eq!(
        workspace.snapshot().await.unwrap().runtime.as_ref(),
        Some(&tabs)
    );
    let next_run = workspace.begin_run().await.unwrap();
    assert_eq!(
        workspace.agent_command(&first_run, create()).await,
        Err(WorkspaceError::Admission(RunAdmissionError::StaleRun))
    );
    workspace.finish_run(&next_run).await.unwrap();
    assert_eq!(factory.creates.load(Ordering::SeqCst), 1);
    assert_eq!(
        workspace.snapshot().await.unwrap().run.input_state,
        BrowserInputState::UserReady
    );
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn provider_changes_and_other_users_cannot_reuse_workspace_authority() {
    let service = BrowserWorkspaceService::new(Arc::new(Factory::default()));
    let one = service
        .ensure(key("u", "c"), "a".into(), BrowserProfile::Ephemeral)
        .await
        .unwrap();
    let same = service
        .ensure(key("u", "c"), "a".into(), BrowserProfile::Ephemeral)
        .await
        .unwrap();
    assert!(Arc::ptr_eq(&one, &same));
    assert!(matches!(
        service
            .ensure(key("u", "c"), "b".into(), BrowserProfile::Ephemeral)
            .await,
        Err(WorkspaceError::ProviderChanged)
    ));
    let other = service
        .ensure(
            key("other-user", "c"),
            "a".into(),
            BrowserProfile::Ephemeral,
        )
        .await
        .unwrap();
    let run = one.begin_run().await.unwrap();
    assert_eq!(
        other.agent_command(&run, create()).await,
        Err(WorkspaceError::Admission(RunAdmissionError::StaleRun))
    );
    one.finish_run(&run).await.unwrap();
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn explicit_close_revokes_old_handles_and_recreate_uses_new_generation() {
    let service = BrowserWorkspaceService::new(Arc::new(Factory::default()));
    let workspace = service
        .ensure(key("u", "c"), "a".into(), BrowserProfile::Ephemeral)
        .await
        .unwrap();
    let old = workspace.user_command(create()).await.unwrap();
    let run = workspace.begin_run().await.unwrap();
    service.close(&key("u", "c")).await.unwrap();
    assert_eq!(
        workspace.agent_command(&run, create()).await,
        Err(WorkspaceError::WorkspaceClosed)
    );
    assert_eq!(
        workspace.user_command(create()).await,
        Err(WorkspaceError::WorkspaceClosed)
    );
    let next = service
        .ensure(key("u", "c"), "a".into(), BrowserProfile::Ephemeral)
        .await
        .unwrap();
    assert!(
        next.user_command(create())
            .await
            .unwrap()
            .runtime_generation
            > old.runtime_generation
    );
    service.shutdown().await.unwrap();
    assert!(matches!(
        service
            .ensure(key("u", "c"), "a".into(), BrowserProfile::Ephemeral)
            .await,
        Err(WorkspaceError::WorkspaceClosed)
    ));
}

#[test]
fn native_bounds_reject_invalid_geometry() {
    let bounds = BrowserSurfaceBounds {
        x: 380.0,
        y: 60.0,
        width: 640.0,
        height: 600.0,
    };
    assert!(bounds.is_valid());
    assert!(
        !BrowserSurfaceBounds {
            width: f64::NAN,
            ..bounds
        }
        .is_valid()
    );
    assert!(!BrowserSurfaceBounds { x: -1.0, ..bounds }.is_valid());
    assert!(
        !BrowserSurfaceBounds {
            height: 0.0,
            ..bounds
        }
        .is_valid()
    );
    assert!(
        !BrowserSurfaceBounds {
            x: 32_000.0,
            width: 1024.0,
            ..bounds
        }
        .is_valid()
    );
}

#[tokio::test]
async fn human_close_rejects_active_and_settling_runs_without_cancelling_them() {
    let service=Arc::new(BrowserWorkspaceService::new(Arc::new(Factory::default())));
    let workspace=service.ensure(key("u","idle-close"),"old".into(),BrowserProfile::Ephemeral).await.unwrap();
    workspace.user_command(create()).await.unwrap();
    let run=workspace.begin_run().await.unwrap();
    assert_eq!(service.close_idle(key("u","idle-close"),1).await,Err(WorkspaceError::Admission(RunAdmissionError::UserInputLocked)));
    assert!(workspace.agent_command(&run,create()).await.is_ok(),"rejected user close must not cancel Agent authority");
    workspace.settle_run(&run).await.unwrap();
    assert_eq!(service.close_idle(key("u","idle-close"),1).await,Err(WorkspaceError::Admission(RunAdmissionError::UserInputLocked)));
    workspace.finish_run(&run).await.unwrap();
    service.close_idle(key("u","idle-close"),1).await.unwrap();
    assert!(workspace.native_close_proven().await);
    assert!(service.get(&key("u","idle-close")).await.is_none());
}

#[tokio::test]
async fn failed_human_close_retains_authority_until_retry_then_allows_new_provider() {
    let factory=Arc::new(Factory::default());
    let service=Arc::new(BrowserWorkspaceService::new(factory.clone()));
    let key=key("u","close-retry");
    let old=service.ensure(key.clone(),"old".into(),BrowserProfile::Ephemeral).await.unwrap();
    let generation=old.user_command(create()).await.unwrap().runtime_generation;
    factory.fail_close_once.store(true,Ordering::SeqCst);
    assert_eq!(service.close_idle(key.clone(),generation).await,Err(WorkspaceError::NativeCommandFailed));
    assert!(!old.native_close_proven().await);
    assert!(Arc::ptr_eq(&old,&service.get(&key).await.unwrap()));
    assert!(matches!(service.ensure(key.clone(),"new".into(),BrowserProfile::Ephemeral).await,Err(WorkspaceError::WorkspaceClosed)));
    service.close_idle(key.clone(),generation).await.unwrap();
    assert!(old.native_close_proven().await);
    let new=service.ensure(key,"new".into(),BrowserProfile::Ephemeral).await.unwrap();
    assert!(new.user_command(create()).await.unwrap().runtime_generation>generation);
    assert!(matches!(old.begin_run().await,Err(WorkspaceError::WorkspaceClosed)));
    service.shutdown().await.unwrap();
}
