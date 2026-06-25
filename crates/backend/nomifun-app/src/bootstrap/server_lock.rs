//! Exclusive per-data-dir server lock.
//!
//! Every host (the desktop shell's embedded backend, `nomifun-web`, the
//! `nomicore` bin) defaults to ONE shared data directory (see
//! [`crate::cli::default_data_dir`]). Two live backends on the same directory
//! would double-fire every cron job (each process arms its own timers from
//! the shared DB), fight over channel polling (Telegram allows a single
//! `getUpdates` consumer and the on-disk update watermark is
//! last-writer-wins, reopening the dedup window), interleave writes to the
//! rolling log file, and — worst — race the SQLite corruption-recovery path,
//! which renames a database the other process still holds open.
//!
//! So the server takes an OS-level exclusive lock on `{data_dir}/server.lock`
//! before touching the data layer and holds it for the process lifetime. A
//! second server fails fast with an actionable message instead of silently
//! corrupting shared state.
//!
//! The lock is advisory (`flock` on Unix, `LockFileEx` on Windows, via `fs2`
//! — the same dependency nomifun-db's migrate lock uses) and is released by
//! the OS when the process exits *or crashes*; a leftover `server.lock` FILE
//! is harmless and needs no staleness heuristics. Read-only companions are
//! deliberately unaffected: `nomicore doctor` opens the DB without this lock
//! (it is designed to run while the server is alive), and the `mcp-*` stdio
//! helpers never touch the data dir at all.

use std::fs::{File, OpenOptions};
use std::path::Path;

use anyhow::{Context, Result, bail};
use fs2::FileExt;

/// Lock file name under the data dir. The lock lives on the open handle, not
/// on the file's existence — the file itself is just an address.
pub const SERVER_LOCK_FILE: &str = "server.lock";

/// Sidecar naming the current holder (pid + exe), written by the winner AFTER
/// acquiring. It must be a separate, never-locked file: on Windows the
/// exclusive `LockFileEx` range makes `server.lock` itself unreadable to the
/// losing process, so a breadcrumb stored inside the lock file could never
/// reach the error message that needs it.
const SERVER_LOCK_INFO_FILE: &str = "server.lock.info";

/// Held by [`super::ServerEnvironment`] for the process lifetime; dropping it
/// (process exit) releases the lock.
#[derive(Debug)]
pub struct ServerLock {
    _file: File,
}

pub(super) fn acquire_server_lock(data_dir: &Path) -> Result<ServerLock> {
    std::fs::create_dir_all(data_dir)
        .with_context(|| format!("failed to create data dir {}", data_dir.display()))?;

    let path = data_dir.join(SERVER_LOCK_FILE);
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&path)
        .with_context(|| format!("failed to open server lock {}", path.display()))?;

    if let Err(e) = file.try_lock_exclusive() {
        // Only a CONTENDED lock means "another backend is running". Anything
        // else (filesystems without lock support — NFS sans lockd, some FUSE
        // mounts — report ENOLCK/EOPNOTSUPP) must surface as the IO error it
        // is, or the user gets sent hunting for an instance that doesn't exist.
        if e.raw_os_error() != fs2::lock_contended_error().raw_os_error() {
            return Err(anyhow::Error::new(e)
                .context(format!("failed to lock {} (filesystem without lock support?)", path.display())));
        }
        let holder = std::fs::read_to_string(data_dir.join(SERVER_LOCK_INFO_FILE)).unwrap_or_default();
        let holder = holder.trim();
        bail!(
            "data directory {} is already in use by another running NomiFun backend{} — \
             close the other instance (the desktop app, `bun run web` / `dev:webui`, or `nomicore`) \
             and retry, or point this one at its own directory via NOMIFUN_DATA_DIR / --data-dir",
            data_dir.display(),
            if holder.is_empty() {
                String::new()
            } else {
                format!(" ({holder})")
            },
        );
    }

    // Best-effort holder breadcrumb for the next contender's error message.
    // Failures here must not fail the boot — the lock is already held.
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "unknown".to_owned());
    let _ = std::fs::write(
        data_dir.join(SERVER_LOCK_INFO_FILE),
        format!("pid {} • {exe}\n", std::process::id()),
    );

    Ok(ServerLock { _file: file })
}

#[cfg(test)]
mod tests {
    use super::acquire_server_lock;

    /// Both `flock` (per open-file-description) and `LockFileEx` (per handle)
    /// conflict across two handles within one process, so this exercises the
    /// real contention path portably.
    #[test]
    fn second_lock_on_same_dir_fails_until_first_released() {
        let dir = tempfile::tempdir().expect("tempdir");

        let first = acquire_server_lock(dir.path()).expect("first lock must succeed");

        let err = acquire_server_lock(dir.path()).expect_err("second lock must fail while held");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("already in use"),
            "error should explain the conflict, got: {msg}"
        );
        assert!(
            msg.contains("NOMIFUN_DATA_DIR"),
            "error should point at the escape hatch, got: {msg}"
        );
        let pid = std::process::id().to_string();
        assert!(
            msg.contains(&pid),
            "error should name the holder via the sidecar breadcrumb, got: {msg}"
        );

        drop(first);
        let _again = acquire_server_lock(dir.path()).expect("lock must be reacquirable after release");
    }

    #[test]
    fn creates_missing_data_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let nested = dir.path().join("not-yet").join("created");
        let _lock = acquire_server_lock(&nested).expect("lock should create the data dir");
        assert!(nested.is_dir());
    }
}
