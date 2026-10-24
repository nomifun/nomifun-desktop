use super::*;
use async_trait::async_trait;
use tokio::sync::mpsc;
use crate::planner::{LeadThinkingSink, AdjustedExecutionPlan};

struct DropSignal(String, mpsc::UnboundedSender<String>);
impl Drop for DropSignal { fn drop(&mut self) { let _ = self.1.send(self.0.clone()); } }
struct HangingPlanner { started: mpsc::UnboundedSender<String>, dropped: mpsc::UnboundedSender<String> }
#[async_trait]
impl PlanProducer for HangingPlanner {
    async fn produce(&self, goal: &str, _: &[ExecutionParticipant], _: Option<&LeadThinkingSink>) -> Result<PlannedExecution, AppError> {
        let _drop = DropSignal(goal.to_owned(), self.dropped.clone());
        self.started.send(goal.to_owned()).unwrap(); std::future::pending().await
    }
    async fn adjust(&self, intent: &str, _: &AgentExecutionDetail, _: Option<&LeadThinkingSink>) -> Result<AdjustedExecutionPlan, AppError> {
        let _drop = DropSignal(intent.to_owned(), self.dropped.clone());
        self.started.send(intent.to_owned()).unwrap(); std::future::pending().await
    }
}

async fn seed(pool: &nomifun_db::SqlitePool, owner: &str, goal: &str, status: &str) -> String {
    let id = generate_id();
    sqlx::query("INSERT INTO agent_executions(execution_id,user_id,goal,status,adaptation_policy,decision_policy,delegation_policy,max_parallel,initial_plan_input,version,event_sequence,created_at,updated_at) VALUES(?,?,?,?,'adaptive','ask_user','automatic',1,'{\"mode\":\"automatic\"}',1,1,1,1)")
        .bind(&id).bind(owner).bind(goal).bind(status).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO agent_execution_events(execution_id,sequence,event_type,actor_type,on_behalf_of_user_id,payload,created_at) VALUES(?,1,'created','system',?,'{}',1)")
        .bind(&id).bind(owner).execute(pool).await.unwrap(); id
}
async fn next(receiver: &mut mpsc::UnboundedReceiver<String>) -> String {
    tokio::time::timeout(Duration::from_secs(3), receiver.recv()).await.unwrap().unwrap()
}

struct CountingPlan {calls:std::sync::atomic::AtomicUsize,entered:tokio::sync::Notify,release:tokio::sync::Notify,fail:bool}
#[async_trait]
impl PlanProducer for CountingPlan {
    async fn produce(&self,_:&str,_:&[ExecutionParticipant],_:Option<&LeadThinkingSink>)->Result<PlannedExecution,AppError> {
        self.calls.fetch_add(1,std::sync::atomic::Ordering::SeqCst);self.entered.notify_one();self.release.notified().await;
        if self.fail {return Err(AppError::BadRequest("original planner candidate failed".into()));}
        Ok(serde_json::from_value(json!({"steps":[{"title":"original materialized step","spec":"Observe the fixture's explicit unavailable participant outcome.",
            "kind":"agent","participant_index":0,"failure_policy":"fail_execution"}]})).unwrap())
    }
    async fn adjust(&self,_:&str,_:&AgentExecutionDetail,_:Option<&LeadThinkingSink>)->Result<AdjustedExecutionPlan,AppError> {panic!("initial plan recovery cannot become another adjustment")}
}

async fn seed_counted_plan(pool:&nomifun_db::SqlitePool,owner:&str)->String {
    let execution=seed(pool,owner,"original counted plan","planning").await;
    // A valid original plan can materialize. Its participant intentionally has
    // no provider/model, so the real scheduler must produce that known failure
    // without invoking this fixture's forbidden native Session adapter.
    sqlx::query("INSERT INTO agent_execution_participants(participant_id,execution_id,source_agent_id,introduced_in_revision,created_at) VALUES(?,?,?,0,1)")
        .bind(generate_id()).bind(&execution).bind(generate_id()).execute(pool).await.unwrap();execution
}

async fn await_original_planning_exit(engine:&AgentExecutionEngine,owner:&str,execution:&str)->AgentExecutionDetail {
    tokio::time::timeout(Duration::from_secs(20),async {loop {
        let detail=engine.get(owner,execution).await.unwrap();
        if detail.execution.status.is_terminal()&&!engine.planning_operations().iter().any(|(id,_,_)|id==execution) {break detail;}
        tokio::task::yield_now().await;
    }}).await.expect("the original planning owner hands off or settles instead of leaving Planning/Running without custody")
}

#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn original_planned_result_survives_commit_busy_and_original_failure_busy_without_another_model_call() {
    for fail in [false,true] {
        let dir=tempfile::tempdir().unwrap();let db=nomifun_db::init_database(&dir.path().join("planning-busy.sqlite")).await.unwrap();
        let owner=nomifun_db::installation_owner_id(db.pool()).await.unwrap();let lifecycle=AgentExecutionLifecycle::new();
        let planner=Arc::new(CountingPlan {calls:Default::default(),entered:Default::default(),release:Default::default(),fail});
        let engine=crate::production::tests::planning_engine(db.pool().clone(),planner.clone(),lifecycle.clone(),dir.path().to_owned());
        let execution=seed_counted_plan(db.pool(),&owner).await;engine.spawn_initial_plan(owner.clone(),execution.clone(),1);
        tokio::time::timeout(Duration::from_secs(3),planner.entered.notified()).await.unwrap();
        let writer=db.pool().begin_with("BEGIN IMMEDIATE").await.unwrap();planner.release.notify_one();
        tokio::time::timeout(Duration::from_secs(20),async {while engine.planning_controls.database_retries()==0 {tokio::task::yield_now().await;}}).await
            .expect("the isolated real SQLite writer returns typed Busy before release");
        assert_eq!(planner.calls.load(std::sync::atomic::Ordering::SeqCst),1);assert!(engine.planning_controls.has_live_owner(&execution));
        assert_eq!(engine.get(&owner,&execution).await.unwrap().execution.status,AgentExecutionStatus::Planning,
            "temporary commit admission is not a new business failure");
        writer.commit().await.unwrap();let detail=await_original_planning_exit(&engine,&owner,&execution).await;
        assert_eq!(planner.calls.load(std::sync::atomic::Ordering::SeqCst),1);
        if fail {assert!(detail.steps.is_empty());assert!(detail.execution.summary.as_deref().unwrap().contains("original planner candidate failed"));}
        else {assert_eq!(detail.steps.len(),1);assert_eq!(detail.attempts.len(),1);assert!(detail.attempts[0].error.as_deref().unwrap().contains("provider and model"));}
        lifecycle.shutdown().await.unwrap();db.close().await;
    }
}

#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn initial_plan_commit_ack_loss_and_version_conflict_handoff_the_exact_plan_once() {
    for lost_ack in [true,false] {
        let dir=tempfile::tempdir().unwrap();let db=nomifun_db::init_database(&dir.path().join("planning-ack.sqlite")).await.unwrap();
        let owner=nomifun_db::installation_owner_id(db.pool()).await.unwrap();let lifecycle=AgentExecutionLifecycle::new();
        let planner=Arc::new(CountingPlan {calls:Default::default(),entered:Default::default(),release:Default::default(),fail:false});
        let engine=crate::production::tests::planning_engine(db.pool().clone(),planner.clone(),lifecycle.clone(),dir.path().to_owned());
        let execution=seed_counted_plan(db.pool(),&owner).await;
        if lost_ack {engine.planning_controls.lose_next_plan_ack();}
        engine.spawn_initial_plan(owner.clone(),execution.clone(),1);
        tokio::time::timeout(Duration::from_secs(3),planner.entered.notified()).await.unwrap();
        if !lost_ack {engine.rename(&owner,&AgentExecutionActor::User {user_id:owner.clone()},&execution,
            RenameAgentExecutionRequest {goal:"original counted plan".into(),expected_version:1}).await.unwrap();}
        planner.release.notify_one();let detail=await_original_planning_exit(&engine,&owner,&execution).await;
        assert_eq!(planner.calls.load(std::sync::atomic::Ordering::SeqCst),1);assert_eq!(detail.steps.len(),1);assert_eq!(detail.attempts.len(),1);
        let events=engine.events(&owner,&execution,None,Some(200)).await.unwrap();
        assert_eq!(events.iter().filter(|event|event.payload["change"]=="initial_plan").count(),1,"a lost ACK or stale version cannot duplicate materialized Steps");
        assert!(detail.attempts[0].error.as_deref().unwrap().contains("provider and model"),"the scheduler actually received the original committed task");
        lifecycle.shutdown().await.unwrap();db.close().await;
    }
}

#[tokio::test]
async fn initial_planning_owner_is_visible_before_the_first_poll_and_stop_preempts_its_real_barrier() {
    let db=nomifun_db::init_database_memory().await.unwrap();let owner=nomifun_db::installation_owner_id(db.pool()).await.unwrap();
    let dir=tempfile::tempdir().unwrap();let lifecycle=AgentExecutionLifecycle::new();
    let(started,mut starts)=mpsc::unbounded_channel();let(dropped,_drops)=mpsc::unbounded_channel();
    let engine=crate::production::tests::planning_engine(db.pool().clone(),Arc::new(HangingPlanner {started,dropped}),lifecycle.clone(),dir.path().to_owned());
    let execution=seed(db.pool(),&owner,"exact-initial-planning-barrier","planning").await;
    assert!(!engine.collaboration_has_dispatch_owner(&owner,&execution).await.unwrap(),"Planning text alone is not task custody");
    let(entered,_release)=engine.hold_initial_planning_for_test("exact-initial-planning-barrier").unwrap();
    engine.spawn_initial_plan(owner.clone(),execution.clone(),1);
    assert!(engine.planning_controls.has_live_owner(&execution),"registration precedes this async task's first repository read");
    engine.spawn_initial_plan(owner.clone(),execution.clone(),1);
    assert_eq!(engine.planning_operations().iter().filter(|(id,_,_)|id==&execution).count(),1,"a duplicate observer cannot create another planner");
    tokio::time::timeout(Duration::from_secs(3),entered.notified()).await.unwrap();
    assert!(engine.collaboration_has_dispatch_owner(&owner,&execution).await.unwrap());assert!(starts.try_recv().is_err());
    engine.cancel(&owner,&AgentExecutionActor::User {user_id:owner.clone()},&execution,VersionedAgentExecutionCommand {expected_version:1}).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3),async {while engine.planning_operations().iter().any(|(id,_,_)|id==&execution) {tokio::task::yield_now().await;}}).await
        .expect("Stop must drop the original planning guard without releasing the held barrier");
    assert!(!engine.collaboration_has_dispatch_owner(&owner,&execution).await.unwrap());
    let detail=engine.get(&owner,&execution).await.unwrap();assert_eq!(detail.execution.status,AgentExecutionStatus::Cancelled);assert!(detail.steps.is_empty());
    assert!(starts.try_recv().is_err());lifecycle.shutdown().await.unwrap();db.close().await;
}

#[tokio::test]
async fn planning_stop_drops_only_original_model_request_and_keeps_durable_cancel() {
    let db = nomifun_db::init_database_memory().await.unwrap();
    let owner = nomifun_db::installation_owner_id(db.pool()).await.unwrap();
    let dir = tempfile::tempdir().unwrap(); let lifecycle = AgentExecutionLifecycle::new();
    let (started, mut starts) = mpsc::unbounded_channel(); let (dropped, mut drops) = mpsc::unbounded_channel();
    let engine = crate::production::tests::planning_engine(db.pool().clone(), Arc::new(HangingPlanner { started, dropped }), lifecycle.clone(), dir.path().to_owned());
    let first = seed(db.pool(), &owner, "first", "planning").await;
    let sibling = seed(db.pool(), &owner, "sibling", "planning").await;
    engine.recover().await.unwrap();
    let mut begun = vec![next(&mut starts).await, next(&mut starts).await]; begun.sort(); assert_eq!(begun, ["first", "sibling"]);
    engine.recover().await.unwrap(); // Same initial owner; recovery cannot duplicate its request.
    engine.cancel(&owner, &AgentExecutionActor::User { user_id: owner.clone() }, &first,
        VersionedAgentExecutionCommand { expected_version: 1 }).await.unwrap();
    assert_eq!(next(&mut drops).await, "first");
    assert!(starts.try_recv().is_err()); assert!(drops.try_recv().is_err());
    assert_eq!(engine.get(&owner, &first).await.unwrap().execution.status, AgentExecutionStatus::Cancelled);
    assert_eq!(engine.get(&owner, &sibling).await.unwrap().execution.status, AgentExecutionStatus::Planning);
    // A late recovery never recreates a cancelled model request or fallback Step.
    engine.recover().await.unwrap(); assert!(engine.get(&owner, &first).await.unwrap().steps.is_empty());
    lifecycle.shutdown().await.unwrap(); assert_eq!(next(&mut drops).await, "sibling");
    assert!(starts.try_recv().is_err()); db.close().await;
}

#[tokio::test]
async fn whole_stop_preempts_manual_adjustment_model_without_waiting_for_its_http_caller() {
    let db = nomifun_db::init_database_memory().await.unwrap();
    let owner = nomifun_db::installation_owner_id(db.pool()).await.unwrap();
    let dir = tempfile::tempdir().unwrap(); let lifecycle = AgentExecutionLifecycle::new();
    let (started, mut starts) = mpsc::unbounded_channel(); let (dropped, mut drops) = mpsc::unbounded_channel();
    let engine = crate::production::tests::planning_engine(db.pool().clone(), Arc::new(HangingPlanner { started, dropped }), lifecycle.clone(), dir.path().to_owned());
    let execution = seed(db.pool(), &owner, "manual", "running").await;
    let actor = AgentExecutionActor::User { user_id: owner.clone() };
    let operation = { let engine = engine.clone(); let owner = owner.clone(); let execution = execution.clone(); let actor = actor.clone();
        tokio::spawn(async move {engine.adjust(&owner, &actor, &execution,
            AdjustAgentExecutionRequest { expected_version: 1, intent: "adjust".into() }).await}) };
    assert_eq!(next(&mut starts).await, "adjust");
    engine.cancel(&owner, &actor, &execution, VersionedAgentExecutionCommand { expected_version: 1 }).await.unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(3), operation).await.unwrap().unwrap().is_err());
    assert_eq!(next(&mut drops).await, "adjust");
    let detail = engine.get(&owner, &execution).await.unwrap();
    assert_eq!(detail.execution.status, AgentExecutionStatus::Cancelled); assert!(detail.steps.is_empty());
    assert!(starts.try_recv().is_err()); lifecycle.shutdown().await.unwrap(); db.close().await;
}

/// Poll the real public command once, as an HTTP handler does. Its first
/// pending boundary must already have transferred the complete command to the
/// original lifecycle owner; cancelling this outer waiter cannot abort it.
async fn drop_public_waiter<F,T>(lifecycle:&AgentExecutionLifecycle,command:F)
where F:std::future::Future<Output=Result<T,AppError>>+Send+'static,T:Send+'static {
    let before=lifecycle.tracked_count();
    let (polled,first_poll)=tokio::sync::oneshot::channel();
    let waiter=tokio::spawn(async move {
        let mut command=Box::pin(command);let mut polled=Some(polled);
        futures::future::poll_fn(move|cx| {
            let result=command.as_mut().poll(cx);
            if let Some(polled)=polled.take() {let _=polled.send(result.is_pending());}
            result
        }).await
    });
    assert!(tokio::time::timeout(Duration::from_secs(3),first_poll).await.unwrap().unwrap());
    assert!(lifecycle.tracked_count()>before,"the original tracker owns the mutation before its external await");
    waiter.abort();match waiter.await {Err(error)=>assert!(error.is_cancelled()),Ok(_)=>panic!("external waiter was not cancelled")};
    assert!(lifecycle.tracked_count()>before,"dropping the HTTP-like waiter cannot remove its blocked mutation owner");
}

#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn public_whole_stop_and_delete_retain_their_owner_after_a_waiter_is_dropped_at_the_sqlite_writer() {
    for operation in ["cancel","delete"] {
        let dir=tempfile::tempdir().unwrap();
        let db=nomifun_db::init_database(&dir.path().join("original-stop.sqlite")).await.unwrap();
        let owner=nomifun_db::installation_owner_id(db.pool()).await.unwrap();
        let lifecycle=AgentExecutionLifecycle::new();
        let(started,mut starts)=mpsc::unbounded_channel();let(dropped,mut drops)=mpsc::unbounded_channel();
        let engine=crate::production::tests::planning_engine(db.pool().clone(),Arc::new(HangingPlanner {started,dropped}),lifecycle.clone(),dir.path().to_owned());
        let target=seed(db.pool(),&owner,"original-target","planning").await;
        let sibling=seed(db.pool(),&owner,"independent-sibling","planning").await;
        engine.recover().await.unwrap();
        let mut begun=vec![next(&mut starts).await,next(&mut starts).await];begun.sort();
        assert_eq!(begun,["independent-sibling","original-target"]);
        // A real WAL writer prevents the cancellation commit, while original
        // reads/planner observers remain live. No mock commit boundary exists.
        let writer=db.pool().begin_with("BEGIN IMMEDIATE").await.unwrap();
        let stopping=engine.clone();let user=owner.clone();let captured=target.clone();
        drop_public_waiter(&lifecycle,async move {
            let actor=AgentExecutionActor::User {user_id:user.clone()};
            if operation=="cancel" {
                stopping.cancel(&user,&actor,&captured,VersionedAgentExecutionCommand {expected_version:1}).await.map(|_|())
            } else {stopping.delete(&user,&actor,&captured,VersionedAgentExecutionCommand {expected_version:1}).await}
        }).await;
        assert!(drops.try_recv().is_err(),"the planner is not signalled before its durable Stop commits");
        assert_eq!(engine.get(&owner,&target).await.unwrap().execution.status,AgentExecutionStatus::Planning);
        writer.commit().await.unwrap();
        assert_eq!(next(&mut drops).await,"original-target","the retained owner must signal its exact planner after commit");
        if operation=="cancel" {
            assert_eq!(engine.get(&owner,&target).await.unwrap().execution.status,AgentExecutionStatus::Cancelled);
        } else {
            assert!(matches!(engine.get(&owner,&target).await,Err(AppError::NotFound(_))));
            let deleted:bool=sqlx::query_scalar("SELECT deleted_at IS NOT NULL FROM agent_executions WHERE execution_id=?")
                .bind(&target).fetch_one(db.pool()).await.unwrap();assert!(deleted);
        }
        assert_eq!(engine.get(&owner,&sibling).await.unwrap().execution.status,AgentExecutionStatus::Planning);
        assert!(starts.try_recv().is_err());assert!(drops.try_recv().is_err());
        engine.recover().await.unwrap();
        assert!(starts.try_recv().is_err(),"neither the stopped scope nor its still-owned sibling is replayed");
        lifecycle.shutdown().await.unwrap();assert_eq!(next(&mut drops).await,"independent-sibling");
        assert_eq!(lifecycle.tracked_count(),0);assert!(starts.try_recv().is_err());db.close().await;
    }
}

async fn pending_step(pool:&nomifun_db::SqlitePool,execution:&str,title:&str)->String {
    let step=generate_id();
    sqlx::query("INSERT INTO agent_execution_steps(step_id,execution_id,title,spec,kind,agent_mode,status,version,introduced_in_revision,created_at,updated_at) VALUES(?,?,?,?,'agent','normal','pending',0,0,1,1)")
        .bind(&step).bind(execution).bind(title).bind(format!("Original {title}")).execute(pool).await.unwrap();step
}

fn scoped_model(engine:&AgentExecutionEngine,execution:&str,step:&str,label:&str) {
    // The exact ephemeral planning-scope capability is what post-commit Stop
    // must reach. No Session/native task is manufactured by this unit fixture.
    let control=engine.planning_controls.register(execution,1,Some(step),false,engine.lifecycle.cancellation_child()).unwrap().unwrap();
    let planner=engine.planner.clone();let goal=label.to_owned();
    engine.lifecycle.spawn(async move {tokio::select! {biased;
        _=control.cancelled()=>{},
        result=planner.produce(&goal,&[],None)=>panic!("hanging original scope unexpectedly returned: {result:?}"),
    }});
}

#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn public_task_stop_survives_a_dropped_writer_waiter_and_signals_only_its_captured_planning_scope() {
    let dir=tempfile::tempdir().unwrap();let db=nomifun_db::init_database(&dir.path().join("original-task-stop.sqlite")).await.unwrap();
    let owner=nomifun_db::installation_owner_id(db.pool()).await.unwrap();let lifecycle=AgentExecutionLifecycle::new();
    let(started,mut starts)=mpsc::unbounded_channel();let(dropped,mut drops)=mpsc::unbounded_channel();
    let engine=crate::production::tests::planning_engine(db.pool().clone(),Arc::new(HangingPlanner {started,dropped}),lifecycle.clone(),dir.path().to_owned());
    let execution=seed(db.pool(),&owner,"original-plan","planning").await;
    let selected=pending_step(db.pool(),&execution,"selected").await;let sibling=pending_step(db.pool(),&execution,"sibling").await;
    scoped_model(&engine,&execution,&selected,"selected-planning");scoped_model(&engine,&execution,&sibling,"sibling-planning");
    let mut begun=vec![next(&mut starts).await,next(&mut starts).await];begun.sort();assert_eq!(begun,["selected-planning","sibling-planning"]);
    let writer=db.pool().begin_with("BEGIN IMMEDIATE").await.unwrap();
    let stopping=engine.clone();let user=owner.clone();let target=execution.clone();let step=selected.clone();
    drop_public_waiter(&lifecycle,async move {stopping.cancel_step(&user,&AgentExecutionActor::User {user_id:user.clone()},&target,&step,
        CancelExecutionStepRequest {expected_execution_version:1,expected_step_version:0,scope:nomifun_common::AgentExecutionCancellationScope::Task}).await}).await;
    assert!(drops.try_recv().is_err());writer.commit().await.unwrap();
    assert_eq!(next(&mut drops).await,"selected-planning");
    let detail=engine.get(&owner,&execution).await.unwrap();
    assert_eq!(detail.execution.status,AgentExecutionStatus::Planning);
    assert_eq!(detail.steps.iter().find(|step|step.step_id==selected).unwrap().status,ExecutionStepStatus::Cancelled);
    assert_eq!(detail.steps.iter().find(|step|step.step_id==sibling).unwrap().status,ExecutionStepStatus::Pending);
    assert!(detail.attempts.is_empty());assert!(starts.try_recv().is_err());assert!(drops.try_recv().is_err());
    let events=engine.repository.list_events(&owner,&execution,0,100).await.unwrap();
    assert_eq!(events.iter().filter(|event|event.event_type==AgentExecutionEventKind::StepChanged.as_str()).count(),1);
    lifecycle.shutdown().await.unwrap();assert_eq!(next(&mut drops).await,"sibling-planning");
    assert_eq!(lifecycle.tracked_count(),0);db.close().await;
}

#[tokio::test]
async fn public_stop_and_delete_keep_authorized_cleanup_admission_after_lifecycle_shutdown() {
    let db=nomifun_db::init_database_memory().await.unwrap();let owner=nomifun_db::installation_owner_id(db.pool()).await.unwrap();
    let dir=tempfile::tempdir().unwrap();let lifecycle=AgentExecutionLifecycle::new();
    let(started,_)=mpsc::unbounded_channel();let(dropped,_)=mpsc::unbounded_channel();
    let engine=crate::production::tests::planning_engine(db.pool().clone(),Arc::new(HangingPlanner {started,dropped}),lifecycle.clone(),dir.path().to_owned());
    let cancelled=seed(db.pool(),&owner,"cancel during shutdown","running").await;
    let deleted=seed(db.pool(),&owner,"delete during shutdown","running").await;
    let task=seed(db.pool(),&owner,"task during shutdown","planning").await;
    let step=pending_step(db.pool(),&task,"shutdown task").await;
    lifecycle.shutdown().await.unwrap();let actor=AgentExecutionActor::User {user_id:owner.clone()};
    engine.cancel(&owner,&actor,&cancelled,VersionedAgentExecutionCommand {expected_version:1}).await.unwrap();
    engine.delete(&owner,&actor,&deleted,VersionedAgentExecutionCommand {expected_version:1}).await.unwrap();
    engine.cancel_step(&owner,&actor,&task,&step,CancelExecutionStepRequest {expected_execution_version:1,expected_step_version:0,
        scope:nomifun_common::AgentExecutionCancellationScope::Task}).await.unwrap();
    assert_eq!(engine.get(&owner,&cancelled).await.unwrap().execution.status,AgentExecutionStatus::Cancelled);
    assert!(matches!(engine.get(&owner,&deleted).await,Err(AppError::NotFound(_))));
    assert_eq!(engine.get(&owner,&task).await.unwrap().steps[0].status,ExecutionStepStatus::Cancelled);
    lifecycle.shutdown().await.unwrap();assert_eq!(lifecycle.tracked_count(),0);db.close().await;
}

#[tokio::test]
async fn automation_local_user_scope_preserves_original_system_control_and_repository_authority() {
    let db=nomifun_db::init_database_memory().await.unwrap();let owner=nomifun_db::installation_owner_id(db.pool()).await.unwrap();
    let dir=tempfile::tempdir().unwrap();let lifecycle=AgentExecutionLifecycle::new();
    let(started,_)=mpsc::unbounded_channel();let(dropped,_)=mpsc::unbounded_channel();
    let engine=crate::production::tests::planning_engine(db.pool().clone(),Arc::new(HangingPlanner {started,dropped}),lifecycle.clone(),dir.path().to_owned());
    let execution=seed(db.pool(),&owner,"original internal automation control","running").await;
    let root=pending_step(db.pool(),&execution,"original root").await;
    let lead=generate_id();
    sqlx::query("INSERT INTO conversation_execution_links(conversation_id,execution_id,relation,active,created_at,updated_at) VALUES(?,?,'lead',1,1,1)")
        .bind(&lead).bind(&execution).execute(db.pool()).await.unwrap();
    sqlx::query("UPDATE agent_executions SET initial_plan_input=? WHERE execution_id=?")
        .bind(json!({"mode":"automation","source":{"requirement_id":generate_id(),"claim_generation":1,"operation_id":"autowork:original-system-control"},
            "plan":{"steps":[{"title":"original root","spec":"preserve original control ownership"}]}}).to_string())
        .bind(&execution).execute(db.pool()).await.unwrap();
    let command=||CancelExecutionStepRequest {expected_execution_version:1,expected_step_version:0,
        scope:nomifun_common::AgentExecutionCancellationScope::Task};
    // System scoped Stop has always reached the repository's user-only Step
    // authority boundary. Do not turn that original guard into a newly imposed
    // human-root Forbidden, or impersonate a User to bypass the DB contract.
    let error=engine.cancel_step(&owner,&AgentExecutionActor::System,&execution,&root,command()).await.unwrap_err();
    assert!(matches!(error,AppError::Conflict(ref reason) if reason.contains("user Step cancellation authority")),"{error}");
    let before=engine.get(&owner,&execution).await.unwrap();
    assert_eq!(before.execution.version,1);assert_eq!(before.steps[0].status,ExecutionStepStatus::Pending);
    for actor in [AgentExecutionActor::User {user_id:owner.clone()},AgentExecutionActor::agent(lead.clone(),None)] {
        assert!(matches!(engine.cancel(&owner,&actor,&execution,VersionedAgentExecutionCommand {expected_version:1}).await,Err(AppError::Conflict(_))),
            "local human Step permission cannot authorize generic automation aggregate commands");
    }
    // The actual original System-owned aggregate cancellation still commits
    // with its own actor, unchanged by the new local User permission.
    let stopped=engine.cancel(&owner,&AgentExecutionActor::System,&execution,VersionedAgentExecutionCommand {expected_version:1}).await.unwrap();
    assert_eq!(stopped.execution.status,AgentExecutionStatus::Cancelled);
    let events=engine.repository.list_events(&owner,&execution,0,100).await.unwrap();
    let original=events.iter().find(|event|event.event_type==AgentExecutionEventKind::StatusChanged.as_str()).unwrap();
    assert_eq!(original.actor_type,"system");
    assert!(original.actor_id.is_none());assert_eq!(original.on_behalf_of_user_id,owner);
    lifecycle.shutdown().await.unwrap();db.close().await;
}

async fn seed_automation(
    pool: &nomifun_db::SqlitePool,
    owner: &str,
    source: &AutomationExecutionSource,
) -> String {
    let execution = seed(pool, owner, "original automation control", "running").await;
    sqlx::query("INSERT INTO conversation_execution_links(conversation_id,execution_id,relation,active,created_at,updated_at) VALUES(?,?,'lead',1,1,1)")
        .bind(generate_id()).bind(&execution).execute(pool).await.unwrap();
    sqlx::query("UPDATE agent_executions SET initial_plan_input=? WHERE execution_id=?")
        .bind(json!({"mode":"automation","source":source,"plan":{"steps":[]}}).to_string())
        .bind(&execution).execute(pool).await.unwrap();
    execution
}

fn source() -> AutomationExecutionSource {
    AutomationExecutionSource {
        requirement_id: generate_id(), claim_generation: 1,
        operation_id: "autowork:original-source-stop".into(),
    }
}

#[tokio::test]
async fn automation_stop_retries_only_a_forward_cas_of_the_original_execution_without_retargeting() {
    let db = nomifun_db::init_database_memory().await.unwrap();
    let owner = nomifun_db::installation_owner_id(db.pool()).await.unwrap();
    let dir = tempfile::tempdir().unwrap(); let lifecycle = AgentExecutionLifecycle::new();
    let (started, mut starts) = mpsc::unbounded_channel(); let (dropped, _) = mpsc::unbounded_channel();
    let engine = crate::production::tests::planning_engine(db.pool().clone(),
        Arc::new(HangingPlanner { started, dropped }), lifecycle.clone(), dir.path().to_owned());
    let original_source = source();
    let original = seed_automation(db.pool(), &owner, &original_source).await;
    let successor_source = AutomationExecutionSource { claim_generation: 2, ..original_source.clone() };
    let successor = seed_automation(db.pool(), &owner, &successor_source).await;
    // Capture the same real source lookup/detail that precedes the original
    // cancellation CAS. An independent recovery commits after that capture.
    assert_eq!(engine.repository.automation_source_cleanup_execution(&owner,
        &original_source.requirement_id, 1, &original_source.operation_id).await.unwrap().as_deref(), Some(original.as_str()));
    let captured = engine.get(&owner, &original).await.unwrap();
    engine.repository.append_event(&owner, &original, captured.execution.version,
        &actor_event(&AgentExecutionActor::System, AgentExecutionEventKind::AttemptChanged,
            None, None, json!({"reason":"queued_before_restart"}))).await.unwrap();
    assert!(matches!(engine.repository.cancel_execution(&owner, &original, captured.execution.version,
        &actor_event(&AgentExecutionActor::System, AgentExecutionEventKind::StatusChanged,
            None, None, explicit_cancel_payload())).await, Err(nomifun_db::DbError::Conflict(_))));
    let cancelled = engine.cancel_original_automation_execution(&owner, captured).await.unwrap().unwrap();
    assert_eq!(cancelled, AutomationExecutionReceipt::Cancelled { execution_id: original.clone(), replay_safe: true });
    let final_detail = engine.get(&owner, &original).await.unwrap();
    assert_eq!(final_detail.execution.status, AgentExecutionStatus::Cancelled);
    assert_eq!(final_detail.execution.version, 3);
    assert!(final_detail.attempts.is_empty());
    let events = engine.repository.list_events(&owner, &original, 0, 100).await.unwrap();
    assert_eq!(events.len(), 3);
    assert_eq!(events.iter().filter(|event| event.event_type == AgentExecutionEventKind::StatusChanged.as_str()).count(), 1);
    let untouched = engine.get(&owner, &successor).await.unwrap();
    assert_eq!(untouched.execution.version, 1);
    assert_eq!(untouched.execution.status, AgentExecutionStatus::Running);
    assert!(untouched.attempts.is_empty());
    assert_eq!(engine.cancel_automation(&owner, &original_source).await.unwrap(), Some(cancelled));
    assert_eq!(engine.repository.list_events(&owner, &original, 0, 100).await.unwrap().len(), 3);
    assert!(starts.try_recv().is_err(), "control contention never invokes the business model");
    lifecycle.shutdown().await.unwrap(); db.close().await;
}

#[tokio::test]
async fn automation_stop_preserves_a_terminal_winner_on_the_same_original_execution() {
    let db = nomifun_db::init_database_memory().await.unwrap();
    let owner = nomifun_db::installation_owner_id(db.pool()).await.unwrap();
    let dir = tempfile::tempdir().unwrap(); let lifecycle = AgentExecutionLifecycle::new();
    let (started, mut starts) = mpsc::unbounded_channel(); let (dropped, _) = mpsc::unbounded_channel();
    let engine = crate::production::tests::planning_engine(db.pool().clone(),
        Arc::new(HangingPlanner { started, dropped }), lifecycle.clone(), dir.path().to_owned());
    let original = seed_automation(db.pool(), &owner, &source()).await;
    let captured = engine.get(&owner, &original).await.unwrap();
    engine.repository.update_execution(&owner, &original, captured.execution.version, None,
        &UpdateAgentExecutionParams { status: Some(AgentExecutionStatus::Completed),
            summary: Some(Some("the original result".into())), ..Default::default() },
        &actor_event(&AgentExecutionActor::System, AgentExecutionEventKind::StatusChanged,
            None, None, json!({"status":"completed"}))).await.unwrap();
    assert_eq!(engine.cancel_original_automation_execution(&owner, captured).await.unwrap(),
        Some(AutomationExecutionReceipt::Completed { execution_id: original.clone(), summary: Some("the original result".into()) }));
    let final_detail = engine.get(&owner, &original).await.unwrap();
    assert_eq!(final_detail.execution.status, AgentExecutionStatus::Completed);
    assert_eq!(final_detail.execution.version, 2);
    assert_eq!(engine.repository.list_events(&owner, &original, 0, 100).await.unwrap().len(), 2);
    assert!(starts.try_recv().is_err()); lifecycle.shutdown().await.unwrap(); db.close().await;
}

#[tokio::test]
async fn automation_stop_does_not_retry_same_version_conflicts_or_database_failures() {
    for failure in ["IGNORE", "FAIL, 'original control writer unavailable'"] {
        let db = nomifun_db::init_database_memory().await.unwrap();
        let owner = nomifun_db::installation_owner_id(db.pool()).await.unwrap();
        let dir = tempfile::tempdir().unwrap(); let lifecycle = AgentExecutionLifecycle::new();
        let (started, mut starts) = mpsc::unbounded_channel(); let (dropped, _) = mpsc::unbounded_channel();
        let engine = crate::production::tests::planning_engine(db.pool().clone(),
            Arc::new(HangingPlanner { started, dropped }), lifecycle.clone(), dir.path().to_owned());
        let original = seed_automation(db.pool(), &owner, &source()).await;
        let captured = engine.get(&owner, &original).await.unwrap();
        sqlx::query(&format!("CREATE TRIGGER original_cancel_fault BEFORE UPDATE OF status ON agent_executions WHEN NEW.status='cancelled' BEGIN SELECT RAISE({failure}); END"))
            .execute(db.pool()).await.unwrap();
        let error = tokio::time::timeout(Duration::from_secs(2),
            engine.cancel_original_automation_execution(&owner, captured)).await
            .expect("an unchanged target or I/O failure must return, not retry").unwrap_err();
        if failure == "IGNORE" { assert!(is_automation_cancel_cas_conflict(&error), "{error}"); }
        else { assert!(!is_automation_cancel_cas_conflict(&error), "{error}"); }
        let unchanged = engine.get(&owner, &original).await.unwrap();
        assert_eq!(unchanged.execution.version, 1);
        assert_eq!(unchanged.execution.status, AgentExecutionStatus::Running);
        assert_eq!(engine.repository.list_events(&owner, &original, 0, 100).await.unwrap().len(), 1);
        assert!(starts.try_recv().is_err()); lifecycle.shutdown().await.unwrap(); db.close().await;
    }
}

#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn automation_stop_retains_the_original_control_after_its_public_waiter_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let db = nomifun_db::init_database(&dir.path().join("original-automation-stop.sqlite")).await.unwrap();
    let owner = nomifun_db::installation_owner_id(db.pool()).await.unwrap();
    let lifecycle = AgentExecutionLifecycle::new();
    let (started, mut starts) = mpsc::unbounded_channel(); let (dropped, _) = mpsc::unbounded_channel();
    let engine = crate::production::tests::planning_engine(db.pool().clone(),
        Arc::new(HangingPlanner { started, dropped }), lifecycle.clone(), dir.path().to_owned());
    let original_source = source();
    let original = seed_automation(db.pool(), &owner, &original_source).await;
    let sibling = seed_automation(db.pool(), &owner, &source()).await;
    let writer = db.pool().begin_with("BEGIN IMMEDIATE").await.unwrap();
    let stopping = engine.clone(); let user = owner.clone();
    drop_public_waiter(&lifecycle, async move { stopping.cancel_automation(&user, &original_source).await }).await;
    assert_eq!(engine.get(&owner, &original).await.unwrap().execution.status, AgentExecutionStatus::Running);
    writer.commit().await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if engine.get(&owner, &original).await.unwrap().execution.status == AgentExecutionStatus::Cancelled { break; }
            tokio::task::yield_now().await;
        }
    }).await.expect("the retained exact source owner commits after its observer is dropped");
    assert_eq!(engine.get(&owner, &sibling).await.unwrap().execution.status, AgentExecutionStatus::Running);
    assert_eq!(engine.repository.list_events(&owner, &original, 0, 100).await.unwrap().len(), 2);
    assert!(starts.try_recv().is_err()); lifecycle.shutdown().await.unwrap(); db.close().await;
}
