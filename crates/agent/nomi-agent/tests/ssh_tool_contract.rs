//! Contract: the SSH remote tool family must delegate to `SshBackend` and never
//! touch the local filesystem or process runtime. `nomi-process-runtime`'s
//! architecture_contract only guards bash.rs/exec_command.rs/write_stdin.rs by
//! filename, so the remote path would otherwise escape that protection.
use std::fs;

#[test]
fn ssh_tools_stay_off_local_execution_primitives() {
    let src = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/ssh_tools.rs"
    ))
    .expect("read ssh_tools.rs");
    // Strip the test module so mock helpers don't trip the scan.
    let production = src
        .split("#[cfg(test)]")
        .next()
        .expect("production section");
    let compact: String = production.split_whitespace().collect();

    for forbidden in [
        "tokio::process::Command",
        "std::process::Command",
        "std::fs::",
        "tokio::fs::",
        "ProcessSupervisor",
        "Pty::spawn",
        "nomi_process_runtime",
    ] {
        let needle: String = forbidden.split_whitespace().collect();
        assert!(
            !compact.contains(&needle),
            "ssh_tools.rs production code must not use `{forbidden}` — remote tools go through SshBackend only"
        );
    }
}
