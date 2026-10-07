//! Private, independent shutdown guardian for the macOS CEF host.
//!
//! This runs in the helper executable before sandbox/CEF initialization. The
//! socket is a launch-local rendezvous, not a backend IPC endpoint. Main control
//! always comes from the kernel peer and the recorded actual parent; bodies
//! cannot supply a PID or widen the registered-child signal authority.

use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{self, Read, Write},
    os::{
        fd::AsRawFd,
        unix::{fs::{FileTypeExt, MetadataExt, PermissionsExt}, net::{UnixListener, UnixStream}},
    },
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use nomi_process_runtime::{
    DarwinRegisteredChildAuthority, ExactProcessIdentity, own_registered_child_process,
    probe_process_identity,
};
use serde::{Deserialize, Serialize};

pub const MAX_FRAME_BYTES: usize = 16 * 1024;
pub const RPC_TIMEOUT: Duration = Duration::from_secs(2);
pub const NATIVE_SHUTDOWN_BUDGET: Duration = Duration::from_secs(30);
pub const HELPER_CLEANUP_BUDGET: Duration = Duration::from_secs(5);
const MAX_PEERS: usize = 64;
const MAX_DECLARATIONS: usize = 4096;
const MAX_ERRORS: usize = 16;
// Even the worst JSON escape expansion (6 bytes for one control byte), sixteen
// receipt errors and one RPC error stay within the shared 16 KiB frame limit.
const MAX_ERROR_BYTES: usize = 128;
const HELPER_PATH_COUNT: usize = 5;
const HELPER_ROLES: [&str; 3] = ["renderer", "gpu-process", "utility"];
const LOOP_INTERVAL: Duration = Duration::from_millis(10);

/// One length-prefixed JSON request (u32 big endian, then at most 16 KiB).
/// Each connection carries exactly one RPC. Unknown operations/fields fail
/// closed. A nonce is a 32-byte random value encoded as 64 lowercase hex digits.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// Exact canonical executable paths, ordered generic/GPU/Renderer/Plugin/Alerts.
    Configure { helper_paths: Vec<PathBuf> },
    Declare { nonce: String, role: String },
    Register { nonce: String, role: String },
    EnterNativeShutdown,
    NativeReturned,
    /// Initialization failed before the native shutdown call was entered.
    /// Cleanup still requires exact registered generation absence proof.
    AbortInitialization,
    Status,
    Stop,
}

// Serde's internally tagged *unit* variant visitor intentionally ignores
// remaining map fields, even with the enum's deny_unknown_fields setting.
// Empty struct variants enforce the schema while keeping the public no-args
// API convenient for the client. In particular, Status { pid: ... } fails.
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum WireRequest {
    Configure { helper_paths: Vec<PathBuf> },
    Declare { nonce: String, role: String },
    Register { nonce: String, role: String },
    EnterNativeShutdown {},
    NativeReturned {},
    AbortInitialization {},
    Status {},
    Stop {},
}

impl<'de> Deserialize<'de> for Request {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match WireRequest::deserialize(deserializer)? {
            WireRequest::Configure { helper_paths } => Self::Configure { helper_paths },
            WireRequest::Declare { nonce, role } => Self::Declare { nonce, role },
            WireRequest::Register { nonce, role } => Self::Register { nonce, role },
            WireRequest::EnterNativeShutdown {} => Self::EnterNativeShutdown,
            WireRequest::NativeReturned {} => Self::NativeReturned,
            WireRequest::AbortInitialization {} => Self::AbortInitialization,
            WireRequest::Status {} => Self::Status,
            WireRequest::Stop {} => Self::Stop,
        })
    }
}

/// A proof about registered *execution generations*, never CEF verification.
/// Missing declarations and identity/probe errors prevent `complete`, including
/// after Main reports that the native CEF call returned.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CleanupReceipt {
    pub complete: bool,
    pub generation_absence_only: bool,
    pub registered: usize,
    pub absent: usize,
    pub unresolved_declarations: usize,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Reply {
    /// `ok` means the RPC was accepted; cleanup proof is in `receipt.complete`.
    pub status: String,
    pub phase: String,
    /// Entered native shutdown and has not received authenticated NativeReturned.
    pub native_running: bool,
    pub deadline_unix_ms: Option<u64>,
    pub receipt: Option<CleanupReceipt>,
    pub error: Option<String>,
}

/// Run the early helper guardian mode. The client owns the enclosing 0700
/// directory; the guardian owns only its new 0600 socket and removes that exact
/// socket on return. It never removes an existing path or kills Main.
pub fn run_guardian(socket: &Path) -> Result<(), String> {
    let main = bootstrap_main()?;
    validate_private_directory(socket)?;
    let listener = UnixListener::bind(socket).map_err(|error| format!("guardian bind: {error}"))?;
    let socket_owner = SocketOwner::capture(socket)?;
    fs::set_permissions(socket, fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("guardian socket permissions: {error}"))?;
    listener.set_nonblocking(true).map_err(|error| format!("guardian listener: {error}"))?;
    let mut state = ServerState::new(main);
    let mut peers = Vec::<Peer>::new();
    let mut parent_check = Instant::now();

    loop {
        // All peer I/O is nonblocking. A silent/partial peer cannot hold this
        // loop inside read_exact or defer the independent native deadline.
        if !state.stopping {
            for _ in 0..MAX_PEERS {
                match listener.accept() {
                    Ok((stream, _)) => {
                        if peers.len() < MAX_PEERS {
                            if let Ok(peer) = Peer::new(stream) { peers.push(peer); }
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => return Err(format!("guardian accept: {error}")),
                }
            }
        }

        let mut index = 0;
        while index < peers.len() {
            if Instant::now() >= peers[index].deadline {
                peers.swap_remove(index);
                state.tick(Instant::now());
                continue;
            }
            let peer = &mut peers[index];
            let result = if peer.response.is_some() {
                peer.flush_response()
            } else {
                match peer.read_request() {
                    Ok(Some(request)) if Instant::now() < peer.deadline => {
                        let reply = state.dispatch(&peer.stream, request, peer.deadline);
                        peer.set_response(&reply).and_then(|()| peer.flush_response())
                    }
                    Ok(Some(_)) => Err("guardian request deadline expired".into()),
                    Ok(None) => Ok(false),
                    Err(error) => {
                        let reply = state.reply(Some(&format!("guardian request: {error}")));
                        peer.set_response(&reply).and_then(|()| peer.flush_response())
                    }
                }
            };
            if !matches!(result, Ok(false)) || Instant::now() >= peer.deadline {
                peers.swap_remove(index);
            } else {
                index += 1;
            }
            state.tick(Instant::now());
        }

        state.tick(Instant::now());
        if !state.stopping && Instant::now() >= parent_check {
            parent_check = Instant::now() + Duration::from_millis(250);
            match strict_main_is_current(&state.main) {
                Ok(true) => {}
                Ok(false) => {
                    state.parent_lost = true;
                    state.begin_cleanup();
                }
                Err(error) => {
                    // A transient probe error does not authorize an early
                    // cleanup of an otherwise running browser.
                    state.parent_probe_error = Some(bounded_error(&error));
                }
            }
        }
        if state.stopping && peers.is_empty() {
            drop(socket_owner);
            return Ok(());
        }
        if state.parent_lost && state.receipt.is_some() {
            let complete = state.receipt.as_ref().is_some_and(|receipt| receipt.complete);
            drop(socket_owner);
            return if complete { Ok(()) } else {
                Err("guardian parent disappeared; helper generation cleanup is incomplete".into())
            };
        }
        std::thread::sleep(LOOP_INTERVAL);
    }
}

fn bootstrap_main() -> Result<ExactProcessIdentity, String> {
    // SAFETY: getppid is a read-only kernel query. It supplies authority rather
    // than accepting any PID from arguments, environment, or request bodies.
    let parent = unsafe { libc::getppid() };
    if parent <= 1 { return Err("guardian requires its actual Main parent".into()); }
    let mut identity = probe_process_identity(parent as u32)
        .map_err(|error| format!("guardian parent identity: {error}"))?
        .ok_or("guardian parent already exited")?;
    let executable = identity.executable.as_ref().ok_or("guardian parent executable is unreadable")?;
    identity.executable = Some(canonical_file(executable)?);
    if identity.platform_start_key == 0 || unsafe { libc::getppid() } != parent
        || !strict_main_is_current(&identity)? {
        return Err("guardian actual parent changed during bootstrap".into());
    }
    Ok(identity)
}

fn strict_main_is_current(expected: &ExactProcessIdentity) -> Result<bool, String> {
    let Some(live) = probe_process_identity(expected.pid)
        .map_err(|error| format!("guardian Main identity probe: {error}"))? else { return Ok(false); };
    let live_path = live.executable.as_ref().ok_or("guardian Main executable is unreadable")?;
    let live_path = canonical_file(live_path)?;
    Ok(live.pid == expected.pid && live.platform_start_key == expected.platform_start_key
        && expected.platform_start_key != 0 && expected.executable.as_ref() == Some(&live_path))
}

fn authenticate_main(peer: &UnixStream, expected: &ExactProcessIdentity) -> Result<(), String> {
    let pid = peer_pid(peer)?;
    let mut uid = 0;
    let mut gid = 0;
    // SAFETY: the output pointers are writable uid_t/gid_t SDK values.
    if unsafe { libc::getpeereid(peer.as_raw_fd(), &raw mut uid, &raw mut gid) } != 0 {
        return Err(format!("guardian peer credentials: {}", io::Error::last_os_error()));
    }
    if pid <= 1 || pid as u32 != expected.pid
        || uid != unsafe { libc::geteuid() } || gid != unsafe { libc::getegid() }
        || !strict_main_is_current(expected)? {
        return Err("guardian control peer is not the recorded Main generation and executable".into());
    }
    Ok(())
}

fn peer_pid(peer: &UnixStream) -> Result<libc::pid_t, String> {
    let mut pid: libc::pid_t = 0;
    let mut size = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
    // SAFETY: LOCAL_PEERPID initializes one exact SDK pid_t.
    if unsafe { libc::getsockopt(peer.as_raw_fd(), libc::SOL_LOCAL, libc::LOCAL_PEERPID,
        (&raw mut pid).cast(), &raw mut size) } != 0 {
        return Err(format!("guardian kernel peer PID: {}", io::Error::last_os_error()));
    }
    if size as usize != std::mem::size_of::<libc::pid_t>() {
        return Err("guardian kernel peer PID size mismatch".into());
    }
    Ok(pid)
}

fn canonical_file(path: &Path) -> Result<PathBuf, String> {
    let canonical = path.canonicalize().map_err(|error| format!("guardian executable path: {error}"))?;
    if !canonical.is_file() { return Err("guardian executable path must be a regular file".into()); }
    Ok(canonical)
}

fn validate_private_directory(socket: &Path) -> Result<(), String> {
    let parent = socket.parent().filter(|path| !path.as_os_str().is_empty())
        .ok_or("guardian socket requires a private parent directory")?;
    let metadata = fs::symlink_metadata(parent).map_err(|error| format!("guardian directory: {error}"))?;
    if !socket.is_absolute() || socket.file_name().is_none() || !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o777 != 0o700 {
        return Err("guardian socket parent must be an absolute owner-only 0700 directory".into());
    }
    if fs::symlink_metadata(socket).is_ok() {
        return Err("guardian refuses to replace an existing socket path".into());
    }
    Ok(())
}

struct SocketOwner { path: PathBuf, device: u64, inode: u64 }
impl SocketOwner {
    fn capture(path: &Path) -> Result<Self, String> {
        let metadata = fs::symlink_metadata(path).map_err(|error| format!("guardian socket identity: {error}"))?;
        if !metadata.file_type().is_socket() { return Err("guardian bound path is not a socket".into()); }
        Ok(Self { path: path.to_owned(), device: metadata.dev(), inode: metadata.ino() })
    }
}
impl Drop for SocketOwner {
    fn drop(&mut self) {
        if let Ok(metadata) = fs::symlink_metadata(&self.path) {
            if metadata.file_type().is_socket() && metadata.dev() == self.device && metadata.ino() == self.inode {
                let _ = fs::remove_file(&self.path);
            }
        }
    }
}

#[derive(Default)]
struct NativePhase {
    entered: bool,
    running: bool,
    returned: bool,
    deadline: Option<Instant>,
    deadline_unix_ms: Option<u64>,
}
impl NativePhase {
    fn enter(&mut self, now: Instant) -> Result<(), String> {
        if self.returned { return Err("native shutdown already returned".into()); }
        if !self.entered {
            self.entered = true;
            self.running = true;
            self.deadline = Some(now + NATIVE_SHUTDOWN_BUDGET);
            self.deadline_unix_ms = SystemTime::now().duration_since(UNIX_EPOCH).ok()
                .and_then(|value| value.as_millis().checked_add(NATIVE_SHUTDOWN_BUDGET.as_millis()))
                .and_then(|value| u64::try_from(value).ok());
        }
        Ok(())
    }
    fn returned(&mut self) -> Result<(), String> {
        if !self.entered { return Err("native shutdown was never entered".into()); }
        // Commit disarm before forming/writing the acknowledgement. A lost
        // socket response cannot convert a returned native call into a timeout.
        self.returned = true;
        self.running = false;
        self.deadline = None;
        self.deadline_unix_ms = None;
        Ok(())
    }
    fn expired(&self, now: Instant) -> bool {
        self.running && self.deadline.is_some_and(|deadline| now >= deadline)
    }
}

struct ServerState {
    main: ExactProcessIdentity,
    paths: Option<Vec<PathBuf>>,
    pending: HashMap<String, String>,
    seen_nonces: HashSet<String>,
    children: Vec<Arc<DarwinRegisteredChildAuthority>>,
    identity_errors: Vec<String>,
    native: NativePhase,
    cleanup: Option<mpsc::Receiver<TimedCleanupResult>>,
    cleanup_deadline: Option<Instant>,
    cleanup_started: bool,
    receipt: Option<CleanupReceipt>,
    stopping: bool,
    parent_lost: bool,
    parent_probe_error: Option<String>,
    initialization_aborted: bool,
}
impl ServerState {
    fn new(main: ExactProcessIdentity) -> Self {
        Self { main, paths: None, pending: HashMap::new(), seen_nonces: HashSet::new(),
            children: Vec::new(), identity_errors: Vec::new(), native: NativePhase::default(),
            cleanup: None, cleanup_deadline: None, cleanup_started: false, receipt: None, stopping: false,
            parent_lost: false, parent_probe_error: None, initialization_aborted: false }
    }

    fn dispatch(&mut self, peer: &UnixStream, request: Request, rpc_deadline: Instant) -> Reply {
        if !matches!(&request, Request::Register { .. }) {
            if let Err(error) = authenticate_main(peer, &self.main) { return self.reply(Some(&error)); }
            self.parent_probe_error = None;
        }
        if Instant::now() >= rpc_deadline { return self.reply(Some("guardian request deadline expired")); }
        if self.stopping { return self.reply(Some("guardian is stopping")); }
        let result = match request {
            Request::Configure { helper_paths } => self.configure(helper_paths),
            Request::Declare { nonce, role } => self.declare(nonce, role),
            Request::Register { nonce, role } => self.register(peer, nonce, role),
            Request::EnterNativeShutdown => {
                if self.paths.is_none() { Err("guardian must be configured before native entry".into()) }
                else if self.cleanup_started { Err("guardian cleanup has already started".into()) }
                else { self.native.enter(Instant::now()) }
            }
            Request::NativeReturned => self.native.returned().map(|()| self.begin_cleanup()),
            Request::AbortInitialization => {
                if self.native.entered { Err("cannot abort initialization after native shutdown entry".into()) }
                else {
                    self.initialization_aborted = true;
                    self.begin_cleanup();
                    Ok(())
                }
            }
            Request::Status => Ok(()),
            Request::Stop => {
                if self.receipt.as_ref().is_some_and(|receipt| receipt.complete) {
                    self.stopping = true;
                    Ok(())
                } else { Err("guardian cannot stop without complete helper generation absence proof".into()) }
            }
        };
        self.reply(result.as_ref().err().map(String::as_str))
    }

    fn configure(&mut self, paths: Vec<PathBuf>) -> Result<(), String> {
        if self.paths.is_some() { return Err("guardian configuration is one-time".into()); }
        if paths.len() != HELPER_PATH_COUNT { return Err("guardian requires exactly five canonical helper paths".into()); }
        let mut canonical = Vec::with_capacity(HELPER_PATH_COUNT);
        for path in paths {
            let exact = canonical_file(&path)?;
            if !path.is_absolute() || exact != path || canonical.contains(&exact) {
                return Err("guardian helper paths must be distinct exact canonical executable paths".into());
            }
            canonical.push(exact);
        }
        self.paths = Some(canonical);
        Ok(())
    }

    fn declare(&mut self, nonce: String, role: String) -> Result<(), String> {
        if self.paths.is_none() || self.native.entered || self.cleanup_started {
            return Err("guardian helper declarations require configured, pre-shutdown Main".into());
        }
        validate_nonce_role(&nonce, &role)?;
        if self.seen_nonces.len() >= MAX_DECLARATIONS { return Err("guardian declaration inventory limit reached".into()); }
        if !self.seen_nonces.insert(nonce.clone()) { return Err("guardian nonce was already declared".into()); }
        self.pending.insert(nonce, role);
        Ok(())
    }

    fn register(&mut self, peer: &UnixStream, nonce: String, role: String) -> Result<(), String> {
        validate_nonce_role(&nonce, &role)?;
        if self.cleanup_started { return Err("guardian registration is closed during cleanup".into()); }
        let expected_role = self.pending.get(&nonce).ok_or("guardian registration nonce is unknown or already used")?;
        if expected_role != &role { return Err("guardian registration role does not match the declaration".into()); }
        // Consume the launch nonce before identity admission. A failed attempt
        // cannot retry the nonce against another execution generation. It also
        // remains an explicit inventory error, never a disappearance proof.
        self.pending.remove(&nonce);
        let admission = (|| {
            let pid = peer_pid(peer)?;
            if pid <= 1 { return Err("guardian helper kernel peer PID is invalid".into()); }
            let identity = probe_process_identity(pid as u32)
                .map_err(|error| format!("guardian helper identity probe: {error}"))?
                .ok_or("guardian helper already exited")?;
            let executable = canonical_file(identity.executable.as_ref()
                .ok_or("guardian helper executable is unreadable")?)?;
            // CEF may launch several roles through its generic helper. Roles
            // bind the declared launch, while the kernel path must match one
            // of the five exact canonical bundle executables.
            if !self.paths.as_ref().ok_or("guardian is not configured")?.contains(&executable) {
                return Err("guardian helper executable is outside the configured exact paths".into());
            }
            own_registered_child_process(peer, &self.main, &executable)
                .map_err(|error| format!("guardian registered-child identity admission: {error}"))
        })();
        match admission {
            Ok(authority) => {
                if self.children.iter().any(|child| child.identity() == authority.identity()
                    && child.pidversion() == authority.pidversion()) {
                    let error = "guardian execution generation was already registered";
                    record_error(&mut self.identity_errors, error);
                    return Err(error.into());
                }
                self.children.push(Arc::new(authority));
                self.parent_probe_error = None;
                Ok(())
            }
            Err(error) => {
                record_error(&mut self.identity_errors, &error);
                Err(error)
            }
        }
    }

    fn begin_cleanup(&mut self) {
        if self.cleanup_started { return; }
        self.cleanup_started = true;
        self.native.deadline = None;
        self.native.deadline_unix_ms = None;
        let children = self.children.clone();
        let deadline = Instant::now() + HELPER_CLEANUP_BUDGET;
        self.cleanup_deadline = Some(deadline);
        let (sender, receiver) = mpsc::channel();
        self.cleanup = Some(receiver);
        // This worker can block in the exact authority API; the accept loop and
        // NativeReturned disarm remain responsive. Every child uses one shared
        // five-second deadline, including queueing and scheduling time.
        let spawn = std::thread::Builder::new().name("cef-guardian-cleanup".into()).spawn(move || {
            let result = cleanup_generations(children, deadline);
            let _ = sender.send(TimedCleanupResult { finished_at: Instant::now(), result });
        });
        if let Err(error) = spawn {
            self.cleanup = None;
            self.cleanup_deadline = None;
            self.finish_cleanup(CleanupResult { absent: 0,
                errors: vec![bounded_error(&format!("guardian cleanup worker: {error}"))] });
        }
    }

    fn tick(&mut self, now: Instant) {
        if self.native.expired(now) { self.begin_cleanup(); }
        let result = self.cleanup.as_ref().map(mpsc::Receiver::try_recv);
        match result {
            Some(Ok(report)) => {
                let timely = self.cleanup_deadline.is_some_and(|deadline| report.finished_at <= deadline);
                self.cleanup = None;
                self.cleanup_deadline = None;
                if timely { self.finish_cleanup(report.result); }
                else { self.finish_cleanup(CleanupResult { absent: 0,
                    errors: vec!["guardian cleanup result completed after the shared deadline".into()] }); }
                return;
            }
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                self.cleanup = None;
                self.cleanup_deadline = None;
                self.finish_cleanup(CleanupResult { absent: 0,
                    errors: vec!["guardian cleanup worker ended without proof".into()] });
                return;
            }
            _ => {}
        }
        if self.cleanup.is_some() && self.cleanup_deadline.is_some_and(|deadline| now >= deadline) {
            // Native probes or a worker scheduling stall cannot turn five
            // seconds into an unbounded cleanup_running phase. Late worker
            // results cannot replace this explicit incomplete receipt.
            self.cleanup = None;
            self.cleanup_deadline = None;
            self.finish_cleanup(CleanupResult { absent: 0,
                errors: vec!["guardian shared helper cleanup deadline expired".into()] });
            return;
        }
    }

    fn finish_cleanup(&mut self, result: CleanupResult) {
        let mut errors = self.identity_errors.clone();
        for error in result.errors { record_error(&mut errors, &error); }
        self.receipt = Some(CleanupReceipt {
            complete: self.pending.is_empty() && errors.is_empty() && result.absent == self.children.len(),
            generation_absence_only: true,
            registered: self.children.len(), absent: result.absent,
            unresolved_declarations: self.pending.len(), errors,
        });
    }

    fn reply(&self, error: Option<&str>) -> Reply {
        let error = error.or(self.parent_probe_error.as_deref());
        let phase = if self.stopping { "stopping" }
            else if let Some(receipt) = &self.receipt {
                if receipt.complete { "cleanup_complete" } else { "cleanup_incomplete" }
            } else if self.cleanup_started { "cleanup_running" }
            else if self.native.returned { "native_returned" }
            else if self.initialization_aborted { "initialization_aborted" }
            else if self.native.running { "native_running" }
            else if self.paths.is_some() { "ready" } else { "bootstrapping" };
        Reply { status: if error.is_some() { "error" } else { "ok" }.into(), phase: phase.into(),
            native_running: self.native.running, deadline_unix_ms: self.native.deadline_unix_ms,
            receipt: self.receipt.clone(), error: error.map(bounded_error) }
    }
}

fn validate_nonce_role(nonce: &str, role: &str) -> Result<(), String> {
    if nonce.len() != 64 || !nonce.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) {
        return Err("guardian nonce must encode exactly 32 random bytes as lowercase hex".into());
    }
    if !HELPER_ROLES.contains(&role) { return Err("guardian helper role is invalid".into()); }
    Ok(())
}

#[derive(Debug)]
struct CleanupResult { absent: usize, errors: Vec<String> }
#[derive(Debug)]
struct TimedCleanupResult { finished_at: Instant, result: CleanupResult }

fn cleanup_generations(children: Vec<Arc<DarwinRegisteredChildAuthority>>, deadline: Instant) -> CleanupResult {
    let (sender, receiver) = mpsc::channel();
    let total = children.len();
    // Bounded worker count prevents an unusually large admitted inventory from
    // exhausting native threads. Waiting in any worker consumes the same budget.
    let workers = total.min(16);
    let mut batches: Vec<Vec<Arc<DarwinRegisteredChildAuthority>>> = (0..workers).map(|_| Vec::new()).collect();
    for (index, child) in children.into_iter().enumerate() { batches[index % workers].push(child); }
    for batch in batches {
        let worker_sender = sender.clone();
        let spawn = std::thread::Builder::new().name("cef-guardian-generation".into()).spawn(move || {
            for child in batch {
                if Instant::now() >= deadline { break; }
                let result = match child.generation_is_absent() {
                    Ok(true) => Ok(()),
                    Ok(false) => {
                        let remaining = deadline.saturating_duration_since(Instant::now());
                        if remaining.is_zero() {
                            Err(io::Error::new(io::ErrorKind::TimedOut, "shared helper cleanup deadline expired before signal"))
                        } else {
                            child.terminate_and_prove_generation_absent(remaining).map(|_| ())
                        }
                    }
                    Err(error) => Err(error),
                };
                let result = result.map_err(|error| bounded_error(&format!(
                    "registered generation {}:{} absence proof: {error}",
                    child.identity().pid, child.pidversion())));
                let _ = worker_sender.send(result);
            }
        });
        if let Err(error) = spawn {
            // Missing worker reports remain incomplete even if another worker
            // proves every generation in its own disjoint batch absent.
            let _ = sender.send(Err(bounded_error(&format!("guardian generation worker: {error}"))));
        }
    }
    drop(sender);
    let mut result = CleanupResult { absent: 0, errors: Vec::new() };
    let mut reports = 0;
    while reports < total {
        if Instant::now() >= deadline {
            record_error(&mut result.errors, "guardian shared helper cleanup deadline expired");
            break;
        }
        match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(Ok(())) if Instant::now() < deadline => { reports += 1; result.absent += 1; }
            Ok(Ok(())) => {
                record_error(&mut result.errors, "guardian generation absence proof arrived after the shared cleanup deadline");
                break;
            }
            Ok(Err(error)) => { reports += 1; record_error(&mut result.errors, &error); }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                record_error(&mut result.errors, "guardian shared helper cleanup deadline expired");
                break;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                record_error(&mut result.errors, "guardian helper cleanup reports are incomplete");
                break;
            }
        }
    }
    result
}

fn bounded_error(error: &str) -> String {
    if error.len() <= MAX_ERROR_BYTES { return error.into(); }
    let mut end = MAX_ERROR_BYTES;
    while !error.is_char_boundary(end) { end -= 1; }
    error[..end].to_owned()
}
fn record_error(errors: &mut Vec<String>, error: &str) {
    if errors.len() < MAX_ERRORS { errors.push(bounded_error(error)); }
}

struct Peer {
    stream: UnixStream,
    deadline: Instant,
    header: [u8; 4],
    header_read: usize,
    body: Vec<u8>,
    body_read: usize,
    response: Option<Vec<u8>>,
    written: usize,
}
impl Peer {
    fn new(stream: UnixStream) -> io::Result<Self> {
        stream.set_nonblocking(true)?;
        Ok(Self { stream, deadline: Instant::now() + RPC_TIMEOUT, header: [0; 4],
            header_read: 0, body: Vec::new(), body_read: 0, response: None, written: 0 })
    }
    fn read_request(&mut self) -> Result<Option<Request>, String> {
        if self.header_read < self.header.len() {
            if !read_available(&mut self.stream, &mut self.header, &mut self.header_read)? { return Ok(None); }
            let length = frame_length(self.header)?;
            self.body.resize(length, 0);
        }
        if !read_available(&mut self.stream, &mut self.body, &mut self.body_read)? { return Ok(None); }
        serde_json::from_slice(&self.body).map(Some).map_err(|error| format!("invalid guardian JSON: {error}"))
    }
    fn set_response(&mut self, reply: &Reply) -> Result<(), String> {
        let body = serde_json::to_vec(reply).map_err(|error| format!("guardian reply JSON: {error}"))?;
        if body.is_empty() || body.len() > MAX_FRAME_BYTES { return Err("guardian reply frame is out of bounds".into()); }
        let mut frame = Vec::with_capacity(body.len() + 4);
        frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
        frame.extend_from_slice(&body);
        self.response = Some(frame);
        Ok(())
    }
    fn flush_response(&mut self) -> Result<bool, String> {
        let bytes = self.response.as_ref().ok_or("guardian reply is missing")?;
        while self.written < bytes.len() {
            match self.stream.write(&bytes[self.written..]) {
                Ok(0) => return Err("guardian reply peer closed".into()),
                Ok(count) => self.written += count,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(false),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(format!("guardian reply write: {error}")),
            }
        }
        Ok(true)
    }
}

fn frame_length(header: [u8; 4]) -> Result<usize, String> {
    let length = u32::from_be_bytes(header) as usize;
    if length == 0 || length > MAX_FRAME_BYTES { return Err("guardian request frame is out of bounds".into()); }
    Ok(length)
}
fn read_available(stream: &mut UnixStream, buffer: &mut [u8], read: &mut usize) -> Result<bool, String> {
    while *read < buffer.len() {
        match stream.read(&mut buffer[*read..]) {
            Ok(0) => return Err("guardian request peer closed".into()),
            Ok(count) => *read += count,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(false),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(format!("guardian request read: {error}")),
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> ServerState {
        ServerState::new(ExactProcessIdentity { pid: 7, start_time_epoch_seconds: 1,
            platform_start_key: 1, executable: Some(PathBuf::from("/Main")) })
    }

    #[test]
    fn protocol_is_tagged_and_rejects_identity_or_unknown_fields() {
        let encoded = serde_json::to_string(&Request::Declare { nonce: "a".repeat(64), role: "renderer".into() }).unwrap();
        assert!(encoded.contains("\"op\":\"declare\""));
        assert!(serde_json::from_str::<Request>("{\"op\":\"status\",\"pid\":123}").is_err());
        assert!(serde_json::from_str::<Request>("{\"op\":\"kill\",\"pid\":123}").is_err());
        assert!(serde_json::from_str::<Request>("{\"op\":\"register\",\"nonce\":\"a\",\"role\":\"renderer\",\"pid\":123}").is_err());
    }

    #[test]
    fn nonce_is_exact_bounded_hex_and_one_use() {
        assert!(validate_nonce_role(&"a".repeat(64), "renderer").is_ok());
        for invalid in ["a".repeat(63), "a".repeat(65), "A".repeat(64), "g".repeat(64)] {
            assert!(validate_nonce_role(&invalid, "renderer").is_err());
        }
        assert!(validate_nonce_role(&"a".repeat(64), "utility").is_ok());
        assert!(validate_nonce_role(&"a".repeat(64), "gpu-process").is_ok());
        assert!(validate_nonce_role(&"a".repeat(64), "gpu").is_err());
        let mut state = state();
        state.paths = Some(vec![PathBuf::from("/helper"); 5]);
        state.declare("a".repeat(64), "renderer".into()).unwrap();
        assert!(state.declare("a".repeat(64), "renderer".into()).is_err());
        state.pending.remove(&"a".repeat(64));
        assert!(state.declare("a".repeat(64), "renderer".into()).is_err());
    }

    #[test]
    fn repeated_native_entry_does_not_extend_deadline() {
        let mut phase = NativePhase::default();
        let start = Instant::now();
        phase.enter(start).unwrap();
        phase.enter(start + Duration::from_secs(20)).unwrap();
        assert_eq!(phase.deadline, Some(start + NATIVE_SHUTDOWN_BUDGET));
        assert!(!phase.expired(start + Duration::from_secs(29)));
        assert!(phase.expired(start + NATIVE_SHUTDOWN_BUDGET));
    }

    #[test]
    fn native_return_disarms_before_any_acknowledgement() {
        let mut phase = NativePhase::default();
        assert!(phase.returned().is_err());
        let start = Instant::now();
        phase.enter(start).unwrap();
        phase.returned().unwrap();
        assert!(!phase.running);
        assert_eq!(phase.deadline, None);
        assert_eq!(phase.deadline_unix_ms, None);
        assert!(!phase.expired(start + Duration::from_secs(600)));
        phase.returned().unwrap();
        assert!(phase.enter(start).is_err());
    }

    #[test]
    fn unresolved_launch_or_identity_error_is_never_complete_proof() {
        let mut state = state();
        state.pending.insert("a".repeat(64), "renderer".into());
        state.finish_cleanup(CleanupResult { absent: 0, errors: Vec::new() });
        assert!(!state.receipt.as_ref().unwrap().complete);
        assert_eq!(state.receipt.as_ref().unwrap().unresolved_declarations, 1);
        state.pending.clear();
        state.identity_errors.push("identity admission failed".into());
        state.finish_cleanup(CleanupResult { absent: 0, errors: Vec::new() });
        assert!(!state.receipt.as_ref().unwrap().complete);
        state.identity_errors.clear();
        state.finish_cleanup(CleanupResult { absent: 0, errors: Vec::new() });
        let receipt = state.receipt.as_ref().unwrap();
        assert!(receipt.complete && receipt.generation_absence_only);
    }

    #[test]
    fn frame_limits_reject_empty_oversize_and_unbounded_allocations() {
        assert!(frame_length(0_u32.to_be_bytes()).is_err());
        assert_eq!(frame_length((MAX_FRAME_BYTES as u32).to_be_bytes()).unwrap(), MAX_FRAME_BYTES);
        assert!(frame_length((MAX_FRAME_BYTES as u32 + 1).to_be_bytes()).is_err());
        assert!(frame_length(u32::MAX.to_be_bytes()).is_err());
    }

    #[test]
    fn partial_peer_does_not_block_native_timer() {
        let (mut writer, reader) = UnixStream::pair().unwrap();
        let mut peer = Peer::new(reader).unwrap();
        writer.write_all(&[0, 0]).unwrap();
        assert!(peer.read_request().unwrap().is_none());
        let mut phase = NativePhase::default();
        let now = Instant::now();
        phase.enter(now).unwrap();
        assert!(phase.expired(now + NATIVE_SHUTDOWN_BUDGET));
        assert!(peer.read_request().unwrap().is_none());
    }

    #[test]
    fn stalled_cleanup_worker_is_bounded_and_late_proof_cannot_replace_failure() {
        let mut state = state();
        let (sender, receiver) = mpsc::channel();
        let now = Instant::now();
        state.cleanup_started = true;
        state.cleanup = Some(receiver);
        state.cleanup_deadline = Some(now);
        state.tick(now);
        assert_eq!(state.reply(None).phase, "cleanup_incomplete");
        assert!(!state.receipt.as_ref().unwrap().complete);
        assert!(sender.send(TimedCleanupResult { finished_at: now,
            result: CleanupResult { absent: 0, errors: Vec::new() } }).is_err());
        state.tick(now + Duration::from_secs(1));
        assert!(!state.receipt.as_ref().unwrap().complete);
    }

    #[test]
    fn queued_timely_cleanup_proof_survives_next_tick_after_deadline() {
        let mut state = state();
        let (sender, receiver) = mpsc::channel();
        let deadline = Instant::now() + Duration::from_millis(10);
        state.cleanup_started = true;
        state.cleanup = Some(receiver);
        state.cleanup_deadline = Some(deadline);
        sender.send(TimedCleanupResult { finished_at: deadline - Duration::from_millis(1),
            result: CleanupResult { absent: 0, errors: Vec::new() } }).unwrap();
        state.tick(deadline + LOOP_INTERVAL);
        assert!(state.receipt.as_ref().unwrap().complete);
    }

    #[test]
    fn queued_late_cleanup_proof_is_explicitly_incomplete() {
        let mut state = state();
        let (sender, receiver) = mpsc::channel();
        let deadline = Instant::now();
        state.cleanup_started = true;
        state.cleanup = Some(receiver);
        state.cleanup_deadline = Some(deadline);
        sender.send(TimedCleanupResult { finished_at: deadline + Duration::from_millis(1),
            result: CleanupResult { absent: 0, errors: Vec::new() } }).unwrap();
        state.tick(deadline + LOOP_INTERVAL);
        assert!(!state.receipt.as_ref().unwrap().complete);
    }

    #[test]
    fn receipt_and_errors_fit_one_bounded_frame() {
        let mut state = state();
        for text in ["错".repeat(1000), "\u{0001}".repeat(1000), "\\\"".repeat(1000)] {
            state.identity_errors.clear();
            for _ in 0..(MAX_ERRORS + 3) { record_error(&mut state.identity_errors, &text); }
            state.finish_cleanup(CleanupResult { absent: 0, errors: Vec::new() });
            assert_eq!(state.receipt.as_ref().unwrap().errors.len(), MAX_ERRORS);
            let bytes = serde_json::to_vec(&state.reply(Some(&text))).unwrap();
            assert!(bytes.len() < MAX_FRAME_BYTES);
        }
    }
}
