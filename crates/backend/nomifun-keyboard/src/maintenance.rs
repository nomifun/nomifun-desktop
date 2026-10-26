//! Short-lived package maintenance. The installer owns this lease until all
//! executable replacement (or an untouched cancellation) has finished.
//!
//! Endpoint JSON is an address book, never evidence of a live owner. Stale
//! endpoints are reclaimed only after acquiring the corresponding lifetime lock.
use crate::{desktop_control, service};
use anyhow::{Context, Result, ensure};
use fs2::FileExt;
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
mod replacement;
pub use replacement::FileReplacement;

fn open_lock(dir: &Path, name: &str) -> Result<File> {
    let path = dir.join(name);
    ensure!(!path.is_symlink(), "maintenance lock is a symlink: {name}");
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}

fn contended(error: &std::io::Error) -> bool {
    error.raw_os_error() == fs2::lock_contended_error().raw_os_error()
}

/// Held only while a new GUI or device service acquires its own lifetime lock
/// and publishes its endpoint. Existing processes do not hold this gate: the
/// maintenance caller must be able to fence new starts before stopping them.
pub struct StartupAdmission(File);

#[derive(Debug)]
pub(crate) struct MaintenanceBusy;
impl std::fmt::Display for MaintenanceBusy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("NomiFun is being installed or updated; keyboard startup is paused")
    }
}
impl std::error::Error for MaintenanceBusy {}

pub(crate) fn admit_startup(dir: &Path) -> Result<StartupAdmission> {
    let lock = open_lock(dir, "maintenance.lock")?;
    match FileExt::try_lock_shared(&lock) {
        Ok(()) => {}
        Err(error) if contended(&error) => return Err(MaintenanceBusy.into()),
        Err(error) => return Err(error.into()),
    }
    Ok(StartupAdmission(lock))
}

/// Run before storage, single-instance setup or native windows. Retain the
/// returned guard until the GUI lifetime endpoint has been registered, then
/// release it; a shared admission must not be held for the whole GUI lifetime.
/// None means a live package maintenance lease owns startup, while filesystem
/// and permission failures remain errors.
pub fn try_admit_startup() -> Result<Option<StartupAdmission>> {
    match admit_startup(&service::runtime_dir()?) {
        Ok(admission) => Ok(Some(admission)),
        Err(error) if error.is::<MaintenanceBusy>() => Ok(None),
        Err(error) => Err(error),
    }
}

impl Drop for StartupAdmission {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

/// Capture running intent before asking any participant to stop. Preferences
/// (including enabled and launch-at-login) are never changed by maintenance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResumeIntent {
    pub desktop: bool,
    pub service: bool,
}

impl ResumeIntent {
    pub fn capture(dir: &Path) -> Result<Self> {
        fn busy(dir: &Path, name: &str) -> Result<bool> {
            let file = open_lock(dir, name)?;
            match FileExt::try_lock_exclusive(&file) {
                Ok(()) => Ok(false),
                Err(error) if contended(&error) => Ok(true),
                Err(error) => Err(error.into()),
            }
        }
        Ok(Self {
            desktop: busy(dir, "desktop.lock")?,
            service: busy(dir, "owner.lock")?,
        })
    }
}

/// Holding both lifetime locks blocks even already-released versions that do
/// not yet implement the maintenance admission gate. Drop releases OS locks;
/// it does not start a program or edit persisted user configuration.
pub struct MaintenanceGuard {
    _owner: File,
    _desktop: desktop_control::DesktopQuiescence,
    _admission: File,
    runtime: PathBuf,
}

impl MaintenanceGuard {
    pub fn prepare() -> Result<Self> {
        Self::prepare_in(service::runtime_dir()?, Duration::from_secs(30))
    }

    /// An explicit directory is useful to package validation and embedders. It
    /// must already be a private runtime directory owned by the current user.
    pub fn prepare_in(runtime: PathBuf, wait: Duration) -> Result<Self> {
        Self::prepare_with_intent_in(runtime, wait, &mut ResumeIntent::default())
    }

    /// Refresh intent after the startup gate is exclusive, including on partial
    /// preparation failure, so a just-started participant is never lost on abort.
    pub fn prepare_with_intent_in(runtime: PathBuf, wait: Duration, intent: &mut ResumeIntent) -> Result<Self> {
        Self::prepare_with_intent_checked_in(runtime, wait, intent, |_| Ok(()))
    }

    /// The package may validate native process ownership after new starts are
    /// fenced and before sending any stop request to an existing installation.
    pub fn prepare_with_intent_checked_in(
        runtime: PathBuf,
        wait: Duration,
        intent: &mut ResumeIntent,
        check: impl FnOnce(ResumeIntent) -> Result<()>,
    ) -> Result<Self> {
        ensure!(runtime.is_dir() && !runtime.is_symlink(), "invalid keyboard runtime directory");
        let admission = open_lock(&runtime, "maintenance.lock")?;
        let admission_deadline = Instant::now() + wait;
        loop {
            match FileExt::try_lock_exclusive(&admission) {
                Ok(()) => break,
                Err(error) if contended(&error) => {
                    ensure!(Instant::now() < admission_deadline, "NomiFun startup or another installation did not release maintenance admission");
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(error) => return Err(error.into()),
            }
        }
        let observed = ResumeIntent::capture(&runtime)?;
        intent.desktop |= observed.desktop;
        intent.service |= observed.service;
        check(observed)?;
        let desktop = desktop_control::request_at(runtime.clone(), wait)?;
        let owner = open_lock(&runtime, "owner.lock")?;
        let deadline = Instant::now() + wait;
        let mut shutdown_sent = false;
        loop {
            match FileExt::try_lock_exclusive(&owner) {
                Ok(()) => break,
                Err(error) if contended(&error) => {}
                Err(error) => return Err(error.into()),
            }
            // A live owner may be between lifetime-lock acquisition and
            // descriptor publication. Wait briefly instead of inferring absence.
            if !shutdown_sent && runtime.join("owner.json").exists() {
                service::request_shutdown_after_desktop_exit_in(&runtime, &desktop)?;
                shutdown_sent = true;
            }
            ensure!(Instant::now() < deadline, "keyboard service did not release its lifetime lock");
            std::thread::sleep(Duration::from_millis(25));
        }
        let descriptor = runtime.join("owner.json");
        ensure!(!descriptor.is_symlink(), "keyboard owner descriptor is a symlink");
        if descriptor.exists() {
            fs::remove_file(descriptor).context("remove stopped keyboard service endpoint")?;
        }
        Ok(Self {
            _owner: owner,
            _desktop: desktop,
            _admission: admission,
            runtime,
        })
    }

    pub fn runtime(&self) -> &Path {
        &self.runtime
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_services_are_normal_and_lease_fences_both_lifetimes() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(ResumeIntent::capture(dir.path()).unwrap(), ResumeIntent::default());
        let guard = MaintenanceGuard::prepare_in(dir.path().into(), Duration::from_millis(100)).unwrap();
        assert!(admit_startup(dir.path()).is_err());
        for name in ["desktop.lock", "owner.lock", "maintenance.lock"] {
            let file = open_lock(dir.path(), name).unwrap();
            assert!(FileExt::try_lock_exclusive(&file).is_err(), "{name}");
        }
        assert!(MaintenanceGuard::prepare_in(dir.path().into(), Duration::from_millis(100)).is_err());
        drop(guard);
        let _startup = admit_startup(dir.path()).unwrap();
        for name in ["desktop.lock", "owner.lock"] {
            let file = open_lock(dir.path(), name).unwrap();
            FileExt::try_lock_exclusive(&file).unwrap();
        }
    }

    #[test]
    fn stale_endpoints_are_reclaimed_under_lifetime_locks() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("owner.json"), "not a live endpoint").unwrap();
        fs::write(dir.path().join("desktop.json"), "not a live endpoint").unwrap();
        let guard = MaintenanceGuard::prepare_in(dir.path().into(), Duration::from_millis(100)).unwrap();
        assert!(!dir.path().join("owner.json").exists());
        assert!(!dir.path().join("desktop.json").exists());
        drop(guard);
        assert!(MaintenanceGuard::prepare_in(dir.path().into(), Duration::from_millis(100)).is_ok());
    }

    #[test]
    fn busy_owner_without_endpoint_fails_and_releases_maintenance_gate() {
        let dir = tempfile::tempdir().unwrap();
        let owner = open_lock(dir.path(), "owner.lock").unwrap();
        FileExt::try_lock_exclusive(&owner).unwrap();
        assert!(ResumeIntent::capture(dir.path()).unwrap().service);
        assert!(MaintenanceGuard::prepare_in(dir.path().into(), Duration::from_millis(50)).is_err());
        assert!(admit_startup(dir.path()).is_ok());
        let desktop = open_lock(dir.path(), "desktop.lock").unwrap();
        FileExt::try_lock_exclusive(&desktop).unwrap();
    }

    #[test]
    fn startup_publication_and_maintenance_cannot_overtake_each_other() {
        let dir = tempfile::tempdir().unwrap();
        let admission = admit_startup(dir.path()).unwrap();
        assert!(MaintenanceGuard::prepare_in(dir.path().into(), Duration::from_millis(50)).is_err());
        drop(admission);
        let guard = MaintenanceGuard::prepare_in(dir.path().into(), Duration::from_millis(50)).unwrap();
        assert!(admit_startup(dir.path()).is_err());
        drop(guard);
        assert!(admit_startup(dir.path()).is_ok());
    }

    #[test]
    fn accepted_shutdown_is_not_resent_while_owner_finishes_teardown() {
        use std::{io::BufReader, net::TcpListener};
        let dir = tempfile::tempdir().unwrap();
        let owner = open_lock(dir.path(), "owner.lock").unwrap();
        FileExt::try_lock_exclusive(&owner).unwrap();
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let token = "a".repeat(64);
        fs::write(dir.path().join("owner.json"), serde_json::to_vec(&serde_json::json!({
            "port":listener.local_addr().unwrap().port(), "token":token, "pid":std::process::id()
        })).unwrap()).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let auth: serde_json::Value = service::read_line(&mut reader).unwrap();
            assert_eq!(auth["token"], token);
            service::send_line(&mut stream, &serde_json::json!({"accepted":true})).unwrap();
            let command: service::Control = service::read_line(&mut reader).unwrap();
            assert!(matches!(command, service::Control::Shutdown));
            service::send_line(&mut stream, &serde_json::json!({"completed":true})).unwrap();
            // Shutdown receipts precede destruction of native owner resources.
            // A second management request would now fail on the closed listener.
            drop(listener);
            drop(stream);
            std::thread::sleep(Duration::from_millis(125));
            drop(owner);
        });
        let guard = MaintenanceGuard::prepare_in(dir.path().into(), Duration::from_secs(2)).unwrap();
        server.join().unwrap();
        assert!(!dir.path().join("owner.json").exists());
        drop(guard);
    }

    #[test]
    fn startup_io_failure_is_not_reported_as_maintenance_pause() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("maintenance.lock")).unwrap();
        let error = match admit_startup(dir.path()) { Ok(_) => panic!("unexpected admission"), Err(error) => error };
        assert!(!error.is::<MaintenanceBusy>());
    }
}
