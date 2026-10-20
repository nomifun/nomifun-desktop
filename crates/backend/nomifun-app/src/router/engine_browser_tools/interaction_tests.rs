use super::*;
use crate::browser_workspace_provider::BrowserUserClosePort;
use nomifun_agent_contracts::{
    ActionId, AgentBindingValue, AgentPresetId, CorrelationId, EventId, IdempotencyKey,
    PresetRevisionRef, SemanticSessionEventDraft, SessionEventAppend, SessionEventKind,
    SessionEventPayloadRef,
};
use nomifun_agent_domain_wave2::Wave2HostContext;
use nomifun_browser_platform::{
    run_guard::{BrowserInputState, NativeInputGate},
    runtime::{BrowserNativeSurfacePort, BrowserProfilePersistence, BrowserProfileStore,
        BrowserRuntime, BrowserRuntimeFactory, CreateBrowserRuntime},
    workspace::managed_workspace_key,
};
use serde_json::json;
use tokio::sync::{Notify, Semaphore};

struct NativeControl {
    locked: AtomicBool,
    command_started: Notify,
    commands: Semaphore,
    block_cleanup: AtomicBool,
    cleanup_started: Notify,
    cleanup: Semaphore,
    pending_continuation: AtomicBool,
    fail_command: AtomicBool,
    generations: Mutex<Vec<u64>>,
}

impl NativeControl {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            locked: AtomicBool::new(false), command_started: Notify::new(), commands: Semaphore::new(0),
            block_cleanup: AtomicBool::new(false), cleanup_started: Notify::new(), cleanup: Semaphore::new(0),
            pending_continuation: AtomicBool::new(false), fail_command: AtomicBool::new(false),
            generations: Mutex::new(Vec::new()),
        })
    }
}

struct NativeRuntime { generation: u64, control: Arc<NativeControl> }

#[async_trait::async_trait]
impl NativeInputGate for NativeRuntime {
    async fn lock_user_input(&self) -> Result<(), RunAdmissionError> {
        self.control.locked.store(true, Ordering::SeqCst);
        Ok(())
    }
    async fn release_pressed_input(&self) -> Result<(), RunAdmissionError> { Ok(()) }
    async fn unlock_user_input(&self) -> Result<(), RunAdmissionError> {
        self.control.locked.store(false, Ordering::SeqCst);
        Ok(())
    }
    async fn try_release_for_user(&self) -> Result<bool, RunAdmissionError> {
        if self.control.block_cleanup.load(Ordering::SeqCst) {
            self.control.cleanup_started.notify_one();
            self.control.cleanup.acquire().await.unwrap().forget();
        }
        Ok(!self.control.pending_continuation.load(Ordering::SeqCst))
    }
}

#[async_trait::async_trait]
impl BrowserRuntime for NativeRuntime {
    fn surface(&self) -> Option<&dyn BrowserNativeSurfacePort> { None }
    async fn snapshot(&self) -> Result<BrowserRuntimeSnapshot, WorkspaceError> {
        Ok(BrowserRuntimeSnapshot { runtime_generation: self.generation, revision: 1,
            active_tab_id: None, tabs: vec![], downloads: vec![] })
    }
    async fn execute(&self, command: BrowserTabCommand, _: tokio_util::sync::CancellationToken)
        -> Result<BrowserRuntimeSnapshot, WorkspaceError> {
        if matches!(&command, BrowserTabCommand::Create { url } | BrowserTabCommand::Navigate { url, .. }
            if url.contains("/agent")) {
            self.control.generations.lock().unwrap().push(self.generation);
            self.control.command_started.notify_one();
            self.control.commands.acquire().await.unwrap().forget();
            if self.control.fail_command.load(Ordering::SeqCst) {
                return Err(WorkspaceError::NativeCommandFailed);
            }
        }
        self.snapshot().await
    }
    async fn close(&self) -> Result<(), WorkspaceError> { Ok(()) }
}

struct NativeFactory(Arc<NativeControl>);
#[async_trait::async_trait]
impl BrowserRuntimeFactory for NativeFactory {
    async fn create(&self, request: CreateBrowserRuntime) -> Result<Arc<dyn BrowserRuntime>, WorkspaceError> {
        self.0.locked.store(!request.user_input_enabled, Ordering::SeqCst);
        Ok(Arc::new(NativeRuntime { generation: request.runtime_generation, control: self.0.clone() }))
    }
}

struct Fixture {
    database: nomifun_db::Database,
    _root: tempfile::TempDir,
    store: nomifun_agent_session::AgentSessionStore,
    owner: Arc<BrowserRoleOwner>,
    admission: Arc<BrowserTurnAdmission>,
    control: Arc<NativeControl>,
    started_event: EventId,
}

impl Fixture {
    async fn new() -> Self {
        let database = nomifun_db::init_database_memory().await.unwrap();
        let store = nomifun_agent_session::AgentSessionStore::from_pool(database.pool().clone()).await.unwrap();
        let sessions = nomifun_conversation::CanonicalAgentSessionOwner::from_pool(database.pool().clone()).await.unwrap();
        let principal = PrincipalRef { principal_kind: "user".into(),
            principal_id: "0190f5fe-7c00-7a00-8000-000000000001".into() };
        let mut admission = super::tests::render_admission(&principal.principal_id, "before-open");
        Arc::get_mut(&mut admission).unwrap().binding.operations = std::collections::BTreeSet::from(["navigate".into()]);
        let opened = sessions.open(principal.clone(), AgentBindingValue {
            preset_revision_ref: PresetRevisionRef { preset_id: AgentPresetId::from("browser-interaction-test"),
                revision: 1, revision_digest: "a".repeat(64).into() },
            resolved_snapshot_ref: admission.snapshot.clone(), typed_resource_bindings: vec![admission.binding.clone()], binding_version: 1,
        }, None, vec![], "browser-interaction-test", 1).await.unwrap();
        let session = opened.session.agent_session_id;
        let turn = sessions.start_turn(&principal, &session, "browser-interaction-turn",
            json!({"content":"Use the browser, then continue working."})).await.unwrap();
        let started_event = sessions.turn_receipt(&principal, &session, &turn.operation_id).await.unwrap().started_event.unwrap().event_id;
        let value = Arc::get_mut(&mut admission).unwrap();
        value.key = TurnKey::new(&principal.principal_id, session.as_ref(), turn.operation_id.as_ref());
        let descriptor = browser_workspace_provider::provider_descriptor(&value.provider, &value.binding).unwrap();
        value.authority = BrowserSessionAuthority::from_action_ids(&principal.principal_id, session.as_ref(),
            ["browser/navigate"], browser_workspace_provider::browser_resource_binding(value.binding.clone(), descriptor).unwrap()).unwrap();
        let root = tempfile::tempdir().unwrap();
        crate::config::load_or_create_storage_generation(root.path()).unwrap();
        let control = NativeControl::new();
        let resources = Arc::new(BrowserResourceService::new(Arc::new(NativeFactory(control.clone()))));
        let owner = BrowserRoleOwner::new(Some(resources), None, root.path().to_path_buf(), None, store.clone());
        owner.turns.lock().unwrap().insert(admission.key.clone(), admission.clone());
        Self { database, _root: root, store, owner, admission, control, started_event }
    }

    async fn browser(&self) -> Arc<BrowserWorkspace> {
        let key = managed_workspace_key(&self.admission.key.principal_id, &self.admission.key.agent_session_id).unwrap();
        let profile = BrowserProfileStore::new(self._root.path()).unwrap().profile_for(&key, BrowserProfilePersistence::Persistent).unwrap();
        self.owner.resources.as_ref().unwrap().ensure_user(&self.admission.key.principal_id,
            &self.admission.key.agent_session_id, profile).await.unwrap()
    }

    async fn request(&self, key: &str) -> Wave2TypedHostRequest {
        let operation = OperationId::from(format!("browser-operation-{key}"));
        self.store.append_event(&SessionEventAppend {
            agent_session_id: self.admission.key.agent_session_id.clone().into(),
            event_id: format!("browser-call-{key}").into(), producer_id: "capability_host".into(),
            idempotency_key: format!("browser-call-{key}").into(), semantic_event: SemanticSessionEventDraft {
                kind: SessionEventKind("tool/call-started".into()), kind_version: 1,
                correlation_id: self.admission.key.turn_id.clone().into(), causation_event_id: Some(self.started_event.clone()),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                    "operation_id":operation.as_ref(), "capability_id":"browser", "action_id":"browser/navigate"
                }))),
            },
        }).await.unwrap();
        Wave2TypedHostRequest {
            context: Wave2HostContext {
                authorized_actions: nomifun_agent_domain_wave2::action_ids(nomifun_agent_domain_wave2::BROWSER_MODULE_ID),
            control: Default::default(),
                principal: self.admission.principal.clone(), agent_session_id: self.admission.key.agent_session_id.clone().into(),
                turn_id: self.admission.key.turn_id.clone().into(), operation_id: operation,
                idempotency_key: IdempotencyKey::from(format!("browser-invoke-{key}")), correlation_id: CorrelationId::from(key),
                resolved_snapshot_ref: self.admission.snapshot.clone(), registry_generation: self.admission.registry_generation,
                capability_id: CapabilityId::from(BROWSER_MODULE_ID), action_id: ActionId::from("browser/navigate"),
                role_provider: Some(self.admission.provider.clone()), resource_bindings: vec![self.admission.binding.clone()],
                state: super::super::agent_wave2_host::tests::test_state_handle(),
            },
            operation: Wave2TypedCapabilityOperation::BrowserNavigate { input: StrictJsonValue(json!({"url":"https://example.test/agent"})) },
        }
    }

    async fn invoke(&self, key: &str) -> tokio::task::JoinHandle<Result<StrictJsonValue, Wave2HostPortError>> {
        let owner = self.owner.clone();
        let request = self.request(key).await;
        let invocation = tokio::spawn(async move { owner.invoke_typed(request).await });
        wait_for(&self.control.command_started).await;
        invocation
    }

    async fn close(&self, generation: u64) -> Result<(), WorkspaceError> {
        self.owner.close_user_workspace(&self.admission.key.principal_id,
            &self.admission.key.agent_session_id, generation).await
    }

    async fn assert_turn_running(&self) {
        let head = self.store.head(&self.admission.key.agent_session_id.clone().into()).await.unwrap();
        assert_eq!(head.status, "running");
        assert_eq!(head.active_turn_id.as_deref(), Some(self.admission.key.turn_id.as_str()));
    }

    async fn shutdown(self) {
        self.owner.resources.as_ref().unwrap().shutdown().await.unwrap();
        self.database.close().await;
    }
}

async fn wait_for(notification: &Notify) {
    tokio::time::timeout(std::time::Duration::from_secs(5), notification.notified()).await.unwrap();
}

fn user_command() -> BrowserTabCommand { BrowserTabCommand::Create { url: "https://example.test/human".into() } }
fn is_locked(result: Result<(), WorkspaceError>) -> bool {
    matches!(result, Err(WorkspaceError::Admission(RunAdmissionError::UserInputLocked)))
}

#[tokio::test]
async fn completed_browser_invocations_allow_user_control_and_reopen_during_the_same_running_turn() {
    let fixture = Fixture::new().await;
    let first = fixture.browser().await;
    let first_generation = first.runtime_generation();
    let invocation = fixture.invoke("first").await;
    assert!(fixture.control.locked.load(Ordering::SeqCst));
    assert!(matches!(first.user_command(user_command()).await,
        Err(WorkspaceError::Admission(RunAdmissionError::UserInputLocked))));
    assert!(is_locked(fixture.close(first_generation).await));
    fixture.control.commands.add_permits(1);
    invocation.await.unwrap().unwrap();
    fixture.assert_turn_running().await;
    assert!(!fixture.control.locked.load(Ordering::SeqCst));
    assert!(fixture.admission.active_run().unwrap().is_none());
    assert!(fixture.owner.managed_turns.lock().unwrap().is_empty());
    first.user_command(user_command()).await.unwrap();
    fixture.close(first_generation).await.unwrap();

    let reopened = fixture.browser().await;
    assert!(reopened.runtime_generation() > first_generation);
    let invocation = fixture.invoke("reopened").await;
    assert!(is_locked(fixture.close(reopened.runtime_generation()).await));
    fixture.control.commands.add_permits(1);
    invocation.await.unwrap().unwrap();
    fixture.assert_turn_running().await;
    assert_eq!(*fixture.control.generations.lock().unwrap(), vec![first_generation, reopened.runtime_generation()]);
    reopened.user_command(user_command()).await.unwrap();
    fixture.close(reopened.runtime_generation()).await.unwrap();
    fixture.shutdown().await;
}

#[tokio::test]
async fn dropping_the_invocation_future_retains_native_cleanup_and_releases_input_when_cleanup_finishes() {
    let fixture = Fixture::new().await;
    let browser = fixture.browser().await;
    fixture.control.block_cleanup.store(true, Ordering::SeqCst);
    let invocation = fixture.invoke("dropped").await;
    invocation.abort();
    assert!(invocation.await.unwrap_err().is_cancelled());
    assert!(fixture.admission.operation.try_lock().is_err());
    assert!(is_locked(fixture.close(browser.runtime_generation()).await));
    fixture.control.commands.add_permits(1);
    wait_for(&fixture.control.cleanup_started).await;
    assert!(fixture.admission.operation.try_lock().is_err(), "the retained worker owns admission through native cleanup");
    assert!(fixture.control.locked.load(Ordering::SeqCst));
    assert!(matches!(browser.user_command(user_command()).await,
        Err(WorkspaceError::Admission(RunAdmissionError::UserInputLocked))));
    assert!(is_locked(fixture.close(browser.runtime_generation()).await));
    assert!(!fixture.store.has_unsettled_effects(&fixture.admission.key.agent_session_id.clone().into()).await.unwrap(),
        "durable effect settlement must precede native input release");
    fixture.control.cleanup.add_permits(1);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let _finished = fixture.admission.operation.lock().await;
    }).await.unwrap();
    fixture.assert_turn_running().await;
    assert!(!fixture.control.locked.load(Ordering::SeqCst));
    assert_eq!(browser.snapshot().await.unwrap().run.input_state, BrowserInputState::UserReady);
    browser.user_command(user_command()).await.unwrap();
    fixture.close(browser.runtime_generation()).await.unwrap();
    fixture.shutdown().await;
}

#[tokio::test]
async fn native_continuations_keep_input_locked_until_a_later_completed_invocation_releases_them() {
    let fixture = Fixture::new().await;
    let browser = fixture.browser().await;
    fixture.control.pending_continuation.store(true, Ordering::SeqCst);
    let invocation = fixture.invoke("continuation").await;
    fixture.control.commands.add_permits(1);
    invocation.await.unwrap().unwrap();
    fixture.assert_turn_running().await;
    assert!(fixture.control.locked.load(Ordering::SeqCst));
    assert!(fixture.admission.active_run().unwrap().is_some());
    assert!(is_locked(fixture.close(browser.runtime_generation()).await));
    fixture.control.pending_continuation.store(false, Ordering::SeqCst);
    let invocation = fixture.invoke("continuation-finished").await;
    fixture.control.commands.add_permits(1);
    invocation.await.unwrap().unwrap();
    assert!(!fixture.control.locked.load(Ordering::SeqCst));
    assert!(fixture.admission.active_run().unwrap().is_none());
    fixture.close(browser.runtime_generation()).await.unwrap();
    fixture.shutdown().await;
}

#[tokio::test]
async fn an_unknown_browser_effect_retains_its_recovery_owner_after_the_invocation_returns() {
    let fixture = Fixture::new().await;
    let browser = fixture.browser().await;
    fixture.control.fail_command.store(true, Ordering::SeqCst);
    let invocation = fixture.invoke("unknown").await;
    fixture.control.commands.add_permits(1);
    assert_eq!(invocation.await.unwrap().unwrap_err().code, "BROWSER_NATIVE_COMMAND_FAILED");
    assert!(fixture.store.has_unsettled_effects(&fixture.admission.key.agent_session_id.clone().into()).await.unwrap());
    assert!(fixture.control.locked.load(Ordering::SeqCst));
    assert!(fixture.admission.active_run().unwrap().is_some());
    assert!(is_locked(fixture.close(browser.runtime_generation()).await));
    fixture.shutdown().await;
}
