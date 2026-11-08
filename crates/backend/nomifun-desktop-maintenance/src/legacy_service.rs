//! Authenticated retirement client for an already-installed legacy keyboard owner.
//! This module never starts, listens for, or restores a service.
use crate::ipc::{runtime_dir, read_line, send_line};
use crate::identity::validate_descriptor;
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
