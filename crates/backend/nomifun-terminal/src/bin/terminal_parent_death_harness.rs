#[cfg(unix)]
use std::collections::HashMap;
#[cfg(unix)]
use std::path::Path;
#[cfg(unix)]
use std::time::{Duration, Instant};

#[cfg(unix)]
use nomifun_terminal::pty::{PtyHandle, SpawnParams};

#[cfg(unix)]
#[tokio::main]
async fn main() {
    let mut args = std::env::args_os().skip(1);
    let leader_marker = args.next().expect("leader marker argument");
    let grandchild_marker = args.next().expect("grandchild marker argument");
    assert!(args.next().is_none(), "unexpected harness argument");

    let quote = |path: &Path| {
        format!(
            "'{}'",
            path.to_string_lossy().replace('\'', "'\"'\"'")
        )
    };
    let script = format!(
        "echo $$ > {}; sleep 60 & echo $! > {}; wait",
        quote(Path::new(&leader_marker)),
        quote(Path::new(&grandchild_marker))
    );
    let handle = PtyHandle::spawn(
        SpawnParams {
            program: "/bin/sh".to_owned(),
            args: vec!["-c".to_owned(), script],
            cwd: String::new(),
            env: HashMap::new(),
            cols: 80,
            rows: 24,
        },
        1,
        |_chunk| {},
        |_exit, _scrollback| {},
    )
    .await
    .expect("terminal PTY should start");
    handle.activate();

    let deadline = Instant::now() + Duration::from_secs(5);
    while !(Path::new(&leader_marker).is_file() && Path::new(&grandchild_marker).is_file()) {
        assert!(
            Instant::now() < deadline,
            "PTY helper did not publish its process identities"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    // Model a backend crash: skip Rust destructors and every normal terminal
    // cleanup path. The process-runtime watchdog is the only remaining owner.
    unsafe { libc::_exit(0) }
}

#[cfg(not(unix))]
fn main() {}
