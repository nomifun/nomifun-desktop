#![cfg(target_os = "macos")]

use std::{
    env, fs, io::{BufRead, BufReader, Read, Write},
    os::unix::{fs::PermissionsExt, net::{UnixListener, UnixStream}, process::CommandExt},
    path::{Path, PathBuf}, process::{Child, Command, Stdio}, thread, time::{Duration, Instant},
};

use nomi_process_runtime::{DarwinApplicationExit, launched_application_peer_pid, own_launched_application};

const MODE: &str = "NOMI_DARWIN_APP_FIXTURE_MODE";
const SOCKET: &str = "NOMI_DARWIN_APP_FIXTURE_SOCKET";
const TIMEOUT: Duration = Duration::from_secs(5);

fn executable() -> PathBuf { env::current_exe().unwrap().canonicalize().unwrap() }

struct Endpoint {
    _directory: tempfile::TempDir,
    path: PathBuf,
    listener: UnixListener,
}

impl Endpoint {
    fn new() -> Self {
        let directory = tempfile::Builder::new().prefix("nomi-la-").tempdir_in("/tmp").unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let path = directory.path().join("peer.sock");
        let listener = UnixListener::bind(&path).unwrap();
        listener.set_nonblocking(true).unwrap();
        Self { _directory: directory, path, listener }
    }
    fn accept(&self) -> UnixStream {
        let deadline = Instant::now() + TIMEOUT;
        let mut connection = loop {
            match self.listener.accept() {
                Ok((connection, _)) => break connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
                Err(error) => panic!("application fixture admission failed: {error}"),
            }
        };
        connection.set_read_timeout(Some(TIMEOUT)).unwrap();
        connection.set_write_timeout(Some(TIMEOUT)).unwrap();
        let mut ready = [0; 6];
        connection.read_exact(&mut ready).unwrap();
        assert_eq!(&ready, b"READY\n");
        connection
    }
}

fn command(endpoint: &Endpoint, mode: &str, name: &str) -> Command {
    let mut command = Command::new(executable());
    command.args(["--exact", name, "--nocapture", "--test-threads=1"])
        .env(MODE, mode).env(SOCKET, &endpoint.path)
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::inherit());
    command
}

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        // This fixture still owns its real unreaped std Child. The observer
        // itself never has signal authority or a process-group fallback.
        if !matches!(self.0.try_wait(), Ok(Some(_))) { let _ = self.0.kill(); }
        let _ = self.0.wait();
    }
}

fn exit(observer: &mut nomi_process_runtime::DarwinLaunchedApplicationObserver) -> DarwinApplicationExit {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if let Some(exit) = observer.poll_exit(Duration::from_millis(50)).unwrap() { return exit; }
        assert!(Instant::now() < deadline, "authenticated application must publish a kernel exit status");
    }
}

#[test]
fn native_nonchild_exit_status_is_observed_without_waitpid_or_signals() {
    let endpoint = Endpoint::new();
    let output = command(&endpoint, "launcher", "fixture_launcher").stdout(Stdio::piped()).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    eprintln!("nonchild fixture launcher stdout:\n{stdout}");
    // libtest can leave its progress prefix on the same line as fixture
    // stdout. Accept that exact publisher prefix, while rejecting multiple
    // PID records, unrelated publishers, malformed PIDs and controller PIDs.
    let records = stdout.lines().filter_map(|line| line.split_once("APP_PID=")).collect::<Vec<_>>();
    assert_eq!(records.len(), 1, "fixture must publish one unambiguous callback PID: {stdout}");
    let (publisher, value) = records[0];
    assert!(publisher.is_empty() || publisher == "test fixture_launcher ... ", "unexpected fixture PID publisher: {stdout}");
    let pid = value.trim().parse::<u32>().expect("fixture callback PID must be an integer");
    assert!(pid > 1 && pid != std::process::id());
    let mut peer = endpoint.accept();
    assert_eq!(launched_application_peer_pid(&peer).unwrap(), pid, "provisional lookup must use the actual kernel peer PID");
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    assert_eq!(unsafe { libc::proc_pidinfo(pid as libc::pid_t, libc::PROC_PIDTBSDINFO, 0, (&raw mut info).cast(), size) }, size);
    assert_ne!(info.pbi_ppid, std::process::id(), "fixture must actually be a nonchild");
    let mut observer = own_launched_application(&peer, pid, &executable()).unwrap();
    observer.confirm_running_generation().unwrap();
    assert_eq!(observer.identity().pid, pid);
    assert_ne!(observer.pidversion(), 0);
    assert_eq!(observer.poll_exit(Duration::ZERO).unwrap(), None);
    // ACK is modelled by this first command, only after native admission.
    peer.write_all(b"EXIT 7\n").unwrap();
    assert_eq!(exit(&mut observer), DarwinApplicationExit { code: Some(7), signal: None });
    assert_eq!(observer.poll_exit(Duration::ZERO).unwrap(), Some(DarwinApplicationExit { code: Some(7), signal: None }));
    assert_eq!(observer.confirm_running_generation().unwrap_err().kind(), std::io::ErrorKind::NotFound);
    assert_eq!(observer.poll_exit(Duration::ZERO).unwrap(), Some(DarwinApplicationExit { code: Some(7), signal: None }), "ready rejection must not discard a known exact exit");
}

#[test]
fn wrong_callback_pid_or_executable_is_rejected_and_observer_drop_does_not_kill() {
    let endpoint = Endpoint::new();
    let mut child = OwnedChild(command(&endpoint, "application", "fixture_application").spawn().unwrap());
    let mut peer = endpoint.accept();
    assert!(own_launched_application(&peer, child.0.id() + 1, &executable()).is_err());
    assert!(own_launched_application(&peer, child.0.id(), Path::new("/bin/sh")).is_err());
    let observer = own_launched_application(&peer, child.0.id(), &executable()).unwrap();
    drop(observer);
    assert!(child.0.try_wait().unwrap().is_none(), "read-only observer Drop must not kill its application");
    peer.write_all(b"EXIT 0\n").unwrap();
    assert!(child.0.wait().unwrap().success());
}

#[test]
fn observed_exec_invalidates_admission_even_if_replacement_exits_zero() {
    let endpoint = Endpoint::new();
    let mut child = OwnedChild(command(&endpoint, "application", "fixture_application").spawn().unwrap());
    let mut peer = endpoint.accept();
    let mut observer = own_launched_application(&peer, child.0.id(), &executable()).unwrap();
    peer.write_all(b"EXEC\n").unwrap();
    let error = observer.poll_exit(TIMEOUT).unwrap_err();
    assert!(error.to_string().contains("exec") || error.to_string().contains("generation"));
    assert!(child.0.wait().unwrap().success(), "replacement deliberately exits zero");
    assert!(observer.poll_exit(Duration::ZERO).is_err(), "uncertain exec must remain sticky rather than become success");
}

#[test]
fn authenticated_exit_from_signal_preserves_the_kernel_signal_status() {
    let endpoint = Endpoint::new();
    let mut child = OwnedChild(command(&endpoint, "application", "fixture_application").spawn().unwrap());
    let mut peer = endpoint.accept();
    let mut observer = own_launched_application(&peer, child.0.id(), &executable()).unwrap();
    peer.write_all(b"SIGNAL\n").unwrap();
    assert_eq!(exit(&mut observer), DarwinApplicationExit { code: None, signal: Some(libc::SIGTERM) });
    assert!(!child.0.wait().unwrap().success());
}

#[test]
fn fixture_launcher() {
    if env::var(MODE).as_deref() != Ok("launcher") { return; }
    let endpoint = env::var_os(SOCKET).unwrap();
    let child = Command::new(executable()).args(["--exact", "fixture_application", "--nocapture", "--test-threads=1"])
        .env(MODE, "application").env(SOCKET, endpoint)
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    println!("APP_PID={}", child.id());
    // Drop the launcher without killing/reaping its child. launchd becomes
    // the application's parent, matching LaunchServices' nonchild boundary.
    drop(child);
}

#[test]
fn fixture_application() {
    if env::var(MODE).as_deref() != Ok("application") { return; }
    let mut peer = UnixStream::connect(env::var_os(SOCKET).unwrap()).unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
    peer.write_all(b"READY\n").unwrap();
    let mut request = String::new();
    if BufReader::new(&peer).read_line(&mut request).is_err() { std::process::exit(1); }
    match request.trim() {
        "EXIT 0" => std::process::exit(0),
        "EXIT 7" => std::process::exit(7),
        "SIGNAL" => {
            // libtest runs this fixture on a worker thread. A process-directed
            // signal followed immediately by exit(1) can lose the fatal-signal
            // race. Target this unblocked thread and wait for kernel delivery.
            let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
            action.sa_sigaction = libc::SIG_DFL;
            unsafe { libc::sigemptyset(&mut action.sa_mask); }
            assert_eq!(unsafe { libc::sigaction(libc::SIGTERM, &action, std::ptr::null_mut()) }, 0);
            let mut mask: libc::sigset_t = unsafe { std::mem::zeroed() };
            unsafe { libc::sigemptyset(&mut mask); libc::sigaddset(&mut mask, libc::SIGTERM); }
            assert_eq!(unsafe { libc::sigprocmask(libc::SIG_UNBLOCK, &mask, std::ptr::null_mut()) }, 0);
            unsafe { libc::alarm(15); }
            assert_eq!(unsafe { libc::raise(libc::SIGTERM) }, 0);
            loop { unsafe { libc::pause(); } }
        }
        "EXEC" => {
            let error = Command::new("/bin/sleep").arg("0.05").exec();
            panic!("fixture exec failed: {error}");
        }
        _ => std::process::exit(1),
    }
}
