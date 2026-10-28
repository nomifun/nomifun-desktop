//! Authenticated retirement client for an already-installed legacy keyboard owner.
//! This module never starts, listens for, or restores a service.
use crate::ipc::{runtime_dir, read_line, send_line};
use fs2::FileExt;
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{fs::File, io::BufReader, net::TcpStream, path::{Path, PathBuf}, time::Duration};

#[derive(Deserialize)]
struct Descriptor { port: u16, token: String }

/// External maintenance only: the typed guard proves the GUI process has
/// exited and prevents a replacement GUI until this management call finishes.
pub fn request_shutdown_after_desktop_exit(
    proof: &crate::desktop_control::DesktopQuiescence,
) -> Result<()> {
    let dir = runtime_dir()?;
    request_shutdown_after_desktop_exit_in(&dir, proof)
}
pub(crate) fn request_shutdown_after_desktop_exit_in(
    dir: &Path,
    proof: &crate::desktop_control::DesktopQuiescence,
) -> Result<()> {
    request_shutdown_with_proof_at(dir.join("owner.json"), Some(proof.proof_for(dir)?))
}
fn request_shutdown_with_proof_at(path: PathBuf, desktop_exit_proof: Option<&str>) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    ensure!(!path.is_symlink(), "IPC descriptor is a symlink");
    ensure!(std::fs::metadata(&path)?.len() <= 32768, "legacy owner descriptor is too large");
    let descriptor: Descriptor = serde_json::from_reader(File::open(path)?)?;
    ensure!(descriptor.port != 0 && descriptor.token.len() == 64, "invalid legacy owner descriptor");
    let mut stream = TcpStream::connect_timeout(
        &([127, 0, 0, 1], descriptor.port).into(),
        Duration::from_secs(1),
    )?;
    stream.set_read_timeout(Some(Duration::from_secs(13)))?;
    stream.set_write_timeout(Some(Duration::from_secs(1)))?;
    send_line(
        &mut stream,
        &json!({"protocol":1,"token":descriptor.token,"mode":"control","desktop_exit_proof":desktop_exit_proof}),
    )?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let accepted: Value = read_line(&mut reader).context("management authentication reply")?;
    ensure!(
        accepted["accepted"] == true,
        "management IPC authentication rejected"
    );
    send_line(&mut stream, &json!({"kind":"shutdown"})).context("management shutdown send")?;
    let accepted: Value = read_line(&mut reader).context("management final shutdown proof")?;
    ensure!(
        accepted["completed"] == true,
        "shutdown not completed: {}",
        accepted["error"]
            .as_str()
            .unwrap_or("missing capture stop proof")
    );
    Ok(())
}

/// Observe only a lifetime lock. An endpoint file alone never proves an owner.
pub fn owner_running_in(dir: &Path) -> Result<bool> {
    let path = dir.join("owner.lock");
    ensure!(!path.is_symlink(), "legacy owner lifetime lock is a symlink");
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)] {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options.open(path)?;
    match FileExt::try_lock_exclusive(&lock) {
        Ok(()) => Ok(false),
        Err(error) if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => Ok(true),
        Err(error) => Err(error.into()),
    }
}

/// Check OS-observed executable identity before sending either stop request.
/// In particular, a developer executable cannot retire an installed app's owner.
pub fn validate_runtime_ownership(installed: &Path, runtime: &Path, desktop_active: bool) -> Result<()> {
    let owner_active = owner_running_in(runtime)?;
    if !desktop_active && !owner_active { return Ok(()); }
    let expected = installed.canonicalize().context("resolve maintenance application identity")?;
    if desktop_active { validate_descriptor(&expected, &runtime.join("desktop.json"))?; }
    if owner_active {
        let owner = runtime.join("owner.json");
        if owner.exists() || owner.is_symlink() {
            validate_descriptor(&expected, &owner)?;
        } else {
            // The replacement GUI owns serial in process and publishes only a
            // desktop endpoint. A busy lock without either endpoint fails closed.
            ensure!(desktop_active, "serial owner is busy without an authenticated installation endpoint");
        }
    }
    Ok(())
}

fn validate_descriptor(expected: &Path, descriptor: &Path) -> Result<()> {
    ensure!(!descriptor.is_symlink() && std::fs::metadata(descriptor)?.len() <= 32768, "unsafe keyboard participant descriptor");
    let descriptor: Value = serde_json::from_reader(File::open(descriptor)?)?;
    let pid = descriptor["pid"].as_u64().filter(|pid| *pid > 0 && *pid <= u32::MAX as u64)
        .context("keyboard participant PID is unavailable")? as u32;
    let actual = process_image(pid)?.canonicalize().context("resolve keyboard participant executable")?;
    #[cfg(windows)]
    let matches = actual.to_string_lossy().to_lowercase() == expected.to_string_lossy().to_lowercase();
    #[cfg(not(windows))]
    let matches = actual == expected;
    ensure!(matches, "another NomiFun installation owns the keyboard participant; quit that installation before proceeding");
    Ok(())
}

#[cfg(target_os = "linux")]
fn process_image(pid: u32) -> Result<PathBuf> {
    Ok(std::fs::read_link(format!("/proc/{pid}/exe"))?)
}
#[cfg(target_os = "macos")]
fn process_image(pid: u32) -> Result<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    unsafe extern "C" { fn proc_pidpath(pid: i32, buffer: *mut std::ffi::c_void, size: u32) -> i32; }
    let mut bytes = vec![0u8; 4096];
    let length = unsafe { proc_pidpath(pid as i32, bytes.as_mut_ptr().cast(), bytes.len() as u32) };
    ensure!(length > 0, "cannot inspect keyboard participant executable: {}", std::io::Error::last_os_error());
    bytes.truncate(bytes.iter().position(|byte| *byte == 0).unwrap_or(length as usize));
    Ok(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
}
#[cfg(windows)]
fn process_image(pid: u32) -> Result<PathBuf> {
    use std::{ffi::c_void, os::windows::ffi::OsStringExt};
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn QueryFullProcessImageNameW(process: *mut c_void, flags: u32, path: *mut u16, size: *mut u32) -> i32;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }
    let handle = unsafe { OpenProcess(0x1000, 0, pid) };
    ensure!(!handle.is_null(), "cannot inspect keyboard participant process: {}", std::io::Error::last_os_error());
    let mut image = vec![0u16; 32768];
    let mut size = image.len() as u32;
    let result = unsafe { QueryFullProcessImageNameW(handle, 0, image.as_mut_ptr(), &mut size) };
    let error = std::io::Error::last_os_error();
    unsafe { CloseHandle(handle); }
    ensure!(result != 0, "cannot inspect keyboard participant executable: {error}");
    Ok(PathBuf::from(std::ffi::OsString::from_wide(&image[..size as usize])))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn participant_identity_uses_the_os_executable_path() {
        let dir = tempfile::tempdir().unwrap();
        let descriptor = dir.path().join("desktop.json");
        std::fs::write(&descriptor, serde_json::to_vec(&json!({"pid":std::process::id()})).unwrap()).unwrap();
        validate_descriptor(&std::env::current_exe().unwrap().canonicalize().unwrap(), &descriptor).unwrap();
        let other = dir.path().join("other-installation");
        std::fs::write(&other, "unrelated executable").unwrap();
        assert!(validate_descriptor(&other.canonicalize().unwrap(), &descriptor).is_err());
    }

    #[test]
    fn first_install_without_runtime_participants_needs_no_existing_executable() {
        let dir = tempfile::tempdir().unwrap();
        let installed = dir.path().join("not-installed-yet");
        validate_runtime_ownership(&installed, dir.path(), false).unwrap();
    }

    #[test]
    fn active_participant_without_executable_or_endpoint_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let installed = dir.path().join("missing-installation");
        assert!(validate_runtime_ownership(&installed, dir.path(), true).is_err());
        let owner = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(dir.path().join("owner.lock")).unwrap();
        FileExt::try_lock_exclusive(&owner).unwrap();
        assert!(validate_runtime_ownership(&installed, dir.path(), false).is_err());
        assert!(validate_runtime_ownership(&std::env::current_exe().unwrap(), dir.path(), false).is_err());
    }

    #[test]
    fn stale_owner_endpoint_does_not_authorize_a_request() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("owner.json"), "stale").unwrap();
        validate_runtime_ownership(&std::env::current_exe().unwrap(), dir.path(), false).unwrap();
        assert!(!owner_running_in(dir.path()).unwrap());
    }
}
