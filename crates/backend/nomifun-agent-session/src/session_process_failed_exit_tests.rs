//! Physical retirement must not turn a failed command into success, or retain
//! its already-reaped native owner forever because its tool result is_error.
use super::*;

fn failed_terminal(running:&Value,state:&str)->Value {
    let text="Synthetic command failure diagnostic. ".repeat(120);
    let mut output=process_test_output();
    output["text"]=json!(text);output["retained_bytes"]=json!(text.len());output["next_cursor"]=json!(text.len());
    let mut terminal=json!({"process_id":running["process_id"],"state":state,"output":output,
        "cleanup":{"interrupt_attempted":false,"terminate_attempted":false,"force_kill_attempted":false,
            "reaped":true,"elapsed_ms":2,"errors":[]},"success":false});
    if state=="exited" {terminal["exit_code"]=json!(1);terminal["signal"]=Value::Null;}
    terminal
}

fn failed_settlement(running:Value,state:&str)->Value {
    let mut settled=process_settled("launch-original","launch-model-call",failed_terminal(&running,state));
    settled["result"]["is_error"]=json!(true);settled
}

#[tokio::test]
async fn failed_exit_shared_tool_content_proves_reap_without_rewriting_business_failure() {
    for state in ["exited","cancelled","timed_out"] {
        let(store,session,release,_)=retained_process_fixture_with_launch(|running|failed_settlement(running,state)).await;
        let sid=&session.agent_session_id;let process=&release.retention.processes[0];
        let raw:String=sqlx::query_scalar("SELECT inline_json FROM agent_events WHERE session_id=? AND event_id='retained-launch-result'")
            .bind(sid.as_ref()).fetch_one(store.test_pool()).await.unwrap();
        let raw:Value=serde_json::from_str(&raw).unwrap();
        assert!(raw.pointer("/event/result/output").unwrap().is_null());
        assert_eq!(raw["_canonical_shared_content"]["references"][0]["pointer"],"/event/result/output");
        assert!(raw["_canonical_shared_content"]["references"][0]["byte_len"].as_u64().unwrap()>1024);
        let facts=store.resolved_event_payloads(sid,&["retained-launch-result".into()]).await.unwrap();
        let native=&facts["retained-launch-result"]["event"];
        assert_eq!(native["result"]["is_error"],true);
        let result:nomifun_engine_core::EngineToolResult=serde_json::from_value(native["result"].clone()).unwrap();
        let terminal:Value=serde_json::from_str(&result.output_text()).unwrap();
        assert_eq!(terminal["state"],state);assert_eq!(terminal["success"],false);
        if state=="exited" {assert_eq!(terminal["exit_code"],1);}
        assert_eq!(store.confirmed_reaped_session_processes(&release.retention).await.unwrap(),BTreeSet::from([process.process_id.clone()]));
        let receipt=nomifun_agent_contracts::SessionProcessReapedV1 {retention:release.retention.clone(),terminal:Some(StrictJsonValue({
            let mut native=terminal.clone();native.as_object_mut().unwrap().remove("process_id");native.as_object_mut().unwrap().remove("success");native
        })),terminal_ref:None,output_replay:None,output_archive:None,component_identity:None};
        let ack=store.record_session_process_reaped(&owner(),receipt.clone()).await.unwrap();
        assert_eq!(store.record_session_process_reaped(&owner(),receipt).await.unwrap(),ack,"the original reap is idempotent");
        assert!(store.session_process_resource_stopped(&owner(),sid,&release.retention.owner_generation,&process.process_id,
            process.creator_turn_id.as_ref(),process.creator_call_id.as_ref()).await.unwrap());
        let (observed,_)=store.session_process_terminal(&owner(),sid,&process.process_id,&process.resolved_snapshot_ref,
            &process.resource_binding_digest,&"workspace.process/poll".into(),0).await.unwrap().unwrap();
        assert_eq!(serde_json::to_value(observed).unwrap(),{
            let mut expected=terminal.clone();expected.as_object_mut().unwrap().remove("process_id");expected.as_object_mut().unwrap().remove("success");expected
        },"historical read-only poll preserves the actual failed exit or cancellation");
        assert!(store.unreleased_session_process_generations(&owner(),sid).await.unwrap().is_empty());
        store.rebuild_projections(sid).await.unwrap();
        assert_eq!(store.resolved_event_payloads(sid,&["retained-launch-result".into()]).await.unwrap(),facts,
            "retirement and projection rebuilding preserve the original failure bytes and is_error flag");
        assert_eq!(store.confirmed_reaped_session_processes(&release.retention).await.unwrap(),BTreeSet::from([process.process_id.clone()]));
        assert!(store.session_process_resource_stopped(&owner(),sid,&release.retention.owner_generation,&process.process_id,
            process.creator_turn_id.as_ref(),process.creator_call_id.as_ref()).await.unwrap());
        let root=tempfile::tempdir().unwrap();store.validate_workspace_relocation(root.path(),root.path()).await.unwrap();
        assert!(!store.read_events(sid,None,500).await.unwrap().events.iter().any(|event|event.kind.0==SESSION_PROCESS_OWNER_RELEASED),
            "one process's confirmed failure cleanup never forges a whole-owner release");
        let request=ClearSessionContext {idempotency_key:"failed-process-clear".into(),reason:"the original native process was physically reaped".into()};
        let ContextClearPreparation::Ready(boundary)=store.preflight_context_clear(&owner(),sid,&request).await.unwrap() else {panic!("confirmed failure cleanup must permit ordinary context clear");};
        store.clear_context(&owner(),sid,&request,&boundary).await.unwrap();
        assert_eq!(store.resolved_event_payloads(sid,&["retained-launch-result".into()]).await.unwrap(),facts);
    }
}

#[tokio::test]
async fn failed_tool_flag_does_not_promote_owner_errors_unknown_or_nonexecuted_results_to_launch_evidence() {
    for changed in ["owner_error","call","invalid_shape","running","lost","spawn_failed","not_executed","unreaped","cleanup_error"] {
        let(store,_,release,_)=retained_process_fixture_with_launch(|running| {
            let mut terminal=failed_terminal(&running,"exited");
            match changed {
                "invalid_shape"=>terminal["invented_native_field"]=json!(true),
                "running"=>terminal=json!({"process_id":running["process_id"],"state":"running","pid":100,"output":process_test_output()}),
                "lost"=>{terminal["state"]=json!("lost");terminal["pid"]=json!(100);terminal.as_object_mut().unwrap().remove("exit_code");terminal.as_object_mut().unwrap().remove("signal");},
                "spawn_failed"=>terminal=json!({"process_id":running["process_id"],"state":"spawn_failed","code":"SPAWN_FAILED","message":"not launched"}),
                "not_executed"=>terminal=json!({"process_id":running["process_id"],"state":"not_executed","success":false}),
                "unreaped"=>terminal["cleanup"]["reaped"]=json!(false),
                "cleanup_error"=>terminal["cleanup"]["errors"]=json!(["missing native cleanup proof"]),
                _=>{},
            }
            let mut settled=process_settled("launch-original","launch-model-call",terminal);settled["result"]["is_error"]=json!(true);
            if changed=="owner_error" {settled["error"]=json!("native owner outcome unavailable");}
            if changed=="call" {settled["result"]["call_id"]=json!("another-call");}
            settled
        }).await;
        assert!(store.confirmed_reaped_session_processes(&release.retention).await.unwrap().is_empty(),"{changed}");
        assert_eq!(store.session_process_owner_retention(&owner(),&release.retention.agent_session_id,&release.retention.owner_generation).await.unwrap(),Some(release.retention.clone()),"{changed}");
    }
}
