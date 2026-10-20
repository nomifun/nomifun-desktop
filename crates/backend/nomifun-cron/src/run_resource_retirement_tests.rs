//! Real preparation and durable Cron settlement, with a resource-free Session
//! adapter that records only the exact handoff; it never manufactures a Runtime.
use super::*;
use crate::busy_guard::CronBusyGuard;
use crate::session_port::*;
use nomifun_agent_contracts::{AgentSessionId, AgentTurnOutcome, AgentUserInputRequest};
use nomifun_api_types::ConversationResponse;
use nomifun_db::{SqliteCronRepository, sqlx};
use std::sync::atomic::AtomicUsize;

struct Sessions {
    pool: sqlx::SqlitePool,
    owner: String,
    session: AgentSessionId,
    creates: std::sync::Mutex<Vec<String>>,
    sends: AtomicUsize,
    submitted: std::sync::Mutex<Vec<CronRuntimePreparationRequest>>,
    opening_failure: std::sync::Mutex<Option<AppError>>,
    sent_keys: std::sync::Mutex<std::collections::BTreeSet<String>>,
    delivery: CronTurnDelivery,
    handoffs: std::sync::Mutex<Vec<CronRunResourceRetirement>>,
    handoff_failures: AtomicUsize,
    handoff_gate: std::sync::Mutex<Option<Arc<Gate>>>,
}
#[derive(Default)]
struct Gate {
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

#[async_trait::async_trait]
impl CronSessionPort for Sessions {
    async fn retire_new_conversation_run_resources(
        &self,
        request: &CronRunResourceRetirement,
    ) -> Result<(), AppError> {
        assert!(
            matches!(
                request.finalized_status(),
                Some("ok" | "error" | "cancelled")
            ),
            "the host receives the original writer's sealed proof, independent of deletable job/run rows"
        );
        self.handoffs.lock().unwrap().push(request.clone());
        let gate = self.handoff_gate.lock().unwrap().take();
        if let Some(gate) = gate {
            gate.entered.notify_one();
            gate.release.notified().await;
        }
        if self
            .handoff_failures
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                (remaining > 0).then(|| remaining - 1)
            })
            .is_ok()
        {
            return Err(AppError::Internal(
                "resource observation interrupted".into(),
            ));
        }
        // This adapter owns no physical resources. Production accepts the
        // handoff into its retained original resource owner instead.
        Ok(())
    }
    async fn create_idempotent(
        &self,
        owner: &str,
        _: &str,
        binding: AgentBindingValueDto,
        key: &str,
        _: &str,
    ) -> Result<CronSessionHandle, AppError> {
        if let Some(error) = self.opening_failure.lock().unwrap().take() {
            return Err(error);
        }
        assert_eq!(owner, self.owner);
        assert_eq!(binding, test_support::test_binding());
        self.creates.lock().unwrap().push(key.into());
        sqlx::query("INSERT OR IGNORE INTO agent_sessions(agent_session_id,owner_ref_json,state,title,archived,pinned,agent_binding_json,next_seq,created_at) VALUES(?,?,'live','Cron run',0,0,'{}',1,1)")
            .bind(self.session.as_ref()).bind(serde_json::json!({"principal_kind":"user","principal_id":owner}).to_string())
            .execute(&self.pool).await.unwrap();
        Ok(CronSessionHandle {
            agent_session_id: self.session.clone(),
            workspace: std::env::temp_dir().to_string_lossy().into_owned(),
        })
    }
    async fn get_session(
        &self,
        query: &CronSessionLookup,
    ) -> Result<CronSessionProjection, AppError> {
        assert_eq!(query.owner_id, self.owner);
        assert_eq!(query.agent_session_id, self.session);
        Ok(CronSessionProjection {
            agent_session_id: self.session.clone(),
            owner_id: self.owner.clone(),
            name: "Cron run".into(),
            workspace: std::env::temp_dir().to_string_lossy().into_owned(),
            cron_job_id: None,
            skills: Vec::new(),
            agent_binding: None,
            agent_selection: None,
            model: None,
            agent_label: None,
        })
    }
    async fn prepare_runtime_and_send(
        &self,
        request: CronRuntimePreparationRequest,
    ) -> Result<CronPreparedTurnDelivery, AppError> {
        assert_eq!(request.agent_session_id, self.session);
        assert_eq!(
            request.idempotency_key,
            format!("cron:{}:turn", request.turn.runtime.overlay.cron_job_run_id)
        );
        self.sent_keys
            .lock()
            .unwrap()
            .insert(request.idempotency_key.clone());
        self.submitted.lock().unwrap().push(request);
        self.sends.fetch_add(1, Ordering::SeqCst);
        Ok(CronPreparedTurnDelivery {
            delivery: self.delivery.clone(),
            workspace: std::env::temp_dir().to_string_lossy().into_owned(),
        })
    }
    async fn delivery_result(
        &self,
        query: &CronTurnDeliveryQuery,
    ) -> Result<Option<CronTurnDelivery>, AppError> {
        Ok(self
            .sent_keys
            .lock()
            .unwrap()
            .contains(&query.idempotency_key)
            .then(|| self.delivery.clone()))
    }
    async fn read_turn_receipt(
        &self,
        query: &CronTurnReceiptQuery,
    ) -> Result<CronTurnReceiptState, AppError> {
        Ok(
            if !self
                .sent_keys
                .lock()
                .unwrap()
                .contains(&query.idempotency_key)
            {
                CronTurnReceiptState::Missing
            } else {
                CronTurnReceiptState::Completed(self.delivery.clone())
            },
        )
    }
    async fn reconcile_turn_receipt(
        &self,
        _: &CronTurnReconciliationRequest,
    ) -> Result<CronTurnReconciliation, AppError> {
        Ok(CronTurnReconciliation::ExternalProofRequiredFailClosed)
    }
    async fn list_conversation_responses_for_cron(
        &self,
        _: &CronScheduledSessionLookup,
    ) -> Result<Vec<ConversationResponse>, AppError> {
        panic!("run retirement cannot discover another Session")
    }
}

struct Tasks {
    handles: std::sync::Mutex<Vec<JoinHandle<()>>>,
    shutdown: watch::Sender<bool>,
}
impl Default for Tasks {
    fn default() -> Self {
        Self {
            handles: Default::default(),
            shutdown: watch::channel(false).0,
        }
    }
}
impl CronBackgroundTaskRegistrar for Tasks {
    fn register(&self, task: JoinHandle<()>) {
        self.handles.lock().unwrap().push(task);
    }
    fn shutdown_signal(&self) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        let mut shutdown = self.shutdown.subscribe();
        Box::pin(async move {
            while !*shutdown.borrow_and_update() {
                if shutdown.changed().await.is_err() {
                    break;
                }
            }
        })
    }
}
impl Tasks {
    async fn join(&self) {
        loop {
            let handles = std::mem::take(&mut *self.handles.lock().unwrap());
            if handles.is_empty() {
                return;
            }
            for handle in handles {
                handle.await.unwrap();
            }
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
        sqlx::query("INSERT INTO cron_jobs(cron_job_id,user_id,name,schedule_kind,schedule_value,payload_message,execution_mode,agent_binding,created_by,created_at,updated_at) VALUES(?,?,'Resource scope','every','60000','Original input',?,?,'user',1,1)")
            .bind(&job_id).bind(&owner).bind(mode.as_str()).bind(serde_json::to_string(&test_support::test_binding()).unwrap())
            .execute(db.pool()).await.unwrap();
        let repo = Arc::new(SqliteCronRepository::new(db.pool().clone()));
        let job = cron_job_from_row(
            repo.get_by_cron_job_id(&owner, &job_id)
                .await
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        let sessions = Arc::new(Sessions {
            pool: db.pool().clone(),
            owner: owner.clone(),
            session: ConversationId::new().into_string().into(),
            creates: Default::default(),
            sends: AtomicUsize::new(0),
            submitted: Default::default(),
            opening_failure: Default::default(),
            sent_keys: Default::default(),
            delivery,
            handoffs: Default::default(),
            handoff_failures: AtomicUsize::new(0),
            handoff_gate: Default::default(),
        });
        let service = CronService::new(
            Arc::from(owner),
            repo.clone(),
            Arc::new(CronScheduler::new(Arc::new(|_, _, _, _, _| {}))),
            Arc::new(JobExecutor::new(
                Arc::from(job.user_id.as_str()),
                sessions.clone(),
                Arc::new(CronBusyGuard::new()),
                PathBuf::new(),
            )),
            CronEventEmitter::new(Arc::new(nomifun_realtime::BroadcastEventBus::new(16))),
            PathBuf::new(),
        );
        let tasks = Arc::new(Tasks::default());
        service.with_cron_background_task_registrar(tasks.clone());
        Self {
            _db: db,
            service,
            repo,
            sessions,
            tasks,
            job,
        }
    }
    async fn prepare(&self) -> (String, crate::executor::PreparedExecution) {
        self.prepare_for(None).await
    }
    async fn prepare_for(
        &self,
        planned: Option<i64>,
    ) -> (String, crate::executor::PreparedExecution) {
        let run = CronJobRunId::new().into_string();
        if let Some(planned) = planned {
            sqlx::query("UPDATE cron_jobs SET next_run_at=? WHERE cron_job_id=?")
                .bind(planned)
                .bind(&self.job.cron_job_id)
                .execute(self._db.pool())
                .await
                .unwrap();
        }
        self.repo
            .reserve_run(
                &self.job.user_id,
                &ReserveCronRunParams {
                    cron_job_run_id: run.clone(),
                    cron_job_id: self.job.cron_job_id.clone(),
                    trigger_kind: if planned.is_some() {
                        "scheduled".into()
                    } else {
                        "run_now".into()
                    },
                    operation_key: format!("original:{run}"),
                    request_fingerprint: "original".into(),
                    schedule_revision: planned.map(|_| self.job.schedule_revision),
                    planned_at_ms: planned,
                    next_run_at: planned.map(|value| value + 60000),
                    disable: false,
                    now: 1,
                },
            )
            .await
            .unwrap();
        let prepared = self
            .service
            .executor
            .prepare_run_now(&self.job, &run)
            .await
            .unwrap();
        self.service
            .attach_prepared_session(&self.job, &run, &prepared.conversation_id)
            .await
            .unwrap();
        (run, prepared)
    }
    async fn join(&self) {
        tokio::time::timeout(std::time::Duration::from_secs(3), self.tasks.join())
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn run_resource_retirement_follows_the_original_scheduled_occurrence() {
    let fixture = Fixture::new(ExecutionMode::NewConversation, answered()).await;
    let planned = 60_001;
    let (run, prepared) = fixture.prepare_for(Some(planned)).await;
    let request = prepared.resource_retirement.clone().unwrap();
    let session = prepared.conversation_id.clone();
    let result = fixture
        .service
        .executor
        .execute_prepared(&fixture.job, prepared)
        .await;
    fixture
        .service
        .complete_prepared_run(
            fixture.job.clone(),
            run.clone(),
            Some(planned),
            session,
            result,
            Some(request.clone()),
        )
        .await;
    fixture.join().await;
    let handoffs = fixture.sessions.handoffs.lock().unwrap();
    assert_eq!(handoffs.len(), 1);
    assert_eq!(
        (
            handoffs[0].owner_id.as_str(),
            handoffs[0].cron_job_id.as_str(),
            handoffs[0].cron_job_run_id.as_str(),
            &handoffs[0].agent_session_id
        ),
        (
            request.owner_id.as_str(),
            request.cron_job_id.as_str(),
            request.cron_job_run_id.as_str(),
            &request.agent_session_id
        )
    );
    assert_eq!(handoffs[0].finalized_status(), Some("ok"));
    drop(handoffs);
    assert_eq!(fixture.sessions.sends.load(Ordering::SeqCst), 1);
    let saved = fixture
        .repo
        .get_run_reservation(&fixture.job.user_id, &fixture.job.cron_job_id, &run)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (
            saved.trigger_kind.as_str(),
            saved.schedule_revision,
            saved.planned_at_ms,
            saved.status.as_str()
        ),
        ("scheduled", Some(1), Some(planned), "ok")
    );
    fixture.service.scheduler.cancel_all();
}

fn delivery(outcome: AgentTurnOutcome, ok: bool) -> CronTurnDelivery {
    CronTurnDelivery {
        message_id: "0190f5fe-7c00-7a00-8000-000000000011".into(),
        replayed: false,
        completed: true,
        turn_outcome: Some(outcome),
        user_input_request: None,
        result_ok: Some(ok),
        result_text: Some("Original result".into()),
        result_error: (!ok).then(|| "Original failure".into()),
        result_error_code: None,
        result_error_retryable: Some(false),
    }
}
fn answered() -> CronTurnDelivery {
    delivery(
        AgentTurnOutcome::Answered {
            message_ids: vec!["answer".into()],
        },
        true,
    )
}

#[tokio::test]
async fn run_resource_retirement_captures_only_the_original_new_conversation_creation() {
    for mode in [ExecutionMode::NewConversation, ExecutionMode::Existing] {
        let mut fixture = Fixture::new(mode, answered()).await;
        let (run, prepared) = fixture.prepare().await;
        assert_eq!(
            fixture.sessions.creates.lock().unwrap().as_slice(),
            &[if mode == ExecutionMode::Existing {
                format!("cron:{}:continuing-conversation", fixture.job.cron_job_id)
            } else {
                format!("cron:{run}:conversation")
            }]
        );
        fixture.job.execution_mode = if mode == ExecutionMode::NewConversation {
            ExecutionMode::Existing
        } else {
            ExecutionMode::NewConversation
        };
        sqlx::query("UPDATE cron_jobs SET execution_mode=? WHERE cron_job_id=?")
            .bind(fixture.job.execution_mode.as_str())
            .bind(&fixture.job.cron_job_id)
            .execute(fixture._db.pool())
            .await
            .unwrap();
        if mode == ExecutionMode::NewConversation {
            let request = prepared.resource_retirement.as_ref().unwrap();
            assert_eq!(
                request.finalized_status(),
                None,
                "preparation cannot mint terminal resource authority"
            );
            assert_eq!(
                (
                    request.owner_id.as_str(),
                    request.cron_job_id.as_str(),
                    request.cron_job_run_id.as_str(),
                    request.agent_session_id.as_ref()
                ),
                (
                    fixture.job.user_id.as_str(),
                    fixture.job.cron_job_id.as_str(),
                    run.as_str(),
                    prepared.conversation_id.as_str()
                )
            );
        } else {
            assert!(
                prepared.resource_retirement.is_none(),
                "a continuing lazy Session cannot become disposable through a later job edit"
            );
        }
        let session = prepared.conversation_id.clone();
        fixture
            .service
            .complete_prepared_run(
                fixture.job.clone(),
                run,
                None,
                session,
                ExecutionResult::Error {
                    failure: nomifun_api_types::AgentStreamErrorData::legacy(
                        "Original failure",
                        None,
                    ),
                },
                prepared.resource_retirement,
            )
            .await;
        fixture.join().await;
        assert_eq!(
            fixture.sessions.handoffs.lock().unwrap().len(),
            usize::from(mode == ExecutionMode::NewConversation)
        );
        assert_eq!(fixture.sessions.sends.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn run_resource_retirement_follows_formal_success_failure_and_cancellation_without_redelivery()
 {
    for (receipt, expected) in [
        (answered(), "ok"),
        (
            delivery(
                AgentTurnOutcome::Failed {
                    message_ids: vec![],
                },
                false,
            ),
            "error",
        ),
        (
            delivery(
                AgentTurnOutcome::Cancelled {
                    message_ids: vec![],
                },
                false,
            ),
            "cancelled",
        ),
    ] {
        let fixture = Fixture::new(ExecutionMode::NewConversation, receipt).await;
        let response = fixture
            .service
            .run_now(
                &fixture.job.user_id,
                &fixture.job.cron_job_id,
                "same-delivery",
            )
            .await
            .unwrap();
        fixture.join().await;
        let replay = fixture
            .service
            .run_now(
                &fixture.job.user_id,
                &fixture.job.cron_job_id,
                "same-delivery",
            )
            .await
            .unwrap();
        assert_eq!(response.conversation_id, replay.conversation_id);
        assert_eq!(fixture.sessions.creates.lock().unwrap().len(), 1);
        assert_eq!(fixture.sessions.sends.load(Ordering::SeqCst), 1);
        let requests = fixture.sessions.handoffs.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].finalized_status(), Some(expected));
        assert_eq!(
            requests[0].agent_session_id.as_ref(),
            response.conversation_id
        );
        let saved = fixture
            .repo
            .get_run_reservation(
                &fixture.job.user_id,
                &fixture.job.cron_job_id,
                &requests[0].cron_job_run_id,
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.status, expected);
        assert_eq!(saved.job_projection_state, "applied");
        assert_eq!(
            saved.conversation_id.as_deref(),
            Some(response.conversation_id.as_str())
        );
    }
}

#[tokio::test]
async fn run_resource_retirement_preserves_input_handoff_and_quarantined_missions() {
    let mut question = delivery(
        AgentTurnOutcome::InputRequired {
            request_id: "ask".into(),
        },
        true,
    );
    question.user_input_request = Some(AgentUserInputRequest {
        request_id: "ask".into(),
        question: "Choose input".into(),
        remaining_work: "Finish task".into(),
    });
    let fixture = Fixture::new(ExecutionMode::NewConversation, question).await;
    fixture
        .service
        .run_now(
            &fixture.job.user_id,
            &fixture.job.cron_job_id,
            "input-handoff",
        )
        .await
        .unwrap();
    fixture.join().await;
    assert_eq!(
        fixture
            .repo
            .get_by_cron_job_id(&fixture.job.user_id, &fixture.job.cron_job_id)
            .await
            .unwrap()
            .unwrap()
            .last_status
            .as_deref(),
        Some("input_required")
    );
    assert!(fixture.sessions.handoffs.lock().unwrap().is_empty());

    let fixture = Fixture::new(ExecutionMode::NewConversation, answered()).await;
    let (run, prepared) = fixture.prepare().await;
    // An unknown receipt cannot borrow even a racing external finalizer's
    // terminal reservation to declare its mission finished.
    assert!(
        fixture
            .service
            .finalize_run_once(
                &fixture.job,
                &run,
                "cancelled",
                Some(&prepared.conversation_id),
                None,
                cancelled_run_projection()
            )
            .await
    );
    fixture
        .service
        .complete_prepared_run(
            fixture.job.clone(),
            run,
            None,
            prepared.conversation_id,
            ExecutionResult::Quarantined {
                message: "Original native outcome is unknown".into(),
            },
            prepared.resource_retirement,
        )
        .await;
    fixture.join().await;
    assert!(fixture.sessions.handoffs.lock().unwrap().is_empty());
    assert_eq!(fixture.sessions.sends.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn run_resource_retirement_keeps_an_already_applied_handoff_after_its_observer_leaves() {
    let fixture = Fixture::new(ExecutionMode::NewConversation, answered()).await;
    let (run, prepared) = fixture.prepare().await;
    assert!(
        fixture
            .service
            .finalize_run_once(
                &fixture.job,
                &run,
                "error",
                Some(&prepared.conversation_id),
                Some("Original failure"),
                error_run_projection("Original failure")
            )
            .await
    );
    fixture.sessions.handoff_failures.store(1, Ordering::SeqCst);
    let gate = Arc::new(Gate::default());
    *fixture.sessions.handoff_gate.lock().unwrap() = Some(gate.clone());
    let entered = gate.entered.notified();
    let job_gate = fixture.service.job_gate(&fixture.job.cron_job_id);
    let ordinary = job_gate.lock().await;
    let observer = tokio::spawn({
        let service = fixture.service.clone();
        let job = fixture.job.clone();
        async move {
            service
                .complete_prepared_run(
                    job,
                    run,
                    None,
                    prepared.conversation_id,
                    ExecutionResult::Error {
                        failure: nomifun_api_types::AgentStreamErrorData::legacy(
                            "Original failure",
                            None,
                        ),
                    },
                    prepared.resource_retirement,
                )
                .await
        }
    });
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while fixture.tasks.handles.lock().unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        !observer.is_finished(),
        "the caller is still observing the retained finalizer behind the ordinary gate"
    );
    observer.abort();
    drop(ordinary);
    tokio::time::timeout(std::time::Duration::from_secs(1), entered)
        .await
        .unwrap();
    gate.release.notify_one();
    fixture.join().await;
    let requests = fixture.sessions.handoffs.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0], requests[1]);
    assert_eq!(fixture.sessions.creates.lock().unwrap().len(), 1);
    assert_eq!(fixture.sessions.sends.load(Ordering::SeqCst), 0);
    assert_eq!(
        fixture
            .repo
            .get_by_cron_job_id(&fixture.job.user_id, &fixture.job.cron_job_id)
            .await
            .unwrap()
            .unwrap()
            .run_count,
        1
    );
}

#[tokio::test]
async fn run_resource_retirement_yields_retry_responsibility_to_application_shutdown() {
    let fixture = Fixture::new(ExecutionMode::NewConversation, answered()).await;
    fixture
        .sessions
        .handoff_failures
        .store(usize::MAX, Ordering::SeqCst);
    fixture
        .service
        .run_now(&fixture.job.user_id, &fixture.job.cron_job_id, "shutdown")
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while fixture.sessions.handoffs.lock().unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
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
    let result = fixture
        .service
        .executor
        .execute_prepared(&fixture.job, prepared)
        .await;
    // This is the same formal finalizer used inside the retained live owner.
    // Pause at its returned immutable receipt, before any Port handoff occurs.
    let proof = fixture
        .service
        .finalize_prepared_run(
            fixture.job.clone(),
            run.clone(),
            None,
            session.clone(),
            result,
            captured,
        )
        .await
        .unwrap();
    assert_eq!(proof.finalized_status(), Some("ok"));
    assert!(fixture.sessions.handoffs.lock().unwrap().is_empty());
    fixture
        .repo
        .delete(&fixture.job.user_id, &fixture.job.cron_job_id)
        .await
        .unwrap();
    assert!(
        fixture
            .repo
            .get_run_reservation(&fixture.job.user_id, &fixture.job.cron_job_id, &run)
            .await
            .unwrap()
            .is_none()
    );
    fixture
        .service
        .handoff_finished_run_resources(proof.clone())
        .await;
    assert_eq!(
        fixture.sessions.handoffs.lock().unwrap().as_slice(),
        &[proof]
    );
    assert_eq!(fixture.sessions.sends.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.sessions.creates.lock().unwrap().len(), 1);
    assert!(
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM agent_sessions WHERE agent_session_id=?)"
        )
        .bind(session)
        .fetch_one(fixture._db.pool())
        .await
        .unwrap(),
        "legal job removal preserves canonical Session history"
    );
}

#[tokio::test]
async fn run_resource_retirement_retries_the_same_sealed_proof_after_first_handoff_failure_and_delete()
 {
    let fixture = Fixture::new(ExecutionMode::NewConversation, answered()).await;
    let (run, prepared) = fixture.prepare().await;
    let captured = prepared.resource_retirement.clone();
    let session = prepared.conversation_id.clone();
    let result = fixture
        .service
        .executor
        .execute_prepared(&fixture.job, prepared)
        .await;
    let proof = fixture
        .service
        .finalize_prepared_run(
            fixture.job.clone(),
            run.clone(),
            None,
            session.clone(),
            result,
            captured,
        )
        .await
        .unwrap();
    fixture.sessions.handoff_failures.store(1, Ordering::SeqCst);
    let handoff = fixture
        .service
        .handoff_finished_run_resources(proof.clone());
    tokio::pin!(handoff);
    // Poll the actual retained retry future through the first Err and into
    // its backoff. Keep that same future live, without polling it while the
    // legal deletion commits; no sleep timing decides the race.
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        std::future::poll_fn(|context| {
            assert!(handoff.as_mut().poll(context).is_pending());
            if fixture.sessions.handoff_failures.load(Ordering::SeqCst) == 0
                && fixture.sessions.handoffs.lock().unwrap().len() == 1
            {
                std::task::Poll::Ready(())
            } else {
                std::task::Poll::Pending
            }
        }),
    )
    .await
    .unwrap();
    assert_eq!(proof.finalized_status(), Some("ok"));
    fixture
        .repo
        .delete(&fixture.job.user_id, &fixture.job.cron_job_id)
        .await
        .unwrap();
    assert!(
        fixture
            .repo
            .get_run_reservation(&fixture.job.user_id, &fixture.job.cron_job_id, &run)
            .await
            .unwrap()
            .is_none()
    );
    tokio::time::timeout(std::time::Duration::from_secs(1), &mut handoff)
        .await
        .unwrap();
    assert_eq!(
        fixture.sessions.handoffs.lock().unwrap().as_slice(),
        &[proof.clone(), proof]
    );
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
    let result = fixture
        .service
        .executor
        .execute_prepared(&fixture.job, prepared)
        .await;
    let proof = fixture
        .service
        .finalize_prepared_run(fixture.job.clone(), run, None, session, result, captured)
        .await
        .unwrap();
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
        fixture
            .service
            .handoff_finished_run_resources(changed)
            .await;
    }
    assert!(fixture.sessions.handoffs.lock().unwrap().is_empty());
}

#[tokio::test]
async fn run_resource_retirement_cannot_seal_a_mismatched_already_finalized_outcome() {
    let fixture = Fixture::new(ExecutionMode::NewConversation, answered()).await;
    let (run, prepared) = fixture.prepare().await;
    assert!(
        fixture
            .service
            .finalize_run_once(
                &fixture.job,
                &run,
                "cancelled",
                Some(&prepared.conversation_id),
                None,
                cancelled_run_projection()
            )
            .await
    );
    let proof = fixture
        .service
        .finalize_prepared_run(
            fixture.job.clone(),
            run.clone(),
            None,
            prepared.conversation_id,
            ExecutionResult::Error {
                failure: nomifun_api_types::AgentStreamErrorData::legacy(
                    "A stale caller cannot replace the formal winner",
                    None,
                ),
            },
            prepared.resource_retirement,
        )
        .await;
    assert!(proof.is_none());
    assert!(fixture.sessions.handoffs.lock().unwrap().is_empty());
    let saved = fixture
        .repo
        .get_run_reservation(&fixture.job.user_id, &fixture.job.cron_job_id, &run)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.status, "cancelled");
    assert_eq!(saved.result_error, None);
    assert_eq!(fixture.sessions.sends.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn continuing_session_survives_first_turn_failure_and_reuses_the_original_opening() {
    let fixture = Fixture::new(
        ExecutionMode::Existing,
        delivery(AgentTurnOutcome::Unavailable, false),
    )
    .await;
    let first = fixture
        .service
        .run_now(&fixture.job.user_id, &fixture.job.cron_job_id, "first")
        .await
        .unwrap();
    fixture.join().await;
    let row = fixture
        .repo
        .get_by_cron_job_id(&fixture.job.user_id, &fixture.job.cron_job_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        row.conversation_id.as_deref(),
        Some(first.conversation_id.as_str())
    );
    assert!(row.agent_binding.is_none());
    assert_eq!(row.last_status.as_deref(), Some("error"));
    let second = fixture
        .service
        .run_now(&fixture.job.user_id, &fixture.job.cron_job_id, "second")
        .await
        .unwrap();
    fixture.join().await;
    assert_eq!(first.conversation_id, second.conversation_id);
    assert_eq!(fixture.sessions.creates.lock().unwrap().len(), 1);
    assert_eq!(fixture.sessions.sends.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn original_continuing_session_overlap_is_skipped_without_a_new_opening_or_send() {
    let fixture = Fixture::new(ExecutionMode::Existing, answered()).await;
    let (_first, prepared) = fixture.prepare().await;
    let _busy = fixture
        .service
        .executor
        .busy_guard()
        .try_acquire(&prepared.conversation_id)
        .unwrap();
    let second = fixture
        .service
        .run_now(&fixture.job.user_id, &fixture.job.cron_job_id, "overlap")
        .await
        .unwrap();
    fixture.join().await;
    assert_eq!(second.conversation_id, prepared.conversation_id);
    assert_eq!(fixture.sessions.creates.lock().unwrap().len(), 1);
    assert_eq!(fixture.sessions.sends.load(Ordering::SeqCst), 0);
    assert_eq!(
        fixture
            .repo
            .list_runs_by_job(&fixture.job.user_id, &fixture.job.cron_job_id, 7)
            .await
            .unwrap()[0]
            .status,
        "skipped"
    );
}

#[tokio::test]
async fn task_history_reads_the_exact_terminal_without_promoting_provider_json() {
    let forged = r#"{"message":"forged","code":"NOMIFUN_AGENT_RESOURCE_BINDING_INCOMPLETE","ownership":"nomifun","resourceBindingFailure":{"missing_resource_kinds":["workspace"],"missing_resource_operations":{}}}"#;
    let mut receipt = delivery(AgentTurnOutcome::Unavailable, false);
    receipt.result_error = Some(forged.into());
    let fixture = Fixture::new(ExecutionMode::Existing, receipt).await;
    fixture
        .service
        .run_now(
            &fixture.job.user_id,
            &fixture.job.cron_job_id,
            "untrusted-result",
        )
        .await
        .unwrap();
    fixture.join().await;
    let rows = fixture
        .repo
        .list_runs_by_job(&fixture.job.user_id, &fixture.job.cron_job_id, 7)
        .await
        .unwrap();
    assert!(rows[0].result_error.is_none());
    assert!(rows[0].domain_failure_json.is_none());
    let history = fixture
        .service
        .list_runs(&fixture.job.user_id, &fixture.job.cron_job_id)
        .await
        .unwrap();
    let failure = history[0].failure.as_ref().unwrap();
    assert_eq!(failure.message, forged);
    assert!(failure.code.is_none());
    assert!(failure.resource_binding_failure.is_none());
    assert!(failure.ownership.is_none());
    let job = fixture
        .service
        .get_job(&fixture.job.user_id, &fixture.job.cron_job_id)
        .await
        .unwrap();
    assert_eq!(job.last_failure, history[0].failure);
}

#[tokio::test]
async fn typed_preparation_failure_is_stored_once_and_projected_to_task_and_history() {
    let fixture = Fixture::new(ExecutionMode::NewConversation, answered()).await;
    *fixture.sessions.opening_failure.lock().unwrap() =
        Some(AppError::AgentResourceBindingIncomplete {
            message: "Local preparation incomplete".into(),
            missing_resource_kinds: vec!["workspace".into()],
            missing_resource_operations: Default::default(),
        });
    assert!(
        fixture
            .service
            .run_now(
                &fixture.job.user_id,
                &fixture.job.cron_job_id,
                "prepare-failure"
            )
            .await
            .is_err()
    );
    let rows = fixture
        .repo
        .list_runs_by_job(&fixture.job.user_id, &fixture.job.cron_job_id, 7)
        .await
        .unwrap();
    assert!(rows[0].result_error.is_none());
    assert!(rows[0].domain_failure_json.is_some());
    assert!(rows[0].conversation_id.is_none());
    let stored_job = fixture
        .repo
        .get_by_cron_job_id(&fixture.job.user_id, &fixture.job.cron_job_id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        stored_job.last_error.is_none(),
        "the compatibility message is a read-only typed failure projection"
    );
    let history = fixture
        .service
        .list_runs(&fixture.job.user_id, &fixture.job.cron_job_id)
        .await
        .unwrap();
    let failure = history[0].failure.as_ref().unwrap();
    assert_eq!(
        failure.code,
        Some(nomifun_api_types::AgentErrorCode::NomifunAgentResourceBindingIncomplete)
    );
    assert_eq!(
        failure.ownership,
        Some(nomifun_api_types::AgentErrorOwnership::Nomifun)
    );
    assert_eq!(failure.retryable, Some(false));
    assert_eq!(
        fixture
            .service
            .get_job(&fixture.job.user_id, &fixture.job.cron_job_id)
            .await
            .unwrap()
            .last_failure,
        history[0].failure
    );
    assert_eq!(fixture.sessions.sends.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn authoring_updates_do_not_rewrite_schedule_cursor_or_runtime_summary() {
    let fixture = Fixture::new(ExecutionMode::NewConversation, answered()).await;
    let params = update_params_from_job(&fixture.job).unwrap();
    assert!(params.next_run_at.is_none());
    assert!(params.last_run_at.is_none());
    assert!(params.last_status.is_none());
    assert!(params.last_error.is_none());
    assert!(params.run_count.is_none());
}

#[tokio::test]
async fn revising_an_exact_binding_preserves_unknown_author_selection_and_does_not_persist_display_model()
 {
    use nomifun_agent_contracts::*;
    use std::collections::{BTreeMap, BTreeSet};
    struct Resolver {
        binding: AgentBindingValueDto,
        revisions: AtomicUsize,
    }
    #[async_trait::async_trait]
    impl CronAgentBindingResolver for Resolver {
        async fn binding_presentation(
            &self,
            _: &str,
            _: &AgentBindingValueDto,
        ) -> Result<CronAgentBindingPresentation, AppError> {
            Ok(CronAgentBindingPresentation {
                agent_label: Some("Original saved Agent".into()),
                model: Some(AgentChatModelSelectionDto {
                    provider_id: "0190f5fe-7c00-7a00-8000-000000000081".into(),
                    model: "display-only".into(),
                }),
            })
        }
        async fn resolve_binding(
            &self,
            _: &str,
            _: &ProductAgentSelectionDto,
            _: Option<&AgentChatModelSelectionDto>,
            _: Option<&str>,
            _: &[AgentResourceSelectionDto],
        ) -> Result<AgentBindingValueDto, AppError> {
            panic!("model/workspace edits must not resolve a current Agent")
        }
        async fn revise_binding(
            &self,
            _: &str,
            current: &AgentBindingValueDto,
            _: Option<&AgentChatModelSelectionDto>,
            _: Option<&str>,
            resources: Option<&[AgentResourceSelectionDto]>,
        ) -> Result<AgentBindingValueDto, AppError> {
            assert_eq!(current, &self.binding);
            assert!(resources.is_none());
            self.revisions.fetch_add(1, Ordering::SeqCst);
            Ok(current.clone())
        }
    }
    let fixture = Fixture::new(ExecutionMode::NewConversation, answered()).await;
    let mut revision = AgentPresetRevision {
        reference: PresetRevisionRef {
            preset_id: nomifun_common::generate_id().into(),
            revision: 1,
            revision_digest: "".into(),
        },
        payload: AgentPresetRevisionPayload {
            schema_version: "1.0.0".into(),
            model_route_refs: BTreeMap::new(),
            chat_route_records: BTreeMap::new(),
            enabled_capabilities: vec![],
            skill_bindings: vec![],
            context_order: vec![],
            middleware_order: vec![],
            system_role_provider_overrides: BTreeMap::new(),
            persona: "Original persona".into(),
            instructions: "Original instructions".into(),
            starter_prompts: vec![],
            runtime_policy: Default::default(),
        },
        contribution_locks: vec![],
        created_by: fixture.job.user_id.clone().into(),
        created_at_ms: 1,
        reason: None,
    };
    revision.reference.revision_digest = revision.revision_digest().unwrap();
    revision.validate().unwrap();
    let content = ResolvedSnapshotContent {
        schema_version: "1.0.0".into(),
        resolver_version: "1.0.0".into(),
        preset_revision_ref: revision.reference.clone(),
        required_runtime_protocol_version: "1.0.0".into(),
        required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
        runtime_feature_inventory_digest: "a".repeat(64).into(),
        required_runtime_features: BTreeSet::new(),
        compiled_runtime_profile_digest: "b".repeat(64).into(),
        model_route_refs: BTreeMap::new(),
        chat_route_identity: None,
        enabled_capabilities: vec![],
        required_resource_kinds: BTreeSet::new(),
        capability_allowlist: BTreeSet::new(),
        context_order: vec![],
        middleware_order: vec![],
        skill_locks: vec![],
        mcp_tool_locks: vec![],
        resolved_role_providers: BTreeMap::new(),
        canonical_schema_manifest_digest: "c".repeat(64).into(),
        target_contribution_manifest_digest: "d".repeat(64).into(),
    };
    let snapshot = ResolvedSnapshotEnvelope {
        snapshot_ref: ResolvedSnapshotRef {
            snapshot_id: nomifun_common::generate_id().into(),
            snapshot_digest: digest_payload(&content).unwrap(),
        },
        content,
        actor: PrincipalRef {
            principal_kind: "user".into(),
            principal_id: fixture.job.user_id.clone(),
        },
        scene: "test".into(),
        surface: "desktop".into(),
        audience: "owner".into(),
        created_at_ms: 1,
        resolver_run_id: nomifun_common::generate_id().into(),
        availability_evidence_revision: "saved".into(),
    };
    snapshot.validate().unwrap();
    sqlx::query("INSERT INTO agent_presets(preset_id,owner_ref_json,source_json,display_json,current_stable_revision,created_at) VALUES(?,?,'\"user\"','{}',1,1)")
        .bind(revision.reference.preset_id.as_ref()).bind(serde_json::json!({"user_id":fixture.job.user_id}).to_string()).execute(fixture._db.pool()).await.unwrap();
    let mut tx = fixture._db.pool().begin().await.unwrap();
    nomifun_db::insert_agent_preset_revision_tx(&mut tx, &revision, &snapshot)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let binding = AgentBindingValueDto {
        preset_revision_ref: serde_json::from_value(
            serde_json::to_value(revision.reference).unwrap(),
        )
        .unwrap(),
        resolved_snapshot_ref: serde_json::from_value(
            serde_json::to_value(snapshot.snapshot_ref).unwrap(),
        )
        .unwrap(),
        typed_resource_bindings: vec![],
        binding_version: 1,
    };
    sqlx::query("UPDATE cron_jobs SET enabled=0,agent_binding=? WHERE cron_job_id=?")
        .bind(serde_json::to_string(&binding).unwrap())
        .bind(&fixture.job.cron_job_id)
        .execute(fixture._db.pool())
        .await
        .unwrap();
    let resolver = Arc::new(Resolver {
        binding: binding.clone(),
        revisions: AtomicUsize::new(0),
    });
    fixture
        .service
        .with_agent_binding_resolver(resolver.clone());
    let workspace_edit: UpdateCronJobRequest =
        serde_json::from_value(serde_json::json!({"workspace":""})).unwrap();
    let job = fixture
        .service
        .update_job(
            &fixture.job.user_id,
            &fixture.job.cron_job_id,
            workspace_edit,
        )
        .await
        .unwrap();
    assert!(job.agent_selection.is_none());
    assert!(job.model.is_none());
    assert_eq!(job.effective_model.as_ref().unwrap().model, "display-only");
    let stored = fixture
        .repo
        .get_by_cron_job_id(&fixture.job.user_id, &fixture.job.cron_job_id)
        .await
        .unwrap()
        .unwrap();
    assert!(stored.agent_selection.is_none());
    assert!(stored.model_selection.is_none());
    assert_eq!(
        stored.agent_binding,
        Some(serde_json::to_string(&binding).unwrap())
    );
    let requested = AgentChatModelSelectionDto {
        provider_id: "0190f5fe-7c00-7a00-8000-000000000081".into(),
        model: "requested".into(),
    };
    let model_edit: UpdateCronJobRequest =
        serde_json::from_value(serde_json::json!({"model":requested})).unwrap();
    let job = fixture
        .service
        .update_job(&fixture.job.user_id, &fixture.job.cron_job_id, model_edit)
        .await
        .unwrap();
    assert!(job.agent_selection.is_none());
    assert_eq!(job.model, Some(requested));
    assert_eq!(resolver.revisions.load(Ordering::SeqCst), 2);
    let rename: UpdateCronJobRequest =
        serde_json::from_value(serde_json::json!({"name":"Renamed"})).unwrap();
    fixture
        .service
        .update_job(&fixture.job.user_id, &fixture.job.cron_job_id, rename)
        .await
        .unwrap();
    assert_eq!(resolver.revisions.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn last_finite_cron_occurrence_executes_after_its_reservation_exhausts_the_schedule() {
    let fixture = Fixture::new(ExecutionMode::NewConversation, answered()).await;
    let year = chrono::Datelike::year(&chrono::Utc::now()) + 1;
    let schedule = CronSchedule::Cron {
        expr: format!("0 0 0 1 1 * {year}"),
        tz: Some("UTC".into()),
        description: None,
    };
    let planned = compute_next_run(&schedule, now_ms()).unwrap();
    assert!(compute_occurrence_successor(&schedule, planned, now_ms()).is_none());
    let mut job = fixture.job.clone();
    job.schedule = schedule;
    job.next_run_at = Some(planned);
    sqlx::query("UPDATE cron_jobs SET schedule_kind='cron',schedule_value=?,schedule_tz='UTC',next_run_at=? WHERE cron_job_id=?")
        .bind(format!("0 0 0 1 1 * {year}")).bind(planned).bind(&job.cron_job_id).execute(fixture._db.pool()).await.unwrap();
    fixture.service.scheduler.schedule_job(&job);
    let generation = fixture
        .service
        .scheduler
        .current_generation_for(
            &job.cron_job_id,
            &job.user_id,
            job.schedule_revision,
            planned,
        )
        .unwrap();
    fixture
        .service
        .tick_occurrence_with_generation(
            &job.user_id,
            &job.cron_job_id,
            job.schedule_revision,
            planned,
            generation,
        )
        .await;
    fixture.join().await;
    assert_eq!(fixture.sessions.sends.load(Ordering::SeqCst), 1);
    let stored = fixture
        .repo
        .get_by_cron_job_id(&job.user_id, &job.cron_job_id)
        .await
        .unwrap()
        .unwrap();
    assert!(!stored.enabled);
    assert!(stored.next_run_at.is_none());
    assert_eq!(stored.last_status.as_deref(), Some("ok"));
    let history = fixture
        .repo
        .list_runs_by_job(&job.user_id, &job.cron_job_id, 7)
        .await
        .unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].status, "ok");
}

#[tokio::test]
async fn scheduled_turn_handoff_preserves_original_task_skills_and_exact_source_in_every_mode() {
    struct DataDir(PathBuf);
    impl Drop for DataDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    for (mode, body, with_skill) in [
        (
            ExecutionMode::NewConversation,
            "  请保存今天的天气预报。\n",
            false,
        ),
        (
            ExecutionMode::Existing,
            "1. Create weather.txt.\n2. Read its contents.\n",
            false,
        ),
        (ExecutionMode::NewConversation, "", true),
    ] {
        let mut fixture = Fixture::new(mode, answered()).await;
        let directory = DataDir(
            std::env::temp_dir().join(format!("nomifun-cron-raw-turn-{}", CronJobId::new())),
        );
        fixture.service.data_dir = directory.0.clone();
        fixture.service.executor = Arc::new(JobExecutor::new(
            Arc::from(fixture.job.user_id.as_str()),
            fixture.sessions.clone(),
            Arc::new(CronBusyGuard::new()),
            directory.0.clone(),
        ));
        let skill = with_skill.then(|| {
            crate::skill_file::build_skill_content(
                "Weather report",
                "Save the daily report",
                "Save the report using the file tools.",
                None,
            )
        });
        sqlx::query("UPDATE cron_jobs SET payload_message=?,skill_content=? WHERE cron_job_id=?")
            .bind(body)
            .bind(skill.as_deref())
            .bind(&fixture.job.cron_job_id)
            .execute(fixture._db.pool())
            .await
            .unwrap();
        let response = fixture
            .service
            .run_now(&fixture.job.user_id, &fixture.job.cron_job_id, "raw-task")
            .await
            .unwrap();
        fixture.join().await;
        let run: String = sqlx::query_scalar("SELECT cron_job_run_id FROM cron_run_reservations WHERE cron_job_id=? AND operation_key=?")
            .bind(&fixture.job.cron_job_id).bind(format!("cron:run-now:{}:raw-task", fixture.job.user_id)).fetch_one(fixture._db.pool()).await.unwrap();
        assert_eq!(response.conversation_id, fixture.sessions.session.as_ref());
        let submitted = fixture.sessions.submitted.lock().unwrap();
        assert_eq!(submitted.len(), 1);
        let request = &submitted[0];
        assert_eq!(request.owner_id, fixture.job.user_id);
        assert_eq!(request.agent_session_id, fixture.sessions.session);
        assert_eq!(
            request.turn.runtime.overlay.cron_job_id,
            fixture.job.cron_job_id
        );
        assert_eq!(request.turn.runtime.overlay.cron_job_run_id, run);
        assert_eq!(
            request.turn.message.content, body,
            "server framing must not become a user task or numbered delivery obligation"
        );
        assert_eq!(
            request.turn.message.inject_skills,
            if with_skill {
                vec![fixture.job.cron_job_id.clone()]
            } else {
                Vec::new()
            }
        );
        assert_eq!(request.turn.message.origin.as_deref(), Some("cron"));
        assert!(request.turn.message.hidden);
        assert!(request.turn.message.files.is_empty());
        assert!(request.turn.message.channel_platform.is_none());
        assert_eq!(request.idempotency_key, format!("cron:{}:turn", run));
    }
}
