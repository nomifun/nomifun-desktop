//! A real child-message commit may outlive its Runtime response. Recovery must
//! observe that owner's original receipt and must never send the command again.
use super::*;
use super::runtime_collaboration_tests::{ModelServer,ROOT_REQUEST,create_parent};
use nomifun_agent_runtime::{AgentEngineEvent,AgentReconciliationSource};
use nomifun_agent_session::NativeResumeRequest;

type MessageBarrier=(bool,Arc<tokio::sync::Notify>,Arc<tokio::sync::Semaphore>);
static BARRIERS:OnceLock<Mutex<BTreeMap<String,MessageBarrier>>>=OnceLock::new();

pub(super) async fn cutpoint(session:&str,committed:bool) {
    let barrier={
        let mut barriers=BARRIERS.get_or_init(Default::default).lock().unwrap();
        if barriers.get(session).is_some_and(|(phase,_,_)|*phase==committed) {barriers.remove(session)} else {None}
    };
    if let Some((_,entered,release))=barrier {
        entered.notify_one();release.acquire().await.unwrap().forget();
    }
}

async fn lost_reply(committed:bool) {
    let server=ModelServer::new().await;
    let fixture=Fixture::with_input_model(if committed {"message-recovery-committed"} else {"message-recovery-not-committed"},&server.url).await;
    let (session,_)=create_parent(&fixture).await;
    let entered=Arc::new(tokio::sync::Notify::new());
    let release=Arc::new(tokio::sync::Semaphore::new(0));
    BARRIERS.get_or_init(Default::default).lock().unwrap().insert(session.as_ref().into(),(committed,entered.clone(),release.clone()));
    server.release();
    api(&fixture._router,&format!("/api/agent-sessions/{}/messages",session.as_ref()),
        json!({"idempotency_key":"original-lost-message-reply","input":{"content":ROOT_REQUEST}})).await;
    tokio::time::timeout(Duration::from_secs(45),entered.notified()).await
        .expect("the real parent must reach the selected message-owner commit boundary");
    let host=HOSTS.get().unwrap().lock().unwrap().get(session.as_ref()).and_then(Weak::upgrade)
        .expect("the original installed Host owns this parent");
    let journal=host.active.lock().await.as_ref().unwrap().journal.clone();
    let original_runtime=fixture.services.agent_runtime_sessions.get_runtime(session.as_ref()).unwrap();
    let original_carrier=fixture.services.agent_runtime_sessions.observe_runtime_carrier(session.as_ref());
    let source=journal.message_source_for_call(&"followup-alpha".into()).await.unwrap();
    let principal=PrincipalRef {principal_kind:"user".into(),principal_id:fixture.services.authoritative_user_id.as_ref().into()};
    let operation=OperationId::from(source.turn_operation_id.clone());
    let store=fixture.owner.canonical().store();
    let saved=store.load_native_checkpoint(&principal,&session,&operation).await.unwrap().unwrap();
    let facts=store.turn_output_facts(&session,&operation).await.unwrap();
    let command:nomifun_agent_runtime::AgentCollaborationMessage=serde_json::from_value(
        facts.event_payloads[&source.command_event_id].pointer("/event/call/arguments").unwrap().clone()).unwrap();
    // The domain row is the original owner receipt. There is no Runtime
    // ToolCompleted observation at this held response boundary.
    let before:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_execution_steps")
        .fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(before,if committed {3} else {2});
    let observed:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='tool_completed' AND json_extract(inline_json,'$.event.result.call_id')='followup-alpha'")
        .bind(session.as_ref()).fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(observed,0,"the test holds only the response, not the actual durable owner transaction");
    let parent_io_before:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_effects WHERE session_id=? AND capability_module IN ('workspace.files','workspace.process')")
        .bind(session.as_ref()).fetch_one(fixture.pool()).await.unwrap();
    let pause=journal.pause_message_recovery_for_test().await.unwrap();
    let request=NativeResumeRequest {operation_id:operation.clone(),idempotency_key:"observe-original-message-owner".into(),
        expected_pause_revision:pause.revision,expected_checkpoint_revision:saved.revision,
        expected_checkpoint_digest:saved.digest,budget:Default::default(),cleanup_attestation:None};
    let prepared=crate::router::nomi_core_session::native_execution_control::prepare_resume_for_test(
        fixture.owner.as_ref(),&principal,&session,&request).await.unwrap();
    let observed=prepared.observations.iter().filter_map(|value|serde_json::from_value::<AgentEngineEvent>(value.0.clone()).ok())
        .find_map(|event|match event {
            AgentEngineEvent::ToolOutcomeReconciled {result,source,evidence_event_id,owner_operation_id,..}
                if result.call_id.as_ref()=="followup-alpha"=>Some((result,source,evidence_event_id,owner_operation_id)),
            _=>None,
        }).expect("the production resume preparation must resolve this exact native message from its domain owner");
    assert_eq!(observed.1,AgentReconciliationSource::NativeControlReceipt);
    assert!(observed.2.as_deref().is_some_and(|id|!id.is_empty())&&observed.3.is_some());
    let result:Value=serde_json::from_str(&observed.0.output_text()).unwrap();
    assert_eq!(observed.0.is_error,!committed);
    if committed {
        assert_eq!(result["accepted"],true);
        assert!(result["step_ids"].as_array().is_some_and(|ids|ids.len()==1));
        assert!(prepared.checkpoint_state.0["collaboration"]["pending_child_executions"].as_array()
            .is_some_and(|ids|ids.contains(&result["execution_id"])),"recovery retains the new result handoff obligation");
    } else {
        assert_eq!(result,json!({"state":"not_executed","control_applied":false,"code":"CHILD_MESSAGE_NOT_COMMITTED"}));
    }
    let after:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_execution_steps").fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(after,before,"resume preparation only reads facts and never redispatches the child command");
    // Check the original command owner's writer directly. A paused Host may
    // retain recovery/cleanup metadata; that cache reference is not execution
    // evidence. The old epoch can only observe its already committed receipt,
    // or reject an uncommitted write, even before its heartbeat notices pause.
    let fenced=fixture.voice_execution.message_step(&principal.principal_id,&source,&command).await;
    if committed {assert!(fenced.unwrap().replayed);} else {assert!(fenced.is_err());}
    release.add_permits(1);
    let after_late:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_execution_steps").fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(after_late,before,"the old epoch cannot commit a message after the pause fence");
    fixture.voice_execution.shutdown().await.unwrap();
    fixture.runtime.kill_and_wait(None).await.unwrap();
    // This fault deliberately revokes a still-live producer, unlike an orderly
    // Host pause that settles resources first. Joining that stale carrier must
    // preserve its exact missing cleanup acknowledgement rather than forge one
    // with the revoked lease. Its transport/task stop and canonical recovery
    // evidence are separate from a successful full teardown proof.
    let shutdown=fixture.services.shutdown_nomi_core_host().await.unwrap_err().to_string();
    assert!(shutdown.contains("1 Agent runtime(s)")&&shutdown.contains(session.as_ref())
        &&shutdown.contains("native execution producer no longer owns this Turn"),"unexpected shutdown failure: {shutdown}");
    assert!(!original_runtime.is_transport_healthy());
    assert!(!original_runtime.teardown_proven(),"a fenced producer cannot publish a successful cleanup witness");
    let retained=fixture.services.agent_runtime_sessions.observe_runtime_carrier(session.as_ref());
    assert_eq!(retained.instance_id,original_carrier.instance_id);
    assert!(retained.registered||retained.teardown_pending,"the original unacknowledged owner must remain discoverable");
    assert_eq!(store.native_execution_notification_state(&session,&operation,source.native_generation).await.unwrap().as_deref(),Some("paused"));
    let final_checkpoint=store.load_native_checkpoint(&principal,&session,&operation).await.unwrap().unwrap();
    assert_eq!(final_checkpoint.revision,request.expected_checkpoint_revision);
    assert_eq!(final_checkpoint.digest,request.expected_checkpoint_digest);
    let final_steps:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_execution_steps").fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(final_steps,before,"neither recovery preparation nor stale-owner shutdown dispatches replacement work");
    let parent_io:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_effects WHERE session_id=? AND capability_module IN ('workspace.files','workspace.process')")
        .bind(session.as_ref()).fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(parent_io,parent_io_before,"this fault must not start parent integration I/O after its pause fence");
    server.close().await;
}

#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn committed_child_message_with_lost_runtime_reply_recovers_the_original_receipt_without_resending() {
    lost_reply(true).await;
}

#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn pause_before_child_message_commit_proves_no_execution_and_fences_the_old_future() {
    lost_reply(false).await;
}
