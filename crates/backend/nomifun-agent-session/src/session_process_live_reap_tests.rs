use super::*;
use crate::{NativeExecutionClaim, NativeExecutionLease};
use nomifun_agent_contracts::{NativeProcessOwnerIdentity, NativeProcessWitness, SessionProcessReapedV1};
use nomifun_engine_core::{EngineCleanupReport, EngineProcessOutputReplay, EngineProcessTerminalMetadata};
use serde_json::Value;

struct LiveProcess {
    store:AgentSessionStore,
    session:AgentSessionLiveRecord,
    retention:NativeSessionProcessRetention,
    lease:NativeExecutionLease,
    started:EventId,
}

async fn live_process() -> LiveProcess {
    let store=AgentSessionStore::open_in_memory().await.unwrap();
    let(session,_)=create_ready(&store,"live-native-reap").await;let sid=&session.agent_session_id;
    let(root,started)=store.start_turn(sid,"session_api".into(),"live-input".into(),"live-turn".into(),
        StrictJsonValue(json!({"content":"Start original background work","admission":{"resolved_snapshot_ref":session.agent_binding.resolved_snapshot_ref}}))).await.unwrap();
    let root=root.ack.unwrap().event_id;let started=started.ack.unwrap().event_id;
    let lease=store.claim_native_execution(NativeExecutionClaim {owner:owner(),agent_session_id:sid.clone(),operation_id:"live-turn".into(),
        snapshot:session.agent_binding.resolved_snapshot_ref.clone(),active_set_generation:0,holder:"original-native-owner".into(),expected_fence:0,checkpoint:None}).await.unwrap();
    let retention=NativeSessionProcessRetention {agent_session_id:sid.clone(),owner_ref:owner(),owner_generation:Uuid::now_v7().to_string(),
        processes:vec![NativeRetainedProcess {process_id:Uuid::now_v7().to_string(),pid:100,creator_turn_id:"live-turn".into(),creator_call_id:"live-start".into(),
            resolved_snapshot_ref:session.agent_binding.resolved_snapshot_ref.clone(),resource_binding_digest:digest('c')}]};
    let dispatch=append(sid,"live-dispatch","runtime_supervisor","live-dispatch","runtime/progress-recorded","live-turn",Some(root.clone()),
        json!({"producer_seq":1,"event":{"event":"host_tool_dispatch","dispatch":{"operation_id":"live-start","call_id":"live-call",
            "capability_id":"workspace.process","action_id":"workspace.process/start"}}}));
    store.append_native_event(&lease,&dispatch,None).await.unwrap();
    let tool=append(sid,"live-tool","runtime_supervisor","live-tool","tool/call-started","live-call",Some(started.clone()),
        json!({"operation_id":"live-start","call_id":"live-call","capability_id":"workspace.process","action_id":"workspace.process/start"}));
    let tool=store.append_native_event(&lease,&tool,None).await.unwrap().ack.unwrap().event_id;
    let effect=EffectEventRequest {agent_session_id:sid.clone(),effect_id:"live-start-effect".into(),turn_id:"live-turn".into(),operation_id:"live-start".into(),
        owner_domain:"workspace".into(),capability_module:"workspace.process".into(),action_id:"workspace.process/start".into(),resource_binding_id:None,
        resource_key:Some("original-workspace".into()),input_digest:digest('7'),recorded_at:1_788_000_000_010,event_id:"live-effect-started".into(),
        producer_id:"capability_host".into(),idempotency_key:"live-effect".into(),correlation_id:"live-start-effect".into(),strategy:EffectStrategy::ManagedEffect,
        causation_event_id:Some(tool),payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({})))};
    let effect_started=store.record_effect_started(effect.clone()).await.unwrap().ack.unwrap().event_id;
    let witness=NativeProcessWitness::Dispatch {operation_id:"live-start".into(),ordinal:1,process_owner:Some(NativeProcessOwnerIdentity {
        owner_generation:retention.owner_generation.clone(),resource_binding_digest:retention.processes[0].resource_binding_digest.clone()})};
    let witness=append(sid,"live-native-dispatch","runtime_supervisor","live-native-dispatch","runtime/progress-recorded","live-turn",Some(root.clone()),
        json!({"producer_seq":2,"event":witness}));
    store.append_native_event(&lease,&witness,None).await.unwrap();
    let running=json!({"process_id":retention.processes[0].process_id,"state":"running","pid":100,"output":process_test_output()});
    store.record_effect_terminal(EffectEventRequest {event_id:"live-effect-returned".into(),producer_id:"owning_plugin".into(),causation_event_id:Some(effect_started),
        payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"result":running}))),..effect},EffectTerminalState::Succeeded).await.unwrap();
    let settled=append(sid,"live-returned","runtime_supervisor","live-returned","runtime/progress-recorded","live-turn",Some(root.clone()),
        json!({"producer_seq":3,"event":process_settled("live-start","live-call",running)}));
    store.append_native_observation(&lease,&settled,None).await.unwrap();
    LiveProcess {store,session,retention,lease,started}
}

async fn unrelated_mcp(f:&LiveProcess,unknown:bool) {
    let sid=&f.session.agent_session_id;
    let tool=append(sid,"mcp-tool","runtime_supervisor","mcp-tool","tool/call-started","mcp-call",Some(f.started.clone()),
        json!({"operation_id":"mcp-operation","call_id":"mcp-call","capability_id":"fixture.mcp","action_id":"fixture.mcp/invoke"}));
    let tool=f.store.append_native_event(&f.lease,&tool,None).await.unwrap().ack.unwrap().event_id;
    let effect=EffectEventRequest {agent_session_id:sid.clone(),effect_id:"unrelated-mcp".into(),turn_id:"live-turn".into(),operation_id:"mcp-operation".into(),
        owner_domain:"mcp".into(),capability_module:"fixture.mcp".into(),action_id:"fixture.mcp/invoke".into(),resource_binding_id:None,resource_key:None,
        input_digest:digest('8'),recorded_at:1_788_000_000_020,event_id:"mcp-effect-started".into(),producer_id:"capability_host".into(),
        idempotency_key:"mcp-effect".into(),correlation_id:"unrelated-mcp".into(),strategy:EffectStrategy::ExternalUncertainEffect,causation_event_id:Some(tool),
        payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({})))};
    let started=f.store.record_effect_started(effect.clone()).await.unwrap().ack.unwrap().event_id;
    if unknown {
        f.store.record_effect_terminal(EffectEventRequest {event_id:"mcp-effect-unknown".into(),producer_id:"owning_plugin".into(),causation_event_id:Some(started),
            payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"execution":{"returned":false},"cleanup":{"local":"confirmed","remote":"not_required"}}))),
            ..effect},EffectTerminalState::Uncertain).await.unwrap();
    }
}

fn reap(f:&LiveProcess,output:&EngineProcessOutputReplay,cancelled:bool)->SessionProcessReapedV1 {
    let cleanup=EngineCleanupReport {interrupt_attempted:cancelled,terminate_attempted:cancelled,force_kill_attempted:false,reaped:true,elapsed_ms:7,errors:vec![]};
    let metadata=if cancelled {EngineProcessTerminalMetadata::Cancelled {cleanup}}else {EngineProcessTerminalMetadata::Exited {exit_code:Some(0),signal:None,cleanup}};
    SessionProcessReapedV1 {retention:f.retention.clone(),terminal:Some(StrictJsonValue(serde_json::to_value(metadata.with_output(output,0).unwrap()).unwrap())),
        terminal_ref:None,output_replay:None,output_archive:None,component_identity:None}
}

#[tokio::test]
async fn active_turn_native_reap_uses_exact_returned_launch_without_forging_whole_turn_cleanup_or_settling_mcp() {
    for unknown in [false,true] {
        let f=live_process().await;unrelated_mcp(&f,unknown).await;let sid=&f.session.agent_session_id;
        let original_mcp=f.store.read_effect(sid,"unrelated-mcp").await.unwrap().unwrap();
        let original_head=f.store.head(sid).await.unwrap();
        let output=super::archive_tests::original_output(32);let receipt=reap(&f,&output,unknown);
        let ack=f.store.record_session_process_reaped_with_output(&owner(),receipt.clone(),&output).await.unwrap();
        assert_eq!(f.store.record_session_process_reaped_with_output(&owner(),receipt,&output).await.unwrap(),ack);
        assert_eq!(f.store.read_effect(sid,"unrelated-mcp").await.unwrap().unwrap(),original_mcp);
        let head=f.store.head(sid).await.unwrap();assert_eq!(head.status,original_head.status);assert_eq!(head.active_turn_id,original_head.active_turn_id);
        assert!(f.store.session_process_resource_stopped(&owner(),sid,&f.retention.owner_generation,&f.retention.processes[0].process_id,"live-turn","live-start").await.unwrap());
        let facts=f.store.read_events(sid,None,500).await.unwrap();
        let reaped=facts.events.iter().find(|event|event.event_id==ack.event_id).unwrap();
        assert_eq!(reaped.causation_event_id,Some("live-returned".into()));
        assert!(!facts.events.iter().any(|event|event.kind.0==SESSION_PROCESS_OWNER_RELEASED));
        let values=f.store.resolved_event_payloads(sid,&facts.events.iter().filter(|event|event.kind.0=="runtime/progress-recorded").map(|event|event.event_id.clone()).collect::<Vec<_>>()).await.unwrap();
        assert!(values.values().all(|value|value.pointer("/event/event").and_then(Value::as_str)!=Some("host_cleanup_proven")));
        assert_eq!(f.store.confirmed_reaped_session_processes(&f.retention).await.unwrap(),BTreeSet::from([f.retention.processes[0].process_id.clone()]));
        f.store.rebuild_projections(sid).await.unwrap();
        assert!(f.store.session_process_resource_stopped(&owner(),sid,&f.retention.owner_generation,&f.retention.processes[0].process_id,"live-turn","live-start").await.unwrap());
        assert_eq!(f.store.read_effect(sid,"unrelated-mcp").await.unwrap().unwrap(),original_mcp);
        assert_eq!(facts.events.iter().filter(|event|event.kind.0=="runtime/progress-recorded").count(),3,"no second tool result or cleanup fact");
    }
}

#[tokio::test]
async fn active_turn_native_reap_rejects_foreign_identity_missing_original_binding_and_unreturned_start() {
    for changed in ["owner","caller","generation","process","pid","creator_turn","creator_call","snapshot","resource","no_frozen","missing_result","legacy_dispatch","witness_generation","witness_resource","unreturned_effect","foreign_effect","tool_call","unknown_terminal","unreaped"] {
        let f=live_process().await;let sid=&f.session.agent_session_id;
        let output=super::archive_tests::original_output(32);let mut receipt=reap(&f,&output,false);let mut caller=owner();
        match changed {
            "owner"=>receipt.retention.owner_ref.principal_id="foreign-owner".into(),
            "caller"=>caller.principal_id="foreign-owner".into(),
            "generation"=>receipt.retention.owner_generation=Uuid::now_v7().to_string(),
            "process"=>receipt.retention.processes[0].process_id=Uuid::now_v7().to_string(),
            "pid"=>receipt.retention.processes[0].pid+=1,
            "creator_turn"=>receipt.retention.processes[0].creator_turn_id="another-turn".into(),
            "creator_call"=>receipt.retention.processes[0].creator_call_id="another-call".into(),
            "snapshot"=>receipt.retention.processes[0].resolved_snapshot_ref.snapshot_digest=digest('d'),
            "resource"=>receipt.retention.processes[0].resource_binding_digest=digest('d'),
            "missing_result"=>{sqlx::query("DELETE FROM agent_events WHERE session_id=? AND event_id='live-returned'").bind(sid.as_ref()).execute(f.store.test_pool()).await.unwrap();},
            "legacy_dispatch"=>{sqlx::query("UPDATE agent_events SET inline_json=json_remove(inline_json,'$.event.process_owner') WHERE session_id=? AND event_id='live-native-dispatch'").bind(sid.as_ref()).execute(f.store.test_pool()).await.unwrap();},
            "witness_generation"=>{sqlx::query("UPDATE agent_events SET inline_json=json_set(inline_json,'$.event.process_owner.owner_generation',?) WHERE session_id=? AND event_id='live-native-dispatch'").bind(Uuid::now_v7().to_string()).bind(sid.as_ref()).execute(f.store.test_pool()).await.unwrap();},
            "witness_resource"=>{sqlx::query("UPDATE agent_events SET inline_json=json_set(inline_json,'$.event.process_owner.resource_binding_digest',?) WHERE session_id=? AND event_id='live-native-dispatch'").bind(digest('d').as_ref()).bind(sid.as_ref()).execute(f.store.test_pool()).await.unwrap();},
            "unreturned_effect"=>{sqlx::query("UPDATE agent_effects SET state='unknown' WHERE session_id=? AND effect_id='live-start-effect'").bind(sid.as_ref()).execute(f.store.test_pool()).await.unwrap();},
            "foreign_effect"=>{sqlx::query("UPDATE agent_events SET inline_json=json_set(inline_json,'$.result.process_id',?) WHERE session_id=? AND event_id='live-effect-returned'").bind(Uuid::now_v7().to_string()).bind(sid.as_ref()).execute(f.store.test_pool()).await.unwrap();},
            "tool_call"=>{sqlx::query("UPDATE agent_events SET inline_json=json_set(inline_json,'$.call_id','foreign-call') WHERE session_id=? AND event_id='live-tool'").bind(sid.as_ref()).execute(f.store.test_pool()).await.unwrap();},
            "unknown_terminal"=>{let body=&mut receipt.terminal.as_mut().unwrap().0;body.as_object_mut().unwrap().remove("exit_code");body.as_object_mut().unwrap().remove("signal");body["state"]=json!("lost");body["pid"]=json!(100);},
            "unreaped"=>receipt.terminal.as_mut().unwrap().0["cleanup"]["reaped"]=json!(false),
            _=>{},
        }
        let before=f.store.current_cursor(sid).await.unwrap();
        let result=if changed=="no_frozen" {f.store.record_session_process_reaped(&caller,receipt).await}
            else {f.store.record_session_process_reaped_with_output(&caller,receipt,&output).await};
        assert!(result.is_err(),"{changed}");assert_eq!(f.store.current_cursor(sid).await.unwrap(),before,"{changed}");
        let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='session/process-reaped'").bind(sid.as_ref()).fetch_one(f.store.test_pool()).await.unwrap();
        assert_eq!(count,0,"{changed}");
    }
}
