use super::*;
use nomi_process_runtime::{CommandSpec, ProcessPolicy, ProcessRequest, ProcessSupervisor,
    SupervisorConfig, Transport, normalize_request};

async fn original_native_receipt()->(Value,FrozenOutput) {
    let directory=tempfile::tempdir().unwrap();
    let supervisor=ProcessSupervisor::new(SupervisorConfig::default());
    let command=if cfg!(windows) {
        CommandSpec::Program {program:"powershell.exe".into(),args:vec!["-NoProfile".into(),"-NonInteractive".into(),
            "-Command".into(),"[Console]::Write('archive-proof')".into()]}
    } else {
        CommandSpec::Program {program:"/bin/sh".into(),args:vec!["-c".into(),"printf 'archive-proof'".into()]}
    };
    let request=normalize_request(ProcessRequest {owner:ProcessOwner::new(Uuid::now_v7(),Uuid::now_v7()),command,
        cwd:directory.path().to_owned(),env:Default::default(),transport:Transport::Pipe,
        policy:ProcessPolicy {output_limit_bytes:5,expire_on_idle:false,..Default::default()}},directory.path()).unwrap();
    let handle=supervisor.start(request).await.unwrap();
    let until=std::time::Instant::now()+Duration::from_secs(15);
    let outcome=loop {
        let observed=supervisor.poll(&handle.owner,&handle.session_id,OutputCursor::START,until).await.unwrap();
        if let nomi_process_runtime::PollResult::Finished(outcome)=observed {break outcome;}
        assert!(std::time::Instant::now()<until,"native fixture never completed");
    };
    let nomi_process_runtime::ProcessOutcome::Exited {code,signal,cleanup,..}=outcome else {panic!("fixture did not naturally exit")};
    assert_eq!(code,Some(0));assert!(cleanup.reaped);assert!(cleanup.errors.is_empty());
    let frozen=handle.terminal_witness().frozen_output().unwrap();
    let snapshot=frozen.snapshot_from(OutputCursor::new(10));
    let receipt=json!({"state":"exited","code":code,"signal":signal,
        "output":{"chunks":snapshot.chunks.iter().map(|chunk|json!({"text":chunk.text})).collect::<Vec<_>>(),
            "next_cursor":snapshot.next_cursor.offset(),"retained_bytes":snapshot.retained_bytes,
            "dropped_bytes":snapshot.dropped_bytes,"encoding":{"source_encoding":snapshot.encoding.source_encoding,
                "decode_errors":snapshot.encoding.decode_errors}},
        "cleanup":{"interrupt_attempted":cleanup.interrupt_attempted,"terminate_attempted":cleanup.terminate_attempted,
            "force_kill_attempted":cleanup.force_kill_attempted,"reaped":cleanup.reaped,
            "elapsed_ms":cleanup.elapsed.as_millis(),"errors":cleanup.errors},"output_replay":serde_json::to_value(&frozen).unwrap()});
    assert!(supervisor.shutdown().await.is_exact());
    (receipt,frozen)
}

#[tokio::test]
async fn system_archive_cache_shared_by_original_read_and_stop_capability_replays_full_raw_output() {
    let (receipt,frozen)=original_native_receipt().await;
    let original_owner=Arc::new(SystemTerminalCache::new());
    let captured_stop=original_owner.clone();
    let mut changes=captured_stop.changes.subscribe();
    assert!(original_owner.terminal().is_none());
    assert!(original_owner.frozen_output().is_none());
    let (partial,cursor)=parse_poll(&receipt,123).unwrap();
    assert_eq!(cursor,13);
    let EngineProcessPoll::Exited {output,..}=&partial else {panic!("expected original terminal")};
    assert_eq!(output.text,"oof");
    original_owner.record(&receipt,&partial).unwrap();
    assert!(changes.has_changed().unwrap());
    changes.borrow_and_update();
    drop(original_owner);
    let terminal=captured_stop.terminal().unwrap();
    let EngineProcessPoll::Exited {output,..}=&terminal else {panic!("expected original terminal")};
    assert_eq!(output.text,"proof");
    assert_eq!(output.dropped_bytes,8);
    let replay=captured_stop.frozen_output().unwrap();
    for cursor in 0..=15 {
        assert_eq!(replay.snapshot_from(OutputCursor::new(cursor)),frozen.snapshot_from(OutputCursor::new(cursor)));
    }
    let saved=serde_json::to_vec(&replay).unwrap();
    drop(captured_stop);
    let restored:FrozenOutput=serde_json::from_slice(&saved).unwrap();
    assert_eq!(terminal.replay_output(&restored,10).unwrap(),partial);
}

#[tokio::test]
async fn system_archive_legacy_helpers_complete_normally_without_inventing_a_replay_witness() {
    let (mut receipt,_)=original_native_receipt().await;
    receipt.as_object_mut().unwrap().remove("output_replay");
    let (terminal,_)=parse_poll(&receipt,123).unwrap();
    let cache=SystemTerminalCache::new();
    cache.record(&receipt,&terminal).unwrap();
    let cleanup=crate::process::cancellation_receipt(cache.terminal().unwrap());
    assert!(cleanup.reaped);
    assert!(cleanup.cleanup_confirmed);
    assert!(cache.frozen_output().is_none(),"a legacy text reply cannot dispose its original cursor owner");
}

#[tokio::test]
async fn system_archive_invalid_or_mismatched_receipts_preserve_original_owner_evidence() {
    let (receipt,_)=original_native_receipt().await;
    let (terminal,_)=parse_poll(&receipt,123).unwrap();
    let cache=SystemTerminalCache::new();
    cache.record(&receipt,&terminal).unwrap();
    let original=cache.terminal().unwrap();
    let mut invalid_version=receipt.clone();invalid_version["output_replay"]["version"]=json!(999);
    let mut invalid_metadata=receipt.clone();invalid_metadata["output"]["retained_bytes"]=json!(4);
    let mut running=receipt.clone();running["state"]=json!("running");
    for invalid in [invalid_version,invalid_metadata,running] {
        let (poll,_)=parse_poll(&invalid,123).unwrap();
        assert!(cache.record(&invalid,&poll).is_err());
        assert_eq!(cache.terminal(),Some(original.clone()));
        assert!(cache.frozen_output().is_some());
    }
    // Raw output proves replay, never process-tree cleanup. Unknown native
    // cleanup must remain unknown even when its bytes are completely archived.
    let mut lost=receipt;
    lost["state"]=json!("lost");lost["cleanup"]["reaped"]=json!(false);
    lost["cleanup"]["errors"]=json!(["original cleanup authority unconfirmed"]);
    let (lost_terminal,_)=parse_poll(&lost,123).unwrap();
    let unresolved=SystemTerminalCache::new();
    unresolved.record(&lost,&lost_terminal).unwrap();
    let cleanup=crate::process::cancellation_receipt(unresolved.terminal().unwrap());
    assert!(!cleanup.reaped);
    assert!(!cleanup.cleanup_confirmed);
    assert!(unresolved.frozen_output().is_some());
}
