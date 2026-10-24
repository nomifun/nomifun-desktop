use super::*;
use crate::IAgentExecutionRepository;

async fn fixture(queued:bool)->(crate::Database,String,String,String,String,Option<String>) {
    let db=crate::init_database_memory().await.unwrap();
    let user=crate::installation_owner_id(db.pool()).await.unwrap();
    let execution=nomifun_common::generate_id();let step=nomifun_common::generate_id();let sibling=nomifun_common::generate_id();
    sqlx::query("INSERT INTO agent_executions(execution_id,user_id,goal,status,adaptation_policy,decision_policy,delegation_policy,initial_plan_input,version,event_sequence,created_at,updated_at) VALUES(?,?,'restart fixture','running','fixed','automatic','automatic','{}',1,1,1,1)")
        .bind(&execution).bind(&user).execute(db.pool()).await.unwrap();
    sqlx::query("INSERT INTO agent_execution_events(execution_id,sequence,event_type,actor_type,on_behalf_of_user_id,payload,created_at) VALUES(?,1,'created','system',?,'{}',1)")
        .bind(&execution).bind(&user).execute(db.pool()).await.unwrap();
    for (id,status) in [(&step,if queued {"pending"}else{"failed"}),(&sibling,"completed")] {
        sqlx::query("INSERT INTO agent_execution_steps(step_id,execution_id,title,spec,kind,status,version,introduced_in_revision,created_at,updated_at) VALUES(?,?,'original task','original task','agent',?,1,1,1,1)")
            .bind(id).bind(&execution).bind(status).execute(db.pool()).await.unwrap();
    }
    let attempt=if queued {
        let id=nomifun_common::generate_id();
        let raw=crate::AttemptConversationEffects {pending_conversation_effects:vec![crate::PendingConversationEffect::InitialInput {
            request:json!({"content":"immutable original input","files":["original-file"],"inject_skills":["original-skill"],"hidden":true,"origin":"agent_execution","channel_platform":null})}],review_blocked:None}.encode().unwrap();
        sqlx::query("INSERT INTO agent_execution_attempts(attempt_id,execution_id,step_id,attempt_no,status,trigger_reason,effective_config,runtime_state,version,created_at,updated_at) VALUES(?,?,?,0,'queued','initial','{}',?,1,1,1)")
            .bind(&id).bind(&execution).bind(&step).bind(raw).execute(db.pool()).await.unwrap();Some(id)
    } else {None};
    (db,user,execution,step,sibling,attempt)
}

fn event(user:&str,step:&str,attempt:Option<&str>,key:&str)->NewAgentExecutionEvent {
    NewAgentExecutionEvent {event_type:AgentExecutionEventKind::StepChanged,step_id:Some(step.into()),attempt_id:attempt.map(str::to_owned),
        actor:AgentExecutionActor::User {user_id:user.into()},payload:json!({"change":"restart_requested","operation_id":key,
            "command_request":{"kind":"restart_step","request":{"idempotency_key":key,"expected_execution_version":1,"expected_step_version":1,"expected_attempt_id":attempt}}}).to_string()}
}

#[tokio::test]
async fn failed_before_attempt_restart_resets_only_the_same_step_and_replays_the_original_receipt() {
    let(db,user,execution,step,sibling,attempt)=fixture(false).await;
    let repository=SqliteAgentExecutionRepository::new(db.pool().clone());let request=event(&user,&step,None,"restart-original-once");
    let accepted=repository.request_step_restart(&user,&execution,&step,1,1,None,&request).await.unwrap();
    assert!(!accepted.replayed);assert_eq!(accepted.event.attempt_id,attempt);
    let replay=repository.request_step_restart(&user,&execution,&step,1,1,None,&request).await.unwrap();
    assert!(replay.replayed);assert_eq!(replay.event.sequence,accepted.event.sequence);
    let done=repository.complete_step_restart(&user,&execution,"restart-original-once",&AgentExecutionRestartCompletion::OriginalPending).await.unwrap();
    assert_eq!(done.steps.iter().find(|row|row.step_id==step).unwrap().status,"pending");
    let other=done.steps.iter().find(|row|row.step_id==sibling).unwrap();assert_eq!(other.status,"completed");assert_eq!(other.version,1);
    assert!(done.attempts.is_empty());
    repository.complete_step_restart(&user,&execution,"restart-original-once",&AgentExecutionRestartCompletion::OriginalPending).await.unwrap();
    let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_execution_events WHERE execution_id=? AND json_extract(payload,'$.change')='restart_resolved'")
        .bind(&execution).fetch_one(db.pool()).await.unwrap();assert_eq!(count,1);
    assert!(repository.pending_step_restarts(&user,&execution).await.unwrap().is_empty());
    db.close().await;
}

#[tokio::test]
async fn queued_restart_keeps_the_exact_reservation_input_and_initial_key_after_the_admission_fence() {
    let(db,user,execution,step,_sibling,attempt)=fixture(true).await;let id=attempt.unwrap();
    let repository=SqliteAgentExecutionRepository::new(db.pool().clone());
    let before=repository.get_step_detail(&user,&execution,&step).await.unwrap().unwrap();
    let request=event(&user,&step,Some(&id),"restart-queued-original");
    let accepted=repository.request_step_restart(&user,&execution,&step,1,1,Some(&id),&request).await.unwrap();
    assert_eq!(accepted.event.attempt_id.as_deref(),Some(id.as_str()));
    let fenced=repository.get_step_detail(&user,&execution,&step).await.unwrap().unwrap();
    assert_eq!(fenced.step.version,before.step.version+1);
    let old=before.current_attempt.unwrap();let still=fenced.current_attempt.unwrap();
    assert_eq!(still.attempt.attempt_id,old.attempt.attempt_id);assert_eq!(still.attempt.runtime_state,old.attempt.runtime_state);
    assert_eq!(still.attempt.status,"queued");assert_eq!(still.attempt.version,old.attempt.version);
    let done=repository.complete_step_restart(&user,&execution,"restart-queued-original",&AgentExecutionRestartCompletion::OriginalPending).await.unwrap();
    assert_eq!(done.attempts.len(),1);assert_eq!(done.attempts[0].attempt.attempt_id,id);assert_eq!(done.attempts[0].attempt.status,"queued");
    assert_eq!(repository.execution_attempt_turn_operation_key(&user,&execution,&step,&id).await.unwrap(),format!("{id}:initial-turn"));
    db.close().await;
}

#[tokio::test]
async fn exact_user_stop_supersedes_a_pending_restart_without_reopening_the_step() {
    let(db,user,execution,step,_sibling,attempt)=fixture(true).await;let id=attempt.unwrap();
    let repository=SqliteAgentExecutionRepository::new(db.pool().clone());
    repository.request_step_restart(&user,&execution,&step,1,1,Some(&id),&event(&user,&step,Some(&id),"restart-before-stop")).await.unwrap();
    let before=repository.get_execution_detail(&user,&execution).await.unwrap().unwrap();
    let original=before.steps.iter().find(|row|row.step_id==step).unwrap();
    repository.cancel_step(&user,&execution,before.execution.version,&step,original.version,
        nomifun_common::AgentExecutionCancellationScope::Task,&NewAgentExecutionEvent {event_type:AgentExecutionEventKind::StepChanged,
            step_id:Some(step.clone()),attempt_id:Some(id.clone()),actor:AgentExecutionActor::User {user_id:user.clone()},payload:"{}".into()}).await.unwrap();
    let done=repository.complete_step_restart(&user,&execution,"restart-before-stop",&AgentExecutionRestartCompletion::OriginalPending).await.unwrap();
    assert_eq!(done.steps.iter().find(|row|row.step_id==step).unwrap().status,"cancelled");
    assert_eq!(done.attempts.len(),1);assert_eq!(done.attempts[0].attempt.attempt_id,id);
    let disposition:String=sqlx::query_scalar("SELECT json_extract(payload,'$.disposition') FROM agent_execution_events WHERE execution_id=? AND json_extract(payload,'$.change')='restart_resolved'")
        .bind(&execution).fetch_one(db.pool()).await.unwrap();assert_eq!(disposition,"superseded");
    db.close().await;
}

#[tokio::test]
async fn unknown_original_effect_resolves_restart_as_blocked_instead_of_retaining_a_false_progress_owner() {
    let(db,user,execution,step,_sibling,attempt)=fixture(true).await;let attempt=attempt.unwrap();
    let session=nomifun_common::generate_id();let terminal=nomifun_common::generate_id();let effect_start=nomifun_common::generate_id();let effect_terminal=nomifun_common::generate_id();
    let operation=format!("turn:user:{user}:{session}:{attempt}:initial-turn");
    sqlx::query("UPDATE agent_execution_steps SET status='failed' WHERE step_id=?").bind(&step).execute(db.pool()).await.unwrap();
    sqlx::query("UPDATE agent_execution_attempts SET status='failed',runtime_state=NULL WHERE attempt_id=?").bind(&attempt).execute(db.pool()).await.unwrap();
    sqlx::query("INSERT INTO agent_sessions(agent_session_id,owner_ref_json,state,title,archived,pinned,agent_binding_json,next_seq,created_at) VALUES(?,?,'live','original',0,0,'{}',4,1)")
        .bind(&session).bind(json!({"principal_kind":"user","principal_id":user}).to_string()).execute(db.pool()).await.unwrap();
    for (sequence,id,kind,correlation) in [(1,&terminal,"turn/failed",operation.as_str()),(2,&effect_start,"effect/started","old-effect"),(3,&effect_terminal,"effect/uncertain","old-effect")] {
        sqlx::query("INSERT INTO agent_events(session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,inline_json) VALUES(?,?,?,'runtime_supervisor',?,?,1,?,'{}')")
            .bind(&session).bind(sequence).bind(id).bind(id).bind(kind).bind(correlation).execute(db.pool()).await.unwrap();
    }
    sqlx::query("INSERT INTO agent_turns(session_id,turn_id,operation_id,idempotency_key,state,terminal_event_id,accepted_at,finished_at) VALUES(?,?,?,?,'failed',?,1,2)")
        .bind(&session).bind(&operation).bind(&operation).bind(&operation).bind(&terminal).execute(db.pool()).await.unwrap();
    sqlx::query("INSERT INTO conversation_execution_links(conversation_id,execution_id,relation,step_id,attempt_id,active,cleanup_completed_at,created_at,updated_at) VALUES(?,?,'attempt',?,?,0,2,1,2)")
        .bind(&session).bind(&execution).bind(&step).bind(&attempt).execute(db.pool()).await.unwrap();
    sqlx::query("INSERT INTO agent_effects(effect_id,session_id,turn_id,operation_id,owner_domain,capability_module,action_id,input_digest,strategy,state,started_event_id,terminal_event_id,created_at,settled_at) VALUES(?,?,?,'old-effect','test','test','test',?,'external_uncertain_effect','unknown',?,?,1,2)")
        .bind(nomifun_common::generate_id()).bind(&session).bind(&operation).bind("a".repeat(64)).bind(&effect_start).bind(&effect_terminal).execute(db.pool()).await.unwrap();
    let repository=SqliteAgentExecutionRepository::new(db.pool().clone());
    repository.request_step_restart(&user,&execution,&step,1,1,Some(&attempt),&event(&user,&step,Some(&attempt),"restart-unknown-original")).await.unwrap();
    let done=repository.complete_step_restart(&user,&execution,"restart-unknown-original",&AgentExecutionRestartCompletion::RetryOriginalStep).await.unwrap();
    assert_eq!(done.steps.iter().find(|row|row.step_id==step).unwrap().status,"failed");assert_eq!(done.attempts.len(),1);
    let result:(String,String)=sqlx::query_as("SELECT json_extract(payload,'$.disposition'),json_extract(payload,'$.code') FROM agent_execution_events WHERE execution_id=? AND json_extract(payload,'$.change')='restart_resolved'")
        .bind(&execution).fetch_one(db.pool()).await.unwrap();assert_eq!(result,("blocked".into(),"ORIGINAL_EFFECTS_REQUIRE_RECONCILIATION".into()));
    assert!(repository.pending_step_restarts(&user,&execution).await.unwrap().is_empty());
    let state:String=sqlx::query_scalar("SELECT state FROM agent_effects WHERE session_id=?").bind(&session).fetch_one(db.pool()).await.unwrap();assert_eq!(state,"unknown");
    db.close().await;
}

async fn add_session(db:&crate::Database,user:&str)->String {
    let session=nomifun_common::generate_id();
    sqlx::query("INSERT INTO agent_sessions(agent_session_id,owner_ref_json,state,title,archived,pinned,agent_binding_json,next_seq,created_at) VALUES(?,?,'live','original',0,0,'{}',1,1)")
        .bind(&session).bind(json!({"principal_kind":"user","principal_id":user}).to_string()).execute(db.pool()).await.unwrap();session
}

#[tokio::test]
async fn cached_original_pending_acknowledges_the_same_attempt_linking_or_finishing_without_business_replay() {
    for finished in [false,true] {
        let(db,user,execution,step,_sibling,attempt)=fixture(true).await;let attempt=attempt.unwrap();
        let repository=SqliteAgentExecutionRepository::new(db.pool().clone());
        repository.request_step_restart(&user,&execution,&step,1,1,Some(&attempt),&event(&user,&step,Some(&attempt),"restart-cached-queued")).await.unwrap();
        // The Restart worker has observed Queued and retained OriginalPending.
        // Its original runner wins the ordinary link writer before completion.
        let saved=repository.get_step_detail(&user,&execution,&step).await.unwrap().unwrap();
        let old=saved.current_attempt.as_ref().unwrap();
        let effects=crate::AttemptConversationEffects::decode(old.attempt.runtime_state.as_deref()).unwrap();
        let session=add_session(&db,&user).await;
        let linked=repository.start_attempt(&user,&execution,&step,saved.step.version,&attempt,old.attempt.version,
            &session,crate::AgentExecutionAttemptSessionKind::ChildAttempt,None,effects.pending_conversation_effects.first(),
            &NewAgentExecutionEvent {event_type:AgentExecutionEventKind::AttemptChanged,step_id:Some(step.clone()),attempt_id:Some(attempt.clone()),
                actor:AgentExecutionActor::system(),payload:json!({"status":"running","turn_operation_key":format!("{attempt}:initial-turn")}).to_string()}).await.unwrap();
        if finished {
            let crate::PendingConversationEffect::InitialInput {request}=effects.pending_conversation_effects.first().unwrap() else {unreachable!()};
            let accepted=nomifun_common::generate_id();let started=nomifun_common::generate_id();let terminal=nomifun_common::generate_id();
            let opening=nomifun_common::generate_id();let identity=format!("user:{user}:{session}:{attempt}:initial-turn");let operation=format!("turn:{identity}");
            for (seq,id,key,kind,correlation,cause,body) in [
                (1,&opening,"fixture-opening".into(),"session/opening",opening.clone(),None,json!({})),
                (2,&accepted,format!("{identity}:message"),"message/user-accepted",accepted.clone(),Some(opening.clone()),request.clone()),
                (3,&started,format!("{identity}:turn"),"turn/started",operation.clone(),Some(accepted.clone()),json!({"operation_id":operation,"source_message_id":accepted})),
                (4,&terminal,"fixture-terminal".into(),"turn/completed",operation.clone(),Some(started.clone()),json!({"operation_id":operation})),
            ] {
                sqlx::query("INSERT INTO agent_events(session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,causation_event_id,inline_json) VALUES(?,?,?,'session_api',?,?,1,?,?,?)")
                    .bind(&session).bind(seq).bind(id).bind(key).bind(kind).bind(correlation).bind(cause).bind(body.to_string()).execute(db.pool()).await.unwrap();
            }
            sqlx::query("INSERT INTO agent_turns(session_id,turn_id,operation_id,idempotency_key,state,source_message_id,started_event_id,terminal_event_id,accepted_at,started_at,finished_at) VALUES(?,?,?,?,'completed',?,?,?,1,1,2)")
                .bind(&session).bind(&operation).bind(&operation).bind(&operation).bind(&accepted).bind(&started).bind(&terminal).execute(db.pool()).await.unwrap();
            sqlx::query("UPDATE agent_execution_attempts SET status='completed',runtime_state=NULL,finished_at=2,version=version+1 WHERE attempt_id=?")
                .bind(&attempt).execute(db.pool()).await.unwrap();
            sqlx::query("UPDATE agent_execution_steps SET status='completed',version=version+1 WHERE step_id=?").bind(&step).execute(db.pool()).await.unwrap();
        }
        let done=repository.complete_step_restart(&user,&execution,"restart-cached-queued",&AgentExecutionRestartCompletion::OriginalPending).await.unwrap();
        assert_eq!(done.attempts.len(),1);assert_eq!(done.attempts[0].attempt.attempt_id,attempt);
        assert_eq!(done.steps.iter().find(|row|row.step_id==step).unwrap().status,if finished {"completed"}else{"running"});
        if !finished {assert_eq!(done.attempts[0].attempt.runtime_state,linked.current_attempt.unwrap().attempt.runtime_state);}
        assert!(repository.pending_step_restarts(&user,&execution).await.unwrap().is_empty());
        let disposition:String=sqlx::query_scalar("SELECT json_extract(payload,'$.disposition') FROM agent_execution_events WHERE execution_id=? AND json_extract(payload,'$.change')='restart_resolved'")
            .bind(&execution).fetch_one(db.pool()).await.unwrap();assert_eq!(disposition,"original_pending");
        db.close().await;
    }
}

#[tokio::test]
async fn queued_native_resume_keeps_restart_pending_until_its_exact_authorization_or_blocked_boundary() {
    for result in ["authorized","blocked","superseded"] {
        let(db,user,execution,step,_sibling,attempt)=fixture(true).await;let attempt=attempt.unwrap();
        let session=add_session(&db,&user).await;let operation=format!("turn:user:{user}:{session}:{attempt}:initial-turn");
        let pause=nomifun_agent_contracts::NativePauseState {revision:1,reason:"EXECUTION_NO_PROGRESS".into(),checkpoint_revision:2,
            checkpoint_digest:Some("a".repeat(64).into()),execution_fence:3,cleanup_proven:true,paused_at_ms:1};
        sqlx::query("UPDATE agent_execution_steps SET status='waiting_input' WHERE step_id=?").bind(&step).execute(db.pool()).await.unwrap();
        let review=crate::AttemptConversationEffects {pending_conversation_effects:vec![],review_blocked:Some(crate::RecoveryReviewBlock {
            kind:"native_pause".into(),operation_id:operation.clone(),receipt_state:"paused".into(),reason:pause.reason.clone()})};
        sqlx::query("UPDATE agent_execution_attempts SET status='waiting_input',runtime_state=? WHERE attempt_id=?")
            .bind(review.encode().unwrap()).bind(&attempt).execute(db.pool()).await.unwrap();
        sqlx::query("INSERT INTO conversation_execution_links(conversation_id,execution_id,relation,step_id,attempt_id,active,created_at,updated_at) VALUES(?,?,'attempt',?,?,1,1,1)")
            .bind(&session).bind(&execution).bind(&step).bind(&attempt).execute(db.pool()).await.unwrap();
        sqlx::query("INSERT INTO agent_session_heads(session_id,status,active_turn_id,active_set_generation,last_seq,unread_count) VALUES(?,'paused',?,0,1,0)")
            .bind(&session).bind(&operation).execute(db.pool()).await.unwrap();
        let cause=format!("native-paused:{session}:{operation}:1");
        sqlx::query("INSERT INTO agent_events(session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,inline_json) VALUES(?,1,?,'runtime_supervisor',?,'turn/paused',1,?,?)")
            .bind(&session).bind(&cause).bind(&cause).bind(&operation).bind(serde_json::to_string(&pause).unwrap()).execute(db.pool()).await.unwrap();
        sqlx::query("INSERT INTO agent_turns(session_id,turn_id,operation_id,idempotency_key,state,native_pause_json,native_pause_revision,native_checkpoint_revision,native_checkpoint_digest,execution_fence,accepted_at) VALUES(?,?,?,?,'running',?,1,2,?,3,1)")
            .bind(&session).bind(&operation).bind(&operation).bind(&operation).bind(serde_json::to_string(&pause).unwrap()).bind("a".repeat(64)).execute(db.pool()).await.unwrap();
        let repository=SqliteAgentExecutionRepository::new(db.pool().clone());let command_id="restart-queued-native";
        let accepted=repository.request_step_restart(&user,&execution,&step,1,1,Some(&attempt),&event(&user,&step,Some(&attempt),command_id)).await.unwrap();
        assert!(accepted.target.as_ref().unwrap().pause.is_some());
        let native_key=format!("execution-task-restart:{command_id}");let digest=nomifun_agent_contracts::digest_bytes(native_key.as_bytes());
        let queued=format!("native-resume-request:{session}:{}",digest.as_ref());let authorized=format!("native-resume:{session}:{}",digest.as_ref());
        let request=json!({"operation_id":operation,"idempotency_key":native_key,"expected_pause_revision":1,
            "expected_checkpoint_revision":2,"expected_checkpoint_digest":"a".repeat(64),"budget":nomifun_agent_contracts::NativeBudgetIncrease {retry_stall_guards:true,..Default::default()},"cleanup_attestation":null});
        let request_digest=nomifun_agent_contracts::digest_payload(&request).unwrap();
        sqlx::query("INSERT INTO agent_events(session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,causation_event_id,inline_json) VALUES(?,2,?,'runtime_supervisor',?,'turn/resume-requested',1,?,?,?)")
            .bind(&session).bind(&queued).bind(&queued).bind(&operation).bind(&cause).bind(json!({"request":request,"request_digest":request_digest}).to_string()).execute(db.pool()).await.unwrap();
        let outcome=AgentExecutionRestartCompletion::ContinuedOriginal {resume_event_id:queued.clone(),resume_event_seq:2};
        for _ in 0..2 {
            repository.complete_step_restart(&user,&execution,command_id,&outcome).await.unwrap();
            assert_eq!(repository.pending_step_restarts(&user,&execution).await.unwrap().len(),1,
                "queue acceptance is still owned waiting; review suppression and Restart custody cannot end before authorization");
        }
        if result=="authorized" {
            sqlx::query("INSERT INTO agent_events(session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,causation_event_id,inline_json) VALUES(?,3,?,'runtime_supervisor',?,'turn/resume-authorized',1,?,?,?)")
                .bind(&session).bind(&authorized).bind(&authorized).bind(&operation).bind(&cause)
                .bind(json!({"pause_revision":1,"request_digest":request_digest}).to_string()).execute(db.pool()).await.unwrap();
        } else if result=="blocked" {
            let id=format!("native-resume-blocked:{session}:{}",digest.as_ref());
            sqlx::query("INSERT INTO agent_events(session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,causation_event_id,inline_json) VALUES(?,3,?,'runtime_supervisor',?,'turn/resume-blocked',1,?,?,?)")
                .bind(&session).bind(&id).bind(&id).bind(&operation).bind(&queued).bind(json!({"request_digest":request_digest,"reason":"original gate rejected"}).to_string()).execute(db.pool()).await.unwrap();
        } else {
            let next=nomifun_agent_contracts::NativePauseState {revision:2,execution_fence:4,..pause.clone()};
            sqlx::query("UPDATE agent_turns SET native_pause_json=?,native_pause_revision=2,execution_fence=4 WHERE session_id=? AND operation_id=?")
                .bind(serde_json::to_string(&next).unwrap()).bind(&session).bind(&operation).execute(db.pool()).await.unwrap();
        }
        let done=repository.complete_step_restart(&user,&execution,command_id,&outcome).await.unwrap();
        assert_eq!(done.attempts.len(),1);assert_eq!(done.attempts[0].attempt.attempt_id,attempt);
        assert!(repository.pending_step_restarts(&user,&execution).await.unwrap().is_empty());
        let resolved:Value=sqlx::query_scalar::<_,String>("SELECT payload FROM agent_execution_events WHERE execution_id=? AND json_extract(payload,'$.change')='restart_resolved'")
            .bind(&execution).fetch_one(db.pool()).await.map(|raw|serde_json::from_str(&raw).unwrap()).unwrap();
        assert_eq!(resolved["disposition"],if result=="authorized" {"continued_original"}else{"blocked"});
        if result=="authorized" {assert_eq!(resolved["native_resume_event_id"],authorized);}
        else {assert!(resolved.get("native_resume_event_id").is_none());}
        db.close().await;
    }
}
