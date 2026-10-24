//! The actual Session input owner crosses accepted/dispatch failures without
//! another Turn, Attempt or provider invocation. Every row is test-owned.
use super::*;
use nomifun_agent_execution::AgentExecutionSessionPort;
use nomifun_api_types::SendMessageRequest;

fn busy()->AppError {AppError::DatabaseUnavailable {reason:nomifun_common::DatabaseUnavailableReason::Busy}}

async fn original_attempt(fixture:&Fixture,session:&AgentSessionId,marker:&str)
    ->(nomifun_db::AgentExecutionTurnAuthority,SendMessageRequest) {
    let execution=uuid::Uuid::now_v7().to_string();let step=uuid::Uuid::now_v7().to_string();let attempt=uuid::Uuid::now_v7().to_string();
    let now=nomifun_common::now_ms();let user=fixture.services.authoritative_user_id.as_ref();
    let request=SendMessageRequest {content:format!("COORDINATOR_CHILD_FORK: Return exactly fork_result as the complete result. {marker}"),
        files:vec![],inject_skills:vec![],hidden:false,origin:Some("agent_execution".into()),channel_platform:None};
    let state=nomifun_db::AttemptConversationEffects {pending_conversation_effects:vec![nomifun_db::PendingConversationEffect::InitialInput {
        request:serde_json::to_value(&request).unwrap()}],review_blocked:None}.encode().unwrap();
    sqlx::query("INSERT INTO agent_executions(execution_id,user_id,goal,status,adaptation_policy,decision_policy,delegation_policy,initial_plan_input,lease_owner,lease_expires_at,created_at,updated_at) VALUES(?,?,'original input fixture','running','fixed','automatic','automatic','{}','input-owner-fixture',?,?,?)")
        .bind(&execution).bind(user).bind(now+60_000).bind(now).bind(now).execute(fixture.pool()).await.unwrap();
    sqlx::query("INSERT INTO agent_execution_steps(step_id,execution_id,title,spec,kind,status,version,introduced_in_revision,created_at,updated_at) VALUES(?,?,'original input fixture','Return the exact fixture label','agent','running',1,1,?,?)")
        .bind(&step).bind(&execution).bind(now).bind(now).execute(fixture.pool()).await.unwrap();
    sqlx::query("INSERT INTO agent_execution_attempts(attempt_id,execution_id,step_id,attempt_no,status,trigger_reason,effective_config,runtime_state,version,created_at,updated_at) VALUES(?,?,?,0,'running','initial','{}',?,1,?,?)")
        .bind(&attempt).bind(&execution).bind(&step).bind(state).bind(now).bind(now).execute(fixture.pool()).await.unwrap();
    sqlx::query("INSERT INTO conversation_execution_links(conversation_id,execution_id,relation,step_id,attempt_id,active,created_at,updated_at) VALUES(?,?,'attempt',?,?,1,?,?)")
        .bind(session.as_ref()).bind(&execution).bind(&step).bind(&attempt).bind(now).bind(now).execute(fixture.pool()).await.unwrap();
    (nomifun_db::AgentExecutionTurnAuthority {execution_id:execution,step_id:step,attempt_id:attempt,
        expected_step_version:1,expected_attempt_version:1,lease_owner:"input-owner-fixture".into()},request)
}

fn caller(fixture:&Fixture,session:&AgentSessionId,authority:nomifun_db::AgentExecutionTurnAuthority,request:SendMessageRequest)
    ->tokio::task::JoinHandle<Result<nomifun_agent_execution::AgentExecutionDelivery,AppError>> {
    let owner=fixture.owner.clone();let user=fixture.services.authoritative_user_id.as_ref().to_owned();let session=session.as_ref().to_owned();
    let key=format!("{}:initial-turn",authority.attempt_id);
    tokio::spawn(async move {owner.deliver_turn(&user,&session,&key,authority,request).await})
}

async fn terminal(fixture:&Fixture,session:&AgentSessionId,operation:&nomifun_agent_contracts::OperationId,
    expected:nomifun_agent_session::TurnReceiptStatus) {
    tokio::time::timeout(Duration::from_secs(45),async {loop {
        let receipt=fixture.owner.canonical().store().read_turn_receipt(session,operation).await.unwrap();
        if receipt.status==expected {break;}
        assert_eq!(receipt.status,nomifun_agent_session::TurnReceiptStatus::Running,"original input reached an unexpected exit: {receipt:?}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }}).await.expect("the original accepted input must acquire a real owner or finish its exact Stop");
}

async fn input_failure(stage:&str,marker:&str,stop:bool,cold_lease:bool) {
    let server=ModelServer::new().await;
    let fixture=Fixture::with_input_model(marker,&server.url).await;
    let (session,_)=create_parent_with_collaboration_actions(&fixture,&["agent/fork"]).await;
    let (authority,request)=original_attempt(&fixture,&session,marker).await;
    let key=format!("{}:initial-turn",authority.attempt_id);
    let operation=nomifun_agent_contracts::OperationId::from(format!("turn:user:{}:{}:{key}",fixture.services.authoritative_user_id.as_ref(),session.as_ref()));
    let (entered,release)=fixture.owner.inject_attempt_input_owner_fault(marker,stage,busy(),true);
    let (observed,_)=fixture.owner.inject_attempt_input_owner_fault(marker,"observe",busy(),false);
    let pending=cold_lease.then(||fixture.owner.inject_attempt_input_owner_fault(marker,"recovery_pending",busy(),true));
    let waiter=caller(&fixture,&session,authority.clone(),request.clone());
    tokio::time::timeout(Duration::from_secs(15),entered.notified()).await.unwrap();
    waiter.abort();assert!(waiter.await.unwrap_err().is_cancelled(),"only the receipt observer is dropped");
    let store=fixture.owner.canonical().store();
    let original=store.turn_accepted_input(&session,&operation).await.unwrap().1;
    let before=store.current_cursor(&session).await.unwrap();
    if cold_lease {
        let facts=store.native_recovery_facts(&session,&operation).await.unwrap();
        store.claim_native_execution(nomifun_agent_session::NativeExecutionClaim {
            owner:facts.session.owner_ref.clone(),agent_session_id:session.clone(),operation_id:operation.clone(),
            snapshot:facts.session.agent_binding.resolved_snapshot_ref,active_set_generation:facts.head.active_set_generation,
            holder:"cold-original-native-process".into(),expected_fence:facts.execution_fence,checkpoint:None,
        }).await.unwrap();
    }
    assert!(fixture.owner.collaboration_has_native_progress_owner(session.as_ref(),operation.as_ref()).await.unwrap(),
        "the retained original input task is a real progress owner even after its waiter disappears");
    assert!(server.state.requests.lock().unwrap().iter().all(|body|!body.to_string().contains(marker)),"the fault is before native invocation");
    if stop {
        fixture.owner.canonical().cancel_exact_turn(&nomifun_agent_contracts::PrincipalRef {principal_kind:"user".into(),principal_id:fixture.services.authoritative_user_id.as_ref().into()},
            &session,"stop-before-original-native-claim",&operation).await.unwrap();
    }
    // This observer is already queued on the same Session when the first
    // caller exits its failed dispatch. Native claim/custody, not the duplicate
    // Accepted flag or the input-owner gate, chooses the single producer.
    let concurrent=(!stop).then(||caller(&fixture,&session,authority.clone(),request.clone()));
    release.unwrap().notify_one();
    tokio::time::timeout(Duration::from_secs(5),observed.notified()).await.unwrap();
    if let Some((pending,released))=pending {
        tokio::time::timeout(Duration::from_secs(15),pending.notified()).await.expect("an unexpired old native lease is typed pending, never a permanent recovery rejection");
        assert!(store.native_pause_state(&session,&operation).await.unwrap().is_none());
        assert!(server.state.requests.lock().unwrap().iter().all(|body|!body.to_string().contains(marker)));
        // Advance only this test-owned lease boundary, with no task deadline
        // or sleep. The original canonical operation must then claim normally.
        sqlx::query("UPDATE agent_turns SET execution_lease_until=0 WHERE session_id=? AND operation_id=?")
            .bind(session.as_ref()).bind(operation.as_ref()).execute(fixture.pool()).await.unwrap();
        released.unwrap().notify_one();
    }
    terminal(&fixture,&session,&operation,if stop {nomifun_agent_session::TurnReceiptStatus::Cancelled}else{nomifun_agent_session::TurnReceiptStatus::Completed}).await;
    if let Some(concurrent)=concurrent {
        let delivery=tokio::time::timeout(Duration::from_secs(5),concurrent).await.unwrap().unwrap().unwrap();
        assert_eq!(delivery.message_id,store.turn_accepted_input(&session,&operation).await.unwrap().0.event_id.as_ref());
    }
    let duplicate=caller(&fixture,&session,authority.clone(),request.clone()).await.unwrap().unwrap();assert!(duplicate.completed);
    let reads=server.state.requests.lock().unwrap().iter().filter(|body|body.to_string().contains(marker)).count();
    assert_eq!(reads,usize::from(!stop),"only one original model request can follow the native claim; Stop wins before invocation");
    assert_eq!(store.turn_accepted_input(&session,&operation).await.unwrap().1,original,"the complete original prepared input remains byte-exact");
    let turns:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_turns WHERE session_id=?").bind(session.as_ref()).fetch_one(fixture.pool()).await.unwrap();assert_eq!(turns,1);
    let attempts:Vec<i64>=sqlx::query_scalar("SELECT attempt_no FROM agent_execution_attempts WHERE execution_id=?").bind(&authority.execution_id).fetch_all(fixture.pool()).await.unwrap();assert_eq!(attempts,[0]);
    let failed:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='turn/failed'").bind(session.as_ref()).fetch_one(fixture.pool()).await.unwrap();assert_eq!(failed,0,"temporary pre-driver storage failure is not a business failure");
    assert!(store.current_cursor(&session).await.unwrap().seq>=before.seq);
    let cursor=store.current_cursor(&session).await.unwrap();let mut changed=request;changed.content.push_str(" changed proposal");
    assert!(caller(&fixture,&session,authority,changed).await.unwrap().is_err(),"same key cannot borrow another complete input");
    assert_eq!(store.current_cursor(&session).await.unwrap(),cursor);
    fixture.voice_execution.shutdown().await.unwrap();fixture.finish().await;server.close().await;
}

#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn original_input_commit_ack_unknown_retains_owner_after_the_receipt_waiter_drops() {
    input_failure("admitted","owned-input-commit-ack-unknown",false,false).await;
}
#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn original_input_predispatch_and_observation_busy_preserve_one_native_invocation() {
    input_failure("predispatch","owned-input-predispatch-busy",false,false).await;
}
#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn original_input_stop_before_recovery_claim_remains_cancelled_with_zero_model_calls() {
    input_failure("admitted","owned-input-stop-before-claim",true,false).await;
}

#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn original_input_waits_for_the_cold_native_lease_then_claims_the_same_turn_once() {
    input_failure("admitted","owned-input-cold-native-lease",false,true).await;
}

#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn original_input_completion_between_observation_and_recovery_returns_the_original_ack() {
    let marker="owned-input-terminal-before-recovery-facts";
    let server=ModelServer::new().await;let fixture=Fixture::with_input_model(marker,&server.url).await;
    let (session,_)=create_parent_with_collaboration_actions(&fixture,&["agent/fork"]).await;
    let (authority,request)=original_attempt(&fixture,&session,marker).await;
    let key=format!("{}:initial-turn",authority.attempt_id);
    let operation=nomifun_agent_contracts::OperationId::from(format!("turn:user:{}:{}:{key}",fixture.services.authoritative_user_id.as_ref(),session.as_ref()));
    fixture.owner.inject_attempt_input_owner_fault(marker,"admitted",busy(),false);
    let (entered,release)=fixture.owner.hold_attempt_input_owner_cutpoint(marker,"recovery_facts");
    let original=caller(&fixture,&session,authority.clone(),request.clone());
    tokio::time::timeout(Duration::from_secs(15),entered.notified()).await.unwrap();
    // The second observer may formally claim the uninvoked original. It does
    // not receive authority merely from its duplicate Accepted response.
    let concurrent=caller(&fixture,&session,authority,request);
    terminal(&fixture,&session,&operation,nomifun_agent_session::TurnReceiptStatus::Completed).await;
    release.notify_one();
    let original=tokio::time::timeout(Duration::from_secs(5),original).await.unwrap().unwrap().unwrap();
    let concurrent=tokio::time::timeout(Duration::from_secs(5),concurrent).await.unwrap().unwrap().unwrap();
    assert!(original.completed);assert_eq!(original.message_id,concurrent.message_id);
    let requests=server.state.requests.lock().unwrap().iter().filter(|body|body.to_string().contains(marker)).count();assert_eq!(requests,1);
    let turns:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_turns WHERE session_id=?").bind(session.as_ref()).fetch_one(fixture.pool()).await.unwrap();assert_eq!(turns,1);
    fixture.voice_execution.shutdown().await.unwrap();fixture.finish().await;server.close().await;
}

#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn parent_question_reply_commit_ack_unknown_uses_the_same_owned_delivery_loop() {
    let server=ModelServer::new().await;server.state.question_reply.store(true,Ordering::SeqCst);
    let fixture=Fixture::with_input_model("parent-reply-input-owner-ack-unknown",&server.url).await;
    let (session,_)=create_parent(&fixture).await;
    let (entered,release)=fixture.owner.inject_attempt_input_owner_fault("parent_agent_question_reply","admitted",busy(),true);
    let accepted=api(&fixture._router,&format!("/api/agent-sessions/{}/messages",session.as_ref()),json!({"idempotency_key":"parent-reply-owner-root",
        "input":{"content":"COORDINATOR_QUESTION_ROOT: Delegate the synthetic text module, answer its context question yourself and integrate its exact result."}})).await;
    tokio::time::timeout(Duration::from_secs(45),entered.notified()).await.unwrap();release.unwrap().notify_one();
    let input=nomifun_agent_contracts::EventId::from(accepted["message_id"].as_str().unwrap());
    let operation=fixture.owner.canonical().store().session_input_claim(&session,&input).await.unwrap().unwrap().operation_id;
    terminal(&fixture,&session,&operation,nomifun_agent_session::TurnReceiptStatus::Completed).await;
    let counts:(i64,i64)=sqlx::query_as("SELECT (SELECT COUNT(*) FROM agent_execution_attempts),(SELECT COUNT(*) FROM agent_execution_events WHERE event_type='decision_answered')")
        .fetch_one(fixture.pool()).await.unwrap();assert_eq!(counts,(1,1));
    let replies:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE kind='message/user-accepted' AND json_extract(inline_json,'$.origin')='agent_execution_parent_reply'")
        .fetch_one(fixture.pool()).await.unwrap();assert_eq!(replies,1);
    let child:String=sqlx::query_scalar("SELECT conversation_id FROM conversation_execution_links WHERE relation='attempt' LIMIT 1").fetch_one(fixture.pool()).await.unwrap();
    let turns:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_turns WHERE session_id=?").bind(child).fetch_one(fixture.pool()).await.unwrap();assert_eq!(turns,2);
    fixture.voice_execution.shutdown().await.unwrap();fixture.finish().await;server.close().await;
}
