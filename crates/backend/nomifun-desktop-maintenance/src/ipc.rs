//! Private, bounded IPC used only for short-lived application maintenance.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{fs::{self, File, OpenOptions}, io::{BufRead, BufReader, Read, Write}, net::TcpStream, path::{Path, PathBuf}};

const IPC_LIMIT: u64 = 32768;
#[derive(Clone, Copy)]
enum RuntimeKind { Application, Device }

fn runtime_path(base: &Path, platform: &str, kind: RuntimeKind) -> PathBuf {
    let leaf = match (platform, kind) {
        ("macos", RuntimeKind::Application) => "NomiFun/app-maintenance",
        ("macos", RuntimeKind::Device) => "NomiFun/external-keyboard",
        // Released Windows installers and Linux package hooks coordinate here.
        // Preserve their existing endpoint/lock identity, not a second ledger.
        _ => "NomiFun/keyboard-service",
    };
    base.join(leaf)
}

/// Current application maintenance. On macOS no historical keyboard state is
/// created, queried or migrated; old state lives in a separate directory.
pub fn runtime_dir() -> Result<PathBuf> { runtime_directory(RuntimeKind::Application) }

/// Optional serial admission, separate from modern macOS application lifetime.
pub fn device_runtime_dir() -> Result<PathBuf> { runtime_directory(RuntimeKind::Device) }

fn runtime_directory(kind: RuntimeKind) -> Result<PathBuf> {
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .context("LOCALAPPDATA unavailable")?;
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME unavailable")?
        .join("Library/Application Support");
    #[cfg(target_os = "linux")]
    let base = if let Some(root) = std::env::var_os("XDG_RUNTIME_DIR") {
        PathBuf::from(root)
    } else {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .context("HOME unavailable")?
            .join(".local/state")
    };
    let dir = runtime_path(&base, std::env::consts::OS, kind);
    fs::create_dir_all(&dir)?;
    ensure!(
        !fs::symlink_metadata(&dir)?.file_type().is_symlink(),
        "runtime directory is a symlink"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        ensure!(
            fs::metadata(&dir)?.uid() == unsafe { libc_getuid() },
            "runtime directory owner mismatch"
        );
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(dir)
}
#[cfg(unix)]
unsafe extern "C" {
    #[link_name = "getuid"]
    fn libc_getuid() -> u32;
}
pub(crate) fn private_create(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).read(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}
pub(crate) fn send_line<T: Serialize>(stream: &mut TcpStream, value: &T) -> Result<()> {
    let mut bytes = serde_json::to_vec(value)?;
    ensure!(bytes.len() < IPC_LIMIT as usize, "IPC message too large");
    bytes.push(b'\n');
    stream.write_all(&bytes)?;
    Ok(())
}
pub(crate) fn read_line<T: for<'de> Deserialize<'de>>(
    reader: &mut BufReader<TcpStream>,
) -> Result<T> {
    let mut line = String::new();
    let n = reader.take(IPC_LIMIT + 1).read_line(&mut line)?;
    ensure!(
        n > 0 && n as u64 <= IPC_LIMIT && line.ends_with('\n'),
        "IPC closed or message limit exceeded"
    );
    Ok(serde_json::from_str(&line)?)
}
pub(crate) fn token_equal(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes().zip(b.bytes()).fold(0u8, |v, (a, b)| v | (a ^ b)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn macos_runtime_roles_are_distinct_from_legacy_and_published_other_platforms_stay_stable() {
        let base = Path::new("/private-fixture");
        assert_eq!(runtime_path(base, "macos", RuntimeKind::Application), base.join("NomiFun/app-maintenance"));
        assert_eq!(runtime_path(base, "macos", RuntimeKind::Device), base.join("NomiFun/external-keyboard"));
        for platform in ["windows", "linux"] {
            for kind in [RuntimeKind::Application, RuntimeKind::Device] {
                assert_eq!(runtime_path(base, platform, kind), base.join("NomiFun/keyboard-service"));
            }
        }
    }

    #[test]
    fn macos_application_admission_and_stop_ignore_busy_malformed_legacy_runtime() {
        use fs2::FileExt;
        use crate::maintenance::{self, MaintenanceGuard, ResumeIntent};
        let root = tempfile::tempdir().unwrap();
        let legacy = root.path().join("NomiFun/keyboard-service");
        let modern = runtime_path(root.path(), "macos", RuntimeKind::Application);
        fs::create_dir_all(&legacy).unwrap(); fs::create_dir_all(&modern).unwrap();
        let mut old_locks = Vec::new();
        for name in ["maintenance.lock", "desktop.lock", "owner.lock"] {
            let lock = File::options().create(true).truncate(false).read(true).write(true).open(legacy.join(name)).unwrap();
            FileExt::try_lock_exclusive(&lock).unwrap(); old_locks.push(lock);
        }
        for name in ["desktop.json", "owner.json"] { fs::write(legacy.join(name), b"invalid old runtime data").unwrap(); }
        let admission = maintenance::admit_startup(&modern).unwrap();
        drop(admission);
        let mut intent = ResumeIntent::default();
        let guard = MaintenanceGuard::prepare_application_with_intent_checked_in(
            modern.clone(), std::time::Duration::from_millis(50), &mut intent, |_| Ok(()),
        ).unwrap();
        assert_eq!(intent, ResumeIntent::default());
        assert!(!modern.join("owner.lock").exists());
        assert!(!modern.join("owner.json").exists());
        for name in ["desktop.json", "owner.json"] { assert_eq!(fs::read(legacy.join(name)).unwrap(), b"invalid old runtime data"); }
        drop(guard); drop(old_locks);
    }
}
