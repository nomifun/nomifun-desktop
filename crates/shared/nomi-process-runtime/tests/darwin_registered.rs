#![cfg(target_os = "macos")]

use std::{
    fs,
    io::{self, BufRead, BufReader, Read, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
        process::{CommandExt, ExitStatusExt},
    },
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

use nomi_process_runtime::{
    DarwinGenerationTerminationOutcome, DarwinRegisteredChildAuthority, ExactProcessIdentity,
    own_registered_child_process, probe_process_identity,
};
use tempfile::TempDir;

const FIXTURE_MODE_ENV: &str = "NOMI_DARWIN_REGISTERED_FIXTURE_MODE";
const FIXTURE_SOCKET_ENV: &str = "NOMI_DARWIN_REGISTERED_FIXTURE_SOCKET";
const EXEC_SOCKET_ENV: &str = "NOMI_DARWIN_REGISTERED_EXEC_SOCKET";
const TEST_TIMEOUT: Duration = Duration::from_secs(5);
const FIXTURE_TIMEOUT: Duration = Duration::from_secs(30);

#[test]
fn registered_child_termination_proves_absence_and_preserves_unrelated_child() {
    let parent = current_identity();
    let mut registered = Fixture::spawn(None);
    let mut unrelated = Fixture::spawn(None);
    let authority = registered.adopt(&parent);

    assert_eq!(authority.identity().pid, registered.child.id());
    assert_eq!(
        authority.identity().executable.as_deref(),
        Some(current_executable().as_path()),
    );
    assert!(!authority.generation_is_absent().expect("registered generation should be observable"));

    assert_eq!(
        authority.terminate_and_prove_generation_absent(TEST_TIMEOUT)
            .expect("registered generation should be terminated and proven absent"),
        DarwinGenerationTerminationOutcome::SignalledAndAbsent,
    );
    assert!(authority.generation_is_absent().expect("absence should remain provable"));
    assert_eq!(registered.child.wait().signal(), Some(libc::SIGKILL));
    unrelated.assert_alive();

    assert_eq!(
        authority.terminate_and_prove_generation_absent(TEST_TIMEOUT)
            .expect("repeated cleanup should prove the old generation absent"),
        DarwinGenerationTerminationOutcome::AlreadyAbsent,
    );
    unrelated.assert_alive();
    unrelated.exit_normally();
}

#[test]
fn completed_exec_invalidates_old_authority_and_preserves_new_generation() {
    let parent = current_identity();
    let exec_socket = SocketEndpoint::new();
    let mut fixture = Fixture::spawn(Some(exec_socket.path()));
    let old_authority = fixture.adopt(&parent);
    let old_pidversion = old_authority.pidversion();
    let old_pid = old_authority.identity().pid;

    fixture.connection.write_all(b"EXEC\n").expect("fixture should receive the exec command");
    // A fresh connection is opened by fixture_exec_target after exec has
    // completed. Reusing the inherited connection would retain its old token.
    fixture.connection = exec_socket.accept_ready();
    let new_authority = fixture.adopt(&parent);

    assert_eq!(new_authority.identity().pid, old_pid, "exec should preserve the owned PID");
    assert_eq!(new_authority.identity().pid, fixture.child.id());
    assert_ne!(new_authority.pidversion(), old_pidversion, "completed exec must publish a new audit-token generation");
    assert!(old_authority.generation_is_absent().expect("old exec generation should be absent"));
    assert!(!new_authority.generation_is_absent().expect("new exec generation should be observable"));

    assert_eq!(
        old_authority.terminate_and_prove_generation_absent(TEST_TIMEOUT)
            .expect("old authority should recognize its completed generation"),
        DarwinGenerationTerminationOutcome::AlreadyAbsent,
    );
    fixture.assert_alive();
    assert!(!new_authority.generation_is_absent().expect("old cleanup must leave the new generation live"));

    assert_eq!(
        new_authority.terminate_and_prove_generation_absent(TEST_TIMEOUT)
            .expect("fresh authority should clean up the new generation"),
        DarwinGenerationTerminationOutcome::SignalledAndAbsent,
    );
    assert!(new_authority.generation_is_absent().expect("new generation should be proven absent"));
    assert_eq!(fixture.child.wait().signal(), Some(libc::SIGKILL));
}

#[test]
fn registration_rejects_parent_start_generation_mismatch() {
    let mut parent = current_identity();
    parent.platform_start_key = parent.platform_start_key.checked_add(1)
        .expect("test parent start key should leave space for a different generation");
    let mut fixture = Fixture::spawn(None);

    let error = own_registered_child_process(&fixture.connection, &parent, &current_executable())
        .expect_err("a stale parent generation must not authorize a child");
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    fixture.assert_alive();
    fixture.exit_normally();
}

#[test]
fn registration_rejects_live_process_that_is_not_the_actual_parent() {
    let mut fixture = Fixture::spawn(None);
    let mut unrelated = Fixture::spawn(None);
    let unrelated_parent = probe_process_identity(unrelated.child.id())
        .expect("owned unrelated process identity should be readable")
        .expect("owned unrelated process should still be live");

    let error = own_registered_child_process(&fixture.connection, &unrelated_parent, &current_executable())
        .expect_err("a live sibling cannot authorize a peer whose PPID is the test process");
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    fixture.assert_alive();
    unrelated.assert_alive();
    fixture.exit_normally();
    unrelated.exit_normally();
}

#[test]
fn registration_rejects_wrong_exact_executable_path() {
    let parent = current_identity();
    let mut fixture = Fixture::spawn(None);
    let different_executable = Path::new("/usr/bin/true").canonicalize()
        .expect("macOS should provide an existing different executable");
    assert_ne!(different_executable, current_executable());

    let error = own_registered_child_process(&fixture.connection, &parent, &different_executable)
        .expect_err("an existing wrong executable must not authorize the peer");
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    fixture.assert_alive();
    fixture.exit_normally();
}

#[test]
fn registration_rejects_missing_exact_executable_path() {
    let parent = current_identity();
    let mut fixture = Fixture::spawn(None);
    let missing_executable = fixture.socket.directory.path().join("missing-executable");

    let error = own_registered_child_process(&fixture.connection, &parent, &missing_executable)
        .expect_err("an unreadable expected executable must not create authority");
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    fixture.assert_alive();
    fixture.exit_normally();
}

#[test]
fn registration_requires_expected_parent_executable_evidence() {
    let mut parent = current_identity();
    parent.executable = None;
    let mut fixture = Fixture::spawn(None);

    let error = own_registered_child_process(&fixture.connection, &parent, &current_executable())
        .expect_err("missing parent executable evidence must not be treated as a wildcard");
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    fixture.assert_alive();
    fixture.exit_normally();
}

#[test]
fn registration_normalizes_expected_executable_alias() {
    let parent = current_identity();
    let mut fixture = Fixture::spawn(None);
    let alias = fixture.socket.directory.path().join("fixture-executable");
    std::os::unix::fs::symlink(current_executable(), &alias)
        .expect("private executable alias should be created");

    let authority = own_registered_child_process(&fixture.connection, &parent, &alias)
        .expect("an alias to the exact executable should normalize before validation");
    assert_eq!(authority.identity().executable.as_deref(), Some(current_executable().as_path()));
    assert_eq!(
        authority.terminate_and_prove_generation_absent(TEST_TIMEOUT)
            .expect("normalized authority should clean up its registered child"),
        DarwinGenerationTerminationOutcome::SignalledAndAbsent,
    );
    assert_eq!(fixture.child.wait().signal(), Some(libc::SIGKILL));
}

// These entry points do nothing during an ordinary integration-test run.
// Each self-spawned child runs exactly one, with its private socket supplied
// explicitly. No process is discovered by name, enumeration, or process group.
#[test]
fn fixture_child() {
    if std::env::var(FIXTURE_MODE_ENV).as_deref() != Ok("child") {
        return;
    }
    run_fixture(true).expect("registered-child fixture should complete its command protocol");
}

#[test]
fn fixture_exec_target() {
    if std::env::var(FIXTURE_MODE_ENV).as_deref() != Ok("exec_target") {
        return;
    }
    run_fixture(false).expect("exec-target fixture should complete its command protocol");
}

fn run_fixture(can_exec: bool) -> io::Result<()> {
    let socket_path = std::env::var_os(FIXTURE_SOCKET_ENV)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "fixture socket is missing"))?;
    let mut connection = UnixStream::connect(socket_path)?;
    connection.set_read_timeout(Some(FIXTURE_TIMEOUT))?;
    connection.set_write_timeout(Some(TEST_TIMEOUT))?;
    connection.write_all(b"READY\n")?;
    let mut reader = BufReader::new(connection);
    loop {
        let mut command = String::new();
        if reader.read_line(&mut command)? == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "fixture control socket closed"));
        }
        match command.as_str() {
            "PING\n" => reader.get_mut().write_all(b"ALIVE\n")?,
            "EXIT\n" => return Ok(()),
            "EXEC\n" if can_exec => {
                let exec_socket = std::env::var_os(EXEC_SOCKET_ENV)
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "exec socket is missing"))?;
                let error = fixture_command("fixture_exec_target", "exec_target", Path::new(&exec_socket))
                    .env_remove(EXEC_SOCKET_ENV)
                    .exec();
                return Err(error);
            }
            _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "unexpected fixture command")),
        }
    }
}

fn current_identity() -> ExactProcessIdentity {
    probe_process_identity(std::process::id())
        .expect("test parent identity should be readable")
        .expect("test parent should be live")
}

fn current_executable() -> PathBuf {
    std::env::current_exe().expect("integration test binary should be identifiable")
        .canonicalize().expect("integration test binary should have an exact canonical path")
}

fn fixture_command(test_name: &str, mode: &str, socket: &Path) -> Command {
    let mut command = Command::new(current_executable());
    command.args(["--exact", test_name, "--nocapture", "--test-threads=1"])
        .env(FIXTURE_MODE_ENV, mode)
        .env(FIXTURE_SOCKET_ENV, socket)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    command
}

struct Fixture {
    child: OwnedChild,
    connection: UnixStream,
    socket: SocketEndpoint,
}

impl Fixture {
    fn spawn(exec_socket: Option<&Path>) -> Self {
        let socket = SocketEndpoint::new();
        let mut command = fixture_command("fixture_child", "child", socket.path());
        // Do not let inherited fixture state redirect this child elsewhere.
        command.env_remove(EXEC_SOCKET_ENV);
        if let Some(exec_socket) = exec_socket {
            command.env(EXEC_SOCKET_ENV, exec_socket);
        }
        let child = OwnedChild::new(command.spawn().expect("exact integration fixture should spawn"));
        let connection = socket.accept_ready();
        Self { child, connection, socket }
    }

    fn adopt(&self, parent: &ExactProcessIdentity) -> DarwinRegisteredChildAuthority {
        own_registered_child_process(&self.connection, parent, &current_executable())
            .expect("kernel-identified direct child should be registered")
    }

    fn assert_alive(&mut self) {
        assert!(self.child.try_wait().expect("owned fixture status should be readable").is_none(),
            "unrelated or exec-target fixture must remain live");
        self.connection.write_all(b"PING\n").expect("live fixture should accept a ping");
        let mut response = [0_u8; 6];
        self.connection.read_exact(&mut response).expect("live fixture should answer the ping");
        assert_eq!(&response, b"ALIVE\n");
        assert!(self.child.try_wait().expect("owned fixture should remain observable").is_none());
    }

    fn exit_normally(&mut self) {
        self.connection.write_all(b"EXIT\n").expect("fixture should accept a normal exit command");
        assert!(self.child.wait().success(), "control protocol should exit normally");
    }
}

struct SocketEndpoint {
    directory: TempDir,
    listener: UnixListener,
    path: PathBuf,
}

impl SocketEndpoint {
    fn new() -> Self {
        // /tmp keeps sockaddr_un below macOS's path length limit even when
        // the user's default temporary directory is deeply nested.
        let directory = tempfile::Builder::new().prefix("nomi-dr-").tempdir_in("/tmp")
            .expect("private registration directory should be created");
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))
            .expect("registration directory should be accessible only to its owner");
        let path = directory.path().join("peer.sock");
        let listener = UnixListener::bind(&path).expect("private registration listener should bind");
        listener.set_nonblocking(true).expect("listener should support bounded accept");
        Self { directory, listener, path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn accept_ready(&self) -> UnixStream {
        let deadline = Instant::now() + TEST_TIMEOUT;
        let mut connection = loop {
            match self.listener.accept() {
                Ok((connection, _)) => break connection,
                Err(error) if error.kind() == io::ErrorKind::Interrupted && Instant::now() < deadline => continue,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("fixture must connect within the setup deadline: {error}"),
            }
        };
        connection.set_nonblocking(false).expect("accepted peer should use bounded blocking I/O");
        connection.set_read_timeout(Some(TEST_TIMEOUT)).expect("peer reads should be bounded");
        connection.set_write_timeout(Some(TEST_TIMEOUT)).expect("peer writes should be bounded");
        let mut ready = [0_u8; 6];
        connection.read_exact(&mut ready).expect("child should complete its ready handshake");
        assert_eq!(&ready, b"READY\n");
        connection
    }
}

struct OwnedChild(Option<Child>);

impl OwnedChild {
    fn new(child: Child) -> Self {
        Self(Some(child))
    }

    fn id(&self) -> u32 {
        self.0.as_ref().expect("fixture child should remain owned").id()
    }

    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.0.as_mut().expect("fixture child should remain owned").try_wait()
    }

    fn wait(&mut self) -> ExitStatus {
        let deadline = Instant::now() + TEST_TIMEOUT;
        loop {
            if let Some(status) = self.try_wait().expect("exact owned child should be reapable") {
                self.0.take();
                return status;
            }
            assert!(Instant::now() < deadline, "owned fixture should exit before the deadline");
            thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            // Holding an unreaped std Child pins the child instance on Unix.
            // Even during panic cleanup, never enumerate or signal a group.
            if !matches!(child.try_wait(), Ok(Some(_))) {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
    }
}
