use super::*;
use serde_json::json;
use super::super::engine_journal::{test_fixture,test_fixture_for_operation};
const OWNER:&str="0190f5fe-7c00-7a00-8000-000000000001";
const SESSION:&str="0190f5fe-7c00-7a00-8000-000000000002";

fn authority()->ProcessAuthority {ProcessAuthority {
    snapshot:nomifun_agent_contracts::ResolvedSnapshotRef {snapshot_id:"snapshot".into(),snapshot_digest:"b".repeat(64).into()},
    actions:["start","poll","input","close_stdin","resize","cancel"].into_iter().map(|action|format!("workspace.process/{action}").into()).collect(),resources:Vec::new(),
}}

fn waiting_input()->serde_json::Value {
    #[cfg(windows)] {json!({"operation":"start","command":"powershell.exe","args":["-NoProfile","-NonInteractive","-Command","[Console]::Write('ready'); [Console]::ReadLine() | Out-Null; [Console]::Write('done')"],"wait_ms":0})}
    #[cfg(not(windows))] {json!({"operation":"start","cmd":"printf ready; IFS= read -r answer; printf done","wait_ms":0})}
}

fn waiting_file()->serde_json::Value {
    #[cfg(windows)] {json!({"operation":"start","command":"powershell.exe","args":["-NoProfile","-NonInteractive","-Command","[Console]::Write('before'); Set-Content -LiteralPath 'started-original' -Value 'ready'; while (!(Test-Path -LiteralPath 'release-original')) { Start-Sleep -Milliseconds 10 }; [Console]::Write('after')"],"wait_ms":0})}
    #[cfg(not(windows))] {json!({"operation":"start","cmd":"printf before; printf ready > started-original; while [ ! -f release-original ]; do sleep .01; done; printf after","wait_ms":0})}
}

async fn formal_launch(scope:&EngineProcessScope,journal:&EngineTurnJournal,input:serde_json::Value)->StrictJsonValue {
    journal.append(json!({"event":"host_tool_dispatch","dispatch":{"operation_id":"original-launch","call_id":"original-call",
        "capability_id":"workspace.process","action_id":"workspace.process/start","model_name":"start_process"}}).to_string(),None,EngineJournalWrite::Progress).await.unwrap();
    let output=scope.invoke_for_turn(StrictJsonValue(input),"original-launch",Some("turn")).await.unwrap();
    journal.append(json!({"event":"host_tool_settled","operation_id":"original-launch","call_id":"original-call","error":null,
        "result":nomifun_engine_core::EngineToolResult::text("original-call".into(),output.0.to_string(),false)}).to_string(),None,EngineJournalWrite::Settlement).await.unwrap();
    output
}

async fn finish(scope:&EngineProcessScope,journal:&EngineTurnJournal) {
    let retained=scope.retention().await.unwrap();
    journal.append(json!({"event":"host_cleanup_proven","session_process_retention":retained}).to_string(),None,EngineJournalWrite::Cleanup).await.unwrap();
    journal.append(serde_json::to_string(&nomifun_agent_runtime::AgentEngineEvent::TurnCompleted {model_steps:0,
        finish_reason:nomifun_chat_model_broker::ChatFinishReason::Completed}).unwrap(),None,EngineJournalWrite::Terminal).await.unwrap();
    scope.finish_turn("turn").unwrap();
}

#[tokio::test(flavor="multi_thread",worker_threads=3)]
async fn exact_process_stop_and_inventory_preempt_an_unrelated_process_long_poll() {
    let root=tempfile::tempdir().unwrap();let(journal,pool)=test_fixture().await;
    let scope=Arc::new(EngineProcessScope::new(root.path(),journal).unwrap());
    let a=scope.invoke(StrictJsonValue(waiting_input()),"process-a").await.unwrap();
    let b=scope.invoke(StrictJsonValue(waiting_input()),"process-b").await.unwrap();
    let a=a.0["process_id"].as_str().unwrap().to_owned();let b=b.0["process_id"].as_str().unwrap().to_owned();
    let waiting=scope.clone();let b_id=b.clone();
    let poll=tokio::spawn(async move {waiting.invoke(StrictJsonValue(json!({"operation":"poll","process_id":b_id,"cursor":1000000,"wait_ms":30000})),"poll-b").await});
    tokio::time::timeout(Duration::from_secs(5),async {while scope.state.try_lock().is_ok(){tokio::task::yield_now().await;}}).await.unwrap();
    let before=std::time::Instant::now();let instances=scope.instance_observations().unwrap();
    let target=instances.iter().find(|resource|resource.process_id==a).unwrap();
    assert!(scope.stop_instance("stale-generation",&a,&target.creator_turn_id,&target.creator_call_id).is_err());
    assert!(scope.stop_instance(&target.owner_generation,&a,&target.creator_turn_id,"wrong-creator").is_err());
    assert!(scope.stop_instance(&target.owner_generation,&a,&target.creator_turn_id,&target.creator_call_id).unwrap().accepted);
    assert!(before.elapsed()<Duration::from_secs(1),"Stop cannot wait for B's scope state lock");
    let (a_control,b_control)={let controls=scope.controls.lock().unwrap();(controls[&a].clone(),controls[&b].clone())};
    tokio::time::timeout(Duration::from_secs(10),async {let mut changes=a_control.changes.subscribe();loop {
        if a_control.reaped.load(Ordering::Acquire){break;}changes.changed().await.unwrap();
    }}).await.unwrap();
    assert!(a_control.native.terminal_if_ready().unwrap().cleanup_confirmed,"actual original native cleanup, not Runtime kill");
    assert!(b_control.native.terminal_if_ready().is_none(),"legitimate sibling remains alive");
    assert!(!poll.is_finished());
    scope.finish_turn("fixture-turn").unwrap();poll.await.unwrap().unwrap();
    scope.cleanup().await.unwrap();drop(scope);pool.close().await;
}

#[tokio::test(flavor="multi_thread",worker_threads=3)]
async fn naturally_completed_process_retires_without_another_command_and_new_runtime_reads_original_cursor_output() {
    let root=tempfile::tempdir().unwrap();let(journal,pool)=test_fixture().await;
    let scope=Arc::new(EngineProcessScope::new_session(root.path(),OWNER,SESSION).unwrap());
    let registry=Arc::new(ProcessScopeRegistry::new(HashMap::new()));registry.lock().unwrap().insert((OWNER.into(),SESSION.into()),scope.clone());
    scope.bind_owner(&registry);scope.open_turn("turn",journal.clone(),Some(authority())).unwrap();
    let output=formal_launch(&scope,&journal,waiting_file()).await;let id=output.0["process_id"].as_str().unwrap().to_owned();
    finish(&scope,&journal).await;
    assert!(!registry.lock().unwrap().is_empty(),"live background work cannot gate or be killed by Turn completion");
    std::fs::write(root.path().join("release-original"),b"go").unwrap();
    tokio::time::timeout(Duration::from_secs(15),async {while !registry.lock().unwrap().is_empty(){tokio::time::sleep(Duration::from_millis(10)).await;}}).await.unwrap();
    let store=journal.canonical_store();assert!(store.unreleased_session_process_generations(&nomifun_agent_contracts::PrincipalRef {
        principal_kind:"user".into(),principal_id:OWNER.into()},&SESSION.into()).await.unwrap().is_empty());
    let(next,pool)=test_fixture_for_operation(pool,"successor","successor-input").await;
    let replacement=EngineProcessScope::new_session(root.path(),OWNER,SESSION).unwrap();replacement.open_turn("successor",next,Some(authority())).unwrap();
    let full=replacement.invoke_for_turn(StrictJsonValue(json!({"operation":"poll","process_id":id,"cursor":0})),"observe-only",Some("successor")).await.unwrap();
    assert_eq!(full.0["state"],"exited");assert_eq!(full.0["output"]["text"],"beforeafter");assert_eq!(full.0["cleanup"]["reaped"],true);
    let tail=replacement.invoke_for_turn(StrictJsonValue(json!({"operation":"poll","process_id":id,"cursor":6})),"observe-tail",Some("successor")).await.unwrap();
    assert_eq!(tail.0["output"]["text"],"after");assert_eq!(tail.0["output"]["source_encoding"],full.0["output"]["source_encoding"]);
    assert_eq!(tail.0["output"]["next_cursor"],full.0["output"]["next_cursor"]);
    assert!(replacement.state.lock().await.sessions.is_empty(),"history observation cannot create/replay old native work");
    replacement.finish_turn("successor").unwrap();replacement.cleanup().await.unwrap();drop(replacement);drop(scope);drop(journal);pool.close().await;
}

#[tokio::test(flavor="multi_thread",worker_threads=3)]
async fn native_256k_output_retires_through_canonical_chunks_and_new_runtime_replays_original_raw_cursors() {
    use base64::Engine as _;
    const RAW_BYTES:usize=256*1024;
    let root=tempfile::tempdir().unwrap();
    let script=root.path().join("original-output.mjs");
    std::fs::write(&script,r#"import fs from 'node:fs';
fs.appendFileSync('original-runs', 'once\n');
fs.writeFileSync('original-started', 'ready');
while (!fs.existsSync('release-original')) await Bun.sleep(10);
const bytes = Buffer.alloc(256 * 1024, 120);
bytes.set([0xe4, 0xb8, 0xad, 0xff, 0xe4], bytes.length - 5);
await new Promise((resolve, reject) => process.stdout.write(bytes, error => error ? reject(error) : resolve()));
"#).unwrap();
    let(journal,pool)=test_fixture().await;
    let scope=Arc::new(EngineProcessScope::new_session(root.path(),OWNER,SESSION).unwrap());
    let registry=Arc::new(ProcessScopeRegistry::new(HashMap::new()));
    registry.lock().unwrap().insert((OWNER.into(),SESSION.into()),scope.clone());
    scope.bind_owner(&registry);scope.open_turn("turn",journal.clone(),Some(authority())).unwrap();
    let launched=formal_launch(&scope,&journal,json!({"operation":"start","command":"bun","args":[script],"wait_ms":0})).await;
    let id=launched.0["process_id"].as_str().unwrap().to_owned();
    let control=scope.controls.lock().unwrap()[&id].clone();
    finish(&scope,&journal).await;
    tokio::time::timeout(Duration::from_secs(10),async {
        while !root.path().join("original-started").exists(){tokio::time::sleep(Duration::from_millis(10)).await;}
    }).await.unwrap();
    assert!(!registry.lock().unwrap().is_empty(),"the completed Turn cannot dispose the still-live native producer");
    // Retain the old entry briefly so the test observes exact native registry
    // release separately from deleting the Scope's lightweight map/cache entry.
    let held=scope.state.lock().await;
    std::fs::write(root.path().join("release-original"),b"go").unwrap();
    tokio::time::timeout(Duration::from_secs(20),async {
        while !control.receipt_committed.load(Ordering::Acquire){tokio::time::sleep(Duration::from_millis(10)).await;}
    }).await.unwrap();
    let terminal=control.native.terminal_if_ready().unwrap();
    assert!(terminal.cleanup_confirmed&&terminal.reaped,"proof comes from the original native terminal, not absent maps");
    let terminal=terminal.terminal.unwrap();let frozen=control.native.frozen_output().unwrap();
    let snapshot=frozen.snapshot_from(nomi_process_runtime::OutputCursor::START);
    let mut expected_raw=vec![120;RAW_BYTES];expected_raw[RAW_BYTES-5..].copy_from_slice(&[0xe4,0xb8,0xad,0xff,0xe4]);
    assert_eq!(snapshot.raw_bytes(),expected_raw);assert_eq!(snapshot.retained_bytes,RAW_BYTES);
    assert_eq!(snapshot.next_cursor.offset(),RAW_BYTES as u64);assert_eq!(snapshot.dropped_bytes,0);
    let original=held.sessions.get(&id).unwrap();
    let unavailable=scope.owner.write_stdin(&original.session,&[],CancellationToken::new()).await.unwrap_err();
    assert!(matches!(&unavailable,nomifun_engine_core::EngineProcessError::Process(detail) if detail.starts_with("session_not_found:")),
        "the exact original native registration was released: {unavailable}");
    // Raw output is still captured only once; no flattening or guessed ACP
    // slicing supplies the expected text at split multibyte/invalid/EOF cursors.
    let cursors=[0,RAW_BYTES as u64-5,RAW_BYTES as u64-4,RAW_BYTES as u64-1,RAW_BYTES as u64,RAW_BYTES as u64+128];
    let expected=cursors.into_iter().map(|cursor|with_component_identity(
        process_output(&id,&terminal.replay_output(&frozen,cursor).unwrap(),Operation::Poll),original.component_identity.as_ref())
        .unwrap().0).collect::<Vec<_>>();
    let store=journal.canonical_store();
    let event_id:String=sqlx::query_scalar("SELECT event_id FROM agent_events WHERE session_id=? AND kind='session/process-reaped'")
        .bind(SESSION).fetch_one(&pool).await.unwrap();
    let roots=store.resolved_event_payloads(&SESSION.into(),&[event_id.clone().into()]).await.unwrap();
    let receipt:nomifun_agent_contracts::SessionProcessReapedV1=serde_json::from_value(roots[&event_id].clone()).unwrap();
    assert!(receipt.terminal.is_none()&&receipt.output_replay.is_none(),"the root must not duplicate large stdout/decoder state");
    let archive=receipt.output_archive.unwrap();assert!(archive.terminal.0.get("output").is_none());
    assert!(archive.chunks.len()>=2,"the original numeric raw-byte array exceeds a single canonical Payload");
    let mut archived=Vec::new();
    for chunk in &archive.chunks {
        let(session,media,len,digest,body):(String,String,i64,String,Vec<u8>)=sqlx::query_as(
            "SELECT session_id,media_type,byte_len,digest,body FROM agent_payloads WHERE payload_id=?")
            .bind(chunk.as_ref()).fetch_one(&pool).await.unwrap();
        assert_eq!(session,SESSION);assert_eq!(media,"application/vnd.nomifun.native-process-output+json-fragment");
        let nomifun_agent_contracts::SessionPayloadBody::Base64(encoded)=serde_json::from_slice(&body).unwrap() else {panic!("native chunk must preserve original bytes")};
        let bytes=base64::engine::general_purpose::STANDARD.decode(encoded).unwrap();
        assert_eq!(bytes.len() as i64,len);assert!(bytes.len()<=nomifun_agent_session::MAX_SINGLE_PAYLOAD_BYTES);
        assert_eq!(nomifun_agent_contracts::digest_bytes(&bytes).as_ref(),digest);archived.extend_from_slice(&bytes);
    }
    assert_eq!(archived.len() as u64,archive.byte_len);assert_eq!(nomifun_agent_contracts::digest_bytes(&archived),archive.digest);
    assert_eq!(archived,serde_json::to_vec(&frozen).unwrap(),"canonical chunks preserve the exact original frozen stream");
    drop(held);
    tokio::time::timeout(Duration::from_secs(15),async {
        while !registry.lock().unwrap().is_empty(){tokio::time::sleep(Duration::from_millis(10)).await;}
    }).await.unwrap();
    assert!(scope.state.lock().await.sessions.is_empty());assert!(scope.controls.lock().unwrap().is_empty());
    drop(frozen);drop(terminal);drop(control);drop(scope);
    let(next,pool)=test_fixture_for_operation(pool,"large-output-successor","large-output-successor-input").await;
    let replacement=EngineProcessScope::new_session(root.path(),OWNER,SESSION).unwrap();
    replacement.open_turn("large-output-successor",next,Some(authority())).unwrap();
    for (index,cursor) in cursors.into_iter().enumerate() {
        let observed=replacement.invoke_for_turn(StrictJsonValue(json!({"operation":"poll","process_id":id,"cursor":cursor})),
            &format!("original-cursor-{index}"),Some("large-output-successor")).await.unwrap();
        assert_eq!(observed.0,expected[index],"new Runtime reads the original native cursor {cursor}");
        assert!(replacement.state.lock().await.sessions.is_empty(),"archive observation cannot recreate or consume native process slots");
    }
    assert_eq!(std::fs::read_to_string(root.path().join("original-runs")).unwrap(),"once\n","cache/history observation never reruns the source");
    replacement.finish_turn("large-output-successor").unwrap();replacement.cleanup().await.unwrap();
    drop(replacement);drop(store);drop(journal);pool.close().await;
}

#[tokio::test]
async fn empty_scope_retirement_survives_a_transient_inventory_lock_without_closing_session_admission() {
    let root=tempfile::tempdir().unwrap();let(journal,pool)=test_fixture().await;
    let scope=Arc::new(EngineProcessScope::new(root.path(),journal).unwrap());let registry=Arc::new(ProcessScopeRegistry::new(HashMap::new()));
    registry.lock().unwrap().insert((OWNER.into(),SESSION.into()),scope.clone());scope.bind_owner(&registry);scope.finish_turn("fixture-turn").unwrap();
    let held=registry.lock().unwrap();scope.retire_empty_scope();drop(held);
    tokio::time::timeout(Duration::from_secs(3),async {while !registry.lock().unwrap().is_empty(){tokio::task::yield_now().await;}}).await.unwrap();
    let replacement=EngineProcessScope::new_session(root.path(),OWNER,SESSION).unwrap();assert!(!replacement.closed.load(Ordering::Acquire));
    assert_ne!(replacement.owner_generation,scope.owner_generation);drop(replacement);drop(scope);pool.close().await;
}

#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn quit_cleanup_archives_last_terminal_and_raw_output_when_natural_observer_is_not_running() {
    let root=tempfile::tempdir().unwrap();let(journal,pool)=test_fixture().await;
    // No bind_owner: the natural observer has no runtime/cache owner and is
    // absent, matching Quit after its tracked background waiter has stopped.
    let scope=EngineProcessScope::new_session(root.path(),OWNER,SESSION).unwrap();scope.open_turn("turn",journal.clone(),Some(authority())).unwrap();
    let output=formal_launch(&scope,&journal,waiting_file()).await;let id=output.0["process_id"].as_str().unwrap().to_owned();
    finish(&scope,&journal).await;
    tokio::time::timeout(Duration::from_secs(10),async {while !root.path().join("started-original").exists(){tokio::time::sleep(Duration::from_millis(10)).await;}}).await.unwrap();
    let control=scope.controls.lock().unwrap()[&id].clone();let mut terminal=control.native.terminal_changes().unwrap();
    std::fs::write(root.path().join("release-original"),b"go").unwrap();
    tokio::time::timeout(Duration::from_secs(10),async {while control.native.terminal_if_ready().is_none(){terminal.changed().await.unwrap();}}).await.unwrap();
    scope.cleanup().await.unwrap();
    assert!(control.receipt_committed.load(Ordering::Acquire),"cleanup cannot release the final native frozen output without its receipt");
    assert!(scope.release_receipt().await.unwrap().reaped);
    let authority=authority();let principal=nomifun_agent_contracts::PrincipalRef {principal_kind:"user".into(),principal_id:OWNER.into()};
    let digest=nomifun_agent_contracts::digest_payload(&authority.resources).unwrap();
    let (poll,_)=journal.canonical_store().session_process_terminal(&principal,&SESSION.into(),&id,&authority.snapshot,&digest,
        &"workspace.process/poll".into(),6).await.unwrap().unwrap();
    let poll=serde_json::to_value(poll).unwrap();assert_eq!(poll["output"]["text"],"after");assert_eq!(poll["cleanup"]["reaped"],true);
    drop(control);drop(scope);drop(journal);pool.close().await;
}
