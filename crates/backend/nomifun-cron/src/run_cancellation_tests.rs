//! Cancellation observes/signal the original owner; these adapters panic if
//! cancellation or restart attempts any Session creation or business replay.
use super::*;
use crate::busy_guard::CronBusyGuard;
use crate::session_port::{CronTurnDelivery, test_support::{ReceiptOnlySessions,CancellationGate}};
use nomifun_agent_contracts::AgentTurnOutcome;
use nomifun_db::{SqliteCronRepository, sqlx};

struct Fixture {db:nomifun_db::Database,repo:Arc<SqliteCronRepository>,owner:String,job:String,run:String,
    session:String,service:CronService,sessions:Arc<ReceiptOnlySessions>,tasks:Arc<FixtureTasks>}

#[derive(Default)]
struct FixtureTasks(std::sync::Mutex<Vec<JoinHandle<()>>>);
impl CronBackgroundTaskRegistrar for FixtureTasks {
    fn register(&self,task:JoinHandle<()>) {self.0.lock().unwrap().push(task);}
}
impl FixtureTasks {
    async fn join(&self) {
        loop {
            let tasks=std::mem::take(&mut *self.0.lock().unwrap());
            if tasks.is_empty() {return;}
            for task in tasks {task.await.unwrap();}
        }
    }
}

impl Fixture {
    async fn new(attached:bool)->Self {
        let db=nomifun_db::init_database_memory().await.unwrap();
        let owner=nomifun_db::installation_owner_id(db.pool()).await.unwrap();
        let session=ConversationId::new().into_string();
        let job=CronJobId::new().into_string();
        let run=CronJobRunId::new().into_string();
        sqlx::query("INSERT INTO agent_sessions(agent_session_id,owner_ref_json,state,title,archived,pinned,agent_binding_json,next_seq,created_at) \
            VALUES(?,?,'live','Original',0,0,'{}',1,1)")
            .bind(&session).bind(serde_json::json!({"principal_kind":"user","principal_id":owner}).to_string())
            .execute(db.pool()).await.unwrap();
        sqlx::query("INSERT INTO cron_jobs(cron_job_id,user_id,name,schedule_kind,schedule_value,payload_message,conversation_id,created_by,created_at,updated_at,next_run_at) \
            VALUES(?,?,'Original schedule','every','60000','Original input',?,'user',1,1,60001)")
            .bind(&job).bind(&owner).bind(&session).execute(db.pool()).await.unwrap();
        let repo=Arc::new(SqliteCronRepository::new(db.pool().clone()));
        repo.reserve_run(&owner,&ReserveCronRunParams {cron_job_run_id:run.clone(),cron_job_id:job.clone(),trigger_kind:"run_now".into(),
            operation_key:format!("original:{run}"),request_fingerprint:"original".into(),schedule_revision:None,planned_at_ms:None,next_run_at:None,disable:false,now:1}).await.unwrap();
        if attached {repo.attach_run_conversation(&owner,&run,&session,2).await.unwrap();}
        let initial=if attached {CronTurnReceiptState::Accepted {message_id:"0190f5fe-7c00-7a00-8000-000000000011".into()}}
            else {CronTurnReceiptState::Missing};
        let sessions=Arc::new(ReceiptOnlySessions::new(initial));
        let service=Self::service(&owner,repo.clone(),sessions.clone());
        let tasks=Arc::new(FixtureTasks::default());
        service.with_cron_background_task_registrar(tasks.clone());
        Self {db,repo,owner,job,run,session,service,sessions,tasks}
    }
    fn service(owner:&str,repo:Arc<SqliteCronRepository>,sessions:Arc<ReceiptOnlySessions>)->CronService {
        CronService::new(Arc::from(owner),repo,Arc::new(CronScheduler::new(Arc::new(|_,_,_,_,_|{}))),
            Arc::new(JobExecutor::new(Arc::from(owner),sessions,Arc::new(CronBusyGuard::new()),PathBuf::new())),
            CronEventEmitter::new(Arc::new(nomifun_realtime::BroadcastEventBus::new(16))),PathBuf::new())
    }
    async fn settled(&self) {
        tokio::time::timeout(std::time::Duration::from_secs(2),async {
            loop {
                if self.repo.get_run_reservation(&self.owner,&self.job,&self.run).await.unwrap().unwrap().status=="cancelled" {break;}
                tokio::task::yield_now().await;
            }
        }).await.expect("the original cancellation receipt must settle without executing again");
        self.tasks.join().await;
    }
}

#[tokio::test]
async fn exact_run_stop_preempts_job_fifo_and_a_dropped_waiter_retains_the_original_signal() {
    let fixture=Fixture::new(true).await;
    let future=fixture.repo.get_by_cron_job_id(&fixture.owner,&fixture.job).await.unwrap().unwrap();
    let sibling=CronJobRunId::new().into_string();
    fixture.repo.reserve_run(&fixture.owner,&ReserveCronRunParams {cron_job_run_id:sibling.clone(),cron_job_id:fixture.job.clone(),
        trigger_kind:"run_now".into(),operation_key:format!("sibling:{sibling}"),request_fingerprint:"sibling".into(),
        schedule_revision:None,planned_at_ms:None,next_run_at:None,disable:false,now:2}).await.unwrap();
    let gate=fixture.service.job_gate(&fixture.job);
    let ordinary=gate.lock().await;
    let native=Arc::new(CancellationGate::default());
    *fixture.sessions.cancellation_gate.lock().unwrap()=Some(native.clone());
    let entered=native.entered.notified();
    let waiter=tokio::spawn({let service=fixture.service.clone();let owner=fixture.owner.clone();let job=fixture.job.clone();let run=fixture.run.clone();
        async move {service.cancel_run(&owner,&job,&run).await}});
    tokio::time::timeout(std::time::Duration::from_secs(1),entered).await.expect("Stop must reach its exact native owner without waiting for ordinary job controls");
    waiter.abort();
    assert!(fixture.repo.get_run_reservation(&fixture.owner,&fixture.job,&fixture.run).await.unwrap().unwrap().cancel_requested_at_ms.is_some());
    assert_eq!(fixture.repo.get_run_reservation(&fixture.owner,&fixture.job,&sibling).await.unwrap().unwrap().cancel_requested_at_ms,None);
    native.release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(1),async {
        loop {if matches!(*fixture.sessions.receipt.lock().unwrap(),CronTurnReceiptState::Completed(_)) {break;} tokio::task::yield_now().await;}
    }).await.unwrap();
    drop(ordinary);
    fixture.settled().await;
    let calls=fixture.sessions.cancellations.lock().unwrap();
    assert!(calls.iter().all(|request|request.owner_id==fixture.owner&&request.cron_job_id==fixture.job
        &&request.cron_job_run_id==fixture.run&&request.agent_session_id.as_ref()==fixture.session));
    drop(calls);
    let saved=fixture.repo.get_by_cron_job_id(&fixture.owner,&fixture.job).await.unwrap().unwrap();
    assert_eq!((saved.enabled,saved.schedule_revision,saved.schedule_value,saved.next_run_at),
        (future.enabled,future.schedule_revision,future.schedule_value,future.next_run_at));
    assert_eq!(saved.last_status.as_deref(),Some("cancelled"));
    assert_eq!(saved.run_count,1);

    assert_eq!(fixture.repo.get_run_reservation(&fixture.owner,&fixture.job,&sibling).await.unwrap().unwrap().status,"reserved");
    assert_eq!(fixture.repo.list_runs_by_job(&fixture.owner,&fixture.job,7).await.unwrap().len(),1);
}

#[tokio::test]
async fn unclaimed_stop_is_durable_across_service_restart_and_never_becomes_a_retry() {
    let fixture=Fixture::new(false).await;
    let gate=fixture.service.job_gate(&fixture.job);
    let ordinary=gate.lock().await;
    let receipt=tokio::time::timeout(std::time::Duration::from_secs(1),
        fixture.service.cancel_run(&fixture.owner,&fixture.job,&fixture.run)).await.unwrap().unwrap();
    assert!(receipt.accepted&&receipt.execution_stopped);
    assert_eq!(receipt.conversation_id,None);
    assert!(fixture.repo.attach_run_conversation(&fixture.owner,&fixture.run,&fixture.session,3).await.is_err());
    // Reconstruct only the domain receipt consumer, as startup does. It must
    // consume the cancellation, never prepare/send the old scheduled input.
    let restarted=Fixture::service(&fixture.owner,fixture.repo.clone(),fixture.sessions.clone());
    restarted.recover_interrupted_runs_on_boot().await;
    drop(ordinary);
    fixture.settled().await;
    let replay=restarted.cancel_run(&fixture.owner,&fixture.job,&fixture.run).await.unwrap();
    assert!(replay.accepted&&replay.execution_stopped);
    assert!(fixture.sessions.cancellations.lock().unwrap().is_empty());
    assert_eq!(fixture.repo.list_runs_by_job(&fixture.owner,&fixture.job,7).await.unwrap().len(),1);
    let saved=fixture.repo.get_by_cron_job_id(&fixture.owner,&fixture.job).await.unwrap().unwrap();
    assert!(saved.enabled);
    assert_eq!(saved.next_run_at,Some(60001));
    assert_eq!(saved.run_count,1);
    assert!(saved.last_error.is_none());
    let _=&fixture.db;
}

#[tokio::test]
async fn a_cancel_signal_and_canonical_terminal_do_not_claim_native_cleanup_is_finished() {
    let fixture=Fixture::new(true).await;
    *fixture.sessions.receipt.lock().unwrap()=CronTurnReceiptState::Suspended(CronTurnDelivery {
        message_id:"0190f5fe-7c00-7a00-8000-000000000011".into(),replayed:true,completed:false,
        turn_outcome:Some(AgentTurnOutcome::Paused {message_ids:vec![]}),user_input_request:None,
        result_ok:None,result_text:None,result_error:None,result_error_code:None,result_error_retryable:None});
    let receipt=fixture.service.cancel_run(&fixture.owner,&fixture.job,&fixture.run).await.unwrap();
    assert!(receipt.accepted);
    assert!(!receipt.execution_stopped,"the Session adapter has not acknowledged its native owner exit");
    fixture.settled().await;
}

#[tokio::test]
async fn cancelling_an_old_occurrence_cannot_advance_or_disable_a_new_schedule_revision() {
    let fixture=Fixture::new(false).await;
    let run=CronJobRunId::new().into_string();
    fixture.repo.reserve_run(&fixture.owner,&ReserveCronRunParams {cron_job_run_id:run.clone(),cron_job_id:fixture.job.clone(),
        trigger_kind:"scheduled".into(),operation_key:format!("scheduled:{}:1:60001",fixture.job),request_fingerprint:"revision-one-input".into(),
        schedule_revision:Some(1),planned_at_ms:Some(60001),next_run_at:Some(120001),disable:false,now:2}).await.unwrap();
    // Isolated fixture represents a fully committed subsequent user edit.
    sqlx::query("UPDATE cron_jobs SET schedule_revision=2,schedule_value='120000',next_run_at=123456,updated_at=3 WHERE cron_job_id=?")
        .bind(&fixture.job).execute(fixture.db.pool()).await.unwrap();
    assert!(fixture.service.cancel_run(&fixture.owner,&fixture.job,&run).await.unwrap().execution_stopped);
    tokio::time::timeout(std::time::Duration::from_secs(2),fixture.tasks.join()).await.unwrap();
    let current=fixture.repo.get_by_cron_job_id(&fixture.owner,&fixture.job).await.unwrap().unwrap();
    assert!(current.enabled);
    assert_eq!(current.schedule_revision,2);
    assert_eq!(current.schedule_value,"120000");
    assert_eq!(current.next_run_at,Some(123456));
    let original=fixture.repo.get_run_reservation(&fixture.owner,&fixture.job,&run).await.unwrap().unwrap();
    assert_eq!(original.status,"cancelled");
    assert_eq!((original.schedule_revision,original.planned_at_ms),(Some(1),Some(60001)));
    assert_eq!(original.request_fingerprint,"revision-one-input");
}

#[tokio::test]
async fn an_attached_cancelled_run_survives_missing_job_configuration_without_new_authority() {
    for on_boot in [false,true] {
        let fixture=Fixture::new(true).await;
        if on_boot {fixture.repo.request_run_cancellation(&fixture.owner,&fixture.job,&fixture.run,3).await.unwrap();}
        // Isolated absent-configuration fault, bypassing the normal delete's
        // reserved-run guard. Never repair it by creating a replacement job.
        sqlx::query("DELETE FROM cron_jobs WHERE cron_job_id=?").bind(&fixture.job).execute(fixture.db.pool()).await.unwrap();
        if on_boot {fixture.service.recover_interrupted_runs_on_boot().await;}
        else {assert!(fixture.service.cancel_run(&fixture.owner,&fixture.job,&fixture.run).await.unwrap().accepted);}
        fixture.settled().await;
        let saved=fixture.repo.get_run_reservation(&fixture.owner,&fixture.job,&fixture.run).await.unwrap().unwrap();
        assert_eq!(saved.status,"cancelled");
        assert!(saved.cancel_requested_at_ms.is_some());
        assert!(fixture.repo.get_by_cron_job_id(&fixture.owner,&fixture.job).await.unwrap().is_none());
        assert!(fixture.sessions.cancellations.lock().unwrap().iter().all(|request|request.cron_job_run_id==fixture.run));
        assert!(fixture.repo.list_reserved_runs_for_scheduler().await.unwrap().is_empty());
    }
}
