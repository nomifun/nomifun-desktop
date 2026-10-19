use super::super::{FrozenOutput, OutputBuffer};
use crate::{OutputCursor, OutputStream};

fn round_trip_all_cursors(output: &OutputBuffer) -> FrozenOutput {
    let original = output.freeze();
    original.validate().expect("original native output is valid");
    let bytes = serde_json::to_vec(&original).expect("freeze serializes without flattening");
    let restored: FrozenOutput = serde_json::from_slice(&bytes).expect("frozen output validates");
    restored.validate().unwrap();
    let end = original.snapshot_from(OutputCursor::START).next_cursor.offset();
    for offset in 0..=end + 2 {
        let cursor = OutputCursor::new(offset);
        assert_eq!(
            restored.snapshot_from(cursor),
            original.snapshot_from(cursor),
            "raw chunks, stream order, text, EOF, metadata and raw cursor at {offset}"
        );
    }
    // The persisted receipt owns only immutable bytes/decoder state; its Arc
    // is independent from the source buffer and has no native I/O capability.
    assert_eq!(serde_json::to_vec(&restored).unwrap(), bytes);
    restored
}

#[cfg(windows)]
fn set_code_page(output: &OutputBuffer, stream: OutputStream, code_page: u32) {
    let mut storage = output.inner.lock().unwrap();
    let super::super::OutputBufferState::Live(state) = &mut *storage else {
        panic!("fixture must set original native code page before freeze");
    };
    state.decoders.for_stream_mut(stream).active_code_page = code_page;
    state.base_decoders.for_stream_mut(stream).active_code_page = code_page;
}

#[test]
fn frozen_replay_preserves_evicted_multibyte_prefix_and_cross_stream_raw_cursors() {
    let output = OutputBuffer::new(5);
    output.push(OutputStream::Stdout, &[0xe4, 0xb8]);
    output.push(OutputStream::Stderr, b"x");
    output.push(OutputStream::Stdout, &[0xad, 0xe6, 0x96, 0x87]);
    let restored = round_trip_all_cursors(&output);
    drop(output);
    let replay = restored.snapshot_from(OutputCursor::START);
    assert_eq!(replay.text(), "x中文");
    assert_eq!(replay.raw_bytes(), [b'x', 0xad, 0xe6, 0x96, 0x87]);
    assert_eq!(replay.dropped_bytes, 2);
    assert_eq!(replay.retained_bytes, 5);
    assert_eq!(replay.next_cursor.offset(), 7);
    assert_eq!(replay.chunks[0].start, 2);
}

#[test]
fn frozen_replay_preserves_eof_tail_after_a_live_cursor_without_new_raw_bytes() {
    let output = OutputBuffer::new(4);
    #[cfg(windows)]
    set_code_page(&output, OutputStream::Pty, 65001);
    output.push(OutputStream::Pty, &[0xf0, 0x9f]);
    let live_cursor = output.snapshot_from(OutputCursor::START).next_cursor;
    let restored = round_trip_all_cursors(&output);
    let replay = restored.snapshot_from(live_cursor);
    assert_eq!(replay.text(), "\u{fffd}");
    assert!(replay.raw_bytes().is_empty());
    assert_eq!(replay.next_cursor, live_cursor);
    assert_eq!(replay.encoding.decode_errors, 1);
    assert_eq!(replay.chunks.len(), 1);
    assert_eq!(replay.chunks[0].stream, OutputStream::Pty);
}

#[test]
fn frozen_replay_does_not_resurrect_eof_bytes_after_zero_cap_or_other_stream_eviction() {
    for limit in [0, 1] {
        let output = OutputBuffer::new(limit);
        #[cfg(windows)]
        set_code_page(&output, OutputStream::Stdout, 65001);
        output.push(OutputStream::Stdout, &[0xe4]);
        output.push(OutputStream::Stderr, b"x");
        let restored = round_trip_all_cursors(&output);
        let replay = restored.snapshot_from(OutputCursor::START);
        assert!(!replay.chunks.iter().any(|chunk| chunk.stream == OutputStream::Stdout));
        assert_eq!(replay.text(), if limit == 0 { "" } else { "x" });
        assert_eq!(replay.encoding.decode_errors, 1);
        assert_eq!(replay.next_cursor.offset(), 2);
    }
}

#[test]
fn frozen_replay_rejects_unknown_versions_decoders_and_structural_corruption() {
    let output = OutputBuffer::new(8);
    output.push(OutputStream::Stdout, b"abc");
    let receipt = serde_json::to_value(output.freeze()).unwrap();
    let mut unknown_version = receipt.clone();
    unknown_version["version"] = 2.into();
    let mut foreign_decoder = receipt.clone();
    foreign_decoder["decoder_scheme"] = "unsupported-decoder".into();
    let mut not_eof = receipt.clone();
    not_eof["state"]["finalized"] = false.into();
    let mut corrupt_base = receipt.clone();
    corrupt_base["state"]["base_offset"] = 1.into();
    let mut cursor_gap = receipt.clone();
    cursor_gap["state"]["chunks"][0]["start"] = 1.into();
    let mut wrong_count = receipt.clone();
    wrong_count["state"]["retained"] = 2.into();
    let mut unsafe_seq = receipt.clone();
    unsafe_seq["state"]["next_seq"] = u64::MAX.into();
    let mut bad_carry = receipt.clone();
    bad_carry["state"]["base_decoders"]["stdout"]["utf8_pending"] = serde_json::json!([255]);
    let mut extra_capability = receipt;
    extra_capability["process"] = serde_json::json!({"pid": 123, "stdin": true});
    for corrupt in [unknown_version, foreign_decoder, not_eof, corrupt_base, cursor_gap,
        wrong_count, unsafe_seq, bad_carry, extra_capability] {
        assert!(serde_json::from_value::<FrozenOutput>(corrupt).is_err());
    }
}

#[cfg(windows)]
#[test]
fn frozen_replay_retains_original_cp936_carry_after_prefix_eviction() {
    let output = OutputBuffer::new(3);
    set_code_page(&output, OutputStream::Stdout, 936);
    output.push(OutputStream::Stdout, &[0x81]);
    output.push(OutputStream::Stderr, b"x");
    output.push(OutputStream::Stdout, &[0x40, b'Y', b'Z']);
    let restored = round_trip_all_cursors(&output);
    let replay = restored.snapshot_from(OutputCursor::START);
    assert_eq!(replay.raw_bytes(), [0x40, b'Y', b'Z']);
    assert_eq!(replay.text(), "丂YZ");
    assert_eq!(replay.dropped_bytes, 2);
    assert_eq!(replay.encoding.source_encoding, "mixed");
    // The evicted stderr was UTF-8; stdout must still name original ACP936,
    // independent of the machine's current ACP when this receipt is read.
    let serialized = serde_json::to_value(&restored).unwrap();
    assert_eq!(serialized["state"]["base_decoders"]["stdout"]["windows"]["pending"], serde_json::json!([129]));
    assert_eq!(serialized["state"]["base_decoders"]["stdout"]["active_code_page"], 936);
}

#[cfg(windows)]
#[test]
fn frozen_replay_preserves_mixed_utf8_and_cp936_and_dbcs_eof() {
    let output = OutputBuffer::new(3);
    set_code_page(&output, OutputStream::Stdout, 936);
    output.push(OutputStream::Stdout, "é".as_bytes());
    output.push(OutputStream::Stdout, &[0xd6, 0xd0, 0x81]);
    let restored = round_trip_all_cursors(&output);
    let replay = restored.snapshot_from(OutputCursor::START);
    assert_eq!(replay.raw_bytes(), [0xd6, 0xd0, 0x81]);
    assert_eq!(replay.text(), "中\u{fffd}");
    assert_eq!(replay.encoding.source_encoding, "mixed");
    assert_eq!(replay.dropped_bytes, 2);
    assert_eq!(restored.snapshot_from(replay.next_cursor).text(), "\u{fffd}");
}

#[cfg(windows)]
#[test]
fn frozen_replay_v1_golden_reads_original_cp936_mixed_carry_and_eof_without_wire_drift() {
    // This original decoder sequence produces all persisted edge conditions:
    // evicted UTF-8 text, mixed ACP936, an evicted DBCS lead held by the base
    // decoder, and a terminal ACP fallback/EOF tail in another stream.
    let output=OutputBuffer::new(3);
    for stream in [OutputStream::Stdout,OutputStream::Stderr,OutputStream::Pty] {set_code_page(&output,stream,936);}
    output.push(OutputStream::Stdout,"é".as_bytes());
    output.push(OutputStream::Stdout,&[0x81]);
    output.push(OutputStream::Stderr,b"x");
    output.push(OutputStream::Stdout,&[0x40]);
    output.push(OutputStream::Pty,&[0xe4]);
    let original=output.freeze();
    let golden:serde_json::Value=serde_json::from_str(include_str!("fixtures/frozen-output-v1-windows-cp936.json")).unwrap();
    assert_eq!(serde_json::to_value(&original).unwrap(),golden,"V1 serialized wire fields must remain compatible");
    let restored:FrozenOutput=serde_json::from_value(golden).unwrap();
    for cursor in 0..=8 {
        assert_eq!(restored.snapshot_from(OutputCursor::new(cursor)),original.snapshot_from(OutputCursor::new(cursor)));
    }
    let snapshot=restored.snapshot_from(OutputCursor::START);
    assert_eq!(snapshot.text(),"x丂\u{fffd}");
    assert_eq!(snapshot.raw_bytes(),[b'x',0x40,0xe4]);
    assert_eq!(snapshot.encoding.source_encoding,"mixed");
    assert_eq!(snapshot.encoding.decode_errors,3);
}

#[cfg(not(windows))]
#[test]
fn frozen_replay_v1_golden_reads_original_utf8_eviction_and_eof_without_wire_drift() {
    let output=OutputBuffer::new(3);
    output.push(OutputStream::Stdout,"中".as_bytes());
    output.push(OutputStream::Stderr,b"a");
    output.push(OutputStream::Pty,&[0xf0,0x9f]);
    let original=output.freeze();
    let golden:serde_json::Value=serde_json::from_str(include_str!("fixtures/frozen-output-v1-utf8.json")).unwrap();
    assert_eq!(serde_json::to_value(&original).unwrap(),golden,"V1 serialized wire fields must remain compatible");
    let restored:FrozenOutput=serde_json::from_value(golden).unwrap();
    for cursor in 0..=8 {
        assert_eq!(restored.snapshot_from(OutputCursor::new(cursor)),original.snapshot_from(OutputCursor::new(cursor)));
    }
    let snapshot=restored.snapshot_from(OutputCursor::START);
    assert_eq!(snapshot.text(),"a\u{fffd}");
    assert_eq!(snapshot.raw_bytes(),[b'a',0xf0,0x9f]);
    assert_eq!(snapshot.encoding.source_encoding,"utf-8");
    assert_eq!(snapshot.encoding.decode_errors,1);
}
