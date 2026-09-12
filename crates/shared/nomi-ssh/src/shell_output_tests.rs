use super::*;

fn capture(chunks: &[&[u8]]) -> String {
    let mut output = ShellOutput::default();
    for chunk in chunks {
        output.append(chunk).unwrap();
    }
    output.finish().unwrap();
    output.to_string()
}

#[test]
fn utf8_output_and_sentinel_cwd_survive_every_packet_boundary() {
    let text = "中文 café 🙂__NOMI_END_1__0__/目录\n";
    let bytes = text.as_bytes();
    for split in 0..=bytes.len() {
        let output = capture(&[&bytes[..split], &bytes[split..]]);
        assert_eq!(output, text, "split at byte {split}");
        assert_eq!(find_sentinel(&output, "__NOMI_END_1__").unwrap().2, "/目录");
    }
}

#[test]
fn malformed_and_split_utf8_match_lossy_decoding_of_the_whole_stream() {
    let bytes = [b'a', 0xff, 0xe4, 0xb8, 0xad, b'b', 0xf0, 0x9f];
    let chunks = bytes.chunks(1).collect::<Vec<_>>();
    assert_eq!(capture(&chunks), String::from_utf8_lossy(&bytes));
}

#[test]
fn a_split_character_can_finish_exactly_at_the_output_limit() {
    let first = vec![b'x'; MAX_SSH_OUTPUT_BYTES - "中".len()];
    let output = capture(&[&first, &"中".as_bytes()[..1], &"中".as_bytes()[1..]]);
    assert_eq!(output.len(), MAX_SSH_OUTPUT_BYTES);
    assert!(output.ends_with('中'));
}

#[test]
fn small_byte_streams_match_the_whole_stream_oracle_at_every_split() {
    let alphabet = [b'a', 0x80, 0xc2, 0xe4, 0xb8, 0xf0, 0x9f, 0xff];
    for length in 0..=4u32 {
        for mut index in 0..alphabet.len().pow(length) {
            let mut bytes = vec![0; length as usize];
            for byte in &mut bytes {
                *byte = alphabet[index % alphabet.len()];
                index /= alphabet.len();
            }
            let expected = String::from_utf8_lossy(&bytes);
            for split in 0..=bytes.len() {
                assert_eq!(
                    capture(&[&bytes[..split], &[], &bytes[split..]]),
                    expected,
                    "{bytes:?} at {split}"
                );
            }
            assert_eq!(capture(&bytes.chunks(1).collect::<Vec<_>>()), expected);
        }
    }
}

#[test]
fn an_incomplete_tail_is_flushed_once_and_cannot_expand_past_the_limit() {
    let mut output = ShellOutput::default();
    output.append(b"a\xf0\x9f").unwrap();
    assert_eq!(&*output, "a");
    output.finish().unwrap();
    output.finish().unwrap();
    assert_eq!(&*output, "a�");

    let mut full = ShellOutput::default();
    full.append(&vec![b'x'; MAX_SSH_OUTPUT_BYTES - 1]).unwrap();
    full.append(&[0xc2]).unwrap();
    assert_eq!(full.finish(), Err(MAX_SSH_OUTPUT_BYTES + 2));
    assert_eq!(
        full.len(),
        MAX_SSH_OUTPUT_BYTES - 1,
        "no over-limit text is published"
    );
}

#[test]
fn cleanup_removes_carriage_returns_and_only_one_final_newline() {
    assert_eq!(clean("中\r\n文\r\n\n"), "中\n文\n");
    assert_eq!(clean("plain"), "plain");
    assert_eq!(clean(""), "");
}
