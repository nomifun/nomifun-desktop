use super::*;
use crate::product::{
    BrowserProviderDescriptor, BrowserProviderKind, BrowserResourceBinding,
    BrowserResourceOperation,
};
use crate::run_guard::BrowserInputState;
use crate::runtime::*;
use std::sync::atomic::AtomicUsize;

#[derive(Default)]
struct Factory {
    creates: AtomicUsize,
    shutdowns: AtomicUsize,
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

    async fn shutdown(&self) -> Result<(), WorkspaceError> {
        self.shutdowns.fetch_add(1, Ordering::SeqCst);
        Ok(())
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
        if let BrowserTabCommand::CloseAll { runtime_generation }
        | BrowserTabCommand::OpenDownloads { runtime_generation }
        | BrowserTabCommand::ClearSiteData { runtime_generation } = &command
        {
            if *runtime_generation != snapshot.runtime_generation {
                return Err(WorkspaceError::StaleTarget);
            }
        }
        if matches!(command, BrowserTabCommand::CloseAll { .. }) {
            if self.fail_close_once.swap(false, Ordering::SeqCst) {
                snapshot.tabs.pop();
                snapshot.active_tab_id = snapshot
                    .tabs
                    .first()
                    .map(|tab| tab.target.tab_id.clone());
                snapshot.revision += 1;
                return Err(WorkspaceError::NativeCommandFailed);
            }
            snapshot.tabs.clear();
            snapshot.active_tab_id = None;
        }
        if matches!(command, BrowserTabCommand::ClearSiteData { .. }) {
            snapshot.tabs.clear();
            snapshot.active_tab_id = None;
        }
        if let BrowserTabCommand::Activate { target } = &command {
            snapshot.active_tab_id = Some(target.tab_id.clone());
        }
        if let BrowserTabCommand::Close { target } = &command {
            let previous_count = snapshot.tabs.len();
            snapshot.tabs.retain(|tab| tab.target != *target);
            if snapshot.tabs.len() == previous_count { return Err(WorkspaceError::TabNotFound); }
            if snapshot.active_tab_id.as_ref() == Some(&target.tab_id) {
                snapshot.active_tab_id = snapshot.tabs.first().map(|tab| tab.target.tab_id.clone());
            }
        }
        if let BrowserTabCommand::SetZoom { target, percent } = &command {
            let tab = snapshot.tabs.iter_mut().find(|tab| tab.target == *target).ok_or(WorkspaceError::TabNotFound)?;
            tab.zoom_percent = *percent;
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
                zoom_percent: 100,
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
            if clear.calls.load(Ordering::SeqCst) > 0
                && !clear.completed.load(Ordering::SeqCst)
            {
                clear
                    .closed_before_completion
                    .store(true, Ordering::SeqCst);
            }
        }
        if self.fail_close_once.swap(false, Ordering::SeqCst) {
            return Err(WorkspaceError::NativeCommandFailed);
        }
        self.snapshot.lock().await.tabs.clear();
        Ok(())
    }
}

fn descriptor(id: &str) -> BrowserProviderDescriptor {
    BrowserProviderDescriptor::new(
        id,
        BrowserProviderKind::Managed,
        format!("{id}-immutable-lock"),
        BrowserCapabilityAction::all(),
    )
    .unwrap()
}

fn authority(
    principal: &str,
    session: &str,
    provider_id: &str,
    actions: impl IntoIterator<Item = BrowserCapabilityAction>,
) -> BrowserSessionAuthority {
    BrowserSessionAuthority::new(
        principal,
        session,
        actions,
        BrowserResourceBinding::new(
            format!("binding-{session}"),
            format!("resource-{session}"),
            principal,
            descriptor(provider_id),
            BrowserCapabilityAction::all().map(BrowserCapabilityAction::resource_operation),
        )
        .unwrap(),
    )
    .unwrap()
}

fn service(factory: Arc<Factory>, provider_ids: &[&str]) -> BrowserResourceService {
    assert!(!provider_ids.is_empty());
    BrowserResourceService::new(factory)
}

fn service_with_profiles(
    factory: Arc<Factory>,
    data_dir: &std::path::Path,
) -> (BrowserResourceService, BrowserProfileStore) {
    let store = BrowserProfileStore::new(data_dir.to_path_buf()).unwrap();
    (
        BrowserResourceService::new(factory).with_profile_store(store.clone()),
        store,
    )
}

fn profile_path(store: &BrowserProfileStore, key: &BrowserResourceKey) -> std::path::PathBuf {
    let BrowserProfile::Persistent(path) = store
        .profile_for(key, BrowserProfilePersistence::Persistent)
        .unwrap()
    else {
        panic!("persistent profile")
    };
    path
}

fn all_actions() -> [BrowserCapabilityAction; 7] {
    BrowserCapabilityAction::all()
}

fn create() -> BrowserTabCommand {
    BrowserTabCommand::Create {
        url: "http://localhost:3000".into(),
    }
}

#[tokio::test]
async fn navigate_grant_can_activate_and_close_human_tabs_without_granting_agent_close() {
    let factory = Arc::new(Factory::default());
    let service = service(factory, &["managed"]);
    let resource = service
        .ensure(
            authority("alice", "tab-switch", "managed", [BrowserCapabilityAction::Navigate]),
            BrowserProfile::Ephemeral,
        )
        .await
        .unwrap();
    let first = resource.user_command(create()).await.unwrap().tabs[0].target.clone();
    let second = resource.user_command(create()).await.unwrap().tabs[1].target.clone();
    let snapshot = resource
        .user_command(BrowserTabCommand::Activate { target: second.clone() })
        .await
        .unwrap();
    assert_eq!(snapshot.active_tab_id.as_deref(), Some(second.tab_id.as_str()));
    let closed = resource.user_command(BrowserTabCommand::Close { target: second }).await.unwrap();
    assert_eq!(closed.tabs.len(), 1);
    assert_eq!(closed.active_tab_id.as_deref(), Some(first.tab_id.as_str()));
    let run = resource.begin_run().await.unwrap();
    assert_eq!(
        resource.agent_command(&run, BrowserTabCommand::Close { target: first }).await,
        Err(WorkspaceError::ActionDenied)
    );
    resource.finish_run(&run).await.unwrap();
}

#[tokio::test]
async fn page_zoom_is_bounded_and_only_available_to_the_human_owner() {
    let factory = Arc::new(Factory::default());
    let service = service(factory, &["managed"]);
    let resource = service
        .ensure(
            authority("alice", "page-zoom", "managed", [BrowserCapabilityAction::Navigate]),
            BrowserProfile::Ephemeral,
        )
        .await
        .unwrap();
    let target = resource.user_command(create()).await.unwrap().tabs[0].target.clone();
    let command = |percent| BrowserTabCommand::SetZoom { target: target.clone(), percent };
    let zoomed = resource.user_command(command(125)).await.unwrap();
    assert_eq!(zoomed.tabs[0].zoom_percent, 125);
    assert_eq!(serde_json::to_value(&zoomed).unwrap()["tabs"][0]["zoom_percent"], 125);
    for percent in [49, 201] {
        assert_eq!(resource.user_command(command(percent)).await, Err(WorkspaceError::InvalidZoom));
    }
    assert_eq!(resource.snapshot().await.unwrap().runtime.unwrap().tabs[0].zoom_percent, 125);
    let run = resource.begin_run().await.unwrap();
    assert_eq!(resource.agent_command(&run, command(100)).await, Err(WorkspaceError::UnsupportedAction));
    resource.finish_run(&run).await.unwrap();
}

#[tokio::test]
async fn provider_or_resource_existence_never_grants_agent_actions() {
    let factory = Arc::new(Factory::default());
    let service = service(factory.clone(), &["managed"]);
    let resource = service
        .ensure(
            authority("alice", "delegated-session", "managed", []),
            BrowserProfile::Ephemeral,
        )
        .await
        .unwrap();
    let run = resource.begin_run().await.unwrap();
    assert_eq!(
        resource.agent_command(&run, create()).await,
        Err(WorkspaceError::ActionDenied)
    );
    assert_eq!(factory.creates.load(Ordering::SeqCst), 0);
    resource.finish_run(&run).await.unwrap();
}

#[tokio::test]
async fn managed_resource_owner_rejects_an_attached_provider_binding() {
    let factory = Arc::new(Factory::default());
    let service = service(factory, &["managed"]);
    let provider = BrowserProviderDescriptor::new(
        "attached",
        BrowserProviderKind::AttachedChrome,
        "attached-lock",
        BrowserCapabilityAction::all(),
    )
    .unwrap();
    let resource = BrowserResourceBinding::new(
        "binding",
        "installation-connection",
        "alice",
        provider,
        BrowserCapabilityAction::all().map(BrowserCapabilityAction::resource_operation),
    )
    .unwrap();
    let authority = BrowserSessionAuthority::new(
        "alice",
        "session",
        [BrowserCapabilityAction::Observe],
        resource,
    )
    .unwrap();
    assert!(matches!(
        service.ensure(authority, BrowserProfile::Ephemeral).await,
        Err(WorkspaceError::NativeUnavailable)
    ));
}

#[tokio::test]
async fn delegated_agent_session_uses_the_same_authorized_resource_path() {
    let factory = Arc::new(Factory::default());
    let service = service(factory.clone(), &["managed"]);
    let granted = authority(
        "alice",
        "delegated-session",
        "managed",
        [BrowserCapabilityAction::Navigate],
    );
    let resource = service
        .ensure(granted, BrowserProfile::Ephemeral)
        .await
        .unwrap();
    let run = resource.begin_run().await.unwrap();
    let snapshot = resource.agent_command(&run, create()).await.unwrap();
    assert_eq!(snapshot.tabs.len(), 1);
    assert!(factory.initial_locked.load(Ordering::SeqCst));
    assert_eq!(
        resource.user_command(create()).await,
        Err(WorkspaceError::Admission(RunAdmissionError::UserInputLocked))
    );
    resource.finish_run(&run).await.unwrap();
    assert_eq!(
        resource.snapshot().await.unwrap().run.input_state,
        BrowserInputState::UserReady
    );
}

#[tokio::test]
async fn two_agent_sessions_have_distinct_resources_runtimes_and_profiles() {
    let factory = Arc::new(Factory::default());
    let service = service(factory.clone(), &["managed"]);
    let first_authority = authority("alice", "session-a", "managed", all_actions());
    let second_authority = authority("alice", "session-b", "managed", all_actions());
    let first_profile = BrowserProfile::for_agent_session(
        std::path::Path::new("owned"),
        &first_authority.key(),
        false,
    );
    let second_profile = BrowserProfile::for_agent_session(
        std::path::Path::new("owned"),
        &second_authority.key(),
        false,
    );
    assert_ne!(first_profile, second_profile);
    let first = service
        .ensure(first_authority, first_profile)
        .await
        .unwrap();
    let second = service
        .ensure(second_authority, second_profile)
        .await
        .unwrap();
    assert!(!Arc::ptr_eq(&first, &second));
    let first_snapshot = first.user_command(create()).await.unwrap();
    let second_snapshot = second.user_command(create()).await.unwrap();
    assert_ne!(
        first_snapshot.runtime_generation,
        second_snapshot.runtime_generation
    );
    assert_eq!(factory.creates.load(Ordering::SeqCst), 2);

    service
        .close_agent_session("alice", "session-a")
        .await
        .unwrap();
    assert!(service
        .get(&authority("alice", "session-a", "managed", all_actions()))
        .await
        .unwrap()
        .is_none());
    assert!(service
        .get(&authority("alice", "session-b", "managed", all_actions()))
        .await
        .unwrap()
        .is_some());
    assert_eq!(
        service
            .ensure(
                authority("alice", "session-a", "managed", all_actions()),
                BrowserProfile::Ephemeral,
            )
            .await
            .err(),
        Some(WorkspaceError::WorkspaceClosed)
    );
}

#[tokio::test]
async fn restart_delete_removes_only_exact_persistent_frozen_profiles() {
    let data_dir = tempfile::tempdir().unwrap();
    let (service, store) = service_with_profiles(Arc::new(Factory::default()), data_dir.path());
    let exact = BrowserResourceKey {
        principal_id: "alice".into(),
        agent_session_id: "restart-session".into(),
        resource_binding_id: "persistent-binding".into(),
    };
    let foreign = BrowserResourceKey {
        principal_id: "bob".into(),
        ..exact.clone()
    };
    let other_session = BrowserResourceKey {
        agent_session_id: "other-session".into(),
        ..exact.clone()
    };
    let other_binding = BrowserResourceKey {
        resource_binding_id: "other-binding".into(),
        ..exact.clone()
    };
    let ephemeral = BrowserResourceKey {
        resource_binding_id: "ephemeral-binding".into(),
        ..exact.clone()
    };
    for key in [&exact, &foreign, &other_session, &other_binding, &ephemeral] {
        let path = profile_path(&store, key);
        std::fs::create_dir_all(path.join("nested")).unwrap();
        std::fs::write(path.join("nested/state"), key.resource_binding_id.as_bytes()).unwrap();
    }

    // The process-local map is intentionally empty: deletion must derive the
    // profile identities from the frozen binding set after restart.
    service
        .delete_agent_session(
            "alice",
            "restart-session",
            &[
                BrowserProfileBinding::persistent("persistent-binding").unwrap(),
                BrowserProfileBinding::ephemeral("ephemeral-binding").unwrap(),
            ],
        )
        .await
        .unwrap();

    assert!(!profile_path(&store, &exact).exists());
    assert!(profile_path(&store, &foreign).exists());
    assert!(profile_path(&store, &other_session).exists());
    assert!(profile_path(&store, &other_binding).exists());
    assert!(
        profile_path(&store, &ephemeral).exists(),
        "ephemeral policy must never authorize persistent-directory deletion"
    );
}

#[tokio::test]
async fn native_close_failure_blocks_profile_delete_until_exact_retry() {
    let data_dir = tempfile::tempdir().unwrap();
    let factory = Arc::new(Factory::default());
    let (service, store) = service_with_profiles(factory.clone(), data_dir.path());
    let bound = authority("alice", "delete-retry", "managed", all_actions());
    let key = bound.key();
    let binding_id = key.resource_binding_id.clone();
    let profile = store
        .profile_for(&key, BrowserProfilePersistence::Persistent)
        .unwrap();
    let profile_path = profile_path(&store, &key);
    std::fs::create_dir_all(&profile_path).unwrap();
    std::fs::write(profile_path.join("state"), b"persistent").unwrap();
    let resource = service.ensure(bound, profile).await.unwrap();
    resource.user_command(create()).await.unwrap();

    factory.fail_close_once.store(true, Ordering::SeqCst);
    let bindings = [BrowserProfileBinding::persistent(binding_id).unwrap()];
    assert_eq!(
        service
            .delete_agent_session("alice", "delete-retry", &bindings)
            .await,
        Err(WorkspaceError::NativeCommandFailed)
    );
    assert!(profile_path.exists());
    service
        .delete_agent_session("alice", "delete-retry", &bindings)
        .await
        .unwrap();
    assert!(!profile_path.exists());
}

#[cfg(unix)]
#[tokio::test]
async fn profile_symlink_fails_closed_and_retry_deletes_only_replacement_directory() {
    use std::os::unix::fs::symlink;

    let data_dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let (service, store) = service_with_profiles(Arc::new(Factory::default()), data_dir.path());
    let key = BrowserResourceKey {
        principal_id: "alice".into(),
        agent_session_id: "symlink-session".into(),
        resource_binding_id: "symlink-binding".into(),
    };
    let profile = profile_path(&store, &key);
    std::fs::create_dir_all(profile.parent().unwrap()).unwrap();
    std::fs::write(outside.path().join("sentinel"), b"outside").unwrap();
    symlink(outside.path(), &profile).unwrap();
    let bindings = [BrowserProfileBinding::persistent("symlink-binding").unwrap()];

    assert_eq!(
        service
            .delete_agent_session("alice", "symlink-session", &bindings)
            .await,
        Err(WorkspaceError::ProfileCleanupFailed)
    );
    assert_eq!(std::fs::read(outside.path().join("sentinel")).unwrap(), b"outside");
    std::fs::remove_file(&profile).unwrap();
    std::fs::create_dir(&profile).unwrap();
    std::fs::write(profile.join("state"), b"retry").unwrap();
    service
        .delete_agent_session("alice", "symlink-session", &bindings)
        .await
        .unwrap();
    assert!(!profile.exists());
    assert_eq!(std::fs::read(outside.path().join("sentinel")).unwrap(), b"outside");
}

#[cfg(windows)]
#[tokio::test]
async fn profile_junction_fails_closed_and_retry_deletes_only_replacement_directory() {
    let data_dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let (service, store) = service_with_profiles(Arc::new(Factory::default()), data_dir.path());
    let key = BrowserResourceKey {
        principal_id: "alice".into(),
        agent_session_id: "junction-session".into(),
        resource_binding_id: "junction-binding".into(),
    };
    let profile = profile_path(&store, &key);
    std::fs::create_dir_all(profile.parent().unwrap()).unwrap();
    std::fs::write(outside.path().join("sentinel"), b"outside").unwrap();
    junction::create(outside.path(), &profile).unwrap();
    let bindings = [BrowserProfileBinding::persistent("junction-binding").unwrap()];

    assert_eq!(
        service
            .delete_agent_session("alice", "junction-session", &bindings)
            .await,
        Err(WorkspaceError::ProfileCleanupFailed)
    );
    assert_eq!(std::fs::read(outside.path().join("sentinel")).unwrap(), b"outside");
    junction::delete(&profile).unwrap();
    std::fs::create_dir_all(&profile).unwrap();
    std::fs::write(profile.join("state"), b"retry").unwrap();
    service
        .delete_agent_session("alice", "junction-session", &bindings)
        .await
        .unwrap();
    assert!(!profile.exists());
    assert_eq!(std::fs::read(outside.path().join("sentinel")).unwrap(), b"outside");
}

#[tokio::test]
async fn exact_provider_and_authority_cannot_change_on_a_live_resource() {
    let factory = Arc::new(Factory::default());
    let service = service(factory, &["managed", "attached"]);
    let original = authority(
        "alice",
        "session",
        "managed",
        [BrowserCapabilityAction::Observe],
    );
    let resource = service
        .ensure(original.clone(), BrowserProfile::Ephemeral)
        .await
        .unwrap();
    assert!(Arc::ptr_eq(
        &resource,
        &service
            .ensure(original.clone(), BrowserProfile::Ephemeral)
            .await
            .unwrap()
    ));

    let widened = authority("alice", "session", "managed", all_actions());
    assert!(matches!(
        service.ensure(widened, BrowserProfile::Ephemeral).await,
        Err(WorkspaceError::ActionDenied)
    ));

    let changed_provider = authority(
        "alice",
        "session",
        "attached",
        [BrowserCapabilityAction::Observe],
    );
    assert!(matches!(
        service
            .ensure(changed_provider, BrowserProfile::Ephemeral)
            .await,
        Err(WorkspaceError::ProviderChanged)
    ));
}

#[tokio::test]
async fn close_all_is_user_only_and_never_creates_or_replaces_a_runtime() {
    let factory = Arc::new(Factory::default());
    let service = service(factory.clone(), &["managed"]);
    let bound = authority("alice", "close-all", "managed", all_actions());
    let resource = service
        .ensure(bound, BrowserProfile::Ephemeral)
        .await
        .unwrap();
    let generation = resource.runtime_generation();
    let command = || BrowserTabCommand::CloseAll {
        runtime_generation: generation,
    };
    assert_eq!(
        resource.user_command(command()).await,
        Err(WorkspaceError::TabNotFound)
    );
    assert_eq!(factory.creates.load(Ordering::SeqCst), 0);
    resource.user_command(create()).await.unwrap();
    resource.user_command(create()).await.unwrap();
    let run = resource.begin_run().await.unwrap();
    assert_eq!(
        resource.agent_command(&run, command()).await,
        Err(WorkspaceError::UnsupportedAction)
    );
    resource.finish_run(&run).await.unwrap();
    assert!(resource.user_command(command()).await.unwrap().tabs.is_empty());
    assert_eq!(factory.creates.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn failed_native_close_retains_resource_until_exact_retry() {
    let factory = Arc::new(Factory::default());
    let service = service(factory.clone(), &["managed"]);
    let bound = authority("alice", "close-retry", "managed", all_actions());
    let key = bound.key();
    let resource = service
        .ensure(bound.clone(), BrowserProfile::Ephemeral)
        .await
        .unwrap();
    resource.user_command(create()).await.unwrap();
    factory.fail_close_once.store(true, Ordering::SeqCst);
    assert_eq!(
        service.close(&key).await,
        Err(WorkspaceError::NativeCommandFailed)
    );
    assert!(matches!(
        service.ensure(bound.clone(), BrowserProfile::Ephemeral).await,
        Err(WorkspaceError::WorkspaceClosed)
    ));
    assert_eq!(
        resource.user_command(create()).await,
        Err(WorkspaceError::WorkspaceClosed)
    );
    service.close(&key).await.unwrap();
    let replacement = service
        .ensure(bound, BrowserProfile::Ephemeral)
        .await
        .unwrap();
    assert!(!Arc::ptr_eq(&resource, &replacement));
}

#[tokio::test]
async fn service_shutdown_closes_runtimes_before_process_wide_factory() {
    let factory = Arc::new(Factory::default());
    let service = service(factory.clone(), &["managed"]);
    let resource = service
        .ensure(
            authority("alice", "shutdown", "managed", all_actions()),
            BrowserProfile::Ephemeral,
        )
        .await
        .unwrap();
    resource.user_command(create()).await.unwrap();

    service.shutdown().await.unwrap();

    assert!(resource.native_close_proven().await);
    assert_eq!(factory.shutdowns.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn dropped_site_data_clear_keeps_native_cleanup_authority() {
    let clear = Arc::new(DelayedClear::default());
    let factory = Arc::new(Factory {
        clear: Some(clear.clone()),
        ..Default::default()
    });
    let service = Arc::new(service(factory.clone(), &["managed"]));
    let bound = authority("alice", "clear", "managed", all_actions());
    let resource = service
        .ensure(bound, BrowserProfile::Ephemeral)
        .await
        .unwrap();
    resource.user_command(create()).await.unwrap();
    let generation = resource.runtime_generation();
    let caller = tokio::spawn({
        let resource = resource.clone();
        async move {
            resource
                .user_command(BrowserTabCommand::ClearSiteData {
                    runtime_generation: generation,
                })
                .await
        }
    });
    clear.started.notified().await;
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    let close = tokio::spawn({
        let resource = resource.clone();
        async move { resource.close().await }
    });
    tokio::task::yield_now().await;
    assert!(!close.is_finished());
    assert!(!clear.closed_before_completion.load(Ordering::SeqCst));
    clear.release.notify_one();
    close.await.unwrap().unwrap();
    assert!(clear.completed.load(Ordering::SeqCst));
    assert_eq!(clear.calls.load(Ordering::SeqCst), 1);
    assert_eq!(factory.creates.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn human_close_rejects_active_run_without_cancelling_it() {
    let factory = Arc::new(Factory::default());
    let service = Arc::new(service(factory, &["managed"]));
    let bound = authority("alice", "idle-close", "managed", all_actions());
    let key = bound.key();
    let resource = service
        .ensure(bound.clone(), BrowserProfile::Ephemeral)
        .await
        .unwrap();
    resource.user_command(create()).await.unwrap();
    let run = resource.begin_run().await.unwrap();
    assert_eq!(
        service
            .close_idle(key.clone(), resource.runtime_generation())
            .await,
        Err(WorkspaceError::Admission(RunAdmissionError::UserInputLocked))
    );
    assert!(resource.agent_command(&run, create()).await.is_ok());
    resource.finish_run(&run).await.unwrap();
    service
        .close_idle(key, resource.runtime_generation())
        .await
        .unwrap();
    assert!(resource.native_close_proven().await);
    assert!(service.get(&bound).await.unwrap().is_none());
}

#[test]
fn resource_operations_reject_provider_and_web_search_controls() {
    for operation in [
        "connect",
        "provider",
        "grant",
        "search",
        "web_search",
        "research_search",
    ] {
        assert_eq!(BrowserResourceOperation::parse(operation), None);
    }
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
    assert!(!BrowserSurfaceBounds { width: f64::NAN, ..bounds }.is_valid());
    assert!(!BrowserSurfaceBounds { x: -1.0, ..bounds }.is_valid());
    assert!(!BrowserSurfaceBounds { height: 0.0, ..bounds }.is_valid());
    assert!(!BrowserSurfaceBounds {
        x: 32_000.0,
        width: 1024.0,
        ..bounds
    }
    .is_valid());
}
