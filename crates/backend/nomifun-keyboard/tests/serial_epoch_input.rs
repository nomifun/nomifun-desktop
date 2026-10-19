#![cfg(target_os = "linux")]

//! Exercise the real Linux tty input queue and the production frame parser.
//! These are private PTYs, never enumerated USB devices or a user's serial port.
use nomifun_keyboard::protocol::{Parser, command};
use serialport::{ClearBuffer, SerialPort, TTYPort};
use std::{
    io::{Read, Write},
    time::{Duration, Instant},
};

fn await_input(port: &TTYPort, count: usize) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while port.bytes_to_read().unwrap() < count as u32 {
        assert!(
            Instant::now() < deadline,
            "private PTY did not receive bytes"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn clearing_native_input_removes_a_partial_old_frame_before_a_fresh_parser() {
    let (mut peer, mut host) = TTYPort::pair().unwrap();
    let old =
        command(serde_json::json!({"event":"button_down","button":"k4","padding":"x".repeat(256)}))
            .unwrap();
    let fresh_value =
        serde_json::json!({"event":"policy_applied","voice_key":"none","request_id":42});
    let fresh = command(fresh_value.clone()).unwrap();
    let prefix = &old[..7];

    // Control: a valid but incomplete previous frame consumes the new frame
    // as pending payload. Merely constructing a new Parser does not reset tty
    // bytes already received before the new transport handshake.
    peer.write_all(prefix).unwrap();
    peer.write_all(&fresh).unwrap();
    await_input(&host, prefix.len() + fresh.len());
    let mut received = vec![0; prefix.len() + fresh.len()];
    host.read_exact(&mut received).unwrap();
    let mut contaminated = Parser::default();
    assert!(contaminated.append(&received).unwrap().is_empty());
    assert!(contaminated.finish().is_err());

    // The same driver-level operation used on a newly opened USB serial port
    // must discard that pending input while retaining subsequent fresh bytes.
    peer.write_all(prefix).unwrap();
    await_input(&host, prefix.len());
    host.clear(ClearBuffer::Input).unwrap();
    assert_eq!(host.bytes_to_read().unwrap(), 0);
    let mut parser = Parser::default();
    peer.write_all(&fresh).unwrap();
    await_input(&host, fresh.len());
    let mut received = vec![0; fresh.len()];
    host.read_exact(&mut received).unwrap();
    assert_eq!(parser.append(&received).unwrap(), vec![fresh_value]);
    assert_eq!(parser.discarded, 0);
    parser.finish().unwrap();
}
