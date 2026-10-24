use super::*;
use nomifun_db::IAgentExecutionRepository;
#[path = "native_restart_resume_tests.rs"]
mod native_restart_resume_tests;

fn domain_event(kind: &str) -> nomifun_db::NewAgentExecutionEvent {
    nomifun_db::NewAgentExecutionEvent {event_type:kind.parse().unwrap(),step_id:None,attempt_id:None,
        actor:serde_json::from_value(json!({"type":"system"})).unwrap(),payload:"{}".into()}
}

fn domain_step(id: &str, participant: &str) -> nomifun_db::NewAgentExecutionStep {
    nomifun_db::NewAgentExecutionStep {step_id:id.into(),title:"capacity fixture".into(),spec:"same task".into(),role:None,
        tool_policy:"read_only".parse().unwrap(),kind:"agent".parse().unwrap(),agent_mode:Some("normal".parse().unwrap()),
        profile:None,fanout_group:None,control_policy:None,status:"pending".parse().unwrap(),assigned_participant_id:Some(participant.into()),
        assignment_score:None,assignment_rationale:None,assignment_source:None,assignment_locked:false,
        failure_policy:"fail_execution".parse().unwrap(),preset_prompt:None,graph_x:None,graph_y:None}
}

struct QueueFixture {
    session: AgentSessionLiveRecord,
    owner: PrincipalRef,
    request: NativeResumeRequest,
    execution: String,
    peer: String,
}

impl QueueFixture {
    async fn new(store: &AgentSessionStore, key: &str) -> Self {
        let existing: Option<String> = sqlx::query_scalar("SELECT user_id FROM users ORDER BY id LIMIT 1")
            .fetch_optional(store.test_pool()).await.unwrap();
        let user_id = if let Some(owner) = existing { owner } else {
            let owner = Uuid::now_v7().to_string();
            sqlx::query("INSERT INTO users(user_id,username,password_hash,created_at,updated_at) VALUES(?,?,'isolated-test-owner',1,1)")
                .bind(&owner).bind(format!("native-resume-{key}")).execute(store.test_pool()).await.unwrap();
            owner
        };
        let principal = PrincipalRef {principal_kind:"user".into(),principal_id:user_id};
        let mut session = live_session(session_id());session.owner_ref=principal.clone();
        let preset=Uuid::now_v7().to_string();
        session.agent_binding.preset_revision_ref.preset_id=preset.clone().into();
        session.agent_binding.resolved_snapshot_ref.snapshot_id=Uuid::now_v7().to_string().into();
        let created=store.create_session(create_request(session,key)).await.unwrap();
        let session=created.session;let sid=&session.agent_session_id;
        store.append_event(&append(sid,&format!("ready-{key}"),"runtime_supervisor",&format!("ready-{key}"),
            "session/ready",key,Some(created.opening_ack.event_id),json!({}))).await.unwrap();
        let root=Uuid::now_v7().to_string();let peer=Uuid::now_v7().to_string();
        let participant=Uuid::now_v7().to_string();let source_agent=Uuid::now_v7().to_string();
        sqlx::query("INSERT INTO agent_metadata(agent_id,name,agent_type,agent_source,created_at,updated_at) VALUES(?,'Original queue Agent','nomi','official',1,1)")
            .bind(&source_agent).execute(store.test_pool()).await.unwrap();
        let snapshot=json!({"preset_id":preset,"preset_revision":1,"preset_name":"Original queue Agent",
            "canonical_binding":session.agent_binding,"enabled_capabilities":[],"instructions":"Continue the original admitted task"});
        let repository=nomifun_db::SqliteAgentExecutionRepository::new(store.test_pool().clone());
        let created=repository.create_execution_with_participants(&principal.principal_id,
            &nomifun_db::CreateAgentExecutionParams {goal:"queued native continuation".into(),status:"planning".parse().unwrap(),
                adaptation_policy:"fixed".parse().unwrap(),decision_policy:"automatic".parse().unwrap(),delegation_policy:"automatic".parse().unwrap(),
                max_parallel:1,work_dir:None,lead_conversation_id:None,initial_plan_input:r#"{"mode":"explicit","plan":{"steps":[]}}"#.into()},
            &[nomifun_db::NewAgentExecutionParticipant {participant_id:participant.clone(),source_agent_id:source_agent,
                preset_id:Some(preset),preset_revision:Some(1),agent_snapshot:Some(snapshot.to_string()),provider_id:None,model:None,
                role:None,capability:None,constraints:Some(r#"{"max_concurrency":1}"#.into()),description:None,system_prompt:None,
                enabled_skills:"[]".into(),disabled_builtin_skills:"[]".into(),sort_order:0}],&domain_event("created")).await.unwrap();
        let execution=created.execution_id;
        let planned=repository.reconcile_plan(&principal.principal_id,&execution,created.version,
            &nomifun_db::ReconcileAgentExecutionPlanParams {goal:None,adaptation_policy:None,decision_policy:None,delegation_policy:None,
                keep_step_ids:Vec::new(),new_participants:Vec::new(),retire_participant_ids:Vec::new(),
                new_steps:vec![domain_step(&root,&participant),domain_step(&peer,&participant)],new_dependencies:Vec::new(),
                execution_status:"running".parse().unwrap()},&domain_event("plan_changed")).await.unwrap();
        let queued=repository.create_attempt(&principal.principal_id,&execution,&root,
            planned.steps.iter().find(|step|step.step_id==root).unwrap().version,None,
            &nomifun_db::CreateAgentExecutionAttemptParams {participant_id:Some(participant.clone()),start_immediately:false,
                trigger_reason:"initial".into(),effective_config:"{}".into(),retry_after:None,runtime_state:None},&domain_event("attempt_changed")).await.unwrap();
        let attempt=queued.current_attempt.as_ref().unwrap().attempt.attempt_id.clone();
        repository.start_attempt(&principal.principal_id,&execution,&root,queued.step.version,&attempt,
            queued.current_attempt.as_ref().unwrap().attempt.version,sid.as_ref(),nomifun_db::AgentExecutionAttemptSessionKind::ChildAttempt,
            None, None,&domain_event("attempt_changed")).await.unwrap();
        let operation=OperationId::from(format!("turn:user:{}:{}:{attempt}:initial-turn",principal.principal_id,sid.as_ref()));
        let input=store.append_event(&append(sid,&format!("input-{key}"),"session-api",&format!("input-{key}"),
            "message/user-accepted",operation.as_ref(),None,json!({"content":"Continue the original work"}))).await.unwrap().ack.unwrap();
        let started=store.append_event(&append(sid,&format!("start-{key}"),"session-api",&format!("start-{key}"),
            "turn/started",operation.as_ref(),Some(input.event_id.clone()),json!({"operation_id":operation,"input_event_id":input.event_id,
                "source_message_id":input.event_id,"route_identity":route(),"resolved_snapshot_ref":session.agent_binding.resolved_snapshot_ref}))).await.unwrap().ack.unwrap();
        let lease=store.claim_native_execution(NativeExecutionClaim {owner:principal.clone(),agent_session_id:sid.clone(),
            operation_id:operation.clone(),snapshot:session.agent_binding.resolved_snapshot_ref.clone(),active_set_generation:0,holder:"queue-original".into(),
            expected_fence:0,checkpoint:None}).await.unwrap();
        let state=StrictJsonValue(json!({"version":1,"model_steps":4}));
        let digest=digest_bytes(&canonical_json_bytes(&state.0).unwrap());
        let saved=store.save_native_checkpoint(&append(sid,&format!("checkpoint-{key}"),"runtime_supervisor",&format!("checkpoint-{key}"),
            "runtime/progress-recorded",operation.as_ref(),Some(started.event_id),json!({"producer_seq":1,"event":{
                "event":"execution_checkpoint_saved","step":4,"revision":1,"digest":digest}})),
            NativeCheckpointWrite {owner:principal.clone(),operation_id:operation.clone(),snapshot:session.agent_binding.resolved_snapshot_ref.clone(),active_set_generation:0,
                expected_revision:0,execution_fence:lease.fence(),lease:Some(lease.clone()),state}).await.unwrap();
        let pause=store.pause_native_execution(&lease,"OWNER_REQUESTED",true).await.unwrap();
        let peer_step=repository.get_step_detail(&principal.principal_id,&execution,&peer).await.unwrap().unwrap();
        let peer_queued=repository.create_attempt(&principal.principal_id,&execution,&peer,peer_step.step.version,None,
            &nomifun_db::CreateAgentExecutionAttemptParams {participant_id:Some(participant),start_immediately:false,
                trigger_reason:"initial".into(),effective_config:"{}".into(),retry_after:None,runtime_state:None},&domain_event("attempt_changed")).await.unwrap();
        let mut peer_session=live_session(session_id());
        peer_session.owner_ref=principal.clone();
        peer_session.agent_binding=session.agent_binding.clone();
        let peer_key=format!("{key}-peer");
        let peer_created=store.create_session(create_request(peer_session,&peer_key)).await.unwrap();
        store.append_event(&append(&peer_created.session.agent_session_id,&format!("ready-{peer_key}"),"runtime_supervisor",&format!("ready-{peer_key}"),
            "session/ready",&peer_key,Some(peer_created.opening_ack.event_id),json!({}))).await.unwrap();
        let peer_attempt=&peer_queued.current_attempt.as_ref().unwrap().attempt;
        repository.start_attempt(&principal.principal_id,&execution,&peer,peer_queued.step.version,&peer_attempt.attempt_id,
            peer_attempt.version,peer_created.session.agent_session_id.as_ref(),nomifun_db::AgentExecutionAttemptSessionKind::ChildAttempt,
            None, None,&domain_event("attempt_changed")).await.unwrap();
        let detail=repository.get_execution_detail(&principal.principal_id,&execution).await.unwrap().unwrap();
        assert_eq!(detail.participants.len(),1);assert_eq!(detail.steps.len(),2);assert_eq!(detail.attempts.len(),2);
        Self {session,owner:principal,request:NativeResumeRequest {operation_id:operation,idempotency_key:format!("resume-{key}"),
            expected_pause_revision:pause.revision,expected_checkpoint_revision:saved.revision,expected_checkpoint_digest:saved.digest,
            budget:Default::default(),cleanup_attestation:None},execution,peer}
    }

    async fn prepare(&self, store: &AgentSessionStore) -> NativeResumePreparation {
        let facts=store.native_recovery_facts(&self.session.agent_session_id,&self.request.operation_id).await.unwrap();
        NativeResumePreparation {expected_head_seq:facts.head.last_seq,expected_fence:facts.execution_fence,
            snapshot:self.session.agent_binding.resolved_snapshot_ref.clone(),active_set_generation:0,observations:Vec::new(),
            checkpoint_state:StrictJsonValue(json!({"version":1,"model_steps":4,"turn_operation_id":self.request.operation_id,
                "active_set_generation":0,"binding":{"agent_session_id":self.session.agent_session_id,"resolved_snapshot_ref":self.session.agent_binding.resolved_snapshot_ref}}))}
    }

    async fn finish_peer(&self, store: &AgentSessionStore) {
        let repository=nomifun_db::SqliteAgentExecutionRepository::new(store.test_pool().clone());
        let peer=repository.get_step_detail(&self.owner.principal_id,&self.execution,&self.peer).await.unwrap().unwrap();
        if peer.step.status=="completed" {return;}
        let attempt=&peer.current_attempt.as_ref().unwrap().attempt;
        repository.settle_attempt(&self.owner.principal_id,&self.execution,&self.peer,peer.step.version,&attempt.attempt_id,attempt.version,None,
            &nomifun_db::SettleAgentExecutionAttemptParams {expected_active_session_turn:None,expected_terminal_turn:None,
                attempt_status:"completed".parse().unwrap(),step_status:"completed".parse().unwrap(),execution_status:None,
                question:None,error:None,output_summary:Some(Some("controlled peer completed".into())),output_files:None,tokens:None,
                retry_after:None,runtime_state:None,started_at:None,finished_at:None,loop_repeat_reset:None},&domain_event("attempt_changed")).await.unwrap();
    }
}

#[tokio::test]
async fn native_resume_waits_in_the_original_event_chain_without_advancing_pause_checkpoint_or_budget() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let f=QueueFixture::new(&store,"queue-budget").await;
    let sid=&f.session.agent_session_id;
    let before=store.inspect_latest_native_execution(&f.owner,sid).await.unwrap().unwrap();
    let cursor=store.current_cursor(sid).await.unwrap();
    assert!(matches!(store.commit_native_resume(&f.owner,sid,&f.request,f.prepare(&store).await).await,
        Err(SessionStoreError::ExecutionCapacityPending)));
    assert_eq!(store.current_cursor(sid).await.unwrap(),cursor);
    let first=store.request_native_resume(&f.owner,sid,&f.request).await.unwrap();
    let repeated=store.request_native_resume(&f.owner,sid,&f.request).await.unwrap();
    assert_eq!(first.request_event_id,repeated.request_event_id);assert!(repeated.duplicate);
    let queued=store.inspect_latest_native_execution(&f.owner,sid).await.unwrap().unwrap();
    assert!(queued.resume_requested);assert_eq!(queued.state,"paused");
    assert_eq!(queued.budget,before.budget);assert_eq!(queued.checkpoint_revision,before.checkpoint_revision);
    assert_eq!(queued.execution_generation,before.execution_generation);
    assert_eq!(store.pending_native_resume_requests(0,100).await.unwrap().len(),1);
    f.finish_peer(&store).await;
    let authorized=store.commit_native_resume(&f.owner,sid,&f.request,f.prepare(&store).await).await.unwrap();
    assert_ne!(first.request_event_id,authorized.authorization_event_id);
    let resumed=store.inspect_latest_native_execution(&f.owner,sid).await.unwrap().unwrap();
    assert!(!resumed.resume_requested && resumed.pause.is_none());
    assert_eq!(resumed.budget.revision,before.budget.revision+1);
    assert_eq!(resumed.budget.journal_bytes,before.budget.journal_bytes);
    assert_eq!(resumed.budget.journal_records,before.budget.journal_records);
    assert_eq!(resumed.budget.session_payload_bytes,before.budget.session_payload_bytes);
    assert_eq!(resumed.model_progress["checkpoint_model_steps"],4);
    assert!(store.pending_native_resume_requests(0,100).await.unwrap().is_empty());
    let roots:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_turns WHERE session_id=?")
        .bind(sid.as_ref()).fetch_one(store.test_pool()).await.unwrap();assert_eq!(roots,1);
}

#[tokio::test]
async fn native_resume_queue_stop_and_changed_requests_cannot_authorize_a_replacement() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let f=QueueFixture::new(&store,"queue-stop").await;
    let sid=&f.session.agent_session_id;
    store.request_native_resume(&f.owner,sid,&f.request).await.unwrap();
    let before=store.current_cursor(sid).await.unwrap();
    let mut changed=f.request.clone();changed.budget.additional_journal_mib=1;
    assert!(matches!(store.request_native_resume(&f.owner,sid,&changed).await,Err(SessionStoreError::IdempotencyConflict(_))));
    assert_eq!(store.current_cursor(sid).await.unwrap(),before);
    let prepared=f.prepare(&store).await;
    store.cancel_active_turn(sid,"stop-queued-native".into(),"session-api".into()).await.unwrap();
    f.finish_peer(&store).await;
    assert!(store.native_resume_request_receipt(&f.owner,sid,&f.request).await.unwrap().is_none());
    assert!(store.pending_native_resume_requests(0,100).await.unwrap().is_empty());
    assert!(store.commit_native_resume(&f.owner,sid,&f.request,prepared).await.is_err());
    assert!(store.request_native_resume(&f.owner,sid,&f.request).await.is_err());
    assert_eq!(store.read_turn_receipt(sid,&f.request.operation_id).await.unwrap().status,TurnReceiptStatus::Cancelled);
}

#[tokio::test]
async fn native_resume_blocked_control_retains_the_original_paused_task_and_explains_the_queue_result() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let f=QueueFixture::new(&store,"queue-blocked").await;
    let sid=&f.session.agent_session_id;
    store.request_native_resume(&f.owner,sid,&f.request).await.unwrap();
    store.block_native_resume_request(&f.owner,sid,&f.request,"The original checkpoint needs explicit reconciliation").await.unwrap();
    let observed=store.inspect_latest_native_execution(&f.owner,sid).await.unwrap().unwrap();
    assert!(!observed.resume_requested);assert_eq!(observed.state,"paused");
    assert!(observed.resume_blocked_reason.unwrap().contains("checkpoint"));
    assert!(store.pending_native_resume_requests(0,100).await.unwrap().is_empty());
    assert!(store.native_resume_receipt(&f.owner,sid,&f.request).await.unwrap().is_none());
    assert!(store.request_native_resume(&f.owner,sid,&f.request).await.is_err());
    f.finish_peer(&store).await;
    assert!(store.commit_native_resume(&f.owner,sid,&f.request,f.prepare(&store).await).await.is_err(),
        "a blocked original control cannot silently become an authorization on replay");
}

#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn native_resume_commit_rechecks_capacity_after_waiting_for_the_canonical_writer() {
    let directory=tempfile::tempdir().unwrap();let path=directory.path().join("native-capacity-writer.db");
    let db=nomifun_db::init_database(&path).await.unwrap();
    let store=AgentSessionStore::from_pool(db.pool().clone()).await.unwrap();
    let f=QueueFixture::new(&store,"queue-writer").await;f.finish_peer(&store).await;
    let prepared=f.prepare(&store).await;
    let mut writer=store.test_pool().begin().await.unwrap();
    sqlx::query("UPDATE agent_execution_attempts SET status='running' WHERE execution_id=? AND step_id=?")
        .bind(&f.execution).bind(&f.peer).execute(&mut *writer).await.unwrap();
    sqlx::query("UPDATE agent_execution_steps SET status='running' WHERE execution_id=? AND step_id=?")
        .bind(&f.execution).bind(&f.peer).execute(&mut *writer).await.unwrap();
    let resume_store=store.clone();let principal=f.owner.clone();let session=f.session.agent_session_id.clone();let request=f.request.clone();
    let (started,ready)=tokio::sync::oneshot::channel();
    let resume=tokio::spawn(async move {started.send(()).unwrap();resume_store.commit_native_resume(&principal,&session,&request,prepared).await});
    ready.await.unwrap();writer.commit().await.unwrap();
    assert!(matches!(resume.await.unwrap(),Err(SessionStoreError::ExecutionCapacityPending)));
    assert_eq!(store.head(&f.session.agent_session_id).await.unwrap().status,"paused");
    assert!(store.native_resume_receipt(&f.owner,&f.session.agent_session_id,&f.request).await.unwrap().is_none());
}

#[tokio::test]
async fn native_resume_queue_survives_database_reopen_and_consumes_the_original_request_once() {
    let directory=tempfile::tempdir().unwrap();let path=directory.path().join("native-resume-reopen.db");
    let db=nomifun_db::init_database(&path).await.unwrap();
    let store=AgentSessionStore::from_pool(db.pool().clone()).await.unwrap();
    let f=QueueFixture::new(&store,"queue-reopen").await;
    let queued=store.request_native_resume(&f.owner,&f.session.agent_session_id,&f.request).await.unwrap();
    drop(store);db.close().await;
    let reopened=AgentSessionStore::connect_existing(&path).await.unwrap();
    let pending=reopened.pending_native_resume_requests(0,100).await.unwrap();
    assert_eq!(pending.len(),1);assert_eq!(pending[0].receipt.request_event_id,queued.request_event_id);
    assert_eq!(pending[0].request.operation_id,f.request.operation_id);
    assert_eq!(pending[0].owner,f.owner);
    f.finish_peer(&reopened).await;
    let receipt=reopened.commit_native_resume(&f.owner,&f.session.agent_session_id,&pending[0].request,f.prepare(&reopened).await).await.unwrap();
    let replay=reopened.native_resume_receipt(&f.owner,&f.session.agent_session_id,&f.request).await.unwrap().unwrap();
    assert_eq!(receipt.authorization_event_id,replay.authorization_event_id);
    assert!(replay.duplicate);assert!(reopened.pending_native_resume_requests(0,100).await.unwrap().is_empty());
    reopened.test_pool().close().await;
}

#[tokio::test]
async fn native_claim_after_resume_authorization_still_obeys_the_committed_domain_stop_fence() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let f=QueueFixture::new(&store,"queue-stop-claim").await;
    let sid=&f.session.agent_session_id;f.finish_peer(&store).await;
    store.commit_native_resume(&f.owner,sid,&f.request,f.prepare(&store).await).await.unwrap();
    let saved=store.load_native_checkpoint(&f.owner,sid,&f.request.operation_id).await.unwrap().unwrap();
    let (root,version):(String,i64)=sqlx::query_as("SELECT step_id,version FROM agent_execution_steps WHERE execution_id=? AND status='running'")
        .bind(&f.execution).fetch_one(store.test_pool()).await.unwrap();
    let repository=nomifun_db::SqliteAgentExecutionRepository::new(store.test_pool().clone());
    use nomifun_db::IAgentExecutionRepository;
    let execution=repository.get_execution(&f.owner.principal_id,&f.execution).await.unwrap().unwrap();
    repository.cancel_step(&f.owner.principal_id,&f.execution,execution.version,&root,version,"task".parse().unwrap(),
        &nomifun_db::NewAgentExecutionEvent {event_type:"step_changed".parse().unwrap(),step_id:Some(root.clone()),
            attempt_id:None,actor:serde_json::from_value(json!({"type":"user","user_id":f.owner.principal_id})).unwrap(),payload:"{}".into()}).await.unwrap();
    let before=store.current_cursor(sid).await.unwrap();
    assert!(store.claim_native_execution(NativeExecutionClaim {owner:f.owner.clone(),agent_session_id:sid.clone(),
        operation_id:f.request.operation_id.clone(),snapshot:f.session.agent_binding.resolved_snapshot_ref.clone(),active_set_generation:0,holder:"late-resume-owner".into(),
        expected_fence:saved.execution_fence,checkpoint:Some((saved.revision,saved.digest,saved.through_seq))}).await.is_err());
    assert_eq!(store.current_cursor(sid).await.unwrap(),before);
    assert_eq!(store.read_turn_receipt(sid,&f.request.operation_id).await.unwrap().status,TurnReceiptStatus::Running,
        "the native claim guard cannot fabricate a cancellation terminal while the exact host stop is pending");
}

#[tokio::test]
async fn domain_stop_invalidates_queued_projection_before_native_cleanup_and_cannot_starve_later_intents() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();
    let stopped=QueueFixture::new(&store,"queue-domain-stop").await;
    store.request_native_resume(&stopped.owner,&stopped.session.agent_session_id,&stopped.request).await.unwrap();
    let repository=nomifun_db::SqliteAgentExecutionRepository::new(store.test_pool().clone());
    let detail=repository.get_execution_detail(&stopped.owner.principal_id,&stopped.execution).await.unwrap().unwrap();
    let root=detail.steps.iter().find(|step|step.step_id!=stopped.peer).unwrap();
    repository.cancel_step(&stopped.owner.principal_id,&stopped.execution,detail.execution.version,&root.step_id,root.version,"task".parse().unwrap(),
        &nomifun_db::NewAgentExecutionEvent {actor:serde_json::from_value(json!({"type":"user","user_id":stopped.owner.principal_id})).unwrap(),
            ..domain_event("step_changed")}).await.unwrap();
    // No cleanup owner has delivered a canonical cancellation yet. Only the
    // committed original domain fence makes this continuation ineligible.
    let inspection=store.inspect_latest_native_execution(&stopped.owner,&stopped.session.agent_session_id).await.unwrap().unwrap();
    assert_eq!(inspection.state,"paused");assert!(!inspection.resume_requested);
    assert!(store.native_resume_request_receipt(&stopped.owner,&stopped.session.agent_session_id,&stopped.request).await.unwrap().is_none());
    let live=QueueFixture::new(&store,"queue-after-domain-stop").await;
    let receipt=store.request_native_resume(&live.owner,&live.session.agent_session_id,&live.request).await.unwrap();
    let pending=store.pending_native_resume_requests(0,1).await.unwrap();
    assert_eq!(pending.len(),1);assert_eq!(pending[0].receipt.request_event_id,receipt.request_event_id);
}
