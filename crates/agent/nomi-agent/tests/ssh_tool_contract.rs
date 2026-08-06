//! Contract: the SSH remote tool family must delegate to `SshBackend` and never
//! touch the local filesystem or process runtime. `nomi-process-runtime`'s
//! architecture_contract only guards bash.rs/exec_command.rs/write_stdin.rs by
//! filename, so the remote path would otherwise escape that protection.
use std::fs;

fn production_source(relative: &str) -> String {
    let src = fs::read_to_string(format!("{}/{relative}", env!("CARGO_MANIFEST_DIR")))
        .unwrap_or_else(|e| panic!("read {relative}: {e}"));
    // Strip the test module so mock helpers don't trip the scan, and the comments
    // so prose may keep explaining what the code deliberately does not do.
    let production = src
        .split("#[cfg(test)]")
        .next()
        .expect("production section");
    production
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .flat_map(str::split_whitespace)
        .collect()
}

#[test]
fn ssh_tools_stay_off_local_execution_primitives() {
    // The seam declaration is scanned too: it is the one file both the agent and
    // the transport crate compile against, so a "convenient" local fallback here
    // would leak into every remote session.
    for file in ["/src/ssh_tools.rs", "/src/ssh_backend.rs"] {
        let compact = production_source(file);
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
                "{file} production code must not use `{forbidden}` — remote tools go through SshBackend only"
            );
        }
    }
}

#[test]
fn the_ssh_seam_names_no_backend_type() {
    // `nomifun-ssh` implements this seam and `nomifun-ai-agent` re-exports it.
    // The moment the seam mentions a backend type, that dependency edge becomes a
    // cycle — so the lease/binding types must stay expressed in std + async_trait.
    let compact = production_source("/src/ssh_backend.rs");
    for forbidden in ["nomifun_", "nomifun-", "SshLinkKey", "SshHostId"] {
        assert!(
            !compact.contains(forbidden),
            "ssh_backend.rs must not name `{forbidden}`: the seam is what breaks the dependency cycle"
        );
    }
}
