//! Exact Cron cancellation crosses the real canonical native writer fence.
use super::*;
use crate::{NativeCheckpointWrite, NativeExecutionClaim, NativeResumePreparation, NativeResumeRequest};
use nomifun_db::{ICronRepository, ReserveCronRunParams, SqliteCronRepository};

struct CronFixture { session: AgentSessionLiveRecord, owner: PrincipalRef, operation: OperationId,
    root:EventId,started: EventId, job: String, run: String }

impl CronFixture {
    async fn new(store: &AgentSessionStore, key: &str) -> Self {
        let user = Uuid::now_v7().to_string();
        sqlx::query("INSERT INTO users(user_id,username,password_hash,created_at,updated_at) VALUES(?,?,'isolated-cron-test',1,1)")
            .bind(&user).bind(format!("cron-{key}")).execute(store.test_pool()).await.unwrap();
        let owner = PrincipalRef { principal_kind: "user".into(), principal_id: user.clone() };
        let mut session = live_session(session_id());
        session.owner_ref = owner.clone();
        let created = store.create_session(create_request(session,key)).await.unwrap();
        let session = created.session;
        let sid = &session.agent_session_id;
        store.append_event(&append(sid,&format!("ready-{key}"),"runtime_supervisor",&format!("ready-{key}"),
            "session/ready",key,Some(created.opening_ack.event_id),json!({}))).await.unwrap();
        let job = Uuid::now_v7().to_string();
        let run = Uuid::now_v7().to_string();
        sqlx::query("INSERT INTO cron_jobs(cron_job_id,user_id,name,schedule_kind,schedule_value,payload_message,conversation_id,agent_type,created_by,created_at,updated_at) \
            VALUES(?,?,'Exact Cron','every','60000','Original input',?,'nomi','user',1,1)")
            .bind(&job).bind(&user).bind(sid.as_ref()).execute(store.test_pool()).await.unwrap();
        let repository = SqliteCronRepository::new(store.test_pool().clone());
        repository.reserve_run(&user,&ReserveCronRunParams { cron_job_run_id:run.clone(),cron_job_id:job.clone(),
            trigger_kind:"run_now".into(),operation_key:format!("original:{run}"),request_fingerprint:"original".into(),
            schedule_revision:None,planned_at_ms:None,now:1 }).await.unwrap();
        repository.attach_run_conversation(&user,&run,sid.as_ref(),2).await.unwrap();
        let operation: OperationId = format!("turn:user:{user}:{}:cron:{run}:turn",sid.as_ref()).into();
        let input = append(sid,&format!("input-{key}"),"session-api",&format!("input-{key}"),"message/user-accepted",
            operation.as_ref(),None,json!({"content":"Original scheduled input"}));
        store.append_event(&input).await.unwrap();
        let route = ChatRouteIdentity::new("coding.codex@1",nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT,"cron-test-route".into(),4);
        let started = store.append_event(&append(sid,&format!("start-{key}"),"session-api",&format!("start-{key}"),
            "turn/started",operation.as_ref(),Some(input.event_id.clone()),json!({"operation_id":operation,
                "input_event_id":input.event_id,"source_message_id":input.event_id,"route_identity":route,
                "resolved_snapshot_ref":session.agent_binding.resolved_snapshot_ref}))).await.unwrap().ack.unwrap().event_id;
        Self { session,owner,operation,root:input.event_id,started,job,run }
    }
    fn claim(&self, holder: &str) -> NativeExecutionClaim {
        NativeExecutionClaim { owner:self.owner.clone(),agent_session_id:self.session.agent_session_id.clone(),operation_id:self.operation.clone(),
            snapshot:self.session.agent_binding.resolved_snapshot_ref.clone(),active_set_generation:0,holder:holder.into(),expected_fence:0,checkpoint:None }
    }
    async fn cancel(&self, store: &AgentSessionStore) {
        SqliteCronRepository::new(store.test_pool().clone()).request_run_cancellation(&self.owner.principal_id,&self.job,&self.run,3).await.unwrap();
    }
}

#[tokio::test]
async fn exact_cron_cancellation_survives_store_reopen_and_prevents_last_native_claim() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let cancelled = CronFixture::new(&store,"cancel-before-claim").await;
    let sibling = CronFixture::new(&store,"independent-run").await;
    cancelled.cancel(&store).await;
    let reopened = AgentSessionStore::from_pool(store.test_pool().clone()).await.unwrap();
    assert!(matches!(reopened.claim_native_execution(cancelled.claim("late-after-stop")).await,Err(SessionStoreError::ExecutionFenced)));
    assert!(!reopened.has_native_execution_owner(&cancelled.session.agent_session_id,&cancelled.operation).await.unwrap());
    reopened.claim_native_execution(sibling.claim("independent-original")).await.unwrap();
    assert!(reopened.has_native_execution_owner(&sibling.session.agent_session_id,&sibling.operation).await.unwrap());
}

#[tokio::test]
async fn exact_cron_cancellation_fences_a_saved_native_resume_without_erasing_checkpoint() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let fixture = CronFixture::new(&store,"cancel-before-resume").await;
    let sid = &fixture.session.agent_session_id;
    let lease = store.claim_native_execution(fixture.claim("original-owner")).await.unwrap();
    let checkpoint = StrictJsonValue(json!({"version":1,"model_steps":0}));
    let digest = digest_bytes(&canonical_json_bytes(&checkpoint.0).unwrap());
    let saved = store.save_native_checkpoint(&append(sid,"cron-checkpoint","runtime_supervisor","cron-checkpoint",
        "runtime/progress-recorded",fixture.operation.as_ref(),Some(fixture.started.clone()),json!({"producer_seq":1,"event":{
            "event":"execution_checkpoint_saved","step":0,"revision":1,"digest":digest}})),
        NativeCheckpointWrite {owner:fixture.owner.clone(),operation_id:fixture.operation.clone(),snapshot:fixture.session.agent_binding.resolved_snapshot_ref.clone(),
            active_set_generation:0,expected_revision:0,execution_fence:lease.fence(),lease:Some(lease.clone()),state:checkpoint}).await.unwrap();
    let pause = store.pause_native_execution(&lease,"OWNER_REQUESTED",true).await.unwrap();
    let facts = store.native_recovery_facts(sid,&fixture.operation).await.unwrap();
    let request = NativeResumeRequest {operation_id:fixture.operation.clone(),idempotency_key:"late-cron-resume".into(),
        expected_pause_revision:pause.revision,expected_checkpoint_revision:saved.revision,expected_checkpoint_digest:saved.digest.clone(),
        budget:Default::default(),cleanup_attestation:None};
    let prepared = NativeResumePreparation {expected_head_seq:facts.head.last_seq,expected_fence:facts.execution_fence,
        snapshot:fixture.session.agent_binding.resolved_snapshot_ref.clone(),active_set_generation:0,observations:vec![],
        checkpoint_state:StrictJsonValue(json!({"version":1,"model_steps":0,"turn_operation_id":fixture.operation,"active_set_generation":0,
            "binding":{"agent_session_id":sid,"resolved_snapshot_ref":fixture.session.agent_binding.resolved_snapshot_ref}}))};
    fixture.cancel(&store).await;
    assert!(matches!(store.commit_native_resume(&fixture.owner,sid,&request,prepared).await,Err(SessionStoreError::ExecutionFenced)));
    assert_eq!(store.load_native_checkpoint(&fixture.owner,sid,&fixture.operation).await.unwrap().unwrap().digest,saved.digest);
    assert_eq!(store.head(sid).await.unwrap().status,"paused");
    let mut tx = store.test_pool().begin().await.unwrap();
    assert!(!nomifun_db::native_resume_target_eligible_tx(&mut tx,&fixture.owner.principal_id,sid.as_ref(),fixture.operation.as_ref()).await.unwrap());
    tx.commit().await.unwrap();
}

#[tokio::test]
async fn cron_stop_denies_new_model_tool_and_effect_admission_but_keeps_original_settlement_and_checkpoint() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();
    let fixture=CronFixture::new(&store,"cron-original-settlement").await;
    let sid=&fixture.session.agent_session_id;
    let lease=store.claim_native_execution(fixture.claim("original-native-owner")).await.unwrap();
    let model=ChatOperationClaimRequest {agent_session_id:sid.clone(),operation_id:"original-cron-model".into(),
        turn_operation_id:fixture.operation.clone(),causation_event_id:fixture.root.clone(),
        route_identity:ChatRouteIdentity::new("coding.codex@1",nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT,"cron-test-route".into(),4),
        resolved_snapshot_ref:fixture.session.agent_binding.resolved_snapshot_ref.clone()};
    store.claim_native_chat_operation(&lease,model.clone()).await.unwrap();
    let tool=|key:&str|append(sid,key,"runtime_supervisor",key,"tool/call-started",key,Some(fixture.started.clone()),
        json!({"operation_id":key,"capability_id":"workspace.files","action_id":"workspace.files/write"}));
    let started_tool=store.append_native_event(&lease,&tool("original-cron-write"),None).await.unwrap().ack.unwrap();
    let reserved_tool=store.append_native_event(&lease,&tool("prepared-cron-write"),None).await.unwrap().ack.unwrap();
    let pending=EffectEventRequest {agent_session_id:sid.clone(),effect_id:"original-cron-effect".into(),turn_id:fixture.operation.clone(),
        operation_id:"original-cron-write".into(),owner_domain:"workspace".into(),capability_module:"workspace.files".into(),
        action_id:"workspace.files/write".into(),resource_binding_id:None,resource_key:Some("fixture-exact-resource".into()),
        input_digest:digest('7'),recorded_at:1_788_000_000_010,event_id:"original-cron-effect-started".into(),
        producer_id:"capability-host".into(),idempotency_key:"original-cron-effect-started".into(),correlation_id:"original-cron-effect".into(),
        strategy:EffectStrategy::ManagedEffect,causation_event_id:Some(started_tool.event_id),
        payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({})))};
    let started_effect=store.record_effect_started(pending.clone()).await.unwrap().ack.unwrap();
    fixture.cancel(&store).await;
    assert!(matches!(store.verify_native_execution(&lease).await,Err(SessionStoreError::ExecutionFenced)));
    assert!(matches!(store.claim_native_chat_operation(&lease,ChatOperationClaimRequest {
        operation_id:"late-cron-model".into(),..model}).await,Err(SessionStoreError::ExecutionFenced)));
    assert!(matches!(store.append_native_event(&lease,&tool("late-cron-tool"),None).await,Err(SessionStoreError::ExecutionFenced)));
    let late_effect=EffectEventRequest {effect_id:"late-cron-effect".into(),operation_id:"prepared-cron-write".into(),
        event_id:"late-cron-effect-started".into(),idempotency_key:"late-cron-effect-started".into(),correlation_id:"late-cron-effect".into(),
        causation_event_id:Some(reserved_tool.event_id),..pending.clone()};
    assert!(matches!(store.record_effect_started(late_effect).await,Err(SessionStoreError::ExecutionFenced)));
    let observation=|key:&str,sequence:u64,event:serde_json::Value|append(sid,key,"runtime_supervisor",key,"runtime/progress-recorded",
        fixture.operation.as_ref(),Some(fixture.started.clone()),json!({"producer_seq":sequence,"event":event}));
    store.append_native_observation(&lease,&observation("original-cron-observation",1,
        json!({"event":"host_tool_settled","operation_id":"original-cron-write"})),None).await.unwrap();
    store.record_effect_terminal(EffectEventRequest {event_id:"original-cron-effect-succeeded".into(),
        producer_id:"owning-plugin".into(),causation_event_id:Some(started_effect.event_id),
        payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"receipt":"original-operation-settled"}))),..pending},
        EffectTerminalState::Succeeded).await.unwrap();
    assert_eq!(store.read_effect(sid,"original-cron-effect").await.unwrap().unwrap().state,AgentEffectState::Returned);
    let checkpoint=StrictJsonValue(json!({"version":1,"model_steps":1}));
    let checkpoint_digest=digest_bytes(&canonical_json_bytes(&checkpoint.0).unwrap());
    let saved=store.save_native_checkpoint(&observation("original-cron-checkpoint",2,
        json!({"event":"execution_checkpoint_saved","step":1,"revision":1,"digest":checkpoint_digest})),
        NativeCheckpointWrite {owner:fixture.owner.clone(),operation_id:fixture.operation.clone(),snapshot:fixture.session.agent_binding.resolved_snapshot_ref.clone(),
            active_set_generation:0,expected_revision:0,execution_fence:lease.fence(),lease:Some(lease.clone()),state:checkpoint}).await.unwrap();
    assert_eq!(saved.digest,checkpoint_digest);
    assert_eq!(store.load_native_checkpoint(&fixture.owner,sid,&fixture.operation).await.unwrap().unwrap().digest,saved.digest);
    store.cancel_exact_turn(sid,&fixture.operation,"original-cron-canonical-stop".into(),"session_api".into()).await.unwrap();
    store.append_native_observation(&lease,&observation("original-cron-cleanup-proof",3,
        json!({"event":"host_cleanup_proven"})),None).await.unwrap();
    assert_eq!(store.read_turn_receipt(sid,&fixture.operation).await.unwrap().status,TurnReceiptStatus::Cancelled);
}

#[tokio::test]
async fn attached_cron_cancellation_keeps_original_owner_after_job_loss_and_never_attaches_an_orphan() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();
    let fixture=CronFixture::new(&store,"cron-job-loss").await;
    let repository=SqliteCronRepository::new(store.test_pool().clone());
    let unclaimed=Uuid::now_v7().to_string();
    repository.reserve_run(&fixture.owner.principal_id,&ReserveCronRunParams {cron_job_run_id:unclaimed.clone(),cron_job_id:fixture.job.clone(),
        trigger_kind:"run_now".into(),operation_key:format!("unclaimed:{unclaimed}"),request_fingerprint:"unclaimed-original".into(),
        schedule_revision:None,planned_at_ms:None,now:2}).await.unwrap();
    // Fault injection only: ordinary configuration deletion rejects reserved
    // runs. A historical/damaged absent job must still not erase their fence.
    sqlx::query("DELETE FROM cron_jobs WHERE cron_job_id=?").bind(&fixture.job).execute(store.test_pool()).await.unwrap();
    let native=store.claim_native_execution(fixture.claim("original-without-config")).await.unwrap();
    assert!(store.verify_native_execution(&native).await.is_ok(),"missing configuration is not itself a cancellation");
    let foreign=Uuid::now_v7().to_string();
    assert!(repository.request_run_cancellation(&foreign,&fixture.job,&fixture.run,3).await.is_err());
    repository.request_run_cancellation(&fixture.owner.principal_id,&fixture.job,&fixture.run,4).await.unwrap();
    assert!(matches!(store.verify_native_execution(&native).await,Err(SessionStoreError::ExecutionFenced)));
    let reopened=AgentSessionStore::from_pool(store.test_pool().clone()).await.unwrap();
    assert!(matches!(reopened.claim_native_execution(fixture.claim("late-after-loss-stop")).await,Err(SessionStoreError::ExecutionFenced)));
    assert_eq!(repository.attached_run_owner_for_scheduler(&fixture.run).await.unwrap(),Some(fixture.owner.principal_id.clone()));
    assert!(repository.get_run_reservation(&fixture.owner.principal_id,&fixture.job,&unclaimed).await.unwrap().is_none());
    assert!(repository.request_run_cancellation(&fixture.owner.principal_id,&fixture.job,&unclaimed,5).await.is_err());
    assert!(repository.attach_run_conversation(&fixture.owner.principal_id,&unclaimed,fixture.session.agent_session_id.as_ref(),5).await.is_err());
}
