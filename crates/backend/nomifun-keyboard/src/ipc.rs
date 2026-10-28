//! Private, bounded IPC used only for short-lived application maintenance.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{fs::{self, File, OpenOptions}, io::{BufRead, BufReader, Read, Write}, net::TcpStream, path::{Path, PathBuf}};

const IPC_LIMIT: u64 = 32768;
pub fn runtime_dir() -> Result<PathBuf> {
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
    let dir = base.join("NomiFun/keyboard-service");
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
