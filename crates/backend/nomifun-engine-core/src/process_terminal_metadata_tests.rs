use super::*;
use serde_json::json;

fn replay() -> EngineProcessOutputReplay {
    #[cfg(windows)]
    let fixture = include_str!("../../../shared/nomi-process-runtime/src/io/frozen_output/fixtures/frozen-output-v1-windows-cp936.json");
    #[cfg(not(windows))]
    let fixture = include_str!("../../../shared/nomi-process-runtime/src/io/frozen_output/fixtures/frozen-output-v1-utf8.json");
    serde_json::from_str(fixture).expect("the original fixed V1 native output fixture is valid")
}

fn cleanup() -> EngineCleanupReport {
    EngineCleanupReport { interrupt_attempted: true, terminate_attempted: true, force_kill_attempted: false,
        reaped: true, elapsed_ms: 27, errors: vec![] }
}

fn terminals(output: EngineProcessOutput) -> [EngineProcessPoll; 3] {
    [EngineProcessPoll::Exited { exit_code: Some(17), signal: Some(2), output: output.clone(), cleanup: cleanup() },
        EngineProcessPoll::Cancelled { output: output.clone(), cleanup: cleanup() },
        EngineProcessPoll::TimedOut { output, cleanup: cleanup() }]
}

#[test]
fn terminal_metadata_preserves_original_native_status_without_a_duplicate_output_payload() {
    let frozen = replay();
    let output = frozen_output_at(&frozen, 0).unwrap();
    for terminal in terminals(output) {
        let metadata = terminal.terminal_metadata().unwrap();
        assert!(metadata.is_confirmed_reaped());
        let value = serde_json::to_value(&metadata).unwrap();
        assert!(value.get("output").is_none()); assert!(value.get("pid").is_none());
        assert_eq!(value["cleanup"], serde_json::to_value(cleanup()).unwrap());
        let restored: EngineProcessTerminalMetadata = serde_json::from_value(value).unwrap();
        assert_eq!(restored, metadata);
        assert_eq!(restored.with_output(&frozen, 0).unwrap(), terminal);
    }
}

#[test]
fn terminal_metadata_replays_original_raw_cursor_and_decoder_carry_after_prefix_eviction() {
    let frozen = replay();
    let snapshot = frozen.snapshot_from(OutputCursor::START);
    assert_eq!(snapshot.next_cursor.offset(), 6); assert_eq!(snapshot.retained_bytes, 3); assert_eq!(snapshot.dropped_bytes, 3);
    for terminal in terminals(convert_output(snapshot)) {
        let metadata = terminal.terminal_metadata().unwrap();
        for cursor in 0..=8 {
            let observed = metadata.with_output(&frozen, cursor).unwrap();
            assert_eq!(observed, terminal.replay_output(&frozen, cursor).unwrap());
            let output = match observed { EngineProcessPoll::Exited { output, .. } | EngineProcessPoll::Cancelled { output, .. }
                | EngineProcessPoll::TimedOut { output, .. } => output, _ => unreachable!() };
            let native = frozen.snapshot_from(OutputCursor::new(cursor));
            assert_eq!(output.text, native.text()); assert_eq!(output.next_cursor, 6);
            assert_eq!(output.retained_bytes, 3); assert_eq!(output.dropped_bytes, 3);
            assert_eq!(output.source_encoding, native.encoding.source_encoding); assert_eq!(output.decode_errors, native.encoding.decode_errors);
        }
    }
}

#[test]
fn terminal_metadata_cannot_upgrade_unknown_or_failed_native_cleanup() {
    let frozen = replay(); let output = frozen_output_at(&frozen, 0).unwrap();
    let mut unconfirmed = cleanup(); unconfirmed.reaped = false;
    let mut failed = cleanup(); failed.errors.push("original native close failed".into());
    for cleanup in [unconfirmed, failed] {
        assert!(!cleanup.is_confirmed_reaped());
        let metadata = EngineProcessTerminalMetadata::Cancelled { cleanup: cleanup.clone() };
        assert!(!metadata.is_confirmed_reaped()); assert!(metadata.with_output(&frozen, 0).is_err());
        for terminal in [EngineProcessPoll::Exited { exit_code: Some(0), signal: None, output: output.clone(), cleanup: cleanup.clone() },
            EngineProcessPoll::Cancelled { output: output.clone(), cleanup: cleanup.clone() },
            EngineProcessPoll::TimedOut { output: output.clone(), cleanup }] {
            assert!(terminal.terminal_metadata().is_none());
        }
    }
    for terminal in [EngineProcessPoll::Running { pid: 123, output: output.clone() },
        EngineProcessPoll::Lost { pid: 123, output, cleanup: cleanup() },
        EngineProcessPoll::SpawnFailed { code: "spawn_failed".into(), message: "not started".into() }] {
        assert!(terminal.terminal_metadata().is_none(), "a non-retirable original observation stays distinct");
    }
}

#[test]
fn terminal_metadata_wire_rejects_other_states_output_mirrors_and_unknown_cleanup_fields() {
    let metadata = EngineProcessTerminalMetadata::Exited { exit_code: Some(0), signal: None, cleanup: cleanup() };
    let value = serde_json::to_value(metadata).unwrap();
    let mut output = value.clone(); output["output"] = json!({"text":"must remain in FrozenOutput"});
    let mut pid = value.clone(); pid["pid"] = json!(123);
    let mut unknown_cleanup = value.clone(); unknown_cleanup["cleanup"]["extra"] = json!(true);
    let mut lost = value.clone(); lost["state"] = json!("lost");
    let mut running = value; running["state"] = json!("running");
    for invalid in [output, pid, unknown_cleanup, lost, running] {
        assert!(serde_json::from_value::<EngineProcessTerminalMetadata>(invalid).is_err());
    }
}
