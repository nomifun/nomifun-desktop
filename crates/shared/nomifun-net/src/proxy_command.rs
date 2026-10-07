use std::{process::Stdio, time::Duration};

use nomi_process_runtime::ChildProcessBuilder;
use tokio::io::AsyncReadExt;

const MAX_STDOUT_BYTES: u64 = 64 * 1024;
const CLEANUP_GRACE: Duration = Duration::from_millis(250);

/// Bridge the synchronous proxy detector to the shared managed process owner.
/// A separate current-thread runtime works both inside and outside a caller's
/// Tokio runtime. No unbounded stdout reader thread or raw child is detached.
pub(super) fn command_stdout_with_timeout(
    mut command: ChildProcessBuilder,
    timeout: Duration,
) -> Option<String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    std::thread::Builder::new()
        .name("proxy-probe".into())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .ok()?;
            runtime.block_on(async move {
                let mut process = command.spawn_managed().ok()?;
                let stdout = process.stdout.take()?;
                let output = tokio::time::timeout(timeout, async {
                    let mut bytes = Vec::new();
                    stdout
                        .take(MAX_STDOUT_BYTES + 1)
                        .read_to_end(&mut bytes)
                        .await
                        .ok()?;
                    if bytes.len() > MAX_STDOUT_BYTES as usize {
                        return None;
                    }
                    let status = process.wait().await.ok()?;
                    status
                        .success()
                        .then(|| String::from_utf8_lossy(&bytes).into_owned())
                })
                .await
                .ok()
                .flatten();
                // A root exit does not prove that descendants released the pipe.
                // On timeout/failure the same managed owner performs tree cleanup;
                // cancellation leaves authority with its existing Drop relay.
                if matches!(
                    tokio::time::timeout(CLEANUP_GRACE, process.shutdown()).await,
                    Ok(Ok(()))
                ) {
                    output
                } else {
                    None
                }
            })
        })
        .ok()?
        .join()
        .ok()
        .flatten()
}
