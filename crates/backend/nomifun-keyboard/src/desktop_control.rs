//! Independent GUI stop proof, including when no USB daemon exists.
//! Descriptor absence is never evidence that a GUI has stopped: only an
//! exclusive lifetime lock proves absence and prevents a new GUI during update.
use crate::service::{private_create, read_line, runtime_dir, send_line, token_equal};
use anyhow::{Context, Result, ensure};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::BufReader,
    net::{Ipv4Addr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Descriptor {
    port: u16,
    token: String,
    pid: u32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    protocol: u32,
    token: String,
    operation: String,
}

fn open_lock(dir: &Path) -> Result<File> {
    let path = dir.join("desktop.lock");
    ensure!(!path.is_symlink(), "desktop lifetime lock is a symlink");
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}

/// Keep this in application state before exposing any OS recording action.
/// The stop callback must confirm native capture stopped. It is
/// invoked on this endpoint's thread, never on the UI thread, and only once
/// after a successful result. A failure permits an authenticated retry.
pub struct DesktopGuard {
    alive: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    descriptor: PathBuf,
    _lock: File,
}
impl Drop for DesktopGuard {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let _ = fs::remove_file(&self.descriptor);
    }
}
/// Held by the external caller until daemon teardown and package work finish.
/// Retaining this guard prevents a concurrent GUI from opening its microphone.
pub struct DesktopQuiescence {
    // A new GUI also holds desktop.lock. This second lifetime lock proves the
    // holder of a stale exit file is a live maintenance caller, not a new GUI.
    _proof_lock: File,
    _lock: File,
    path: PathBuf,
    token: String,
}
impl Drop for DesktopQuiescence {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
impl DesktopQuiescence {
    fn new(lock: File, dir: &Path) -> Result<Self> {
        // Tauri's process::exit can skip DesktopGuard::drop. The caller has
        // already acquired the exclusive lifetime lock, so removing this stale
        // endpoint now cannot hide a running GUI or substitute for stop proof.
        let descriptor = dir.join("desktop.json");
        ensure!(!descriptor.is_symlink(), "desktop descriptor is a symlink");
        if descriptor.exists() {
            fs::remove_file(descriptor)?;
        }
        let proof_lock = open_proof_lock(dir)?;
        FileExt::try_lock_exclusive(&proof_lock)
            .context("another maintenance proof is still live")?;
        let path = dir.join("desktop-exited.json");
        ensure!(!path.is_symlink(), "desktop exit proof is a symlink");
        if path.exists() {
            fs::remove_file(&path)?;
        }
        let mut random = [0u8; 32];
        getrandom::getrandom(&mut random)
            .map_err(|_| anyhow::anyhow!("secure random unavailable"))?;
        let token: String = random.iter().map(|value| format!("{value:02x}")).collect();
        let mut file = private_create(&path)?;
        serde_json::to_writer(&mut file, &token)?;
        file.sync_all()?;
        Ok(Self {
            _proof_lock: proof_lock,
            _lock: lock,
            path,
            token,
        })
    }
    pub(crate) fn proof_for(&self, dir: &Path) -> Result<&str> {
        ensure!(
            self.path
                .parent()
                .context("exit proof directory missing")?
                .canonicalize()?
                == dir.canonicalize()?,
            "desktop exit proof belongs to another runtime"
        );
        Ok(&self.token)
    }
}
fn open_proof_lock(dir: &Path) -> Result<File> {
    let path = dir.join("desktop-exited.lock");
    ensure!(!path.is_symlink(), "desktop exit proof lock is a symlink");
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}
/// A live CLI holds the exclusive GUI lock and its fresh private proof. A stale
/// file after CLI death is insufficient because the lifetime lock is then free.
/// Any subsequently registered GUI clears the previous proof before recording.
pub(crate) fn verify_quiescence(dir: &Path, token: &str) -> Result<()> {
    let path = dir.join("desktop-exited.json");
    ensure!(
        !path.is_symlink() && fs::metadata(&path)?.len() <= 128,
        "unsafe desktop exit proof"
    );
    let actual: String = serde_json::from_reader(File::open(path)?)?;
    ensure!(
        token.len() == 64 && token_equal(token, &actual),
        "desktop exit proof mismatch"
    );
    let lock = open_lock(dir)?;
    ensure!(
        FileExt::try_lock_exclusive(&lock).is_err(),
        "desktop exit proof is stale: no maintenance lock is held"
    );
    let proof_lock = open_proof_lock(dir)?;
    ensure!(
        FileExt::try_lock_exclusive(&proof_lock).is_err(),
        "desktop exit proof is stale: its maintenance caller exited"
    );
    Ok(())
}

pub fn register(
    stop: impl Fn() -> Result<()> + Send + Sync + 'static,
    exit: impl Fn() + Send + Sync + 'static,
) -> Result<DesktopGuard> {
    register_with_exit_at(runtime_dir()?, stop, exit)
}
#[cfg(test)]
fn register_at(
    dir: PathBuf,
    stop: impl Fn() -> Result<()> + Send + Sync + 'static,
) -> Result<DesktopGuard> {
    register_with_exit_at(dir, stop, || {})
}
fn register_with_exit_at(
    dir: PathBuf,
    stop: impl Fn() -> Result<()> + Send + Sync + 'static,
    exit: impl Fn() + Send + Sync + 'static,
) -> Result<DesktopGuard> {
    let lock = open_lock(&dir)?;
    FileExt::try_lock_exclusive(&lock)
        .context("another desktop or maintenance operation owns GUI lifetime")?;
    let exited = dir.join("desktop-exited.json");
    ensure!(!exited.is_symlink(), "desktop exit proof is a symlink");
    if exited.exists() {
        fs::remove_file(exited)?;
    }
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    listener.set_nonblocking(true)?;
    let mut random = [0u8; 32];
    getrandom::getrandom(&mut random).map_err(|_| anyhow::anyhow!("secure random unavailable"))?;
    let token: String = random.iter().map(|v| format!("{v:02x}")).collect();
    let path = dir.join("desktop.json");
    ensure!(!path.is_symlink(), "desktop descriptor is a symlink");
    if path.exists() {
        fs::remove_file(&path)?;
    }
    let mut descriptor = private_create(&path)?;
    serde_json::to_writer(
        &mut descriptor,
        &Descriptor {
            port: listener.local_addr()?.port(),
            token: token.clone(),
            pid: std::process::id(),
        },
    )?;
    descriptor.sync_all()?;
    drop(descriptor);
    let alive = Arc::new(AtomicBool::new(true));
    let active = alive.clone();
    let thread = std::thread::Builder::new()
        .name("nomifun-gui-stop".into())
        .spawn(move || {
            let mut completed = false;
            while active.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, address)) => {
                        if !address.ip().is_loopback() {
                            continue;
                        }
                        let result = (|| -> Result<()> {
                            // Winsock may inherit the listener's nonblocking
                            // mode; receive deadlines require a blocking peer.
                            stream.set_nonblocking(false)?;
                            stream.set_read_timeout(Some(Duration::from_secs(1)))?;
                            stream.set_write_timeout(Some(Duration::from_secs(1)))?;
                            let request: Request =
                                read_line(&mut BufReader::new(stream.try_clone()?))?;
                            ensure!(
                                request.protocol == 1
                                    && request.operation == "stop"
                                    && token_equal(&request.token, &token),
                                "desktop stop authentication rejected"
                            );
                            if !completed {
                                stop()?;
                                completed = true;
                            }
                            Ok(())
                        })();
                        let success = result.is_ok();
                        let reply = match result {
                            Ok(()) => json!({"completed":true}),
                            Err(error) => json!({"completed":false,"error":error.to_string()}),
                        };
                        let _ = send_line(&mut stream, &reply);
                        // The native stop proof enters the socket before requesting
                        // process exit, which may release the lock immediately.
                        if success {
                            exit();
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10))
                    }
                    Err(_) => break,
                }
            }
        })?;
    Ok(DesktopGuard {
        alive,
        thread: Some(thread),
        descriptor: path,
        _lock: lock,
    })
}

/// External CLI only. Do not call this from the GUI which owns DesktopGuard.
/// Success means either no GUI existed, or authenticated native stop succeeded
/// and its lifetime lock was released; missing/stale metadata while busy fails.
pub fn request_desktop_shutdown() -> Result<DesktopQuiescence> {
    request_at(runtime_dir()?, Duration::from_secs(18))
}
pub(crate) fn request_at(dir: PathBuf, wait: Duration) -> Result<DesktopQuiescence> {
    let deadline = Instant::now() + wait;
    let lock = open_lock(&dir)?;
    if FileExt::try_lock_exclusive(&lock).is_ok() {
        return DesktopQuiescence::new(lock, &dir);
    }
    let path = dir.join("desktop.json");
    ensure!(!path.is_symlink(), "desktop descriptor is a symlink");
    let descriptor: Descriptor = serde_json::from_reader(File::open(path).context(
        "GUI lifetime is busy but its authenticated endpoint is unavailable; stop is unproven",
    )?)?;
    ensure!(
        descriptor.token.len() == 64 && descriptor.port != 0,
        "invalid desktop descriptor"
    );
    let mut stream = TcpStream::connect_timeout(
        &([127, 0, 0, 1], descriptor.port).into(),
        Duration::from_secs(1),
    )?;
    stream.set_write_timeout(Some(Duration::from_secs(1)))?;
    stream.set_read_timeout(Some(wait))?;
    send_line(
        &mut stream,
        &json!({"protocol":1,"token":descriptor.token,"operation":"stop"}),
    )?;
    let reply: Value =
        read_line(&mut BufReader::new(stream)).context("GUI native capture stop proof")?;
    ensure!(
        reply["completed"] == true,
        "GUI stop not confirmed: {}",
        reply["error"].as_str().unwrap_or("missing proof")
    );
    loop {
        if FileExt::try_lock_exclusive(&lock).is_ok() {
            return DesktopQuiescence::new(lock, &dir);
        }
        ensure!(
            Instant::now() < deadline,
            "GUI reported capture stopped but did not release its lifetime lock; exit is unproven"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, atomic::AtomicUsize, mpsc};

    #[test]
    fn process_exit_descriptor_is_removed_only_after_gui_lifetime_is_proven_absent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("desktop.json");
        fs::write(
            &path,
            serde_json::to_vec(&Descriptor {
                port: 1,
                pid: 4242,
                token: "0".repeat(64),
            })
            .unwrap(),
        )
        .unwrap();
        let lifetime = open_lock(dir.path()).unwrap();
        FileExt::try_lock_exclusive(&lifetime).unwrap();
        assert!(request_at(dir.path().into(), Duration::from_millis(100)).is_err());
        assert!(
            path.exists(),
            "a busy GUI's descriptor must never be removed"
        );
        // process::exit releases OS handles but skips Rust guard destructors.
        drop(lifetime);
        let proof = request_at(dir.path().into(), Duration::from_millis(100)).unwrap();
        assert!(!path.exists());
        assert!(verify_quiescence(dir.path(), proof.proof_for(dir.path()).unwrap()).is_ok());
        assert!(register_at(dir.path().into(), || Ok(())).is_err());
        drop(proof);
        assert!(register_at(dir.path().into(), || Ok(())).is_ok());
    }

    #[test]
    fn exit_proof_requires_a_live_maintenance_lock_and_is_invalidated_by_new_gui() {
        let dir = tempfile::tempdir().unwrap();
        let proof = request_at(dir.path().into(), Duration::from_millis(100)).unwrap();
        let token = proof.proof_for(dir.path()).unwrap().to_owned();
        assert!(verify_quiescence(dir.path(), &token).is_ok());
        assert!(verify_quiescence(dir.path(), "invalid").is_err());
        drop(proof);
        let path = dir.path().join("desktop-exited.json");
        assert!(!path.exists());
        // A killed maintenance process can leave the file, never the lock.
        fs::write(&path, serde_json::to_vec(&token).unwrap()).unwrap();
        assert!(verify_quiescence(dir.path(), &token).is_err());
        // A replacement GUI may acquire desktop.lock immediately before it
        // removes the stale file; it never owns desktop-exited.lock.
        let replacement = open_lock(dir.path()).unwrap();
        FileExt::try_lock_exclusive(&replacement).unwrap();
        assert!(verify_quiescence(dir.path(), &token).is_err());
        drop(replacement);
        let _gui = register_at(dir.path().into(), || Ok(())).unwrap();
        assert!(!path.exists());
        assert!(verify_quiescence(dir.path(), &token).is_err());
    }

    #[test]
    fn missing_descriptor_requires_an_exclusive_lock() {
        let dir = tempfile::tempdir().unwrap();
        let lock = open_lock(dir.path()).unwrap();
        FileExt::try_lock_exclusive(&lock).unwrap();
        assert!(request_at(dir.path().into(), Duration::from_millis(100)).is_err());
        drop(lock);
        let proof = request_at(dir.path().into(), Duration::from_millis(100)).unwrap();
        assert!(register_at(dir.path().into(), || Ok(())).is_err());
        drop(proof);
        assert!(register_at(dir.path().into(), || Ok(())).is_ok());
    }

    #[test]
    fn authentication_cannot_call_stop() {
        let dir = tempfile::tempdir().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let called = calls.clone();
        let _guard = register_at(dir.path().into(), move || {
            called.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
        let descriptor: Descriptor =
            serde_json::from_reader(File::open(dir.path().join("desktop.json")).unwrap()).unwrap();
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, descriptor.port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        send_line(
            &mut stream,
            &json!({"protocol":1,"token":"wrong","operation":"stop"}),
        )
        .unwrap();
        let reply: Value = read_line(&mut BufReader::new(stream)).unwrap();
        assert_eq!(reply["completed"], false);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn authenticated_stop_waits_for_gui_lifetime_and_blocks_restart() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        let guard = register_at(dir.path().into(), move || {
            tx.send(()).unwrap();
            Ok(())
        })
        .unwrap();
        let path = dir.path().to_path_buf();
        let request = std::thread::spawn(move || request_at(path, Duration::from_secs(2)));
        rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(!request.is_finished()); // callback success alone is not exit proof
        drop(guard);
        let proof = request.join().unwrap().unwrap();
        assert!(register_at(dir.path().into(), || Ok(())).is_err());
        drop(proof);
        assert!(register_at(dir.path().into(), || Ok(())).is_ok());
    }

    #[test]
    fn callback_failure_and_missing_exit_are_not_success() {
        let dir = tempfile::tempdir().unwrap();
        let guard = register_at(dir.path().into(), || {
            anyhow::bail!("native stream did not stop")
        })
        .unwrap();
        assert!(
            request_at(dir.path().into(), Duration::from_millis(100))
                .unwrap_err_text()
                .contains("native stream did not stop")
        );
        drop(guard);
        let _guard = register_at(dir.path().into(), || Ok(())).unwrap();
        assert!(
            request_at(dir.path().into(), Duration::from_millis(100))
                .unwrap_err_text()
                .contains("exit is unproven")
        );
    }
    trait ErrorText {
        fn unwrap_err_text(self) -> String;
    }
    impl ErrorText for Result<DesktopQuiescence> {
        fn unwrap_err_text(self) -> String {
            match self {
                Ok(_) => panic!("unexpected stop proof"),
                Err(e) => e.to_string(),
            }
        }
    }

    #[test]
    fn successful_stop_callback_is_idempotent_while_exit_pending() {
        let dir = tempfile::tempdir().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let copy = calls.clone();
        let _guard = register_at(dir.path().into(), move || {
            copy.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
        for _ in 0..2 {
            assert!(request_at(dir.path().into(), Duration::from_millis(70)).is_err());
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn failed_stop_can_be_retried_without_reregistering() {
        let dir = tempfile::tempdir().unwrap();
        let failure = Arc::new(Mutex::new(true));
        let copy = failure.clone();
        let (tx, rx) = mpsc::channel();
        let guard = register_at(dir.path().into(), move || {
            ensure!(!*copy.lock().unwrap(), "driver busy");
            tx.send(()).unwrap();
            Ok(())
        })
        .unwrap();
        assert!(request_at(dir.path().into(), Duration::from_millis(100)).is_err());
        *failure.lock().unwrap() = false;
        let path = dir.path().to_path_buf();
        let request = std::thread::spawn(move || request_at(path, Duration::from_secs(2)));
        rx.recv_timeout(Duration::from_secs(1)).unwrap();
        drop(guard);
        assert!(request.join().unwrap().is_ok());
    }
}
