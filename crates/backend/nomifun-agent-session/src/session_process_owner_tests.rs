use super::*;
use nomifun_agent_contracts::{NativeRetainedProcess,NativeSessionProcessRetention,SessionProcessOwnerReleasedV1,SESSION_PROCESS_OWNER_RELEASED};

async fn retained_process_fixture() ->(AgentSessionStore,AgentSessionLiveRecord,SessionProcessOwnerReleasedV1,EventId) {
    let store=AgentSessionStore::open_in_memory().await.unwrap();
    let(session,_)=create_ready(&store,"retained-process").await;
    let sid=&session.agent_session_id;
    let(_,started)=store.start_turn(sid,"session_api".into(),"retained-input".into(),"retained-turn".into(),StrictJsonValue(json!({"content":"Authorize",
        "admission":{"resolved_snapshot_ref":session.agent_binding.resolved_snapshot_ref}}))).await.unwrap();
    let start=started.ack.unwrap().event_id;
    let retention=NativeSessionProcessRetention {agent_session_id:sid.clone(),owner_ref:owner(),owner_generation:Uuid::now_v7().to_string(),
        processes:vec![NativeRetainedProcess {process_id:Uuid::now_v7().to_string(),pid:100,creator_turn_id:"retained-turn".into(),creator_call_id:"launch-original".into(),
            resolved_snapshot_ref:session.agent_binding.resolved_snapshot_ref.clone(),resource_binding_digest:digest('c')}]};
    process_tool_fact(&store,sid,"retained-launch-dispatch","retained-turn",start.clone(),json!({"event":"host_tool_dispatch","dispatch":{
        "operation_id":"launch-original","call_id":"launch-model-call","capability_id":"workspace.process","action_id":"workspace.process/start"}})).await;
    let running=json!({"process_id":retention.processes[0].process_id,"state":"running","pid":100,"output":process_test_output()});
    process_tool_fact(&store,sid,"retained-launch-result","retained-turn",start.clone(),process_settled("launch-original","launch-model-call",running)).await;
    store.append_event(&append(sid,"retained-dispatch","runtime_supervisor","retained-dispatch","runtime/progress-recorded","retained-turn",Some(start.clone()),
        json!({"event":{"event":"host_process_dispatch","operation_id":"launch-original","ordinal":1}}))).await.unwrap();
    let cleanup=append(sid,"retained-cleanup","runtime_supervisor","retained-cleanup","runtime/progress-recorded","retained-turn",Some(start.clone()),
        json!({"event":{"event":"host_cleanup_proven","session_process_retention":retention}}));
    store.append_event(&cleanup).await.unwrap();
    store.append_event(&append(sid,"retained-completed","runtime_supervisor","retained-completed","turn/completed","retained-turn",Some(start),json!({}))).await.unwrap();
    (store,session,SessionProcessOwnerReleasedV1 {retention,reaped:true},cleanup.event_id)
}

fn process_test_output()->serde_json::Value {json!({"text":"","next_cursor":0,"retained_bytes":0,"dropped_bytes":0,"source_encoding":"utf-8","decode_errors":0})}

fn process_settled(operation:&str,call:&str,observation:serde_json::Value)->serde_json::Value {
    json!({"event":"host_tool_settled","operation_id":operation,"call_id":call,"error":null,
        "result":nomifun_engine_core::EngineToolResult::text(call.into(),observation.to_string(),false)})
}

async fn process_tool_fact(store:&AgentSessionStore,session:&AgentSessionId,id:&str,turn:&str,cause:EventId,native:serde_json::Value) {
    store.append_event(&append(session,id,"runtime_supervisor",id,"runtime/progress-recorded",turn,Some(cause),json!({"event":native}))).await.unwrap();
}

#[tokio::test]
async fn exact_successor_terminal_receipt_retires_old_retention_without_an_owner_release_or_replay() {
    let(store,session,receipt,_)=retained_process_fixture().await;
    let sid=&session.agent_session_id;
    let(_,started)=store.start_turn(sid,"session_api".into(),"observe-input".into(),"observe-turn".into(),StrictJsonValue(json!({"content":"Observe the original process"}))).await.unwrap();
    let start=started.ack.unwrap().event_id;
    process_tool_fact(&store,sid,"observe-dispatch","observe-turn",start.clone(),json!({"event":"host_tool_dispatch","dispatch":{
        "operation_id":"observe-original","call_id":"observe-call","capability_id":"workspace.process","action_id":"workspace.process/poll"}})).await;
    let terminal=json!({"process_id":receipt.retention.processes[0].process_id,"state":"exited","exit_code":0,"signal":null,
        "output":process_test_output(),"cleanup":{"interrupt_attempted":false,"terminate_attempted":false,"force_kill_attempted":false,
            "reaped":true,"elapsed_ms":0,"errors":[]}});
    process_tool_fact(&store,sid,"observe-result","observe-turn",start.clone(),process_settled("observe-original","observe-call",terminal)).await;
    assert_eq!(store.confirmed_reaped_session_processes(&receipt.retention).await.unwrap(),BTreeSet::from([receipt.retention.processes[0].process_id.clone()]));
    assert!(store.session_process_owner_retention(&owner(),sid,&receipt.retention.owner_generation).await.unwrap().is_none());
    assert!(store.unreleased_session_process_generations(&owner(),sid).await.unwrap().is_empty());
    process_tool_fact(&store,sid,"observe-cleanup","observe-turn",start.clone(),json!({"event":"host_cleanup_proven"})).await;
    store.append_event(&append(sid,"observe-completed","runtime_supervisor","observe-completed","turn/completed","observe-turn",Some(start),json!({}))).await.unwrap();
    let root=tempfile::tempdir().unwrap();
    store.validate_workspace_relocation(root.path(),root.path()).await.unwrap();
    store.rebuild_projections(sid).await.unwrap();
    assert!(store.unreleased_session_process_generations(&owner(),sid).await.unwrap().is_empty());
    assert!(!store.read_events(sid,None,500).await.unwrap().events.iter().any(|event|event.kind.0==SESSION_PROCESS_OWNER_RELEASED),
        "a real native terminal is sufficient; no synthetic owner-release event is created");
}

#[tokio::test]
async fn process_retirement_requires_first_party_exact_creator_and_positive_reap() {
    for changed in ["capability","action","process","call","error","running","lost","unreaped","cleanup_error"] {
        let(store,session,receipt,_)=retained_process_fixture().await;
        let sid=&session.agent_session_id;
        let(_,started)=store.start_turn(sid,"session_api".into(),"observe-input".into(),"observe-turn".into(),StrictJsonValue(json!({"content":"Observe"}))).await.unwrap();
        let start=started.ack.unwrap().event_id;
        let mut dispatch=json!({"event":"host_tool_dispatch","dispatch":{"operation_id":"observe-original","call_id":"observe-call",
            "capability_id":"workspace.process","action_id":"workspace.process/poll"}});
        if changed=="capability" {dispatch["dispatch"]["capability_id"]=json!("unrelated.tool");}
        if changed=="action" {dispatch["dispatch"]["action_id"]=json!("workspace.process/start");}
        process_tool_fact(&store,sid,"bad-observe-dispatch","observe-turn",start.clone(),dispatch).await;
        let mut terminal=json!({"process_id":receipt.retention.processes[0].process_id,"state":"exited","exit_code":0,"signal":null,
            "output":process_test_output(),"cleanup":{"interrupt_attempted":false,"terminate_attempted":false,"force_kill_attempted":false,
                "reaped":true,"elapsed_ms":0,"errors":[]}});
        match changed {
            "process"=>terminal["process_id"]=json!(Uuid::now_v7().to_string()),
            "running"=>terminal=json!({"process_id":receipt.retention.processes[0].process_id,"state":"running","pid":100,"output":process_test_output()}),
            "lost"=>{terminal.as_object_mut().unwrap().remove("exit_code");terminal.as_object_mut().unwrap().remove("signal");terminal["state"]=json!("lost");terminal["pid"]=json!(100);},
            "unreaped"=>terminal["cleanup"]["reaped"]=json!(false),
            "cleanup_error"=>terminal["cleanup"]["errors"]=json!(["missing reap witness"]),
            _=>{},
        }
        let mut settled=process_settled("observe-original","observe-call",terminal);
        if changed=="call" {settled["call_id"]=json!("another-call");}
        if changed=="error" {settled["error"]=json!("native owner result lost");}
        process_tool_fact(&store,sid,"bad-observe-result","observe-turn",start,settled).await;
        assert!(store.confirmed_reaped_session_processes(&receipt.retention).await.unwrap().is_empty(),"{changed}");
        assert_eq!(store.session_process_owner_retention(&owner(),sid,&receipt.retention.owner_generation).await.unwrap().unwrap(),receipt.retention,"{changed}");
    }
}

#[tokio::test]
async fn process_retirement_never_hydrates_an_unrelated_large_tool_payload() {
    let(store,session,receipt,_)=retained_process_fixture().await;
    let sid=&session.agent_session_id;
    let(_,started)=store.start_turn(sid,"session_api".into(),"observe-input".into(),"observe-turn".into(),StrictJsonValue(json!({"content":"Observe"}))).await.unwrap();
    let start=started.ack.unwrap().event_id;
    process_tool_fact(&store,sid,"unrelated-dispatch","observe-turn",start.clone(),json!({"event":"host_tool_dispatch","dispatch":{
        "operation_id":"large-file","call_id":"file-call","capability_id":"workspace.files","action_id":"workspace.files/read"}})).await;
    let unrelated=nomifun_engine_core::EngineToolResult::text("file-call".into(),"unrelated file output ".repeat(2000),false);
    process_tool_fact(&store,sid,"unrelated-result","observe-turn",start.clone(),json!({"event":"host_tool_settled",
        "operation_id":"large-file","call_id":"file-call","result":unrelated,"error":null})).await;
    let encoded:String=sqlx::query_scalar("SELECT inline_json FROM agent_events WHERE session_id=? AND event_id='unrelated-result'")
        .bind(sid.as_ref()).fetch_one(store.test_pool()).await.unwrap();
    let envelope:serde_json::Value=serde_json::from_str(&encoded).unwrap();
    let unrelated_payload=envelope["_canonical_shared_content"]["references"][0]["payload_id"].as_str().unwrap();
    sqlx::query("UPDATE agent_payloads SET body=? WHERE payload_id=?")
        .bind(serde_json::to_string(&SessionPayloadBody::Json(StrictJsonValue(json!([])))).unwrap().into_bytes())
        .bind(unrelated_payload).execute(store.test_pool()).await.unwrap();
    process_tool_fact(&store,sid,"observe-dispatch","observe-turn",start.clone(),json!({"event":"host_tool_dispatch","dispatch":{
        "operation_id":"observe-original","call_id":"observe-call","capability_id":"workspace.process","action_id":"workspace.process/poll"}})).await;
    let terminal=json!({"process_id":receipt.retention.processes[0].process_id,"state":"exited","exit_code":0,"signal":null,
        "output":process_test_output(),"cleanup":{"interrupt_attempted":false,"terminate_attempted":false,"force_kill_attempted":false,
            "reaped":true,"elapsed_ms":0,"errors":[]}});
    process_tool_fact(&store,sid,"observe-result","observe-turn",start,process_settled("observe-original","observe-call",terminal)).await;
    assert_eq!(store.confirmed_reaped_session_processes(&receipt.retention).await.unwrap().len(),1,
        "the process proof reader must select its first-party dispatch before loading payload contents");
    assert!(store.session_process_owner_retention(&owner(),sid,&receipt.retention.owner_generation).await.unwrap().is_none());
    assert!(store.resolved_event_payloads(sid,&["unrelated-result".into()]).await.is_err(),
        "the deliberately damaged file payload fails closed when the typed content reader resolves it");
}

#[tokio::test]
async fn session_process_release_is_exact_idempotent_and_required_for_native_storage_quiescence() {
    let(store,session,receipt,_)=retained_process_fixture().await;
    let root=tempfile::tempdir().unwrap();
    assert!(store.validate_workspace_relocation(root.path(),root.path()).await.is_err(),"a finite Turn is not process cleanup");
    let ack=store.record_session_process_owner_released(&owner(),receipt.clone()).await.unwrap().unwrap();
    assert_eq!(store.record_session_process_owner_released(&owner(),receipt.clone()).await.unwrap(),Some(ack));
    store.validate_workspace_relocation(root.path(),root.path()).await.unwrap();
    let before=store.read_events(&session.agent_session_id,None,500).await.unwrap();
    store.rebuild_projections(&session.agent_session_id).await.unwrap();
    assert_eq!(store.read_events(&session.agent_session_id,None,500).await.unwrap(),before);
}

#[tokio::test]
async fn session_process_release_rejects_wrong_owner_generation_process_and_frozen_authority() {
    for changed in ["owner","generation","process","snapshot","resource","unreaped","missing","extra"] {
        let(store,session,mut receipt,_)=retained_process_fixture().await;
        match changed {
            "owner"=>receipt.retention.owner_ref.principal_id="another-owner".into(),
            "generation"=>receipt.retention.owner_generation=Uuid::now_v7().to_string(),
            "process"=>receipt.retention.processes[0].process_id=Uuid::now_v7().to_string(),
            "snapshot"=>receipt.retention.processes[0].resolved_snapshot_ref.snapshot_digest=digest('d'),
            "resource"=>receipt.retention.processes[0].resource_binding_digest=digest('d'),
            "unreaped"=>receipt.reaped=false,
            "extra"=>{let mut extra=receipt.retention.processes[0].clone();extra.process_id=Uuid::now_v7().to_string();receipt.retention.processes.push(extra);},
            _=>receipt.retention.processes.clear(),
        }
        let before=store.current_cursor(&session.agent_session_id).await.unwrap();
        assert!(store.record_session_process_owner_released(&owner(),receipt).await.is_err(),"{changed}");
        assert_eq!(store.current_cursor(&session.agent_session_id).await.unwrap(),before);
    }
}

#[tokio::test]
async fn session_process_release_settles_only_original_generation_behind_delete_fence() {
    let(store,session,receipt,_)=retained_process_fixture().await;
    store.fence_delete(&DeleteAgentSessionCommand {operation_id:"delete-retained-owner".into(),agent_session_id:session.agent_session_id.clone(),
        owner_ref:owner(),requested_at:1_990_000_000_000}).await.unwrap();
    assert!(store.get_live_session(&session.agent_session_id).await.is_err());
    let ack=store.record_session_process_owner_released(&owner(),receipt).await.unwrap().unwrap();
    assert!(ack.seq>0);
    assert!(store.get_deleting_session(&session.agent_session_id).await.is_ok());
}

#[tokio::test]
async fn unreleased_process_owner_does_not_block_an_unrelated_plain_text_turn() {
    let(store,session,receipt,_)=retained_process_fixture().await;
    let sid=&session.agent_session_id;
    let before=store.unreleased_session_process_generations(&owner(),sid).await.unwrap();
    assert_eq!(before,BTreeSet::from([receipt.retention.owner_generation.clone()]));
    let(_,started)=store.start_turn(sid,"session_api".into(),"plain-followup".into(),"plain-followup-turn".into(),
        StrictJsonValue(json!({"content":"Change the subject"}))).await.unwrap();
    store.append_event(&append(sid,"plain-finished","runtime_supervisor","plain-finished","turn/completed","plain-followup-turn",
        Some(started.ack.unwrap().event_id),json!({}))).await.unwrap();
    assert_eq!(store.head(sid).await.unwrap().status,"ready");
    assert_eq!(store.unreleased_session_process_generations(&owner(),sid).await.unwrap(),before,
        "a plain response grants no cleanup or process restart authority");
}

#[tokio::test]
async fn session_process_release_cannot_use_generic_append_or_accept_late_retention() {
    let(store,session,receipt,cause)=retained_process_fixture().await;
    let forged=append(&session.agent_session_id,"forged-release","session_api","forged-release",SESSION_PROCESS_OWNER_RELEASED,
        session.agent_session_id.as_ref(),Some(cause),serde_json::to_value(&receipt).unwrap());
    assert!(store.append_event(&forged).await.is_err());
    store.record_session_process_owner_released(&owner(),receipt.clone()).await.unwrap();
    let late=append(&session.agent_session_id,"late-retention","runtime_supervisor","late-retention","runtime/progress-recorded","retained-turn",None,
        json!({"event":{"event":"host_cleanup_proven","session_process_retention":receipt.retention}}));
    assert!(store.append_event(&late).await.is_err());
}
