use super::*;
use nomifun_engine_core::{EngineCleanupReport,EngineProcessOutputReplay,EngineProcessTerminalMetadata};
use nomifun_agent_contracts::SessionEventAck;
use serde_json::Value;

// A deterministic storage/wire fixture, not a fabricated local native reap.
// Native producer/reap ownership is separately exercised by Scope/Supervisor.
pub(super) fn original_output(size:usize)->EngineProcessOutputReplay {
    let mut decoder=json!({"utf8_pending":[],"decode_errors":0,"source_encoding":"utf-8","observed_bytes":false,"saw_non_ascii_utf8":false});
    #[cfg(windows)] {decoder["active_code_page"]=json!(936);decoder["windows"]=Value::Null;}
    let base=json!({"stdout":decoder,"stderr":decoder,"pty":decoder});
    let mut end=base.clone();end["stdout"]["observed_bytes"]=json!(true);
    let scheme=if cfg!(windows){"windows_acp_v1"}else{"utf8_v1"};
    let mut bytes=format!("{{\"version\":1,\"decoder_scheme\":\"{scheme}\",\"state\":{{\"next_seq\":1,\"next_offset\":{size},\"base_offset\":0,\"retained\":{size},\"dropped_bytes\":0,\"chunks\":[{{\"seq\":0,\"start\":0,\"stream\":\"stdout\",\"bytes\":[");
    // Build the original wire bytes directly; a 16 MiB native ring must not
    // require sixteen million generic JSON Value allocations before archiving.
    bytes.push_str(&"120,".repeat(size-1));bytes.push_str("120]}],\"decoders\":");
    bytes.push_str(&end.to_string());bytes.push_str(",\"base_decoders\":");bytes.push_str(&base.to_string());bytes.push_str(",\"finalized\":true}}");
    serde_json::from_str(&bytes).unwrap()
}

fn archive_receipt(retention:NativeSessionProcessRetention,output:&EngineProcessOutputReplay)->nomifun_agent_contracts::SessionProcessReapedV1 {
    let terminal=EngineProcessTerminalMetadata::Exited {exit_code:Some(0),signal:None,cleanup:EngineCleanupReport {
        interrupt_attempted:false,terminate_attempted:false,force_kill_attempted:false,reaped:true,elapsed_ms:7,errors:vec![]}}.with_output(output,0).unwrap();
    nomifun_agent_contracts::SessionProcessReapedV1 {retention,terminal:Some(StrictJsonValue(serde_json::to_value(terminal).unwrap())),
        terminal_ref:None,output_replay:None,output_archive:None,component_identity:None}
}

async fn archived_root(store:&AgentSessionStore,sid:&AgentSessionId,ack:&SessionEventAck)->nomifun_agent_contracts::SessionProcessReapedV1 {
    let values=store.resolved_event_payloads(sid,&[ack.event_id.clone()]).await.unwrap();
    serde_json::from_value(values[ack.event_id.as_ref()].clone()).unwrap()
}

#[tokio::test]
async fn valid_sixteen_mib_native_ring_archives_in_original_payload_chunks_and_replays_exact_raw_cursors() {
    let(store,session,released,_)=retained_process_fixture().await;let sid=&session.agent_session_id;
    let output=original_output(16*1024*1024);let process=&released.retention.processes[0];
    let receipt=archive_receipt(released.retention.clone(),&output);
    let ack=store.record_session_process_reaped_with_output(&owner(),receipt.clone(),&output).await.unwrap();
    assert_eq!(store.record_session_process_reaped_with_output(&owner(),receipt,&output).await.unwrap(),ack);
    let root=archived_root(&store,sid,&ack).await;let archive=root.output_archive.as_ref().unwrap();
    assert!(root.terminal.is_none()&&root.output_replay.is_none());assert!(archive.byte_len>16*1024*1024);
    assert!(archive.chunks.len()>16,"the original wire bytes keep their per-Payload bound");
    let largest:i64=sqlx::query_scalar("SELECT MAX(byte_len) FROM agent_payloads WHERE session_id=?")
        .bind(sid.as_ref()).fetch_one(store.test_pool()).await.unwrap();assert!(largest<=1024*1024);
    assert!(store.unreleased_session_process_generations(&owner(),sid).await.unwrap().is_empty());
    let cursor=16*1024*1024-37;
    let(actual,_)=store.session_process_terminal(&owner(),sid,&process.process_id,&process.resolved_snapshot_ref,&process.resource_binding_digest,
        &"workspace.process/poll".into(),cursor).await.unwrap().unwrap();
    let expected=serde_json::from_value::<nomifun_engine_core::EngineProcessPoll>(archive_receipt(released.retention.clone(),&output).terminal.unwrap().0).unwrap()
        .replay_output(&output,cursor).unwrap();assert_eq!(actual,expected);
    store.rebuild_projections(sid).await.unwrap();
    let root_dir=tempfile::tempdir().unwrap();store.validate_workspace_relocation(root_dir.path(),root_dir.path()).await.unwrap();
    let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_payloads WHERE session_id=? AND media_type=?")
        .bind(sid.as_ref()).bind("application/vnd.nomifun.native-process-output+json-fragment").fetch_one(store.test_pool()).await.unwrap();
    assert_eq!(count as usize,archive.chunks.len());
}

#[tokio::test]
async fn native_archive_does_not_spend_model_allowance_and_public_payload_cannot_claim_native_classification() {
    let(store,session,released,_)=retained_process_fixture().await;let sid=&session.agent_session_id;
    let(_,started)=store.start_turn(sid,"session_api".into(),"budget-input".into(),"budget-turn".into(),StrictJsonValue(json!({"content":"Continue",
        "admission":{"resolved_snapshot_ref":session.agent_binding.resolved_snapshot_ref}}))).await.unwrap();let start=started.ack.unwrap().event_id;
    let lease=store.claim_native_execution(crate::NativeExecutionClaim {owner:owner(),agent_session_id:sid.clone(),operation_id:"budget-turn".into(),
        snapshot:session.agent_binding.resolved_snapshot_ref.clone(),active_set_generation:store.head(sid).await.unwrap().active_set_generation,
        holder:"archive-budget-owner".into(),expected_fence:0,checkpoint:None}).await.unwrap();
    let make_payload=|index:usize,size:usize,media:&str| {
        let body=json!({"content":"m".repeat(size)});let bytes=canonical_json_bytes(&body).unwrap();
        let payload=SessionPayloadRecord {payload_id:format!("archive-business-{index}").into(),agent_session_id:sid.clone(),
            media_type:media.into(),byte_len:bytes.len() as u64,digest:digest_bytes(&bytes),body:SessionPayloadBody::Json(StrictJsonValue(body))};
        let mut event=append(sid,&format!("archive-business-{index}"),"runtime_supervisor",&format!("archive-business-{index}"),"message/content-part",
            &format!("archive-message-{index}"),Some(start.clone()),json!({}));event.semantic_event.payload=SessionEventPayloadRef::Stored(payload.payload_id.clone());(event,payload)
    };
    for index in 0..15 {let(event,payload)=make_payload(index,1024*1024-512,"text/plain");store.append_native_observation(&lease,&event,Some(&payload)).await.unwrap();}
    let before=store.native_payload_bytes(&lease).await.unwrap();
    let output=original_output(256*1024);let receipt=archive_receipt(released.retention.clone(),&output);
    let archive_ack=store.record_session_process_reaped_with_output(&owner(),receipt,&output).await.unwrap();
    assert_eq!(store.native_payload_bytes(&lease).await.unwrap(),before);
    assert_eq!(store.inspect_latest_native_execution(&owner(),sid).await.unwrap().unwrap().session_payload_bytes,before);
    let root=archived_root(&store,sid,&archive_ack).await;
    let mut reuse=append(sid,"native-as-model","runtime_supervisor","native-as-model","message/content-part","new-message",Some(start.clone()),json!({}));
    reuse.semantic_event.payload=SessionEventPayloadRef::Stored(root.output_archive.unwrap().chunks[0].clone());
    assert!(matches!(store.append_native_observation(&lease,&reuse,None).await,Err(SessionStoreError::InvalidPayload(message)) if message.contains("native archive Payload")),
        "a business event cannot reclassify original native carrier bytes as budget-free model content");
    let(event,payload)=make_payload(16,512*1024,"text/plain");store.append_native_observation(&lease,&event,Some(&payload)).await.unwrap();
    let(event,payload)=make_payload(17,1024*1024-512,"text/plain");
    assert!(matches!(store.append_native_observation(&lease,&event,Some(&payload)).await,Err(SessionStoreError::InvalidPayload(message)) if message.contains("session payload budget")),
        "the original model allowance was not enlarged");
    for (index,media) in ["application/vnd.nomifun.native-process-output+json-fragment","application/vnd.nomifun.native-process-reaped+json"].into_iter().enumerate() {
        let(event,payload)=make_payload(20+index,1,media);assert!(store.append_native_observation(&lease,&event,Some(&payload)).await.is_err(),"only exact native receipt writer can classify archive bytes");
    }
}

#[tokio::test]
async fn original_output_chunks_fail_closed_when_missing_corrupt_foreign_or_reordered_without_reviving_native_owner() {
    for changed in ["missing","bytes","session","media","order"] {
        let(store,session,released,_)=retained_process_fixture().await;let sid=&session.agent_session_id;let output=original_output(256*1024);
        let ack=store.record_session_process_reaped_with_output(&owner(),archive_receipt(released.retention.clone(),&output),&output).await.unwrap();
        let root=archived_root(&store,sid,&ack).await;let chunk=&root.output_archive.as_ref().unwrap().chunks[0];
        match changed {
            "missing"=>{sqlx::query("DELETE FROM agent_payloads WHERE payload_id=?").bind(chunk.as_ref()).execute(store.test_pool()).await.unwrap();},
            "bytes"=>{sqlx::query("UPDATE agent_payloads SET body=? WHERE payload_id=?").bind(serde_json::to_vec(&SessionPayloadBody::Base64("AA==".into())).unwrap())
                .bind(chunk.as_ref()).execute(store.test_pool()).await.unwrap();},
            "session"=>{let(other,_)=create_ready(&store,"foreign-archive").await;sqlx::query("UPDATE agent_payloads SET session_id=? WHERE payload_id=?")
                .bind(other.agent_session_id.as_ref()).bind(chunk.as_ref()).execute(store.test_pool()).await.unwrap();},
            "media"=>{sqlx::query("UPDATE agent_payloads SET media_type='application/json' WHERE payload_id=?").bind(chunk.as_ref()).execute(store.test_pool()).await.unwrap();},
            _=>{let mut changed=root.clone();changed.output_archive.as_mut().unwrap().chunks[0]="another-payload".into();
                assert!(store.record_session_process_reaped(&owner(),changed).await.is_err(),"caller-supplied archive refs never grant writer authority");continue;},
        }
        let process=&released.retention.processes[0];
        assert!(store.session_process_terminal(&owner(),sid,&process.process_id,&process.resolved_snapshot_ref,&process.resource_binding_digest,&"workspace.process/poll".into(),0).await.is_err(),"{changed}");
        assert!(store.record_session_process_reaped_with_output(&owner(),archive_receipt(released.retention.clone(),&output),&output).await.is_err(),
            "a live owner cannot discard its last native archive witness after corrupted/missing chunks: {changed}");
        assert!(store.unreleased_session_process_generations(&owner(),sid).await.unwrap().is_empty(),"missing observation bytes cannot revive native ownership: {changed}");
        store.rebuild_projections(sid).await.unwrap();
    }
}
