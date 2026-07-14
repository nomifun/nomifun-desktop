use nomi_process_runtime::{ProcessEvent, OutputBuffer, OutputCursor, OutputStream};

const MAX_DECODED_TEXT_BYTES_PER_SOURCE_BYTE: usize = 4;

fn event_seq(event: &ProcessEvent) -> u64 {
    match event {
        ProcessEvent::Output { seq, .. }
        | ProcessEvent::StateChanged { seq, .. }
        | ProcessEvent::OutputDropped { seq, .. } => *seq,
    }
}

fn dropped_delta(events: &[ProcessEvent]) -> u64 {
    events
        .iter()
        .map(|event| match event {
            ProcessEvent::OutputDropped { bytes, .. } => *bytes,
            _ => 0,
        })
        .sum()
}

fn output_encoding(events: &[ProcessEvent]) -> &nomi_process_runtime::EncodingMetadata {
    match &events[0] {
        ProcessEvent::Output { encoding, .. } => encoding,
        event => panic!("first push event was not output: {event:?}"),
    }
}

#[test]
fn preserves_cross_stream_observation_order() {
    let out = OutputBuffer::new(1024);
    out.push(OutputStream::Stdout, b"one");
    out.push(OutputStream::Stderr, b"two");

    let snapshot = out.snapshot_from(OutputCursor::START);

    assert_eq!(snapshot.chunks[0].stream, OutputStream::Stdout);
    assert_eq!(snapshot.chunks[1].stream, OutputStream::Stderr);
    assert_eq!(snapshot.raw_bytes(), b"onetwo".to_vec());
}

#[test]
fn decodes_utf8_split_across_chunks_without_replacement() {
    let out = OutputBuffer::new(1024);
    let text = "\u{4e2d}\u{6587}\u{1f642}";

    for byte in text.as_bytes() {
        out.push(OutputStream::Stdout, &[*byte]);
    }

    let snapshot = out.snapshot_from(OutputCursor::START);
    assert_eq!(snapshot.text(), text);
    assert_eq!(snapshot.encoding.source_encoding, "utf-8");
    assert_eq!(snapshot.encoding.decode_errors, 0);
}

#[test]
fn bounded_buffer_reports_exact_dropped_bytes() {
    let out = OutputBuffer::new(8);
    out.push(OutputStream::Stdout, b"123456");
    out.push(OutputStream::Stdout, b"7890");

    let snapshot = out.snapshot_from(OutputCursor::START);

    assert!(snapshot.retained_bytes <= 8);
    assert_eq!(snapshot.retained_bytes, 8);
    assert_eq!(snapshot.dropped_bytes, 2);
    assert_eq!(snapshot.raw_bytes(), b"34567890".to_vec());
}

#[test]
fn pty_stream_identity_is_not_fabricated() {
    let out = OutputBuffer::new(1024);
    out.push(OutputStream::Pty, b"merged");

    let snapshot = out.snapshot_from(OutputCursor::START);

    assert_eq!(snapshot.chunks[0].stream, OutputStream::Pty);
}

#[test]
fn invalid_bytes_are_reported_and_raw_bytes_remain_bounded() {
    let out = OutputBuffer::new(1024);
    out.push(OutputStream::Stdout, &[0xff, 0xfe]);

    let snapshot = out.snapshot_from(OutputCursor::START);

    assert!(snapshot.encoding.decode_errors > 0);
    assert_eq!(snapshot.raw_bytes(), vec![0xff, 0xfe]);
}

#[test]
fn lifetime_encoding_metadata_survives_eviction_and_current_cursor() {
    let out = OutputBuffer::new(1);
    let invalid = out.push(OutputStream::Stdout, &[0xff]);
    let invalid_encoding = output_encoding(&invalid).clone();
    assert!(invalid_encoding.decode_errors > 0);

    let clean = out.push(OutputStream::Stdout, b"x");
    assert_eq!(output_encoding(&clean).decode_errors, 0);

    let retained = out.snapshot_from(OutputCursor::START);
    let current = out.snapshot_from(OutputCursor::new(2));
    assert_eq!(retained.raw_bytes(), b"x".to_vec());
    assert!(current.chunks.is_empty());
    assert_eq!(retained.encoding, invalid_encoding);
    assert_eq!(current.encoding, invalid_encoding);
}

#[cfg(windows)]
#[test]
fn genuinely_different_observed_source_encodings_are_mixed() {
    use windows_sys::Win32::Globalization::GetACP;

    let out = OutputBuffer::new(16);
    out.push(OutputStream::Stdout, "\u{4e2d}".as_bytes());
    out.push(OutputStream::Stderr, &[0xff]);

    let snapshot = out.snapshot_from(OutputCursor::START);
    // SAFETY: GetACP has no parameters and reads the process-wide Windows setting.
    let expected = if unsafe { GetACP() } == 65001 {
        "utf-8"
    } else {
        "mixed"
    };
    assert_eq!(snapshot.encoding.source_encoding, expected);
}

#[cfg(windows)]
#[test]
fn acp_only_event_after_lifetime_mix_uses_the_active_code_page_label() {
    use windows_sys::Win32::Globalization::GetACP;

    // SAFETY: GetACP has no parameters and reads the process-wide Windows setting.
    let code_page = unsafe { GetACP() };
    if code_page == 65001 {
        return;
    }

    let out = OutputBuffer::new(16);
    out.push(OutputStream::Stdout, "\u{4e2d}".as_bytes());
    let fallback = out.push(OutputStream::Stdout, &[0xff]);
    let acp_only = out.push(OutputStream::Stdout, b"x");
    let expected = format!("windows-{code_page}");

    assert_eq!(output_encoding(&fallback).source_encoding, expected);
    assert_eq!(output_encoding(&acp_only).source_encoding, expected);
    assert_eq!(
        out.snapshot_from(OutputCursor::START)
            .encoding
            .source_encoding,
        "mixed"
    );
}

#[cfg(windows)]
#[test]
fn active_windows_code_page_decodes_split_native_text() {
    use windows_sys::Win32::Globalization::GetACP;

    // SAFETY: GetACP has no parameters and reads the process-wide Windows setting.
    let code_page = unsafe { GetACP() };
    let (bytes, expected): (&[u8], &str) = match code_page {
        932 => (&[0x82, 0xa0], "\u{3042}"),
        936 => (&[0xd6, 0xd0], "\u{4e2d}"),
        949 => (&[0xb0, 0xa1], "\u{ac00}"),
        950 => (&[0xa4, 0xa4], "\u{4e2d}"),
        1252 => (&[0x80], "\u{20ac}"),
        _ => return,
    };
    let out = OutputBuffer::new(16);

    let mut emitted = String::new();
    for byte in bytes {
        let events = out.push(OutputStream::Stdout, &[*byte]);
        if let ProcessEvent::Output { text, .. } = &events[0] {
            emitted.push_str(text);
        }
    }

    let snapshot = out.snapshot_from(OutputCursor::START);
    assert_eq!(emitted, expected);
    assert_eq!(snapshot.text(), expected);
    assert_eq!(
        snapshot.encoding.source_encoding,
        format!("windows-{code_page}")
    );
}

#[cfg(windows)]
#[test]
fn retained_active_code_page_expansion_matches_the_snapshot() {
    use windows_sys::Win32::Globalization::GetACP;

    // SAFETY: GetACP has no parameters and reads the process-wide Windows setting.
    let code_page = unsafe { GetACP() };
    let (bytes, expected): (&[u8], &str) = match code_page {
        932 => (&[0x82, 0xa0], "\u{3042}"),
        936 => (&[0xd6, 0xd0], "\u{4e2d}"),
        949 => (&[0xb0, 0xa1], "\u{ac00}"),
        950 => (&[0xa4, 0xa4], "\u{4e2d}"),
        1252 => (&[0x80], "\u{20ac}"),
        _ => return,
    };
    let out = OutputBuffer::new(bytes.len());

    let events = out.push(OutputStream::Stdout, bytes);
    let ProcessEvent::Output {
        bytes: event_bytes,
        text,
        ..
    } = &events[0]
    else {
        panic!("first push event was not output");
    };
    let snapshot = out.snapshot_from(OutputCursor::START);

    assert_eq!(event_bytes, bytes);
    assert_eq!(text, expected);
    assert_eq!(snapshot.text(), expected);
    assert_eq!(snapshot.dropped_bytes, 0);
    assert!(text.len() > bytes.len());
    assert!(
        text.len()
            <= bytes
                .len()
                .saturating_mul(MAX_DECODED_TEXT_BYTES_PER_SOURCE_BYTE)
    );
}

#[test]
fn event_sequences_and_absolute_byte_offsets_are_monotonic() {
    let out = OutputBuffer::new(4);
    let mut events = out.push(OutputStream::Stdout, b"ab");
    events.extend(out.push(OutputStream::Stderr, b"cdef"));
    events.extend(out.push(OutputStream::Stdout, b"g"));

    let sequences: Vec<_> = events.iter().map(event_seq).collect();
    assert_eq!(sequences.len(), 5);
    assert!(
        sequences
            .windows(2)
            .all(|pair| pair[1] == pair[0] + 1)
    );
    assert!(matches!(events[2], ProcessEvent::OutputDropped { bytes: 2, .. }));
    assert!(matches!(events[4], ProcessEvent::OutputDropped { bytes: 1, .. }));

    let snapshot = out.snapshot_from(OutputCursor::START);
    let starts: Vec<_> = snapshot.chunks.iter().map(|chunk| chunk.start).collect();
    let chunk_sequences: Vec<_> = snapshot.chunks.iter().map(|chunk| chunk.seq).collect();
    assert_eq!(starts, vec![3, 6]);
    assert_eq!(chunk_sequences, vec![sequences[1], sequences[3]]);
    assert_eq!(snapshot.next_cursor.offset(), 7);
    assert_eq!(snapshot.raw_bytes(), b"defg".to_vec());
}

#[test]
fn cursor_older_than_retained_base_starts_at_the_base() {
    let out = OutputBuffer::new(5);
    out.push(OutputStream::Stdout, b"abcdefg");

    let snapshot = out.snapshot_from(OutputCursor::new(1));

    assert_eq!(snapshot.chunks.len(), 1);
    assert_eq!(snapshot.chunks[0].start, 2);
    assert_eq!(snapshot.chunks[0].bytes, b"cdefg");
    assert_eq!(snapshot.next_cursor.offset(), 7);
    assert_eq!(snapshot.dropped_bytes, 2);
}

#[test]
fn cursor_inside_a_partially_trimmed_chunk_slices_from_the_absolute_offset() {
    let out = OutputBuffer::new(6);
    out.push(OutputStream::Stdout, b"abcdef");
    out.push(OutputStream::Stderr, b"gh");

    let snapshot = out.snapshot_from(OutputCursor::new(3));

    assert_eq!(snapshot.chunks.len(), 2);
    assert_eq!(snapshot.chunks[0].start, 3);
    assert_eq!(snapshot.chunks[0].bytes, b"def");
    assert_eq!(snapshot.chunks[0].text, "def");
    assert_eq!(snapshot.chunks[1].start, 6);
    assert_eq!(snapshot.chunks[1].bytes, b"gh");
    assert_eq!(snapshot.raw_bytes(), b"defgh".to_vec());
    assert_eq!(snapshot.next_cursor.offset(), 8);
    assert_eq!(snapshot.retained_bytes, 6);
    assert_eq!(snapshot.dropped_bytes, 2);
}

#[test]
fn cursor_inside_a_multibyte_character_replays_without_a_decode_error() {
    let out = OutputBuffer::new(16);
    let encoded = "\u{4e2d}".as_bytes();
    out.push(OutputStream::Stdout, encoded);

    let snapshot = out.snapshot_from(OutputCursor::new(1));

    assert_eq!(snapshot.chunks[0].start, 1);
    assert_eq!(snapshot.raw_bytes(), encoded[1..]);
    assert_eq!(snapshot.text(), "\u{4e2d}");
    assert_eq!(snapshot.encoding.decode_errors, 0);
}

#[test]
fn cumulative_loss_is_exact_and_independent_of_snapshot_cursor() {
    let out = OutputBuffer::new(4);
    let first = out.push(OutputStream::Stdout, b"abcdef");
    let second = out.push(OutputStream::Stdout, b"gh");
    let third = out.push(OutputStream::Stdout, b"ijklm");

    assert_eq!(dropped_delta(&first), 2);
    assert_eq!(dropped_delta(&second), 2);
    assert_eq!(dropped_delta(&third), 5);
    assert_eq!(
        dropped_delta(&first) + dropped_delta(&second) + dropped_delta(&third),
        9
    );

    let from_start = out.snapshot_from(OutputCursor::START);
    let from_base = out.snapshot_from(OutputCursor::new(9));
    let from_current = out.snapshot_from(OutputCursor::new(13));
    assert_eq!(from_start.dropped_bytes, 9);
    assert_eq!(from_base.dropped_bytes, 9);
    assert_eq!(from_current.dropped_bytes, 9);
    assert_eq!(from_base.raw_bytes(), b"jklm".to_vec());
    assert!(from_current.chunks.is_empty());
}

#[test]
fn incremental_decoder_state_is_independent_per_stream() {
    let out = OutputBuffer::new(1024);
    let stdout = "\u{4e2d}".as_bytes();
    let stderr = "\u{1f642}".as_bytes();

    let stdout_pending = out.push(OutputStream::Stdout, &stdout[..1]);
    let stderr_pending = out.push(OutputStream::Stderr, &stderr[..2]);
    let stdout_complete = out.push(OutputStream::Stdout, &stdout[1..]);
    let stderr_complete = out.push(OutputStream::Stderr, &stderr[2..]);

    assert!(matches!(
        &stdout_pending[0],
        ProcessEvent::Output { text, .. } if text.is_empty()
    ));
    assert!(matches!(
        &stderr_pending[0],
        ProcessEvent::Output { text, .. } if text.is_empty()
    ));
    assert!(matches!(
        &stdout_complete[0],
        ProcessEvent::Output { text, .. } if text == "\u{4e2d}"
    ));
    assert!(matches!(
        &stderr_complete[0],
        ProcessEvent::Output { text, .. } if text == "\u{1f642}"
    ));

    let snapshot = out.snapshot_from(OutputCursor::START);
    assert_eq!(snapshot.text(), "\u{4e2d}\u{1f642}");
    assert_eq!(snapshot.encoding.decode_errors, 0);
}

#[test]
fn retained_base_keeps_decoder_state_for_a_character_spanning_eviction() {
    let out = OutputBuffer::new(1);
    let encoded = "\u{4e2d}".as_bytes();

    for byte in encoded {
        out.push(OutputStream::Stdout, &[*byte]);
    }

    let snapshot = out.snapshot_from(OutputCursor::START);
    assert_eq!(snapshot.dropped_bytes, 2);
    assert_eq!(snapshot.raw_bytes(), vec![encoded[2]]);
    assert_eq!(snapshot.text(), "\u{4e2d}");
    assert_eq!(snapshot.encoding.decode_errors, 0);
}

#[test]
fn exact_cap_boundary_retains_every_byte_without_loss() {
    let out = OutputBuffer::new(4);
    let events = out.push(OutputStream::Stdout, b"1234");

    let snapshot = out.snapshot_from(OutputCursor::START);

    assert_eq!(events.len(), 1);
    assert_eq!(snapshot.retained_bytes, 4);
    assert_eq!(snapshot.dropped_bytes, 0);
    assert_eq!(snapshot.raw_bytes(), b"1234".to_vec());
}

#[test]
fn oversized_push_only_persists_the_bounded_tail() {
    const LIMIT: usize = 4;
    let out = OutputBuffer::new(LIMIT);
    out.push(OutputStream::Stdout, b"1234");
    let mut oversized = vec![b'a'; 100_000];
    oversized[0] = 0xff;
    let tail_start = oversized.len() - LIMIT;
    oversized[tail_start..].copy_from_slice(b"tail");

    let events = out.push(OutputStream::Stderr, &oversized);
    let snapshot = out.snapshot_from(OutputCursor::START);

    let ProcessEvent::Output {
        bytes,
        text,
        encoding,
        ..
    } = &events[0]
    else {
        panic!("first push event was not output");
    };

    assert_eq!(dropped_delta(&events), 100_000);
    assert_eq!(bytes.len(), LIMIT);
    assert_eq!(bytes, b"tail");
    assert_eq!(text, "tail");
    assert!(
        text.len()
            <= LIMIT.saturating_mul(MAX_DECODED_TEXT_BYTES_PER_SOURCE_BYTE)
    );
    assert!(encoding.decode_errors > 0);
    assert_eq!(snapshot.retained_bytes, LIMIT);
    assert_eq!(
        snapshot
            .chunks
            .iter()
            .map(|chunk| chunk.bytes.len())
            .sum::<usize>(),
        LIMIT
    );
    assert_eq!(snapshot.chunks.len(), 1);
    assert_eq!(snapshot.chunks[0].start, 100_000);
    assert_eq!(snapshot.raw_bytes(), b"tail".to_vec());
    assert_eq!(snapshot.dropped_bytes, 100_000);
    assert!(snapshot.encoding.decode_errors > 0);
}

#[test]
fn zero_cap_never_persists_raw_output() {
    let out = OutputBuffer::new(0);
    let events = out.push(OutputStream::Stdout, &[0xff, b'x']);

    let snapshot = out.snapshot_from(OutputCursor::START);

    let ProcessEvent::Output { bytes, text, .. } = &events[0] else {
        panic!("first push event was not output");
    };

    assert!(bytes.is_empty());
    assert!(text.is_empty());
    assert_eq!(dropped_delta(&events), 2);
    assert_eq!(snapshot.retained_bytes, 0);
    assert_eq!(snapshot.dropped_bytes, 2);
    assert!(snapshot.encoding.decode_errors > 0);
    assert!(snapshot.chunks.is_empty());
    assert!(snapshot.raw_bytes().is_empty());
}
