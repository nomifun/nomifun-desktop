//! Regression test for the Windows terminal spawn bug (os error 193).
//!
//! npm-installed CLIs expose an extension-less shell shim (e.g. `claude`)
//! alongside `claude.cmd`. portable-pty's own PATH search picked the
//! extension-less shim — a non-PE file `CreateProcessW` rejects with
//! "not a valid Win32 application" (os error 193). The terminal path must
//! resolve the bare name to its `.cmd` shim and run it under ConPTY.
#![cfg(windows)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nomifun_terminal::pty::{PtyHandle, SpawnParams};
use nomifun_terminal::types::resolve_command;

#[test]
fn resolves_and_runs_npm_style_cmd_shim() {
    let dir = tempfile::tempdir().unwrap();
    // Decoy: the extension-less shell shim npm also installs. Picking this
    // non-PE file is exactly what produced os error 193.
    std::fs::write(dir.path().join("winagent"), b"#!/bin/sh\necho DECOY_DO_NOT_RUN\n").unwrap();
    // The runnable Windows shim.
    std::fs::write(dir.path().join("winagent.cmd"), b"@echo off\r\necho SHIM_RAN_OK\r\n").unwrap();

    // Prepend the dir to PATH for the duration of this single-test binary.
    let original = std::env::var_os("PATH");
    let mut entries = vec![dir.path().to_path_buf()];
    if let Some(orig) = &original {
        entries.extend(std::env::split_paths(orig));
    }
    let joined = std::env::join_paths(&entries).unwrap();
    // SAFETY: this integration test binary runs a single test; nothing else
    // reads PATH concurrently.
    unsafe { std::env::set_var("PATH", &joined) };

    // 1. Resolution must pick the `.cmd` shim, not the extension-less decoy.
    let (program, _args) = resolve_command("winagent", &[]);
    assert!(
        program.to_ascii_lowercase().ends_with("winagent.cmd"),
        "expected the .cmd shim, got {program}"
    );

    // 2. The resolved shim must actually run under ConPTY (no os error 193).
    let out = Arc::new(Mutex::new(Vec::<u8>::new()));
    let exited = Arc::new(AtomicBool::new(false));
    let out_cb = out.clone();
    let exit_cb = exited.clone();
    let handle = PtyHandle::spawn(
        SpawnParams {
            program,
            args: vec![],
            cwd: dir.path().to_string_lossy().into_owned(),
            env: HashMap::new(),
            cols: 80,
            rows: 24,
        },
        0,
        move |chunk| out_cb.lock().unwrap().extend_from_slice(&chunk),
        move |_code, _sb| exit_cb.store(true, Ordering::SeqCst),
    )
    .expect("spawn must succeed (regression: os error 193)");

    let deadline = Instant::now() + Duration::from_secs(8);
    let mut ran = false;
    while Instant::now() < deadline {
        if String::from_utf8_lossy(&out.lock().unwrap()).contains("SHIM_RAN_OK") {
            ran = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    drop(handle);

    // Restore PATH before asserting.
    // SAFETY: single-test binary.
    unsafe {
        match original {
            Some(p) => std::env::set_var("PATH", p),
            None => std::env::remove_var("PATH"),
        }
    }

    let captured = String::from_utf8_lossy(&out.lock().unwrap()).into_owned();
    assert!(ran, "shim did not run; output: {captured:?}");
    assert!(
        !captured.contains("DECOY_DO_NOT_RUN"),
        "decoy ran instead of .cmd: {captured:?}"
    );
}
