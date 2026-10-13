#![cfg(any(target_os = "linux", target_os = "macos"))]

#[test]
fn startup_probe_keeps_workers_out_of_host_and_cleans_up_on_timeout() {
    let directory = tempfile::tempdir().expect("private harness directory");
    // The unsafe startup-only API must run in a fresh main process, never in
    // this concurrently executing Rust test harness.
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_startup_probe_harness"))
        .arg(directory.path().join("managed-probe.identity"))
        .output()
        .expect("startup harness should launch");
    assert!(output.status.success(),
        "startup harness failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stdout).contains("single-thread environment mutation passed"));
}
