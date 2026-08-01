use nomi_process_runtime::{OutputBuffer, OutputCursor, OutputStream};

const MAX_DECODED_TEXT_BYTES_PER_SOURCE_BYTE: usize = 4;

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
    out.push(OutputStream::Stdout, &[0xff]);
    let invalid_encoding = out.snapshot_from(OutputCursor::START).encoding;
    assert!(invalid_encoding.decode_errors > 0);

    out.push(OutputStream::Stdout, b"x");

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
fn same_stream_lifetime_mix_reports_a_mixed_source_encoding() {
    use windows_sys::Win32::Globalization::GetACP;

    // SAFETY: GetACP has no parameters and reads the process-wide Windows setting.
    let code_page = unsafe { GetACP() };
    if code_page == 65001 {
        return;
    }

    let out = OutputBuffer::new(16);
    out.push(OutputStream::Stdout, "\u{4e2d}".as_bytes());
    out.push(OutputStream::Stdout, &[0xff]);
    out.push(OutputStream::Stdout, b"x");

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

    for byte in bytes {
        out.push(OutputStream::Stdout, &[*byte]);
    }

    let snapshot = out.snapshot_from(OutputCursor::START);
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

    let dropped = out.push(OutputStream::Stdout, bytes);
    let snapshot = out.snapshot_from(OutputCursor::START);
    let chunk_bytes = &snapshot.chunks[0].bytes;
    let text = snapshot.text();

    assert_eq!(dropped, 0);
    assert_eq!(chunk_bytes, bytes);
    assert_eq!(text, expected);
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
fn chunk_sequences_and_absolute_byte_offsets_are_monotonic() {
    let out = OutputBuffer::new(4);
    let first_dropped = out.push(OutputStream::Stdout, b"ab");
    let second_dropped = out.push(OutputStream::Stderr, b"cdef");
    let third_dropped = out.push(OutputStream::Stdout, b"g");

    assert_eq!(first_dropped, 0);
    assert_eq!(second_dropped, 2);
    assert_eq!(third_dropped, 1);

    let snapshot = out.snapshot_from(OutputCursor::START);
    let starts: Vec<_> = snapshot.chunks.iter().map(|chunk| chunk.start).collect();
    let chunk_sequences: Vec<_> = snapshot.chunks.iter().map(|chunk| chunk.seq).collect();
    assert_eq!(starts, vec![3, 6]);
    assert_eq!(chunk_sequences, vec![1, 2]);
    assert!(
        chunk_sequences
            .windows(2)
            .all(|pair| pair[1] > pair[0])
    );
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

    assert_eq!(first, 2);
    assert_eq!(second, 2);
    assert_eq!(third, 5);
    assert_eq!(first + second + third, 9);

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

    out.push(OutputStream::Stdout, &stdout[..1]);
    out.push(OutputStream::Stderr, &stderr[..2]);
    let pending = out.snapshot_from(OutputCursor::START);
    assert_eq!(
        pending.text(),
        "",
        "partial multi-byte characters must not decode early on either stream"
    );
    assert_eq!(pending.encoding.decode_errors, 0);

    out.push(OutputStream::Stdout, &stdout[1..]);
    out.push(OutputStream::Stderr, &stderr[2..]);

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
    let dropped = out.push(OutputStream::Stdout, b"1234");

    let snapshot = out.snapshot_from(OutputCursor::START);

    assert_eq!(dropped, 0);
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

    let dropped = out.push(OutputStream::Stderr, &oversized);
    let snapshot = out.snapshot_from(OutputCursor::START);

    assert_eq!(dropped, 100_000);
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
    assert_eq!(snapshot.chunks[0].text, "tail");
    assert!(
        snapshot.chunks[0].text.len()
            <= LIMIT.saturating_mul(MAX_DECODED_TEXT_BYTES_PER_SOURCE_BYTE)
    );
    assert_eq!(snapshot.raw_bytes(), b"tail".to_vec());
    assert_eq!(snapshot.dropped_bytes, 100_000);
    assert!(snapshot.encoding.decode_errors > 0);
}

#[test]
fn zero_cap_never_persists_raw_output() {
    let out = OutputBuffer::new(0);
    let dropped = out.push(OutputStream::Stdout, &[0xff, b'x']);

    let snapshot = out.snapshot_from(OutputCursor::START);

    assert_eq!(dropped, 2);
    assert_eq!(snapshot.retained_bytes, 0);
    assert_eq!(snapshot.dropped_bytes, 2);
    assert!(snapshot.encoding.decode_errors > 0);
    assert!(snapshot.chunks.is_empty());
    assert!(snapshot.raw_bytes().is_empty());
    assert!(snapshot.text().is_empty());
}
