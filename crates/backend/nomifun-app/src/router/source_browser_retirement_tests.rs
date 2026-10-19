//! Actual source owners and canonical receipts; physical Browser retirement
//! is independently exercised by BrowserRoleOwner's source/mission tests.
use super::*;
use nomifun_cron::*;
use nomifun_db::{ICronRepository, SqliteCronRepository};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::sync::{Notify, Semaphore};

const REPLY: &str = "data: {\"id\":\"source-finish\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"activity-fixture\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"The original source task has finished.\"},\"finish_reason\":null}]}\n\ndata: {\"id\":\"source-finish\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"activity-fixture\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";

async fn model() -> wiremock::MockServer {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/chat/completions"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_raw(REPLY, "text/event-stream"))
        .mount(&server).await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/v1/models"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({"object":"list","data":[{"id":"activity-fixture","object":"model"}]})))
        .mount(&server).await;
    server
}

async fn model_calls(model: &wiremock::MockServer) -> usize {
    model.received_requests().await.unwrap().iter().filter(|request|
        request.method.as_str() == "POST" && request.url.path() == "/v1/chat/completions").count()
}

async fn wait_completed_original(owner: &NomiCoreSessionOwner, session: &AgentSessionId, operation: &OperationId) {
    tokio::time::timeout(Duration::from_secs(15), async { loop {
        let receipt = owner.canonical.store().read_turn_receipt(session,operation).await.unwrap();
        if matches!(receipt.status,nomifun_agent_session::TurnReceiptStatus::Failed|nomifun_agent_session::TurnReceiptStatus::Cancelled) {
            panic!("original source Turn failed before the next input: {:?}",owner.canonical.store().turn_outcome(session,operation).await.unwrap());
        }
        let head = owner.canonical.store().head(session).await.unwrap();
        if receipt.status == nomifun_agent_session::TurnReceiptStatus::Completed && head.status == "ready" && head.active_turn_id.is_none() { break; }
        tokio::task::yield_now().await;
    }}).await.unwrap();
}

struct CronBridge {
    owner: Arc<NomiCoreSessionOwner>,
    proof: std::sync::Mutex<Option<CronRunResourceRetirement>>,
    entered: Notify,
    release: Semaphore,
    accepted: AtomicBool,
    finished: Notify,
    plant_ordinary_input: AtomicBool,
}

struct NeverInstalledTasks {
    delegate: Arc<dyn nomifun_conversation::BackgroundTaskRegistrar>,
    spawns: AtomicUsize,
    finished: Arc<Notify>,
}
impl nomifun_conversation::BackgroundTaskRegistrar for NeverInstalledTasks {
    fn spawn(&self, future: std::pin::Pin<Box<dyn std::future::Future<Output=()> + Send + 'static>>) -> bool {
        self.spawns.fetch_add(1, Ordering::SeqCst);
        let finished = self.finished.clone();
        self.delegate.spawn(Box::pin(async move { future.await; finished.notify_one(); }))
    }
    fn shutdown_signal(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output=()> + Send + 'static>> {
        self.delegate.shutdown_signal()
    }
}

#[async_trait::async_trait]
impl CronSessionPort for CronBridge {
    async fn retire_new_conversation_run_resources(&self, request: &CronRunResourceRetirement) -> Result<(), AppError> {
        *self.proof.lock().unwrap() = Some(request.clone());
        self.entered.notify_one();
        self.release.acquire().await.unwrap().forget();
        CronSessionPort::retire_new_conversation_run_resources(self.owner.as_ref(), request).await?;
        self.accepted.store(true, Ordering::SeqCst); self.finished.notify_one();
        Ok(())
    }
    async fn cancel_reserved_turn(&self, request: &CronTurnCancellationRequest) -> Result<CronTurnCancellationReceipt, AppError> {
        CronSessionPort::cancel_reserved_turn(self.owner.as_ref(), request).await
    }
    async fn get_session(&self, query: &CronSessionLookup) -> Result<CronSessionProjection, AppError> {
        CronSessionPort::get_session(self.owner.as_ref(), query).await
    }
    async fn list_conversation_responses_for_cron(&self, query: &CronScheduledSessionLookup) -> Result<Vec<ConversationResponse>, AppError> {
        CronSessionPort::list_conversation_responses_for_cron(self.owner.as_ref(), query).await
    }
    async fn bind_cron_relation(&self, request: &CronSessionCronBindingRequest) -> Result<(), AppError> {
        CronSessionPort::bind_cron_relation(self.owner.as_ref(), request).await
    }
    async fn read_turn_receipt(&self, query: &CronTurnReceiptQuery) -> Result<CronTurnReceiptState, AppError> {
        CronSessionPort::read_turn_receipt(self.owner.as_ref(), query).await
    }
    async fn reconcile_turn_receipt(&self, request: &CronTurnReconciliationRequest) -> Result<CronTurnReconciliation, AppError> {
        CronSessionPort::reconcile_turn_receipt(self.owner.as_ref(), request).await
    }
    async fn create_idempotent(&self, owner: &str, request: nomifun_api_types::CreateConversationRequest,
        binding: CronSessionAgentBinding, key: &str) -> Result<CronSessionHandle, AppError> {
        CronSessionPort::create_idempotent(self.owner.as_ref(), owner, request, binding, key).await
    }
    async fn prepare_runtime_and_send(&self, request: CronRuntimePreparationRequest) -> Result<CronPreparedTurnDelivery, AppError> {
        if self.plant_ordinary_input.swap(false, Ordering::SeqCst) {
            // A legitimate original input wins this literal key before Cron
            // reaches admission. The private Cron source must not retrofit it.
            self.owner.dispatch_canonical_turn(&request.owner_id, &request.agent_session_id,
                &request.idempotency_key, cron_turn_message_to_request(request.turn.message.clone()), false, None).await?;
        }
        CronSessionPort::prepare_runtime_and_send(self.owner.as_ref(), request).await
    }
    async fn delivery_result(&self, query: &CronTurnDeliveryQuery) -> Result<Option<CronTurnDelivery>, AppError> {
        CronSessionPort::delivery_result(self.owner.as_ref(), query).await
    }
    async fn append_notice(&self, owner: &str, session: &AgentSessionId, content: &str, kind: &str) -> Result<(), AppError> {
        CronSessionPort::append_notice(self.owner.as_ref(), owner, session, content, kind).await
    }
}

#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn source_browser_cron_finalization_handoff_survives_legal_job_deletion_without_resending() {
    let model = model().await;
    let root = tempfile::tempdir().unwrap();
    let (services, _, state, initial, principal) = fixture_in(root.path(), &format!("{}/v1", model.uri())).await;
    let owner = &principal.principal_id;
    let original_selection = state.session_owner.get_session(owner, initial.as_ref()).await.unwrap();
    let selected = original_selection.model.clone().unwrap();
    let frozen = original_selection.agent_snapshot.clone().unwrap();
    let bridge = Arc::new(CronBridge { owner: state.session_owner.clone(), proof: Default::default(), entered: Notify::new(),
        release: Semaphore::new(0), accepted: AtomicBool::new(false), finished: Notify::new(), plant_ordinary_input:AtomicBool::new(false) });
    let repository = Arc::new(SqliteCronRepository::new(services.database.pool().clone()));
    let scheduler = Arc::new(nomifun_cron::scheduler::CronScheduler::new(Arc::new(|_,_,_,_,_|{})));
    let service = nomifun_cron::service::CronService::new(services.authoritative_user_id.clone(), repository.clone(), scheduler.clone(),
        Arc::new(nomifun_cron::executor::JobExecutor::new(services.authoritative_user_id.clone(), bridge.clone(),
            Arc::new(nomifun_cron::busy_guard::CronBusyGuard::new()), services.data_dir.clone())),
        CronEventEmitter::new(services.event_bus.clone()), services.data_dir.clone());
    service.with_cron_background_task_registrar(services.background_tasks.clone());
    let job = service.add_job(owner, serde_json::from_value(json!({"name":"Original isolated run","schedule":{"kind":"every","every_ms":60000},
        "message":"Finish the original scheduled task.","execution_mode":"new_conversation","agent_type":"nomi","created_by":"user",
        "agent_config":{"name":"Nomi","provider_id":selected.provider_id,"model":selected.model,"agent_snapshot":frozen}})).unwrap()).await.unwrap();
    let result = service.run_now(owner, &job.cron_job_id, "source-retirement-original-run").await.unwrap();
    tokio::time::timeout(Duration::from_secs(15), bridge.entered.notified()).await.unwrap();
    let proof = bridge.proof.lock().unwrap().clone().unwrap();
    assert_eq!(proof.agent_session_id.as_ref(), result.conversation_id);
    let operation = NomiCoreSessionOwner::turn_operation_id(owner, &result.conversation_id, &format!("cron:{}:turn", proof.cron_job_run_id));
    let receipt = state.session_owner.canonical.store().read_turn_receipt(&proof.agent_session_id, &operation).await.unwrap();
    assert_eq!(receipt.status, nomifun_agent_session::TurnReceiptStatus::Completed,
        "the frozen, authorized original Cron task must complete: {:?}",
        state.session_owner.canonical.store().turn_outcome(&proof.agent_session_id,&operation).await.unwrap());
    let (_, original_input) = state.session_owner.canonical.store().turn_accepted_input(&proof.agent_session_id, &operation).await.unwrap();
    let source: nomifun_agent_contracts::CronRunRuntimeSource = serde_json::from_value(original_input.0["cron_runtime_overlay"].clone()).unwrap();
    assert_eq!((source.cron_job_id.as_str(),source.cron_job_run_id.as_str(),source.agent_session_id.as_ref()),
        (proof.cron_job_id.as_str(),proof.cron_job_run_id.as_str(),proof.agent_session_id.as_ref()));
    assert!(original_input.0.get("remote_runtime_source").is_none());
    let cursor = state.session_owner.canonical.store().current_cursor(&proof.agent_session_id).await.unwrap();
    let calls = model_calls(&model).await;
    assert!(calls > 0, "the normal source really invoked the configured model");
    // The original authorized repository delete is legal after finalization;
    // cleanup must not turn its now-removable run row into a new prerequisite.
    repository.delete(owner, &job.cron_job_id).await.unwrap();
    assert!(repository.get_run_reservation(owner, &job.cron_job_id, &proof.cron_job_run_id).await.unwrap().is_none());
    bridge.release.add_permits(1);
    tokio::time::timeout(Duration::from_secs(5), bridge.finished.notified()).await.unwrap();
    assert!(bridge.accepted.load(Ordering::SeqCst));
    assert!(matches!(super::super::source_browser_retirement::finalized_cron_source_operation(
        services.database.pool(), &state.session_owner.canonical, &proof).await.unwrap(),
        super::super::source_browser_retirement::CronSourceResourceState::Ended(ref original) if original == &operation));
    let no_host_tasks = Arc::new(NeverInstalledTasks { delegate: services.background_tasks.clone(), spawns: AtomicUsize::new(0), finished:Arc::new(Notify::new()) });
    let never_installed = NomiCoreSessionOwner::new(state.session_owner.canonical.clone(), state.session_owner.runtime_sessions.clone(),
        services.event_bus.clone(), no_host_tasks.clone(), state.session_owner.managed_workspace_root.clone(),
        services.database.pool().clone(), services.creation_service.clone());
    for _ in 0..3 { CronSessionPort::retire_new_conversation_run_resources(&never_installed, &proof).await.unwrap(); }
    assert!(never_installed.native_engines.get().is_none());
    assert_eq!(no_host_tasks.spawns.load(Ordering::SeqCst), 0,
        "an owner with no native host does not create permanent cleanup retries or warm up a carrier");
    assert!(never_installed.native_engines.set(std::sync::Weak::new()).is_ok());
    CronSessionPort::retire_new_conversation_run_resources(&never_installed, &proof).await.unwrap();
    assert_eq!(no_host_tasks.spawns.load(Ordering::SeqCst), 1,
        "a previously-installed but lost host retains an unknown original resource responsibility");
    assert_eq!(state.session_owner.canonical.store().current_cursor(&proof.agent_session_id).await.unwrap(), cursor);
    assert_eq!(model_calls(&model).await, calls);
    scheduler.cancel_all(); service.shutdown_timers();
    services.shutdown_nomi_core_host().await.unwrap(); services.database.close().await;
}

#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn source_browser_cron_same_named_ordinary_input_never_acquires_source_retirement() {
    let model = model().await;
    let root = tempfile::tempdir().unwrap();
    let (services, _, state, initial, principal) = fixture_in(root.path(), &format!("{}/v1", model.uri())).await;
    let owner = &principal.principal_id;
    let original_selection = state.session_owner.get_session(owner, initial.as_ref()).await.unwrap();
    let selected = original_selection.model.clone().unwrap();
    let frozen = original_selection.agent_snapshot.clone().unwrap();
    let bridge = Arc::new(CronBridge { owner:state.session_owner.clone(), proof:Default::default(), entered:Notify::new(),
        release:Semaphore::new(0), accepted:AtomicBool::new(false), finished:Notify::new(), plant_ordinary_input:AtomicBool::new(true) });
    let repository = Arc::new(SqliteCronRepository::new(services.database.pool().clone()));
    let scheduler = Arc::new(nomifun_cron::scheduler::CronScheduler::new(Arc::new(|_,_,_,_,_|{})));
    let service = nomifun_cron::service::CronService::new(services.authoritative_user_id.clone(), repository.clone(), scheduler.clone(),
        Arc::new(nomifun_cron::executor::JobExecutor::new(services.authoritative_user_id.clone(), bridge.clone(),
            Arc::new(nomifun_cron::busy_guard::CronBusyGuard::new()), services.data_dir.clone())),
        CronEventEmitter::new(services.event_bus.clone()), services.data_dir.clone());
    service.with_cron_background_task_registrar(services.background_tasks.clone());
    let job = service.add_job(owner, serde_json::from_value(json!({"name":"Literal key collision","schedule":{"kind":"every","every_ms":60000},
        "message":"This literal key was taken by another legitimate input.","execution_mode":"new_conversation","agent_type":"nomi","created_by":"user",
        "agent_config":{"name":"Nomi","provider_id":selected.provider_id,"model":selected.model,"agent_snapshot":frozen}})).unwrap()).await.unwrap();
    let result = service.run_now(owner,&job.cron_job_id,"source-retirement-collision").await.unwrap();
    tokio::time::timeout(Duration::from_secs(15),bridge.entered.notified()).await.unwrap();
    let proof = bridge.proof.lock().unwrap().clone().unwrap();
    assert!(proof.finalized_status().is_some(),"the original run really finalized through its owner");
    let operation = NomiCoreSessionOwner::turn_operation_id(owner,&result.conversation_id,&format!("cron:{}:turn",proof.cron_job_run_id));
    let (_,input) = state.session_owner.canonical.store().turn_accepted_input(&proof.agent_session_id,&operation).await.unwrap();
    assert_eq!(input.0.get("origin").and_then(Value::as_str),Some("cron"),"a message origin label cannot grant source ownership");
    assert!(input.0.get("cron_runtime_overlay").is_none(),"the later Cron attempt cannot retrofit the original ordinary input");
    assert!(matches!(super::super::source_browser_retirement::finalized_cron_source_operation(
        services.database.pool(),&state.session_owner.canonical,&proof).await.unwrap(),
        super::super::source_browser_retirement::CronSourceResourceState::ContinuingOrNeverInvoked),
        "formalizing a run after a keyed collision cannot retire the other input's pages");
    let unrelated_tasks = Arc::new(NeverInstalledTasks { delegate:services.background_tasks.clone(),
        spawns:AtomicUsize::new(0), finished:Arc::new(Notify::new()) });
    let unrelated_owner = NomiCoreSessionOwner::new(state.session_owner.canonical.clone(),state.session_owner.runtime_sessions.clone(),
        services.event_bus.clone(),unrelated_tasks.clone(),state.session_owner.managed_workspace_root.clone(),
        services.database.pool().clone(),services.creation_service.clone());
    assert!(unrelated_owner.native_engines.set(std::sync::Weak::new()).is_ok());
    CronSessionPort::retire_new_conversation_run_resources(&unrelated_owner,&proof).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2),unrelated_tasks.finished.notified()).await.unwrap();
    assert_eq!(unrelated_tasks.spawns.load(Ordering::SeqCst),1,
        "a known unrelated purpose completes source observation even when that source has no host; no permanent waiter remains");
    let cursor = state.session_owner.canonical.store().current_cursor(&proof.agent_session_id).await.unwrap();
    let calls = model_calls(&model).await;
    assert!(calls > 0,"the ordinary input at the literal Cron key actually ran the authorized model");
    bridge.release.add_permits(1);
    tokio::time::timeout(Duration::from_secs(5),bridge.finished.notified()).await.unwrap();
    assert!(bridge.accepted.load(Ordering::SeqCst));
    assert_eq!(state.session_owner.canonical.store().current_cursor(&proof.agent_session_id).await.unwrap(),cursor);
    assert_eq!(model_calls(&model).await,calls,"the source-only handoff never resends the original input");
    scheduler.cancel_all(); service.shutdown_timers();
    services.shutdown_nomi_core_host().await.unwrap(); services.database.close().await;
}

#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn source_browser_remote_ready_stays_continuing_and_cancel_uses_original_request_and_turn_receipts() {
    let model = model().await;
    let root = tempfile::tempdir().unwrap();
    let (services, _, state, initial, principal) = fixture_in(root.path(), &format!("{}/v1", model.uri())).await;
    let common_owner = nomifun_common::UserId::parse(&principal.principal_id).unwrap();
    let binding = state.session_owner.canonical.store().get_live_session(&initial).await.unwrap().agent_binding;
    let remote = state.control_plane.create_remote_binding(&nomifun_agent_contracts::UserId::from(principal.principal_id.clone()), nomifun_api_types::CreateRemoteBindingRequest {
        name:"Original Remote source".into(), agent_binding: agent_binding_dto(&binding).unwrap(),
    }).await.unwrap();
    let opened = run_nomi_core_remote_open(state.clone(), &common_owner, RemoteOpenRequestDto { binding_id: remote.remote_binding_id,
        idempotency_key:"source-open".into(), initial_input:None }).await.unwrap();
    let session = AgentSessionId::from(opened["agent_session_id"].as_str().unwrap());
    let ordinary_key = "ordinary-lookalike-remote-operation";
    state.session_owner.dispatch_canonical_turn(&principal.principal_id, &session, ordinary_key,
        serde_json::from_value(json!({"content":"A legitimate ordinary input with the same operation format.","origin":"cron"})).unwrap(), false, None).await.unwrap();
    let ordinary = NomiCoreSessionOwner::turn_operation_id(&principal.principal_id, session.as_ref(), ordinary_key);
    wait_completed_original(&state.session_owner,&session,&ordinary).await;
    let ordinary_row = state.remote_repository.get_session(&principal.principal_id,session.as_ref()).await.unwrap().unwrap();
    let lookalike_target = RemoteCancelTarget { expected_operation_id:Some(ordinary.clone()),
        expected_head_seq:state.session_owner.canonical.store().head(&session).await.unwrap().last_seq,
        expected_binding_version:ordinary_row.binding_version };
    assert!(matches!(super::super::source_browser_retirement::remote_browser_purpose_state(
        &state.session_owner.canonical, &state.remote_repository, &principal.principal_id, &session, &ordinary, &lookalike_target).await.unwrap(),
        super::super::source_browser_retirement::RemotePurposeResourceState::OtherSource),
        "even an identical cancel target is not authority over an ordinary input's Browser purpose");
    let original_calls = model_calls(&model).await;
    run_nomi_core_remote_turn(state.clone(), &common_owner, RemoteTurnRequestDto { agent_session_id:session.as_ref().into(),
        input:json!({"content":"A legitimate ordinary input with the same operation format.","origin":"cron"}),
        idempotency_key:ordinary_key.into() }).await.unwrap();
    assert_eq!(model_calls(&model).await,original_calls,"source-less historical replay is observed without resending");
    let (_, unchanged) = state.session_owner.canonical.store().turn_accepted_input(&session,&ordinary).await.unwrap();
    assert!(unchanged.0.get("remote_runtime_source").is_none(),"replay never retrofits source ownership");
    assert!(bounded_turn_input(json!({"content":"do not import trust", "remote_runtime_source":{
        "remote_binding_id":ordinary_row.remote_binding_id,"binding_version":ordinary_row.binding_version}})).is_err());
    assert!(bounded_turn_input(json!({"content":"do not import trust", "cron_runtime_overlay":{
        "cron_job_id":"literal","cron_job_run_id":"literal","agent_session_id":session}})).is_err());
    assert!(serde_json::from_value::<SendMessageRequest>(json!({"content":"do not import trust", "remote_runtime_source":{
        "remote_binding_id":ordinary_row.remote_binding_id,"binding_version":ordinary_row.binding_version}})).is_err());
    assert!(serde_json::from_value::<SendMessageRequest>(json!({"content":"do not import trust", "cron_runtime_overlay":{
        "cron_job_id":"literal","cron_job_run_id":"literal","agent_session_id":session}})).is_err());
    run_nomi_core_remote_turn(state.clone(), &common_owner, RemoteTurnRequestDto { agent_session_id:session.as_ref().into(),
        input:json!({"content":"Finish only this Remote turn.","files":[],"inject_skills":[]}), idempotency_key:"original-browser-phase".into() }).await.unwrap();
    let operation = NomiCoreSessionOwner::turn_operation_id(&principal.principal_id, session.as_ref(), "original-browser-phase");
    tokio::time::timeout(Duration::from_secs(15), async { loop {
        let receipt = state.session_owner.canonical.store().read_turn_receipt(&session, &operation).await.unwrap();
        if receipt.status == nomifun_agent_session::TurnReceiptStatus::Completed {
            let digest = remote_operation_key_digest("original-browser-phase").unwrap();
            if state.remote_repository.find_event_by_operation_key(&principal.principal_id, session.as_ref(), "turn/completed", &digest).await.unwrap().is_some() { break; }
        }
        tokio::task::yield_now().await;
    }}).await.unwrap();
    let projection = state.remote_repository.get_session(&principal.principal_id, session.as_ref()).await.unwrap().unwrap();
    assert_eq!(projection.state, "ready", "a normal Remote answer preserves its continuing source");
    let target = RemoteCancelTarget { expected_operation_id:None,
        expected_head_seq:state.session_owner.canonical.store().head(&session).await.unwrap().last_seq, expected_binding_version:projection.binding_version };
    assert!(!super::super::source_browser_retirement::cancelled_remote_source_proven(&state.session_owner.canonical,
        &state.remote_repository, &principal.principal_id, &session, "source-stop", &target).await.unwrap());
    let calls = model_calls(&model).await;
    run_nomi_core_remote_cancel(state.clone(), &common_owner, RemoteCancelRequestDto {
        agent_session_id:session.as_ref().into(),idempotency_key:"source-stop".into() }).await.unwrap();
    let requested = state.remote_repository.find_event_by_operation_key(&principal.principal_id, session.as_ref(), "session/cancel-requested",
        &remote_operation_key_digest("source-stop").unwrap()).await.unwrap().unwrap();
    let original = remote_cancel_target(&serde_json::from_str(&requested.payload_json).unwrap()).unwrap().unwrap();
    assert!(super::super::source_browser_retirement::cancelled_remote_source_proven(&state.session_owner.canonical,
        &state.remote_repository, &principal.principal_id, &session, "source-stop", &original).await.unwrap());
    let (_, input) = state.session_owner.canonical.store().turn_accepted_input(&session,&operation).await.unwrap();
    let source:nomifun_agent_contracts::RemoteBindingVersionRef = serde_json::from_value(input.0["remote_runtime_source"].clone()).unwrap();
    assert_eq!((source.remote_binding_id.as_ref(),source.binding_version),
        (projection.remote_binding_id.as_str(),u64::try_from(projection.binding_version).unwrap()));
    assert!(input.0.get("cron_runtime_overlay").is_none());
    assert!(matches!(super::super::source_browser_retirement::remote_browser_purpose_state(&state.session_owner.canonical,
        &state.remote_repository, &principal.principal_id, &session, &operation, &original).await.unwrap(),
        super::super::source_browser_retirement::RemotePurposeResourceState::Ended),
        "an earlier Browser phase belongs only through its original server-authored Remote source");
    assert!(matches!(super::super::source_browser_retirement::remote_browser_purpose_state(&state.session_owner.canonical,
        &state.remote_repository, &principal.principal_id, &session, &ordinary, &lookalike_target).await.unwrap(),
        super::super::source_browser_retirement::RemotePurposeResourceState::OtherSource));
    let no_host_tasks = Arc::new(NeverInstalledTasks { delegate:services.background_tasks.clone(), spawns:AtomicUsize::new(0), finished:Arc::new(Notify::new()) });
    let never_installed = NomiCoreSessionOwner::new(state.session_owner.canonical.clone(), state.session_owner.runtime_sessions.clone(),
        services.event_bus.clone(), no_host_tasks.clone(), state.session_owner.managed_workspace_root.clone(),
        services.database.pool().clone(), services.creation_service.clone());
    assert!(never_installed.retire_cancelled_remote_browser_once(&state.remote_repository, &principal.principal_id,
        &session, "source-stop", &original).await.unwrap());
    assert_eq!(no_host_tasks.spawns.load(Ordering::SeqCst), 0);
    let wrong = RemoteCancelTarget { expected_binding_version:original.expected_binding_version+1, ..original.clone() };
    assert!(super::super::source_browser_retirement::cancelled_remote_source_proven(&state.session_owner.canonical,
        &state.remote_repository, &principal.principal_id, &session, "source-stop", &wrong).await.is_err());
    let cursor = state.session_owner.canonical.store().current_cursor(&session).await.unwrap();
    run_nomi_core_remote_cancel(state.clone(), &common_owner, RemoteCancelRequestDto {
        agent_session_id:session.as_ref().into(), idempotency_key:"source-stop".into() }).await.unwrap();
    assert_eq!(state.session_owner.canonical.store().current_cursor(&session).await.unwrap(), cursor);
    assert_eq!(model_calls(&model).await, calls);
    services.shutdown_nomi_core_host().await.unwrap(); services.database.close().await;
}
