//! Real preparation and durable Cron settlement, with a resource-free Session
//! adapter that records only the exact handoff; it never manufactures a Runtime.
use super::*;
use crate::busy_guard::CronBusyGuard;
use crate::session_port::*;
use nomifun_agent_contracts::{AgentSessionId, AgentTurnOutcome, AgentUserInputRequest};
use nomifun_api_types::{ConversationResponse, CreateConversationRequest};
use nomifun_db::{SqliteCronRepository, sqlx};
use std::sync::atomic::AtomicUsize;

struct Sessions {
    pool: sqlx::SqlitePool,
    owner: String,
    session: AgentSessionId,
    creates: std::sync::Mutex<Vec<String>>,
    sends: AtomicUsize,
    delivery: CronTurnDelivery,
    handoffs: std::sync::Mutex<Vec<CronRunResourceRetirement>>,
    handoff_failures: AtomicUsize,
    handoff_gate: std::sync::Mutex<Option<Arc<Gate>>>,
}
#[derive(Default)]
struct Gate { entered: tokio::sync::Notify, release: tokio::sync::Notify }

#[async_trait::async_trait]
impl CronSessionPort for Sessions {
    async fn retire_new_conversation_run_resources(&self, request: &CronRunResourceRetirement) -> Result<(), AppError> {
        assert!(matches!(request.finalized_status(), Some("ok" | "error" | "cancelled")),
            "the host receives the original writer's sealed proof, independent of deletable job/run rows");
        self.handoffs.lock().unwrap().push(request.clone());
        let gate = self.handoff_gate.lock().unwrap().take();
        if let Some(gate) = gate { gate.entered.notify_one(); gate.release.notified().await; }
        if self.handoff_failures.fetch_update(Ordering::SeqCst, Ordering::SeqCst,
            |remaining| (remaining > 0).then(|| remaining - 1)).is_ok() {
            return Err(AppError::Internal("resource observation interrupted".into()));
        }
        // This adapter owns no physical resources. Production accepts the
        // handoff into its retained original resource owner instead.
        Ok(())
    }
    async fn create_idempotent(&self, owner: &str, _: CreateConversationRequest,
        binding: CronSessionAgentBinding, key: &str) -> Result<CronSessionHandle, AppError> {
        assert_eq!(owner, self.owner);
        assert_eq!(binding, CronSessionAgentBinding::ModelOnly);
        self.creates.lock().unwrap().push(key.into());
        sqlx::query("INSERT OR IGNORE INTO agent_sessions(agent_session_id,owner_ref_json,state,title,archived,pinned,agent_binding_json,next_seq,created_at) VALUES(?,?,'live','Cron run',0,0,'{}',1,1)")
            .bind(self.session.as_ref()).bind(serde_json::json!({"principal_kind":"user","principal_id":owner}).to_string())
            .execute(&self.pool).await.unwrap();
        Ok(CronSessionHandle { agent_session_id: self.session.clone(), workspace: std::env::temp_dir().to_string_lossy().into_owned() })
    }
    async fn get_session(&self, query: &CronSessionLookup) -> Result<CronSessionProjection, AppError> {
        assert_eq!(query.owner_id, self.owner);
        assert_eq!(query.agent_session_id, self.session);
        Ok(CronSessionProjection { agent_session_id: self.session.clone(), owner_id: self.owner.clone(), name: "Cron run".into(),
            agent_type: AgentType::Nomi, model: None, workspace: std::env::temp_dir().to_string_lossy().into_owned(),
            cron_job_id: None, temp_workspace_id: None, skills: Vec::new(), agent_name: None, cli_path: None,
            custom_agent_id: None, preset_id: None, preset_revision: None, agent_snapshot: None })
    }
    async fn prepare_runtime_and_send(&self, request: CronRuntimePreparationRequest) -> Result<CronPreparedTurnDelivery, AppError> {
        assert_eq!(request.agent_session_id, self.session);
        assert_eq!(request.idempotency_key, format!("cron:{}:turn", request.turn.runtime.overlay.cron_job_run_id));
        self.sends.fetch_add(1, Ordering::SeqCst);
        Ok(CronPreparedTurnDelivery { delivery: self.delivery.clone(), workspace: std::env::temp_dir().to_string_lossy().into_owned() })
    }
    async fn delivery_result(&self, _: &CronTurnDeliveryQuery) -> Result<Option<CronTurnDelivery>, AppError> {
        Ok((self.sends.load(Ordering::SeqCst) > 0).then(|| self.delivery.clone()))
    }
    async fn read_turn_receipt(&self, _: &CronTurnReceiptQuery) -> Result<CronTurnReceiptState, AppError> {
        Ok(if self.sends.load(Ordering::SeqCst) == 0 { CronTurnReceiptState::Missing }
            else { CronTurnReceiptState::Completed(self.delivery.clone()) })
    }
    async fn reconcile_turn_receipt(&self, _: &CronTurnReconciliationRequest) -> Result<CronTurnReconciliation, AppError> {
        Ok(CronTurnReconciliation::ExternalProofRequiredFailClosed)
    }
    async fn bind_cron_relation(&self, _: &CronSessionCronBindingRequest) -> Result<(), AppError> { Ok(()) }
    async fn list_conversation_responses_for_cron(&self, _: &CronScheduledSessionLookup) -> Result<Vec<ConversationResponse>, AppError> {
        panic!("run retirement cannot discover another Session")
    }
}

struct Tasks {
    handles: std::sync::Mutex<Vec<JoinHandle<()>>>,
    shutdown: watch::Sender<bool>,
}
impl Default for Tasks {
    fn default() -> Self { Self { handles: Default::default(), shutdown: watch::channel(false).0 } }
}
impl CronBackgroundTaskRegistrar for Tasks {
    fn register(&self, task: JoinHandle<()>) { self.handles.lock().unwrap().push(task); }
    fn shutdown_signal(&self) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        let mut shutdown = self.shutdown.subscribe();
        Box::pin(async move { while !*shutdown.borrow_and_update() { if shutdown.changed().await.is_err() { break; } } })
    }
}
impl Tasks {
    async fn join(&self) {
        loop {
            let handles = std::mem::take(&mut *self.handles.lock().unwrap());
            if handles.is_empty() { return; }
            for handle in handles { handle.await.unwrap(); }
        }
    }
}

struct Fixture {
    _db: nomifun_db::Database,
    service: CronService,
    repo: Arc<SqliteCronRepository>,
    sessions: Arc<Sessions>,
    tasks: Arc<Tasks>,
    job: CronJob,
}
impl Fixture {
    async fn new(mode: ExecutionMode, delivery: CronTurnDelivery) -> Self {
        let db = nomifun_db::init_database_memory().await.unwrap();
        let owner = nomifun_db::installation_owner_id(db.pool()).await.unwrap();
        let job_id = CronJobId::new().into_string();
        let config = serde_json::json!({"name":"Nomi","model":"test-model","provider_id":ProviderId::new().into_string()});
        sqlx::query("INSERT INTO cron_jobs(cron_job_id,user_id,name,schedule_kind,schedule_value,payload_message,execution_mode,agent_config,agent_type,created_by,created_at,updated_at) VALUES(?,?,'Resource scope','every','60000','Original input',?,?,?,'user',1,1)")
            .bind(&job_id).bind(&owner).bind(mode.as_str()).bind(config.to_string()).bind(AgentType::Nomi.serde_name())
            .execute(db.pool()).await.unwrap();
        let repo = Arc::new(SqliteCronRepository::new(db.pool().clone()));
        let job = cron_job_from_row(repo.get_by_cron_job_id(&owner, &job_id).await.unwrap().unwrap()).unwrap();
        let sessions = Arc::new(Sessions { pool: db.pool().clone(), owner: owner.clone(), session: ConversationId::new().into_string().into(),
            creates: Default::default(), sends: AtomicUsize::new(0), delivery, handoffs: Default::default(),
            handoff_failures: AtomicUsize::new(0), handoff_gate: Default::default() });
        let service = CronService::new(Arc::from(owner), repo.clone(), Arc::new(CronScheduler::new(Arc::new(|_,_,_,_,_|{}))),
            Arc::new(JobExecutor::new(Arc::from(job.user_id.as_str()), sessions.clone(), Arc::new(CronBusyGuard::new()), PathBuf::new())),
            CronEventEmitter::new(Arc::new(nomifun_realtime::BroadcastEventBus::new(16))), PathBuf::new());
        let tasks = Arc::new(Tasks::default());
        service.with_cron_background_task_registrar(tasks.clone());
        Self { _db: db, service, repo, sessions, tasks, job }
    }
    async fn prepare(&self) -> (String, crate::executor::PreparedExecution) {
        self.prepare_for(None).await
    }
    async fn prepare_for(&self, planned: Option<i64>) -> (String, crate::executor::PreparedExecution) {
        let run = CronJobRunId::new().into_string();
        if let Some(planned) = planned {
            sqlx::query("UPDATE cron_jobs SET next_run_at=? WHERE cron_job_id=?")
                .bind(planned).bind(&self.job.cron_job_id).execute(self._db.pool()).await.unwrap();
        }
        self.repo.reserve_run(&self.job.user_id, &ReserveCronRunParams { cron_job_run_id: run.clone(), cron_job_id: self.job.cron_job_id.clone(),
            trigger_kind: if planned.is_some() { "scheduled".into() } else { "run_now".into() },
            operation_key: format!("original:{run}"), request_fingerprint: "original".into(),
            schedule_revision: planned.map(|_| self.job.schedule_revision), planned_at_ms: planned, now: 1 }).await.unwrap();
        let prepared = self.service.executor.prepare_run_now(&self.job, &run).await.unwrap();
        self.repo.attach_run_conversation(&self.job.user_id, &run, &prepared.conversation_id, 2).await.unwrap();
        (run, prepared)
    }
    async fn join(&self) { tokio::time::timeout(std::time::Duration::from_secs(3), self.tasks.join()).await.unwrap(); }
}

#[tokio::test]
async fn run_resource_retirement_follows_the_original_scheduled_occurrence() {
    let fixture = Fixture::new(ExecutionMode::NewConversation, answered()).await;
    let planned = 60_001;
    let (run, prepared) = fixture.prepare_for(Some(planned)).await;
    let request = prepared.resource_retirement.clone().unwrap();
    let session = prepared.conversation_id.clone();
    let result = fixture.service.executor.execute_prepared(&fixture.job, prepared).await;
    fixture.service.complete_prepared_run(fixture.job.clone(), run.clone(), Some(planned), session, result, Some(request.clone())).await;
    fixture.join().await;
    let handoffs = fixture.sessions.handoffs.lock().unwrap();
    assert_eq!(handoffs.len(), 1);
    assert_eq!((handoffs[0].owner_id.as_str(), handoffs[0].cron_job_id.as_str(), handoffs[0].cron_job_run_id.as_str(), &handoffs[0].agent_session_id),
        (request.owner_id.as_str(), request.cron_job_id.as_str(), request.cron_job_run_id.as_str(), &request.agent_session_id));
    assert_eq!(handoffs[0].finalized_status(), Some("ok"));
    drop(handoffs);
    assert_eq!(fixture.sessions.sends.load(Ordering::SeqCst), 1);
    let saved = fixture.repo.get_run_reservation(&fixture.job.user_id, &fixture.job.cron_job_id, &run).await.unwrap().unwrap();
    assert_eq!((saved.trigger_kind.as_str(), saved.schedule_revision, saved.planned_at_ms, saved.status.as_str()),
        ("scheduled", Some(1), Some(planned), "ok"));
    fixture.service.scheduler.cancel_all();
}

fn delivery(outcome: AgentTurnOutcome, ok: bool) -> CronTurnDelivery {
    CronTurnDelivery { message_id: "0190f5fe-7c00-7a00-8000-000000000011".into(), replayed: false, completed: true,
        turn_outcome: Some(outcome), user_input_request: None, result_ok: Some(ok), result_text: Some("Original result".into()),
        result_error: (!ok).then(|| "Original failure".into()), result_error_code: None, result_error_retryable: Some(false) }
}
fn answered() -> CronTurnDelivery { delivery(AgentTurnOutcome::Answered { message_ids: vec!["answer".into()] }, true) }

#[tokio::test]
async fn run_resource_retirement_captures_only_the_original_new_conversation_creation() {
    for mode in [ExecutionMode::NewConversation, ExecutionMode::Existing] {
        let mut fixture = Fixture::new(mode, answered()).await;
        let (run, prepared) = fixture.prepare().await;
        assert_eq!(fixture.sessions.creates.lock().unwrap().as_slice(), &[format!("cron:{run}:conversation")]);
        fixture.job.execution_mode = if mode == ExecutionMode::NewConversation { ExecutionMode::Existing } else { ExecutionMode::NewConversation };
        sqlx::query("UPDATE cron_jobs SET execution_mode=? WHERE cron_job_id=?")
            .bind(fixture.job.execution_mode.as_str()).bind(&fixture.job.cron_job_id).execute(fixture._db.pool()).await.unwrap();
        if mode == ExecutionMode::NewConversation {
            let request = prepared.resource_retirement.as_ref().unwrap();
            assert_eq!(request.finalized_status(), None, "preparation cannot mint terminal resource authority");
            assert_eq!((request.owner_id.as_str(), request.cron_job_id.as_str(), request.cron_job_run_id.as_str(), request.agent_session_id.as_ref()),
                (fixture.job.user_id.as_str(), fixture.job.cron_job_id.as_str(), run.as_str(), prepared.conversation_id.as_str()));
        } else { assert!(prepared.resource_retirement.is_none(), "a continuing lazy Session cannot become disposable through a later job edit"); }
        let session = prepared.conversation_id.clone();
        fixture.service.complete_prepared_run(fixture.job.clone(), run, None, session,
            ExecutionResult::Error { message: "Original failure".into() }, prepared.resource_retirement).await;
        fixture.join().await;
        assert_eq!(fixture.sessions.handoffs.lock().unwrap().len(), usize::from(mode == ExecutionMode::NewConversation));
        assert_eq!(fixture.sessions.sends.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn run_resource_retirement_follows_formal_success_failure_and_cancellation_without_redelivery() {
    for (receipt, expected) in [(answered(), "ok"),
        (delivery(AgentTurnOutcome::Failed { message_ids: vec![] }, false), "error"),
        (delivery(AgentTurnOutcome::Cancelled { message_ids: vec![] }, false), "cancelled")] {
        let fixture = Fixture::new(ExecutionMode::NewConversation, receipt).await;
        let response = fixture.service.run_now(&fixture.job.user_id, &fixture.job.cron_job_id, "same-delivery").await.unwrap();
        fixture.join().await;
        let replay = fixture.service.run_now(&fixture.job.user_id, &fixture.job.cron_job_id, "same-delivery").await.unwrap();
        assert_eq!(response.conversation_id, replay.conversation_id);
        assert_eq!(fixture.sessions.creates.lock().unwrap().len(), 1);
        assert_eq!(fixture.sessions.sends.load(Ordering::SeqCst), 1);
        let requests = fixture.sessions.handoffs.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].finalized_status(), Some(expected));
        assert_eq!(requests[0].agent_session_id.as_ref(), response.conversation_id);
        let saved = fixture.repo.get_run_reservation(&fixture.job.user_id, &fixture.job.cron_job_id, &requests[0].cron_job_run_id).await.unwrap().unwrap();
        assert_eq!(saved.status, expected);
        assert_eq!(saved.job_projection_state, "applied");
        assert_eq!(saved.conversation_id.as_deref(), Some(response.conversation_id.as_str()));
    }
}

#[tokio::test]
async fn run_resource_retirement_preserves_input_handoff_and_quarantined_missions() {
    let mut question = delivery(AgentTurnOutcome::InputRequired { request_id: "ask".into() }, true);
    question.user_input_request = Some(AgentUserInputRequest { request_id: "ask".into(), question: "Choose input".into(), remaining_work: "Finish task".into() });
    let fixture = Fixture::new(ExecutionMode::NewConversation, question).await;
    fixture.service.run_now(&fixture.job.user_id, &fixture.job.cron_job_id, "input-handoff").await.unwrap();
    fixture.join().await;
    assert_eq!(fixture.repo.get_by_cron_job_id(&fixture.job.user_id, &fixture.job.cron_job_id).await.unwrap().unwrap().last_status.as_deref(), Some("input_required"));
    assert!(fixture.sessions.handoffs.lock().unwrap().is_empty());

    let fixture = Fixture::new(ExecutionMode::NewConversation, answered()).await;
    let (run, prepared) = fixture.prepare().await;
    // An unknown receipt cannot borrow even a racing external finalizer's
    // terminal reservation to declare its mission finished.
    assert!(fixture.service.finalize_run_once(&fixture.job, &run, "cancelled", Some(&prepared.conversation_id), None, cancelled_run_projection()).await);
    fixture.service.complete_prepared_run(fixture.job.clone(), run, None, prepared.conversation_id,
        ExecutionResult::Quarantined { message: "Original native outcome is unknown".into() }, prepared.resource_retirement).await;
    fixture.join().await;
    assert!(fixture.sessions.handoffs.lock().unwrap().is_empty());
    assert_eq!(fixture.sessions.sends.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn run_resource_retirement_keeps_an_already_applied_handoff_after_its_observer_leaves() {
    let fixture = Fixture::new(ExecutionMode::NewConversation, answered()).await;
    let (run, prepared) = fixture.prepare().await;
    assert!(fixture.service.finalize_run_once(&fixture.job, &run, "error", Some(&prepared.conversation_id), Some("Original failure"), error_run_projection("Original failure")).await);
    fixture.sessions.handoff_failures.store(1, Ordering::SeqCst);
    let gate = Arc::new(Gate::default());
    *fixture.sessions.handoff_gate.lock().unwrap() = Some(gate.clone());
    let entered = gate.entered.notified();
    let job_gate = fixture.service.job_gate(&fixture.job.cron_job_id);
    let ordinary = job_gate.lock().await;
    let observer = tokio::spawn({ let service = fixture.service.clone(); let job = fixture.job.clone();
        async move { service.complete_prepared_run(job, run, None, prepared.conversation_id,
            ExecutionResult::Error { message: "Original failure".into() }, prepared.resource_retirement).await } });
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while fixture.tasks.handles.lock().unwrap().is_empty() { tokio::task::yield_now().await; }
    }).await.unwrap();
    assert!(!observer.is_finished(), "the caller is still observing the retained finalizer behind the ordinary gate");
    observer.abort();
    drop(ordinary);
    tokio::time::timeout(std::time::Duration::from_secs(1), entered).await.unwrap();
    gate.release.notify_one();
    fixture.join().await;
    let requests = fixture.sessions.handoffs.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0], requests[1]);
    assert_eq!(fixture.sessions.creates.lock().unwrap().len(), 1);
    assert_eq!(fixture.sessions.sends.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.repo.get_by_cron_job_id(&fixture.job.user_id, &fixture.job.cron_job_id).await.unwrap().unwrap().run_count, 1);
}

#[tokio::test]
async fn run_resource_retirement_yields_retry_responsibility_to_application_shutdown() {
    let fixture = Fixture::new(ExecutionMode::NewConversation, answered()).await;
    fixture.sessions.handoff_failures.store(usize::MAX, Ordering::SeqCst);
    fixture.service.run_now(&fixture.job.user_id, &fixture.job.cron_job_id, "shutdown").await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while fixture.sessions.handoffs.lock().unwrap().is_empty() { tokio::task::yield_now().await; }
    }).await.unwrap();
    fixture.tasks.shutdown.send_replace(true);
    fixture.join().await;
    assert_eq!(fixture.sessions.sends.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.sessions.creates.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn run_resource_retirement_keeps_the_formal_writer_proof_after_delete_before_first_handoff() {
    let fixture = Fixture::new(ExecutionMode::NewConversation, answered()).await;
    let (run, prepared) = fixture.prepare().await;
    let captured = prepared.resource_retirement.clone();
    let session = prepared.conversation_id.clone();
    let result = fixture.service.executor.execute_prepared(&fixture.job, prepared).await;
    // This is the same formal finalizer used inside the retained live owner.
    // Pause at its returned immutable receipt, before any Port handoff occurs.
    let proof = fixture.service.finalize_prepared_run(fixture.job.clone(), run.clone(), None,
        session.clone(), result, captured).await.unwrap();
    assert_eq!(proof.finalized_status(), Some("ok"));
    assert!(fixture.sessions.handoffs.lock().unwrap().is_empty());
    fixture.repo.delete(&fixture.job.user_id, &fixture.job.cron_job_id).await.unwrap();
    assert!(fixture.repo.get_run_reservation(&fixture.job.user_id, &fixture.job.cron_job_id, &run).await.unwrap().is_none());
    fixture.service.handoff_finished_run_resources(proof.clone()).await;
    assert_eq!(fixture.sessions.handoffs.lock().unwrap().as_slice(), &[proof]);
    assert_eq!(fixture.sessions.sends.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.sessions.creates.lock().unwrap().len(), 1);
    assert!(sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM agent_sessions WHERE agent_session_id=?)")
        .bind(session).fetch_one(fixture._db.pool()).await.unwrap(), "legal job removal preserves canonical Session history");
}

#[tokio::test]
async fn run_resource_retirement_retries_the_same_sealed_proof_after_first_handoff_failure_and_delete() {
    let fixture = Fixture::new(ExecutionMode::NewConversation, answered()).await;
    let (run, prepared) = fixture.prepare().await;
    let captured = prepared.resource_retirement.clone();
    let session = prepared.conversation_id.clone();
    let result = fixture.service.executor.execute_prepared(&fixture.job, prepared).await;
    let proof = fixture.service.finalize_prepared_run(fixture.job.clone(), run.clone(), None,
        session.clone(), result, captured).await.unwrap();
    fixture.sessions.handoff_failures.store(1, Ordering::SeqCst);
    let handoff = fixture.service.handoff_finished_run_resources(proof.clone());
    tokio::pin!(handoff);
    // Poll the actual retained retry future through the first Err and into
    // its backoff. Keep that same future live, without polling it while the
    // legal deletion commits; no sleep timing decides the race.
    tokio::time::timeout(std::time::Duration::from_secs(1), std::future::poll_fn(|context| {
        assert!(handoff.as_mut().poll(context).is_pending());
        if fixture.sessions.handoff_failures.load(Ordering::SeqCst) == 0
            && fixture.sessions.handoffs.lock().unwrap().len() == 1 {
            std::task::Poll::Ready(())
        } else { std::task::Poll::Pending }
    })).await.unwrap();
    assert_eq!(proof.finalized_status(), Some("ok"));
    fixture.repo.delete(&fixture.job.user_id, &fixture.job.cron_job_id).await.unwrap();
    assert!(fixture.repo.get_run_reservation(&fixture.job.user_id, &fixture.job.cron_job_id, &run).await.unwrap().is_none());
    tokio::time::timeout(std::time::Duration::from_secs(1), &mut handoff).await.unwrap();
    assert_eq!(fixture.sessions.handoffs.lock().unwrap().as_slice(), &[proof.clone(), proof]);
    assert_eq!(fixture.sessions.sends.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.sessions.creates.lock().unwrap().len(), 1);
    assert_eq!(session, fixture.sessions.session.as_ref());
}

#[tokio::test]
async fn run_resource_retirement_cannot_borrow_a_seal_for_mutated_public_coordinates() {
    let fixture = Fixture::new(ExecutionMode::NewConversation, answered()).await;
    let (run, prepared) = fixture.prepare().await;
    let captured = prepared.resource_retirement.clone();
    assert_eq!(captured.as_ref().unwrap().finalized_status(), None);
    let session = prepared.conversation_id.clone();
    let result = fixture.service.executor.execute_prepared(&fixture.job, prepared).await;
    let proof = fixture.service.finalize_prepared_run(fixture.job.clone(), run, None, session, result, captured).await.unwrap();
    assert_eq!(proof.finalized_status(), Some("ok"));
    for field in 0..4 {
        let mut changed = proof.clone();
        match field {
            0 => changed.owner_id = UserId::new().into_string(),
            1 => changed.cron_job_id = CronJobId::new().into_string(),
            2 => changed.cron_job_run_id = CronJobRunId::new().into_string(),
            _ => changed.agent_session_id = ConversationId::new().into_string().into(),
        }
        assert_eq!(changed.finalized_status(), None);
        fixture.service.handoff_finished_run_resources(changed).await;
    }
    assert!(fixture.sessions.handoffs.lock().unwrap().is_empty());
}

#[tokio::test]
async fn run_resource_retirement_cannot_seal_a_mismatched_already_finalized_outcome() {
    let fixture = Fixture::new(ExecutionMode::NewConversation, answered()).await;
    let (run, prepared) = fixture.prepare().await;
    assert!(fixture.service.finalize_run_once(&fixture.job, &run, "cancelled", Some(&prepared.conversation_id), None,
        cancelled_run_projection()).await);
    let proof = fixture.service.finalize_prepared_run(fixture.job.clone(), run.clone(), None, prepared.conversation_id,
        ExecutionResult::Error { message: "A stale caller cannot replace the formal winner".into() }, prepared.resource_retirement).await;
    assert!(proof.is_none());
    assert!(fixture.sessions.handoffs.lock().unwrap().is_empty());
    let saved = fixture.repo.get_run_reservation(&fixture.job.user_id, &fixture.job.cron_job_id, &run).await.unwrap().unwrap();
    assert_eq!(saved.status, "cancelled");
    assert_eq!(saved.result_error, None);
    assert_eq!(fixture.sessions.sends.load(Ordering::SeqCst), 0);
}
