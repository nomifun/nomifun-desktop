use super::*;
use crate::execution_port::AutoWorkExecutionAdmission;
use crate::events::RequirementEventEmitter;
use nomifun_api_types::CreateRequirementRequest;
use nomifun_realtime::UserEventSink;
use tokio::sync::Semaphore;

const OWNER: &str = "0190f5fe-7c00-7a00-8000-000000000001";

struct NoEvents;
impl UserEventSink for NoEvents {
    fn send_to_user(&self, _: &str, _: nomifun_api_types::WebSocketMessage<serde_json::Value>) {}
}

struct Workspace {
    calls: std::sync::atomic::AtomicUsize,
    block_preparation: bool,
    preparing: Semaphore,
    release: Semaphore,
}
#[async_trait::async_trait]
impl AutoWorkWorkspacePort for Workspace {
    async fn resolve_frozen_workspace(&self, _: &str, _: &str) -> Result<AutoWorkWorkspaceResolution, AppError> {
        // Call zero validates the loop owner. Call one prepares the first
        // selected Requirement, before Execution admission exists.
        if self.calls.fetch_add(1,Ordering::SeqCst)==1 && self.block_preparation {
            self.preparing.add_permits(1);
            self.release.acquire().await.unwrap().forget();
        }
        Ok(AutoWorkWorkspaceResolution::new(None))
    }
}

struct Execution {
    service: Arc<RequirementService>,
    first: String,
    admitted: Mutex<Vec<String>>,
    admitted_first: Semaphore,
    cancelled_first: Semaphore,
    signals: Mutex<Vec<AutoWorkExecutionSource>>,
    cancel_failures: std::sync::atomic::AtomicUsize,
    lost_lease: AtomicBool,
}
#[async_trait::async_trait]
impl AutoWorkExecutionPort for Execution {
    async fn preflight_automation(&self, _: &str, _: &AutoWorkExecutionRequest) -> Result<(),AppError> { Ok(()) }
    async fn admit_automation(&self, _: &str, request: AutoWorkExecutionRequest) -> Result<AutoWorkExecutionAdmission,AppError> {
        // The production repository repeats this predicate in its SQLite
        // writer; this port models its rejected pre-admission race for the
        // real runner's queue-policy behavior.
        if self.service.get(&request.source.requirement_id).await?.status==RequirementStatus::Cancelled {
            return Err(AppError::Conflict("original claim was cancelled before admission".into()));
        }
        self.admitted.lock().unwrap().push(request.source.requirement_id.clone());
        if request.source.requirement_id==self.first { self.admitted_first.add_permits(1); }
        Ok(AutoWorkExecutionAdmission {execution_id:request.source.requirement_id})
    }
    async fn await_automation(&self, _: &str, admission: &AutoWorkExecutionAdmission) -> Result<AutoWorkExecutionReceipt,AppError> {
        if admission.execution_id==self.first {
            self.cancelled_first.acquire().await.unwrap().forget();
            if self.lost_lease.swap(false,Ordering::SeqCst) {return Err(AppError::Conflict("the original queue lease is no longer held".into()));}
            Ok(AutoWorkExecutionReceipt::Cancelled {execution_id:admission.execution_id.clone(),replay_safe:false})
        } else {
            Ok(AutoWorkExecutionReceipt::Completed {execution_id:admission.execution_id.clone(),summary:Some("successor completed".into())})
        }
    }
    async fn cancel_automation(&self, _: &str, source: &AutoWorkExecutionSource) -> Result<Option<AutoWorkExecutionReceipt>,AppError> {
        assert_eq!(self.service.get(&source.requirement_id).await?.status,RequirementStatus::Cancelled,
            "the original durable fence must precede every native/domain signal");
        self.signals.lock().unwrap().push(source.clone());
        let admitted=self.admitted.lock().unwrap().contains(&source.requirement_id);
        if admitted {self.cancelled_first.add_permits(1);}
        if self.cancel_failures.fetch_update(Ordering::SeqCst,Ordering::SeqCst,|remaining|remaining.checked_sub(1)).is_ok() {
            return Err(AppError::Conflict("original cleanup proof is temporarily unavailable".into()));
        }
        if admitted {
            Ok(Some(AutoWorkExecutionReceipt::Cancelled {execution_id:source.requirement_id.clone(),replay_safe:false}))
        } else { Ok(None) }
    }
}

async fn exercise_current_stop(block_preparation: bool,cancel_failures:usize,drop_api_before_signal:bool,abort_loop_owner:bool) {
    let database=nomifun_db::init_database_memory_with_owner(nomifun_common::UserId::parse(OWNER).unwrap()).await.unwrap();
    let session=ConversationId::new().into_string();
    sqlx::query("INSERT INTO agent_sessions(agent_session_id,owner_ref_json,state,title,archived,pinned,agent_binding_json,next_seq,created_at) \
        VALUES(?,json_object('principal_kind','user','principal_id',?),'live','AutoWork exact Stop',0,0,'{}',1,0)")
        .bind(&session).bind(OWNER).execute(database.pool()).await.unwrap();
    let wake=Arc::new(Notify::new());
    let service=Arc::new(RequirementService::new(Arc::new(nomifun_db::SqliteRequirementRepository::new(database.pool().clone())),
        RequirementEventEmitter::new(Arc::new(NoEvents),Arc::from(OWNER))).with_autowork_waker(wake.clone()));
    let mut requirements=Vec::new();
    for title in ["stop this run","keep the next run"] {
        requirements.push(service.create(CreateRequirementRequest {title:title.into(),content:"Use the original queue".into(),
            tag:"current-only".into(),order_key:None,status:None,created_by:None,attachments:vec![]}).await.unwrap());
    }
    let first=&requirements[0];
    let second=&requirements[1];
    let execution=Arc::new(Execution {service:service.clone(),first:first.requirement_id.clone(),admitted:Mutex::new(vec![]),
        admitted_first:Semaphore::new(0),cancelled_first:Semaphore::new(0),signals:Mutex::new(vec![]),
        cancel_failures:std::sync::atomic::AtomicUsize::new(cancel_failures),lost_lease:AtomicBool::new(false)});
    let workspace=Arc::new(Workspace {calls:std::sync::atomic::AtomicUsize::new(0),block_preparation,
        preparing:Semaphore::new(0),release:Semaphore::new(0)});
    let deps=Arc::new(AutoWorkRunnerDeps {authoritative_user_id:Arc::from(OWNER),service:service.clone(),
        execution:execution.clone(),workspace:workspace.clone(),wake:wake.clone()});
    let runner=AutoWorkRunner::new(deps.clone());
    let cancelled=Arc::new(AtomicBool::new(false));
    let progress=Arc::new(LiveProgress::default());
    let loop_cancelled=cancelled.clone();
    let loop_progress=progress.clone();
    let loop_session=session.clone();
    let shutdown=CancellationToken::new();let loop_shutdown=shutdown.clone();
    let task=tokio::spawn(async move {let _owner=HandleGuard {handles:Arc::new(DashMap::new()),key:(AutoWorkTargetKind::Conversation,loop_session.clone()),
        generation:1,deps:deps.clone(),tag:"current-only".into(),cleanup_handoff:Arc::new(AtomicBool::new(false)),
        cleanup_barrier:Arc::new(tokio::sync::Mutex::new(())),process_shutdown:loop_shutdown};
        run_loop(deps,Arc::new(DashMap::new()),&loop_session,
        AutoWorkTargetKind::Conversation,"current-only",loop_cancelled,loop_progress,None,"fixture-revision".into()).await;});
    timeout(Duration::from_secs(3),async {
        if block_preparation {workspace.preparing.acquire().await.unwrap().forget();}
        else {execution.admitted_first.acquire().await.unwrap().forget();}
    }).await.expect("first claim must reach the selected race boundary");
    let claim=service.repo().get_by_requirement_id(&first.requirement_id).await.unwrap().unwrap();
    if drop_api_before_signal {
        service.cancel_current_claim_exact(&first.requirement_id,&session,claim.claim_generation).await.unwrap();
        if abort_loop_owner {task.abort();} else {execution.lost_lease.store(true,Ordering::SeqCst);execution.cancelled_first.add_permits(1);}
    } else {
        let receipt=runner.stop_current_claim(OWNER,AutoWorkTargetKind::Conversation,&session,&first.requirement_id,claim.claim_generation).await.unwrap();
        assert_eq!(receipt.execution_stopped,cancel_failures==0);
        assert_eq!(receipt.cleanup_error.is_none(),cancel_failures==0);
    }
    if block_preparation {workspace.release.add_permits(1);}
    if abort_loop_owner {
        timeout(Duration::from_secs(3),async {while execution.signals.lock().unwrap().len()<cancel_failures+1 {sleep(Duration::from_millis(5)).await;}})
            .await.expect("HandleGuard must retain cancelled exact claims absent from active recovery");
        assert_eq!(service.get(&second.requirement_id).await.unwrap().status,RequirementStatus::Pending);
        assert_eq!(service.tag_pause_state("current-only").await.unwrap(),(false,None));
        assert_eq!(service.get(&first.requirement_id).await.unwrap().attempt_count,claim.attempt_count);
        assert!(task.await.unwrap_err().is_cancelled());
        let restart_cancel=Arc::new(AtomicBool::new(false));let next_cancel=restart_cancel.clone();
        let restart_session=session.clone();let restart_deps=runner.deps.clone();
        let restart=tokio::spawn(async move {run_loop(restart_deps,Arc::new(DashMap::new()),&restart_session,
            AutoWorkTargetKind::Conversation,"current-only",next_cancel,Arc::new(LiveProgress::default()),None,"fixture-revision".into()).await;});
        timeout(Duration::from_secs(3),async {while service.get(&second.requirement_id).await.unwrap().status!=RequirementStatus::Done {
            sleep(Duration::from_millis(5)).await;
        }}).await.expect("a fresh loop must reconcile the original Stop before admitting the next item");
        assert_eq!(execution.admitted.lock().unwrap().iter().filter(|id|*id==&first.requirement_id).count(),1,
            "startup cleanup must never resubmit the cancelled original work");
        restart_cancel.store(true,Ordering::SeqCst);wake.notify_waiters();
        timeout(Duration::from_secs(3),restart).await.unwrap().unwrap();shutdown.cancel();return;
    }
    timeout(Duration::from_secs(3),async {
        while service.get(&second.requirement_id).await.unwrap().status!=RequirementStatus::Done {
            sleep(Duration::from_millis(5)).await;
        }
    }).await.expect("an exact current-only Stop must leave the queue processing its next item");
    assert_eq!(service.get(&first.requirement_id).await.unwrap().status,RequirementStatus::Cancelled);
    assert_eq!(service.get(&first.requirement_id).await.unwrap().attempt_count,claim.attempt_count);
    assert_eq!(service.tag_pause_state("current-only").await.unwrap(),(false,None));
    runner.stop_current_claim(OWNER,AutoWorkTargetKind::Conversation,&session,&first.requirement_id,claim.claim_generation).await.unwrap();
    assert!(execution.signals.lock().unwrap().iter().all(|source| source.requirement_id==first.requirement_id));
    assert_eq!(service.get(&second.requirement_id).await.unwrap().status,RequirementStatus::Done);
    cancelled.store(true,Ordering::SeqCst);
    shutdown.cancel();
    wake.notify_waiters();
    timeout(Duration::from_secs(3),task).await.expect("test loop must settle").unwrap();
    assert_eq!(progress.completed(),1);
}

#[tokio::test]
async fn current_only_stop_continues_the_real_runner_after_an_admitted_attempt() {
    exercise_current_stop(false,0,false,false).await;
}

#[tokio::test]
async fn current_only_stop_continues_after_a_preparation_admission_race() {
    exercise_current_stop(true,0,false,false).await;
}

#[tokio::test]
async fn current_only_stop_retries_the_original_failed_cleanup_before_the_next_claim() {
    exercise_current_stop(false,2,false,false).await;
}

#[tokio::test]
async fn current_only_stop_survives_an_api_waiter_lost_before_the_original_domain_signal() {
    exercise_current_stop(false,1,true,false).await;
}

#[tokio::test]
async fn dropped_loop_owner_recovers_cancelled_claim_cleanup_without_allocating_successor_work() {
    exercise_current_stop(false,1,true,true).await;
}
