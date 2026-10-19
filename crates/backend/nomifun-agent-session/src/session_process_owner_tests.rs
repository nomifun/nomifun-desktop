use super::*;
use nomifun_agent_contracts::{NativeRetainedProcess,NativeSessionProcessRetention,SessionProcessOwnerReleasedV1,SESSION_PROCESS_OWNER_RELEASED};
use crate::{ClearSessionContext,ContextClearPreparation};
#[path="session_process_archive_tests.rs"]
mod archive_tests;
#[path="session_process_live_reap_tests.rs"]
mod live_reap_tests;

fn native_terminal()->StrictJsonValue {StrictJsonValue(json!({"state":"exited","exit_code":0,"signal":null,
    "output":{"text":"late original output\n","next_cursor":21,"retained_bytes":21,"dropped_bytes":0,"source_encoding":"utf-8","decode_errors":0},
    "cleanup":{"interrupt_attempted":false,"terminate_attempted":false,"force_kill_attempted":false,"reaped":true,"elapsed_ms":0,"errors":[]}}))}

#[tokio::test]
async fn portable_reap_contract_fixture_does_not_require_foreign_decoder_but_explicit_output_replay_fails_closed() {
    let(store,session,release,_)=retained_process_fixture().await;let sid=&session.agent_session_id;
    // This is a foreign canonical receipt/body fixture, not a claim that this
    // host executed or natively validated another platform's process/decoder.
    #[cfg(windows)]
    let (foreign,text,encoding,errors)=(include_str!("../../../shared/nomi-process-runtime/src/io/frozen_output/fixtures/frozen-output-v1-utf8.json"),"a\u{fffd}","utf-8",1);
    #[cfg(not(windows))]
    let (foreign,text,encoding,errors)=(include_str!("../../../shared/nomi-process-runtime/src/io/frozen_output/fixtures/frozen-output-v1-windows-cp936.json"),"x丂\u{fffd}","mixed",3);
    let replay:serde_json::Value=serde_json::from_str(foreign).unwrap();let mut terminal=native_terminal();
    terminal.0["output"]=json!({"text":text,"next_cursor":6,"retained_bytes":3,"dropped_bytes":3,"source_encoding":encoding,"decode_errors":errors});
    let mut receipt=nomifun_agent_contracts::SessionProcessReapedV1 {retention:release.retention.clone(),terminal:Some(terminal),
        terminal_ref:None,output_replay:Some(StrictJsonValue(replay.clone())),output_archive:None,component_identity:None};
    let before=store.current_cursor(sid).await.unwrap();
    assert!(store.record_session_process_reaped(&owner(),receipt.clone()).await.is_err(),"a new writer still requires the actual supported original decoder");
    assert_eq!(store.current_cursor(sid).await.unwrap(),before);
    receipt.output_replay=None;let ack=store.record_session_process_reaped(&owner(),receipt.clone()).await.unwrap();
    receipt.output_replay=Some(StrictJsonValue(replay));let value=serde_json::to_value(receipt).unwrap();
    let bytes=nomifun_agent_contracts::canonical_json_bytes(&value).unwrap();
    let body=serde_json::to_vec(&SessionPayloadBody::Json(StrictJsonValue(value))).unwrap();
    sqlx::query("UPDATE agent_payloads SET body=?,byte_len=?,digest=? WHERE payload_id=(SELECT payload_id FROM agent_events WHERE session_id=? AND event_id=?)")
        .bind(body).bind(bytes.len() as i64).bind(nomifun_agent_contracts::digest_bytes(&bytes).as_ref()).bind(sid.as_ref()).bind(ack.event_id.as_ref())
        .execute(store.test_pool()).await.unwrap();
    assert!(store.unreleased_session_process_generations(&owner(),sid).await.unwrap().is_empty());
    store.rebuild_projections(sid).await.unwrap();
    let root=tempfile::tempdir().unwrap();store.validate_workspace_relocation(root.path(),root.path()).await.unwrap();
    let process=&release.retention.processes[0];
    assert!(store.session_process_resource_stopped(&owner(),sid,&release.retention.owner_generation,&process.process_id,
        process.creator_turn_id.as_ref(),process.creator_call_id.as_ref()).await.unwrap());
    assert!(store.session_process_terminal(&owner(),sid,&process.process_id,&process.resolved_snapshot_ref,&process.resource_binding_digest,
        &"workspace.process/poll".into(),0).await.is_err(),"explicit byte replay never falls back to flattened text");
    let request=ClearSessionContext {idempotency_key:"portable-clear".into(),reason:"fixture clear".into()};
    let ContextClearPreparation::Ready(boundary)=store.preflight_context_clear(&owner(),sid,&request).await.unwrap() else {panic!("new clear expected")};
    store.clear_context(&owner(),sid,&request,&boundary).await.unwrap();
}

#[tokio::test]
async fn exact_native_process_reap_keeps_terminal_output_and_does_not_forge_whole_owner_release() {
    let(store,session,release,_)=retained_process_fixture().await;
    let process=&release.retention.processes[0];
    let receipt=nomifun_agent_contracts::SessionProcessReapedV1 {retention:release.retention.clone(),terminal:Some(native_terminal()),
        terminal_ref:None,output_replay:None,output_archive:None,component_identity:None};
    let ack=store.record_session_process_reaped(&owner(),receipt.clone()).await.unwrap();
    assert_eq!(store.record_session_process_reaped(&owner(),receipt).await.unwrap(),ack);
    assert!(store.unreleased_session_process_generations(&owner(),&session.agent_session_id).await.unwrap().is_empty());
    let (terminal,_)=store.session_process_terminal(&owner(),&session.agent_session_id,&process.process_id,&process.resolved_snapshot_ref,
        &process.resource_binding_digest,&"workspace.process/poll".into(),0).await.unwrap().unwrap();
    assert_eq!(serde_json::to_value(terminal).unwrap(),native_terminal().0);
    let root=tempfile::tempdir().unwrap();store.validate_workspace_relocation(root.path(),root.path()).await.unwrap();
    store.rebuild_projections(&session.agent_session_id).await.unwrap();
    assert!(store.unreleased_session_process_generations(&owner(),&session.agent_session_id).await.unwrap().is_empty());
    assert!(!store.read_events(&session.agent_session_id,None,500).await.unwrap().events.iter().any(|event|event.kind.0==SESSION_PROCESS_OWNER_RELEASED));
}

#[tokio::test]
async fn process_reap_rejects_foreign_creator_unknown_cleanup_or_modified_frozen_authority() {
    for changed in ["owner","generation","process","snapshot","resource","lost","unreaped","cleanup_error","terminal_ref"] {
        let(store,session,release,_)=retained_process_fixture().await;
        let mut receipt=nomifun_agent_contracts::SessionProcessReapedV1 {retention:release.retention,terminal:Some(native_terminal()),
            terminal_ref:None,output_replay:None,output_archive:None,component_identity:None};
        match changed {
            "owner"=>receipt.retention.owner_ref.principal_id="another-owner".into(),
            "generation"=>receipt.retention.owner_generation=Uuid::now_v7().to_string(),
            "process"=>receipt.retention.processes[0].process_id=Uuid::now_v7().to_string(),
            "snapshot"=>receipt.retention.processes[0].resolved_snapshot_ref.snapshot_digest=digest('d'),
            "resource"=>receipt.retention.processes[0].resource_binding_digest=digest('d'),
            "lost"=>{let body=&mut receipt.terminal.as_mut().unwrap().0;body.as_object_mut().unwrap().remove("exit_code");
                body.as_object_mut().unwrap().remove("signal");body["state"]=json!("lost");body["pid"]=json!(100);},
            "unreaped"=>receipt.terminal.as_mut().unwrap().0["cleanup"]["reaped"]=json!(false),
            "cleanup_error"=>receipt.terminal.as_mut().unwrap().0["cleanup"]["errors"]=json!(["original cleanup unknown"]),
            _=>{receipt.terminal=None;receipt.terminal_ref=Some("not-a-process-terminal".into());},
        }
        let before=store.current_cursor(&session.agent_session_id).await.unwrap();
        assert!(store.record_session_process_reaped(&owner(),receipt).await.is_err(),"{changed}");
        assert_eq!(store.current_cursor(&session.agent_session_id).await.unwrap(),before);
    }
}

#[tokio::test]
async fn already_canonical_process_terminal_without_retention_is_referenced_without_a_second_tool_result() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let(session,_)=create_ready(&store,"short-process").await;
    let sid=&session.agent_session_id;
    let(_,started)=store.start_turn(sid,"session_api".into(),"short-input".into(),"short-turn".into(),StrictJsonValue(json!({"content":"Run",
        "admission":{"resolved_snapshot_ref":session.agent_binding.resolved_snapshot_ref}}))).await.unwrap();
    let start=started.ack.unwrap().event_id;
    let retention=NativeSessionProcessRetention {agent_session_id:sid.clone(),owner_ref:owner(),owner_generation:Uuid::now_v7().to_string(),
        processes:vec![NativeRetainedProcess {process_id:Uuid::now_v7().to_string(),pid:100,creator_turn_id:"short-turn".into(),creator_call_id:"short-operation".into(),
            resolved_snapshot_ref:session.agent_binding.resolved_snapshot_ref.clone(),resource_binding_digest:digest('c')}]};
    process_tool_fact(&store,sid,"short-dispatch","short-turn",start.clone(),json!({"event":"host_tool_dispatch","dispatch":{
        "operation_id":"short-operation","call_id":"short-call","capability_id":"workspace.process","action_id":"workspace.process/start"}})).await;
    let mut terminal=native_terminal().0;terminal["process_id"]=json!(retention.processes[0].process_id);
    process_tool_fact(&store,sid,"short-result","short-turn",start.clone(),process_settled("short-operation","short-call",terminal)).await;
    process_tool_fact(&store,sid,"short-cleanup","short-turn",start.clone(),json!({"event":"host_cleanup_proven"})).await;
    store.append_event(&append(sid,"short-completed","runtime_supervisor","short-completed","turn/completed","short-turn",Some(start),json!({}))).await.unwrap();
    let receipt=nomifun_agent_contracts::SessionProcessReapedV1 {retention,terminal:Some(native_terminal()),terminal_ref:None,output_replay:None,output_archive:None,component_identity:None};
    let ack=store.record_session_process_reaped(&owner(),receipt.clone()).await.unwrap();
    assert_eq!(store.record_session_process_reaped(&owner(),receipt).await.unwrap(),ack);
    let facts=store.read_events(sid,None,500).await.unwrap();
    let reaped=facts.events.iter().find(|event|event.kind.0==nomifun_agent_contracts::SESSION_PROCESS_REAPED).unwrap();
    let values=store.resolved_event_payloads(sid,&[reaped.event_id.clone()]).await.unwrap();
    let stored:nomifun_agent_contracts::SessionProcessReapedV1=serde_json::from_value(values[reaped.event_id.as_ref()].clone()).unwrap();
    assert!(stored.terminal.is_none());assert_eq!(stored.terminal_ref,Some("short-result".into()));
    let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='host_tool_settled'")
        .bind(sid.as_ref()).fetch_one(store.test_pool()).await.unwrap();assert_eq!(count,1);
}

#[tokio::test]
async fn single_process_reap_reduces_only_its_exact_tuple_and_storage_waits_for_the_other_original_process() {
    let(store,session,first,_)=retained_process_fixture().await;let sid=&session.agent_session_id;
    let(_,started)=store.start_turn(sid,"session_api".into(),"second-input".into(),"second-turn".into(),StrictJsonValue(json!({"content":"Run second",
        "admission":{"resolved_snapshot_ref":session.agent_binding.resolved_snapshot_ref}}))).await.unwrap();let start=started.ack.unwrap().event_id;
    let mut second=first.retention.clone();second.processes[0].process_id=Uuid::now_v7().to_string();second.processes[0].pid=101;
    second.processes[0].creator_turn_id="second-turn".into();second.processes[0].creator_call_id="second-operation".into();
    process_tool_fact(&store,sid,"second-dispatch","second-turn",start.clone(),json!({"event":"host_tool_dispatch","dispatch":{
        "operation_id":"second-operation","call_id":"second-call","capability_id":"workspace.process","action_id":"workspace.process/start"}})).await;
    process_tool_fact(&store,sid,"second-result","second-turn",start.clone(),process_settled("second-operation","second-call",json!({
        "process_id":second.processes[0].process_id,"state":"running","pid":101,"output":process_test_output()}))).await;
    process_tool_fact(&store,sid,"second-cleanup","second-turn",start.clone(),json!({"event":"host_cleanup_proven","session_process_retention":second})).await;
    store.append_event(&append(sid,"second-completed","runtime_supervisor","second-completed","turn/completed","second-turn",Some(start),json!({}))).await.unwrap();
    let reaped=|retention|nomifun_agent_contracts::SessionProcessReapedV1 {retention,terminal:Some(native_terminal()),terminal_ref:None,output_replay:None,output_archive:None,component_identity:None};
    store.record_session_process_reaped(&owner(),reaped(first.retention.clone())).await.unwrap();
    assert_eq!(store.session_process_owner_retention(&owner(),sid,&first.retention.owner_generation).await.unwrap().unwrap(),second);
    let root=tempfile::tempdir().unwrap();assert!(store.validate_workspace_relocation(root.path(),root.path()).await.is_err());
    store.record_session_process_reaped(&owner(),reaped(second)).await.unwrap();
    store.validate_workspace_relocation(root.path(),root.path()).await.unwrap();
}

#[tokio::test]
async fn ssh_exact_close_requested_and_unknown_result_survive_projection_rebuild_and_owner_checks() {
    use nomifun_agent_contracts::{SessionSshLinkCloseV1,SessionSshLinkClosedV1,SessionSshLinkCloseResult};
    let store=AgentSessionStore::open_in_memory().await.unwrap();let(session,_)=create_ready(&store,"ssh-exact-close").await;
    let close=SessionSshLinkCloseV1 {agent_session_id:session.agent_session_id.clone(),owner_ref:owner(),ssh_host_id:Uuid::now_v7().to_string(),
        pool_instance_id:Uuid::now_v7().to_string(),instance_id:1};
    let request=store.record_session_ssh_link_close_requested(&owner(),close.clone()).await.unwrap();
    assert_eq!(store.record_session_ssh_link_close_requested(&owner(),close.clone()).await.unwrap(),request);
    assert_eq!(store.session_ssh_link_closes(&owner(),&session.agent_session_id).await.unwrap(),vec![(close.clone(),None)]);
    let result=SessionSshLinkClosedV1 {close:close.clone(),result:SessionSshLinkCloseResult::Lost {detail:"channel closed without original exit evidence".into()}};
    let settled=store.record_session_ssh_link_closed(&owner(),result.clone()).await.unwrap();
    assert_eq!(store.record_session_ssh_link_closed(&owner(),result.clone()).await.unwrap(),settled);
    store.rebuild_projections(&session.agent_session_id).await.unwrap();
    assert_eq!(store.session_ssh_link_closes(&owner(),&session.agent_session_id).await.unwrap(),vec![(close.clone(),Some(result.clone()))]);
    let mut foreign=owner();foreign.principal_id="foreign-owner".into();
    assert!(store.session_ssh_link_closes(&foreign,&session.agent_session_id).await.is_err());
    assert!(store.record_session_ssh_link_closed(&foreign,result.clone()).await.is_err());
    let mut changed=result.clone();changed.close.instance_id+=1;
    assert!(store.record_session_ssh_link_closed(&owner(),changed).await.is_err());
    let changed=SessionSshLinkClosedV1 {close,result:SessionSshLinkCloseResult::Reaped {detail:"a new connection is not old cleanup proof".into()}};
    assert!(store.record_session_ssh_link_closed(&owner(),changed).await.is_err(),"unknown original result is immutable");
}

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
