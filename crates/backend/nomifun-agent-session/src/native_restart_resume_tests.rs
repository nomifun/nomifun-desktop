use super::*;
use nomifun_agent_contracts::digest_payload;

async fn bind_restart(store:&AgentSessionStore,f:&mut QueueFixture,key:&str)->nomifun_db::AgentExecutionRestartCommand {
    let repository=nomifun_db::SqliteAgentExecutionRepository::new(store.test_pool().clone());
    let detail=repository.get_execution_detail(&f.owner.principal_id,&f.execution).await.unwrap().unwrap();
    let step=detail.steps.iter().find(|step|step.step_id!=f.peer).unwrap();
    let attempt=detail.attempts.iter().find(|attempt|attempt.attempt.step_id==step.step_id).unwrap();
    let operation=format!("restart-step:{}",digest_payload(&json!({"kind":"restart_step","owner_id":f.owner.principal_id,
        "execution_id":f.execution,"step_id":step.step_id,"idempotency_key":key})).unwrap().as_ref());
    let actor=json!({"type":"user","user_id":f.owner.principal_id});
    let request=json!({"idempotency_key":key,"expected_execution_version":detail.execution.version,
        "expected_step_version":step.version,"expected_attempt_id":attempt.attempt.attempt_id});
    let event=nomifun_db::NewAgentExecutionEvent {event_type:"step_changed".parse().unwrap(),step_id:Some(step.step_id.clone()),
        attempt_id:Some(attempt.attempt.attempt_id.clone()),actor:serde_json::from_value(actor.clone()).unwrap(),payload:json!({"change":"restart_requested",
            "operation_id":operation,"command_request":{"kind":"restart_step","owner_id":f.owner.principal_id,
                "execution_id":f.execution,"step_id":step.step_id,"request":request,"actor":actor}}).to_string()};
    let accepted=repository.request_step_restart(&f.owner.principal_id,&f.execution,&step.step_id,detail.execution.version,
        step.version,Some(&attempt.attempt.attempt_id),&event).await.unwrap();
    f.request.idempotency_key=format!("execution-task-restart:{operation}");
    f.request.budget.retry_stall_guards=true;
    accepted
}

async fn block_restart(store:&AgentSessionStore,f:&QueueFixture,command:&nomifun_db::AgentExecutionRestartCommand) {
    let operation:Value=serde_json::from_str(&command.event.payload).unwrap();
    nomifun_db::SqliteAgentExecutionRepository::new(store.test_pool().clone()).complete_step_restart(&f.owner.principal_id,
        &f.execution,operation["operation_id"].as_str().unwrap(),&nomifun_db::AgentExecutionRestartCompletion::Blocked {
            code:"NATIVE_RESTART_REQUIRES_RECONCILIATION".into()}).await.unwrap();
}

#[tokio::test]
async fn native_restart_key_needs_a_real_original_user_command_and_preserves_ordinary_resume() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let mut f=QueueFixture::new(&store,"restart-key").await;
    let ordinary=f.request.clone();f.request.idempotency_key="execution-task-restart:invented".into();f.request.budget.retry_stall_guards=true;
    f.finish_peer(&store).await;
    let cursor=store.current_cursor(&f.session.agent_session_id).await.unwrap();
    assert!(store.commit_native_resume(&f.owner,&f.session.agent_session_id,&f.request,f.prepare(&store).await).await.is_err());
    assert!(store.request_native_resume(&f.owner,&f.session.agent_session_id,&f.request).await.is_err());
    assert_eq!(store.current_cursor(&f.session.agent_session_id).await.unwrap(),cursor);
    f.request=ordinary;
    store.commit_native_resume(&f.owner,&f.session.agent_session_id,&f.request,f.prepare(&store).await).await.unwrap();
}

#[tokio::test]
async fn native_restart_writer_checks_the_command_frozen_fence_even_when_pause_revision_and_digest_match() {
    for queue in [false,true] {
        let store=AgentSessionStore::open_in_memory().await.unwrap();let mut f=QueueFixture::new(&store,"restart-fence").await;
        bind_restart(&store,&mut f,"frozen-fence").await;
        f.finish_peer(&store).await;
        // Controlled fixture fault: current pause/row agree with each other,
        // but both moved beyond the original command's immutable fence.
        sqlx::query("UPDATE agent_turns SET execution_fence=execution_fence+1,native_pause_json=json_set(native_pause_json,'$.execution_fence',execution_fence+1) WHERE session_id=? AND operation_id=?")
            .bind(f.session.agent_session_id.as_ref()).bind(f.request.operation_id.as_ref()).execute(store.test_pool()).await.unwrap();
        let cursor=store.current_cursor(&f.session.agent_session_id).await.unwrap();
        let failed=if queue {store.request_native_resume(&f.owner,&f.session.agent_session_id,&f.request).await.is_err()}
            else {store.commit_native_resume(&f.owner,&f.session.agent_session_id,&f.request,f.prepare(&store).await).await.is_err()};
        assert!(failed);assert_eq!(store.current_cursor(&f.session.agent_session_id).await.unwrap(),cursor);
        assert!(store.native_resume_receipt(&f.owner,&f.session.agent_session_id,&f.request).await.unwrap().is_none());
    }
}

#[tokio::test]
async fn resolved_restart_or_replaced_target_cannot_authorize_or_queue_a_native_resume() {
    for replaced in [false,true] {
        let store=AgentSessionStore::open_in_memory().await.unwrap();let mut f=QueueFixture::new(&store,"restart-resolved").await;
        let command=bind_restart(&store,&mut f,"resolved-original").await;
        f.finish_peer(&store).await;
        let prepared=f.prepare(&store).await;
        if replaced {
            sqlx::query("UPDATE agent_execution_steps SET superseded_in_revision=2 WHERE execution_id=? AND step_id=?")
                .bind(&f.execution).bind(command.event.step_id.as_deref().unwrap()).execute(store.test_pool()).await.unwrap();
        } else {block_restart(&store,&f,&command).await;}
        let cursor=store.current_cursor(&f.session.agent_session_id).await.unwrap();
        assert!(store.commit_native_resume(&f.owner,&f.session.agent_session_id,&f.request,prepared).await.is_err());
        assert!(store.request_native_resume(&f.owner,&f.session.agent_session_id,&f.request).await.is_err());
        assert_eq!(store.current_cursor(&f.session.agent_session_id).await.unwrap(),cursor);
    }
}

#[tokio::test]
async fn queued_restart_keeps_its_original_ack_but_a_later_block_cannot_turn_it_into_authorization() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let mut f=QueueFixture::new(&store,"restart-queue-block").await;
    let command=bind_restart(&store,&mut f,"queue-original").await;
    assert!(matches!(store.commit_native_resume(&f.owner,&f.session.agent_session_id,&f.request,f.prepare(&store).await).await,
        Err(SessionStoreError::ExecutionCapacityPending)));
    let queued=store.request_native_resume(&f.owner,&f.session.agent_session_id,&f.request).await.unwrap();
    block_restart(&store,&f,&command).await;
    let replay=store.request_native_resume(&f.owner,&f.session.agent_session_id,&f.request).await.unwrap();
    assert!(replay.duplicate);assert_eq!(replay.request_event_id,queued.request_event_id);
    assert!(store.pending_native_resume_requests(0,100).await.unwrap().is_empty());
    f.finish_peer(&store).await;
    assert!(store.commit_native_resume(&f.owner,&f.session.agent_session_id,&f.request,f.prepare(&store).await).await.is_err());
    assert!(store.native_resume_receipt(&f.owner,&f.session.agent_session_id,&f.request).await.unwrap().is_none());
    store.block_native_resume_request(&f.owner,&f.session.agent_session_id,&f.request,"The original command no longer authorizes continuation").await.unwrap();
    let blocked:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='turn/resume-blocked' AND causation_event_id=?")
        .bind(f.session.agent_session_id.as_ref()).bind(queued.request_event_id.as_ref()).fetch_one(store.test_pool()).await.unwrap();
    assert_eq!(blocked,1,"a failed queued Restart must publish its rejection even after its original domain guard closes");
    assert!(store.native_resume_request_receipt(&f.owner,&f.session.agent_session_id,&f.request).await.unwrap().is_none());
}

#[tokio::test]
async fn queued_restart_authorization_consumes_the_original_intent_once_and_old_ack_survives_a_later_pause() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let mut f=QueueFixture::new(&store,"restart-queue-once").await;
    bind_restart(&store,&mut f,"queue-authorize-once").await;
    let queued=store.request_native_resume(&f.owner,&f.session.agent_session_id,&f.request).await.unwrap();
    f.finish_peer(&store).await;
    let authorization=store.commit_native_resume(&f.owner,&f.session.agent_session_id,&f.request,f.prepare(&store).await).await.unwrap();
    assert_ne!(queued.request_event_id,authorization.authorization_event_id);
    assert!(store.native_resume_request_receipt(&f.owner,&f.session.agent_session_id,&f.request).await.unwrap().is_none(),
        "an old queue ACK cannot hide the exact completed authorization");
    let saved=store.load_native_checkpoint(&f.owner,&f.session.agent_session_id,&f.request.operation_id).await.unwrap().unwrap();
    let lease=store.claim_native_execution(NativeExecutionClaim {owner:f.owner.clone(),agent_session_id:f.session.agent_session_id.clone(),
        operation_id:f.request.operation_id.clone(),snapshot:f.session.agent_binding.resolved_snapshot_ref.clone(),active_set_generation:0,
        holder:"restart-next-pause".into(),expected_fence:saved.execution_fence,
        checkpoint:Some((saved.revision,saved.digest,saved.through_seq))}).await.unwrap();
    store.pause_native_execution(&lease,"EXECUTION_INTERVENTION_REQUIRED",true).await.unwrap();
    let cursor=store.current_cursor(&f.session.agent_session_id).await.unwrap();
    let replay=store.commit_native_resume(&f.owner,&f.session.agent_session_id,&f.request,f.prepare(&store).await).await.unwrap();
    assert!(replay.duplicate);assert_eq!(replay.authorization_event_id,authorization.authorization_event_id);
    assert_eq!(store.current_cursor(&f.session.agent_session_id).await.unwrap(),cursor);
}

#[tokio::test]
async fn user_stop_wins_the_native_restart_writer_even_before_original_cleanup_is_delivered() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let mut f=QueueFixture::new(&store,"restart-stop").await;
    let command=bind_restart(&store,&mut f,"stop-before-authorization").await;
    store.request_native_resume(&f.owner,&f.session.agent_session_id,&f.request).await.unwrap();
    let prepared=f.prepare(&store).await;
    let repository=nomifun_db::SqliteAgentExecutionRepository::new(store.test_pool().clone());
    let detail=repository.get_execution_detail(&f.owner.principal_id,&f.execution).await.unwrap().unwrap();
    let step=detail.steps.iter().find(|step|Some(step.step_id.as_str())==command.event.step_id.as_deref()).unwrap();
    repository.cancel_step(&f.owner.principal_id,&f.execution,detail.execution.version,&step.step_id,step.version,
        "task".parse().unwrap(),&nomifun_db::NewAgentExecutionEvent {actor:serde_json::from_value(json!({"type":"user","user_id":f.owner.principal_id})).unwrap(),
            ..domain_event("step_changed")}).await.unwrap();
    f.finish_peer(&store).await;
    let cursor=store.current_cursor(&f.session.agent_session_id).await.unwrap();
    assert!(store.commit_native_resume(&f.owner,&f.session.agent_session_id,&f.request,prepared).await.is_err());
    assert_eq!(store.current_cursor(&f.session.agent_session_id).await.unwrap(),cursor);
    assert!(store.pending_native_resume_requests(0,100).await.unwrap().is_empty());
    assert_eq!(store.read_turn_receipt(&f.session.agent_session_id,&f.request.operation_id).await.unwrap().status,TurnReceiptStatus::Running);
}

#[tokio::test]
async fn a_parent_actor_or_changed_restart_budget_cannot_borrow_a_user_command_for_native_authority() {
    for actor in [false,true] {
        let store=AgentSessionStore::open_in_memory().await.unwrap();let mut f=QueueFixture::new(&store,"restart-user-authority").await;
        let command=bind_restart(&store,&mut f,"user-authority").await;f.finish_peer(&store).await;
        if actor {
            sqlx::query("UPDATE agent_execution_events SET actor_type='agent',actor_id=? WHERE execution_id=? AND sequence=?")
                .bind(f.session.agent_session_id.as_ref()).bind(&f.execution).bind(command.event.sequence).execute(store.test_pool()).await.unwrap();
        } else {f.request.budget.additional_segments=1;}
        let cursor=store.current_cursor(&f.session.agent_session_id).await.unwrap();
        assert!(store.commit_native_resume(&f.owner,&f.session.agent_session_id,&f.request,f.prepare(&store).await).await.is_err());
        assert!(store.request_native_resume(&f.owner,&f.session.agent_session_id,&f.request).await.is_err());
        assert_eq!(store.current_cursor(&f.session.agent_session_id).await.unwrap(),cursor);
    }
}
