//! Main-process ownership of the CEF guardian and bounded local RPC.
//!
//! The guardian is admitted before CEF is loaded. Its managed-child authority
//! and private socket directory remain one lease until exact cleanup succeeds.

use std::{
    ffi::OsString,
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::{ffi::OsStrExt, fs::{MetadataExt, PermissionsExt}, net::UnixStream},
    },
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex, OnceLock, TryLockError},
    time::{Duration, Instant},
};

use nomi_process_runtime::{
    ChildProcessBuilder, ExactProcessIdentity, ManagedChildProcess,
    capture_child_identity, probe_process_identity,
};
use tempfile::TempDir;

use crate::guardian::{HELPER_CLEANUP_BUDGET, MAX_FRAME_BYTES, RPC_TIMEOUT, Reply, Request};

const STARTUP_BUDGET: Duration = Duration::from_secs(5);
const OWNED_CLEANUP_BUDGET: Duration = Duration::from_secs(5);
const STARTUP_RETRY_DELAY: Duration = Duration::from_millis(20);
const STATUS_RETRY_DELAY: Duration = Duration::from_millis(25);
const HELPER_PATH_COUNT: usize = 5;
const HELPER_ROLES: [&str; 3] = ["renderer", "gpu-process", "utility"];
const ENVIRONMENT_KEYS: [&str; 10] = [
    "PATH", "HOME", "TMPDIR", "LANG", "LC_ALL", "USER", "LOGNAME",
    "__CF_USER_TEXT_ENCODING", "MallocNanoZone", "LC_CTYPE",
];

/// The main process is the only owner allowed to stop and reap this child.
pub struct GuardOwner {
    socket: PathBuf,
    guardian: ExactProcessIdentity,
    rpc_lock: Mutex<()>,
    lifecycle: tokio::sync::Mutex<Lifecycle>,
}

struct Lifecycle {
    lease: Option<GuardianLease>,
    stop_ack: Option<Reply>,
    stopped: Option<Reply>,
}

struct GuardianLease {
    process: Option<ManagedChildProcess>,
    directory: Option<TempDir>,
    guardian: Option<ExactProcessIdentity>,
    configured: bool,
    helper_proven: bool,
}

impl GuardOwner {
    /// Called synchronously on the native main thread with a Tokio handle entered.
    pub fn start(
        helper: &Path,
        expected_main: ExactProcessIdentity,
        helper_paths: Vec<PathBuf>,
    ) -> Result<Arc<Self>, String> {
        tokio::runtime::Handle::try_current()
            .map_err(|_| "CEF guardian requires an entered Tokio runtime".to_owned())?;
        verify_main(&expected_main)?;
        let helper = canonical_executable(helper)?;
        if helper_paths.len() != HELPER_PATH_COUNT {
            return Err("CEF guardian requires all five helper executable paths".to_owned());
        }
        let helper_paths = helper_paths.iter().map(|path| canonical_executable(path))
            .collect::<Result<Vec<_>, _>>()?;
        if helper_paths.first() != Some(&helper) {
            return Err("CEF guardian executable must match the main helper path".to_owned());
        }
        let directory = tempfile::Builder::new().prefix(".nfg-")
            .tempdir_in("/private/tmp")
            .map_err(|error| format!("create CEF guardian private directory: {error}"))?;
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("secure CEF guardian private directory: {error}"))?;
        let socket = directory.path().join("control.sock");
        let mut builder = ChildProcessBuilder::new(&helper);
        builder.env_clear().envs(ENVIRONMENT_KEYS.iter().filter_map(|key| {
            std::env::var_os(key).map(|value| (OsString::from(key), value))
        })).arg("--nomifun-cef-guardian")
            .arg(format!("--socket={}", socket.display()))
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        let deadline = Instant::now() + STARTUP_BUDGET;
        let process = builder.spawn_managed()
            .map_err(|error| format!("start CEF guardian: {error}"))?;
        let mut lease = GuardianLease { process: Some(process), directory: Some(directory),
            guardian: None, configured: false, helper_proven: false };
        let startup = (|| {
            let process = lease.process.as_ref().expect("guardian lease owns child");
            let identity = capture_child_identity(process.child())
                .map_err(|error| format!("capture CEF guardian identity: {error}"))?;
            require_identity_path(&identity, &helper)?;
            if process.id() != Some(identity.pid) {
                return Err("CEF guardian direct-child identity changed".to_owned());
            }
            lease.guardian = Some(identity.clone());
            let configure = Request::Configure { helper_paths };
            loop {
                if Instant::now() >= deadline {
                    return Err("CEF guardian readiness exceeded five seconds".to_owned());
                }
                let attempt = rpc(&socket, Some(&identity), &configure, deadline);
                match attempt {
                    Ok(reply) => {
                        ensure_ok(&reply)?;
                        if reply.phase != "ready" || reply.native_running {
                            return Err("CEF guardian returned invalid readiness state".to_owned());
                        }
                        return Ok(identity);
                    }
                    Err(error) if is_startup_pending(&error) => {
                        std::thread::sleep(STARTUP_RETRY_DELAY.min(
                            deadline.saturating_duration_since(Instant::now())));
                    }
                    Err(error) => return Err(format!("configure CEF guardian: {error}")),
                }
            }
        })();
        match startup {
            Ok(guardian) => {
                lease.configured = true;
                Ok(Arc::new(Self {
                socket,
                guardian,
                rpc_lock: Mutex::new(()),
                lifecycle: tokio::sync::Mutex::new(Lifecycle { lease: Some(lease), stop_ack: None, stopped: None }),
            }))
            }
            Err(error) => {
                // No declaration can exist before Configure succeeds. Settle the
                // owned spawn before returning, retaining the lease on failure.
                let cleanup = settle_startup_failure(&mut lease);
                match cleanup {
                    Ok(()) => Err(error),
                    Err(cleanup) => Err(format!("{error}; guardian cleanup remains owned: {cleanup}")),
                }
            }
        }
    }

    pub fn socket_path(&self) -> &Path { &self.socket }

    /// A cryptographic launch capability is declared before CEF creates a helper.
    pub fn declare(&self, role: &str) -> Result<String, String> {
        validate_role(role)?;
        let nonce = launch_nonce()?;
        let reply = self.request(&Request::Declare { nonce: nonce.clone(), role: role.to_owned() })?;
        ensure_ok(&reply)?;
        Ok(nonce)
    }

    pub fn enter_native(&self) -> Result<Reply, String> {
        self.request(&Request::EnterNativeShutdown)
    }

    pub fn native_returned(&self) -> Result<Reply, String> {
        self.request(&Request::NativeReturned)
    }

    /// Startup failure never pretends that a native shutdown returned.
    pub fn abort_initialization(&self) -> Result<Reply, String> {
        self.request(&Request::AbortInitialization)
    }

    /// Settle a failed startup on the native main thread, outside an async
    /// task, with the caller's Tokio handle entered. This path cannot claim a
    /// native CEF return and never terminates guardian before helper proof.
    pub fn settle_failed_initialization(self: &Arc<Self>) -> Result<(), String> {
        let handle = tokio::runtime::Handle::try_current().map_err(|_| {
            "CEF failed initialization cleanup is unconfirmed: entered Tokio runtime unavailable; guardian lease retained".to_owned()
        })?;
        let deadline = Instant::now() + HELPER_CLEANUP_BUDGET + OWNED_CLEANUP_BUDGET + RPC_TIMEOUT;
        let aborted = self.abort_initialization().map_err(|error| format!(
            "CEF failed initialization cleanup is unconfirmed: {error}; guardian lease retained"))?;
        if aborted.native_running {
            return Err("CEF failed initialization cleanup is unconfirmed: guardian still records native shutdown entry; lease retained".to_owned());
        }
        let cleanup = handle.block_on(async {
            tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), self.stop_and_join()).await
        });
        match cleanup {
            Ok(Ok(reply)) if complete_helper_receipt(&reply) && !reply.native_running => Ok(()),
            Ok(Ok(_)) => Err("CEF failed initialization cleanup is unconfirmed: guardian returned invalid initialization proof".to_owned()),
            Ok(Err(error)) => Err(format!(
                "CEF failed initialization cleanup is unconfirmed: {error}; guardian lease retained")),
            Err(_) => Err("CEF failed initialization cleanup is unconfirmed: shared twelve-second deadline exceeded; guardian shutdown task retains its lease".to_owned()),
        }
    }

    pub fn status(&self) -> Result<Reply, String> { self.request(&Request::Status) }

    fn request(&self, request: &Request) -> Result<Reply, String> {
        let deadline = Instant::now() + RPC_TIMEOUT;
        let _lock = loop {
            match self.rpc_lock.try_lock() {
                Ok(guard) => break guard,
                Err(TryLockError::Poisoned(_)) => return Err("CEF guardian RPC lock poisoned".to_owned()),
                Err(TryLockError::WouldBlock) => {
                    let remaining = remaining(deadline).map_err(|error| error.to_string())?;
                    std::thread::sleep(Duration::from_millis(2).min(remaining));
                }
            }
        };
        let reply = rpc(&self.socket, Some(&self.guardian), request, deadline)
            .map_err(|error| format!("CEF guardian RPC: {error}"))?;
        ensure_ok(&reply)?;
        Ok(reply)
    }

    /// Stop is admitted only after the guardian has proved helper cleanup.
    /// RPC/proof failures keep the child and directory attached for a retry.
    pub async fn stop_and_join(self: &Arc<Self>) -> Result<Reply, String> {
        // The caller may abandon its waiter after Stop. The task itself keeps
        // the authority and directory alive until the actual shutdown settles.
        let owner = Arc::clone(self);
        tokio::spawn(async move { owner.stop_and_join_inner().await }).await
            .map_err(|error| format!("CEF guardian shutdown owner task: {error}"))?
    }

    async fn stop_and_join_inner(self: &Arc<Self>) -> Result<Reply, String> {
        let mut lifecycle = self.lifecycle.lock().await;
        if let Some(reply) = &lifecycle.stopped { return Ok(reply.clone()); }
        if lifecycle.stop_ack.is_none() {
            let deadline = Instant::now() + HELPER_CLEANUP_BUDGET + RPC_TIMEOUT;
            loop {
                let owner = Arc::clone(self);
                let reply = tokio::task::spawn_blocking(move || owner.status()).await
                    .map_err(|error| format!("CEF guardian status worker: {error}"))??;
                if complete_helper_receipt(&reply) { break; }
                if reply.native_running {
                    return Err("CEF guardian native shutdown lacks complete helper cleanup proof".to_owned());
                }
                if reply.phase == "cleanup_incomplete" || Instant::now() >= deadline {
                    return Err("CEF guardian helper cleanup was not proven complete".to_owned());
                }
                tokio::time::sleep(STATUS_RETRY_DELAY).await;
            }
            let owner = Arc::clone(self);
            let reply = tokio::task::spawn_blocking(move || owner.request(&Request::Stop)).await
                .map_err(|error| format!("CEF guardian stop worker: {error}"))??;
            if !complete_helper_receipt(&reply) {
                return Err("CEF guardian Stop omitted complete helper cleanup proof".to_owned());
            }
            let lease = lifecycle.lease.as_mut()
                .ok_or_else(|| "CEF guardian managed-child authority is unavailable".to_owned())?;
            lease.helper_proven = true;
            // Preserve the authenticated terminal acknowledgement before any
            // fallible await; a retry joins this owned child without new IPC.
            lifecycle.stop_ack = Some(reply);
        }
        let lease = lifecycle.lease.as_mut()
            .ok_or_else(|| "CEF guardian managed-child authority is unavailable".to_owned())?;
        lease.shutdown().await?;
        lifecycle.lease.take();
        let reply = lifecycle.stop_ack.as_ref().expect("guardian Stop acknowledgement retained").clone();
        lifecycle.stopped = Some(reply.clone());
        Ok(reply)
    }
}

/// Sandboxed helpers call this before framework loading and execute_process.
pub fn register_helper(socket: &Path, nonce: &str, role: &str) -> Result<(), String> {
    validate_role(role)?;
    if nonce.len() != 64 || !nonce.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) {
        return Err("CEF helper launch nonce is invalid".to_owned());
    }
    let reply = rpc(socket, None, &Request::Register {
        nonce: nonce.to_owned(), role: role.to_owned(),
    }, Instant::now() + RPC_TIMEOUT).map_err(|error| format!("register CEF helper: {error}"))?;
    ensure_ok(&reply)
}

fn launch_nonce() -> Result<String, String> {
    let mut entropy = [0_u8; 32];
    // SAFETY: getentropy initializes exactly this writable 32-byte buffer.
    if unsafe { libc::getentropy(entropy.as_mut_ptr().cast(), entropy.len()) } != 0 {
        return Err(format!("CEF helper launch entropy unavailable: {}", io::Error::last_os_error()));
    }
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut nonce = String::with_capacity(64);
    for byte in entropy {
        nonce.push(HEX[(byte >> 4) as usize] as char);
        nonce.push(HEX[(byte & 15) as usize] as char);
    }
    Ok(nonce)
}

fn validate_role(role: &str) -> Result<(), String> {
    if HELPER_ROLES.contains(&role) { Ok(()) }
    else { Err("CEF helper role is not recognized".to_owned()) }
}

fn ensure_ok(reply: &Reply) -> Result<(), String> {
    if reply.status == "ok" && reply.error.is_none() { Ok(()) }
    else { Err(reply.error.clone().unwrap_or_else(|| "CEF guardian rejected request".to_owned())) }
}

fn complete_helper_receipt(reply: &Reply) -> bool {
    reply.receipt.as_ref().is_some_and(|receipt| receipt.complete
        && receipt.generation_absence_only && receipt.registered == receipt.absent
        && receipt.unresolved_declarations == 0 && receipt.errors.is_empty())
}

fn verify_main(expected: &ExactProcessIdentity) -> Result<(), String> {
    if expected.pid != std::process::id() || expected.platform_start_key == 0 {
        return Err("CEF guardian expected parent is not this main process".to_owned());
    }
    verify_live_identity(expected).map_err(|error| format!("verify CEF guardian parent: {error}"))
}

fn canonical_executable(path: &Path) -> Result<PathBuf, String> {
    let canonical = path.canonicalize().map_err(|error| format!("resolve CEF executable: {error}"))?;
    if !canonical.is_file() { return Err("CEF executable is not a regular file".to_owned()); }
    Ok(canonical)
}

fn require_identity_path(identity: &ExactProcessIdentity, expected: &Path) -> Result<(), String> {
    let observed = identity.executable.as_deref()
        .ok_or_else(|| "CEF guardian executable identity is unavailable".to_owned())?;
    if identity.platform_start_key == 0 || canonical_executable(observed)? != expected {
        return Err("CEF guardian executable or birth identity mismatch".to_owned());
    }
    Ok(())
}

fn verify_live_identity(expected: &ExactProcessIdentity) -> io::Result<()> {
    let live = probe_process_identity(expected.pid)?.ok_or_else(||
        io::Error::new(io::ErrorKind::NotFound, "exact guardian process is absent"))?;
    let expected_path = expected.executable.as_ref().ok_or_else(||
        io::Error::new(io::ErrorKind::PermissionDenied, "expected executable is unavailable"))?.canonicalize()?;
    let actual_path = live.executable.as_ref().ok_or_else(||
        io::Error::new(io::ErrorKind::PermissionDenied, "actual executable is unavailable"))?.canonicalize()?;
    if expected.pid != live.pid || expected.platform_start_key == 0
        || expected.platform_start_key != live.platform_start_key || expected_path != actual_path {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "exact guardian process identity changed"));
    }
    Ok(())
}

fn rpc(socket: &Path, expected: Option<&ExactProcessIdentity>, request: &Request, outer_deadline: Instant) -> io::Result<Reply> {
    let deadline = outer_deadline.min(Instant::now() + RPC_TIMEOUT);
    verify_socket_path(socket).map_err(|error| stage_error("socket_path", error))?;
    let mut stream = connect_until(socket, deadline).map_err(|error| stage_error("connect", error))?;
    let admitted = verify_peer(&stream, expected, true)
        .map_err(|error| stage_error("authenticate_before_request", error))?;
    let payload = serde_json::to_vec(request).map_err(|error| stage_error("encode_request", io::Error::other(error)))?;
    if payload.is_empty() || payload.len() > MAX_FRAME_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "guardian request frame exceeds limit"));
    }
    write_until(&mut stream, &(payload.len() as u32).to_be_bytes(), deadline)
        .map_err(|error| stage_error("write_request_length", error))?;
    write_until(&mut stream, &payload, deadline).map_err(|error| stage_error("write_request_body", error))?;
    let mut length = [0_u8; 4];
    read_until(&mut stream, &mut length, deadline).map_err(|error| stage_error("read_reply_length", error))?;
    let length = u32::from_be_bytes(length) as usize;
    if length == 0 || length > MAX_FRAME_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "guardian reply frame exceeds limit"));
    }
    let mut payload = vec![0_u8; length];
    read_until(&mut stream, &mut payload, deadline).map_err(|error| stage_error("read_reply_body", error))?;
    // UNIX connections cannot change their peer. After the server closes its
    // one-shot stream, Darwin no longer exposes LOCAL_PEERPID even though a
    // valid reply remains buffered. Retain the admission binding, then reprove
    // its exact process generation for every nonterminal reply.
    verify_reply_peer(&stream, &admitted, expected, request)
        .map_err(|error| stage_error("authenticate_after_reply", error))?;
    let reply = serde_json::from_slice(&payload).map_err(|error| stage_error("decode_reply", io::Error::other(error)))?;
    remaining(deadline).map_err(|error| stage_error("deadline_after_reply", error))?;
    Ok(reply)
}

fn verify_socket_path(socket: &Path) -> io::Result<()> {
    use std::os::unix::fs::FileTypeExt;
    if !socket.is_absolute() {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "guardian socket must be absolute"));
    }
    let parent = socket.parent().ok_or_else(|| io::Error::other("guardian socket parent unavailable"))?;
    let directory = std::fs::symlink_metadata(parent)?;
    // SAFETY: geteuid is a read-only process credential query.
    let uid = unsafe { libc::geteuid() };
    if !directory.is_dir() || directory.uid() != uid || directory.mode() & 0o777 != 0o700
        || parent.canonicalize()? != parent {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "guardian socket directory is not private"));
    }
    let metadata = std::fs::symlink_metadata(socket)?;
    if !metadata.file_type().is_socket() || metadata.uid() != uid {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "guardian endpoint is not an owned socket"));
    }
    Ok(())
}

struct KernelPeerBinding {
    socket: libc::c_int,
    pid: u32,
    uid: libc::uid_t,
}

fn verify_peer(stream: &UnixStream, expected: Option<&ExactProcessIdentity>, require_live: bool) -> io::Result<KernelPeerBinding> {
    let mut pid: libc::pid_t = 0;
    let mut length = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
    let mut uid: libc::uid_t = 0;
    let mut gid: libc::gid_t = 0;
    // SAFETY: buffers have the exact SDK socket-option and getpeereid types.
    if unsafe { libc::getsockopt(stream.as_raw_fd(), libc::SOL_LOCAL, libc::LOCAL_PEERPID,
        (&raw mut pid).cast(), &raw mut length) } != 0 {
        return Err(stage_error("kernel_peer_pid", io::Error::last_os_error()));
    }
    if length as usize != std::mem::size_of::<libc::pid_t>() || pid <= 1 {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "guardian kernel peer identity unavailable"));
    }
    if unsafe { libc::getpeereid(stream.as_raw_fd(), &raw mut uid, &raw mut gid) } != 0 {
        return Err(stage_error("kernel_peer_credentials", io::Error::last_os_error()));
    }
    // SAFETY: geteuid is a read-only process credential query.
    if uid != unsafe { libc::geteuid() } {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "guardian socket peer belongs to another user"));
    }
    if let Some(expected) = expected {
        if pid as u32 != expected.pid {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "guardian socket peer is not the owned child"));
        }
        if require_live { verify_live_identity(expected).map_err(|error| stage_error("exact_process_identity", error))?; }
    }
    Ok(KernelPeerBinding { socket: stream.as_raw_fd(), pid: pid as u32, uid })
}

fn verify_reply_peer(stream: &UnixStream, admitted: &KernelPeerBinding,
    expected: Option<&ExactProcessIdentity>, request: &Request) -> io::Result<()> {
    // SAFETY: geteuid is a read-only process credential query. The descriptor
    // is still owned by the same local UnixStream; RPC never replaces it.
    if stream.as_raw_fd() != admitted.socket || admitted.uid != unsafe { libc::geteuid() } {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "guardian reply lost its admitted kernel connection binding"));
    }
    if let Some(expected) = expected {
        if admitted.pid != expected.pid {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "guardian reply peer is not the owned child"));
        }
        // Stop legitimately exits after writing its authenticated terminal
        // acknowledgement. All other replies still require live birth/exe.
        if !matches!(request, Request::Stop) {
            verify_live_identity(expected).map_err(|error| stage_error("exact_process_identity", error))?;
        }
    }
    Ok(())
}

fn connect_until(socket: &Path, deadline: Instant) -> io::Result<UnixStream> {
    remaining(deadline)?;
    let path = socket.as_os_str().as_bytes();
    // SAFETY: a zeroed sockaddr_un is valid before its fields are populated.
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if path.is_empty() || path.contains(&0) || path.len() >= address.sun_path.len() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "guardian socket path exceeds native limit"));
    }
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (slot, byte) in address.sun_path.iter_mut().zip(path) { *slot = *byte as libc::c_char; }
    let size = std::mem::offset_of!(libc::sockaddr_un, sun_path) + path.len() + 1;
    address.sun_len = size as u8;
    // SAFETY: socket returns a fresh descriptor; OwnedFd is its single owner.
    let raw = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if raw < 0 { return Err(io::Error::last_os_error()); }
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    // SAFETY: fcntl operates only on the owned, live descriptor.
    if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } < 0
        || unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: address is initialized and size includes its terminated path.
    if unsafe { libc::connect(fd.as_raw_fd(), (&raw const address).cast(), size as libc::socklen_t) } != 0 {
        let error = io::Error::last_os_error();
        if !matches!(error.raw_os_error(), Some(libc::EINPROGRESS | libc::EAGAIN)) { return Err(error); }
        loop {
            let remaining = remaining(deadline)?;
            let mut pollfd = libc::pollfd { fd: fd.as_raw_fd(), events: libc::POLLOUT, revents: 0 };
            let millis = remaining.as_millis().max(1).min(libc::c_int::MAX as u128) as libc::c_int;
            // SAFETY: pollfd is writable for one poll descriptor.
            let result = unsafe { libc::poll(&raw mut pollfd, 1, millis) };
            if result < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted { continue; }
                return Err(error);
            }
            if result == 0 { return Err(timeout_error()); }
            let mut error: libc::c_int = 0;
            let mut length = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
            // SAFETY: SO_ERROR initializes one SDK c_int.
            if unsafe { libc::getsockopt(fd.as_raw_fd(), libc::SOL_SOCKET, libc::SO_ERROR,
                (&raw mut error).cast(), &raw mut length) } != 0 { return Err(io::Error::last_os_error()); }
            if length as usize != std::mem::size_of::<libc::c_int>() { return Err(io::Error::other("guardian socket error size mismatch")); }
            if error != 0 { return Err(io::Error::from_raw_os_error(error)); }
            break;
        }
    }
    // Keep nonblocking mode: changing socket timeouts after the peer closes
    // is rejected by Darwin even while a complete reply remains buffered.
    Ok(UnixStream::from(fd))
}

fn read_until(stream: &mut UnixStream, mut bytes: &mut [u8], deadline: Instant) -> io::Result<()> {
    stream.set_nonblocking(true).map_err(|error| stage_error("set_read_nonblocking", error))?;
    while !bytes.is_empty() {
        remaining(deadline).map_err(|error| stage_error("read_deadline", error))?;
        match stream.read(bytes) {
            Ok(0) => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "guardian reply truncated")),
            Ok(count) => { bytes = &mut bytes[count..]; }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                wait_for_io(stream.as_raw_fd(), libc::POLLIN, deadline)
                    .map_err(|error| stage_error("poll_read", error))?;
            }
            Err(error) => return Err(stage_error("read_syscall", error)),
        }
    }
    remaining(deadline).map(|_| ())
}

fn write_until(stream: &mut UnixStream, mut bytes: &[u8], deadline: Instant) -> io::Result<()> {
    stream.set_nonblocking(true).map_err(|error| stage_error("set_write_nonblocking", error))?;
    while !bytes.is_empty() {
        remaining(deadline).map_err(|error| stage_error("write_deadline", error))?;
        match stream.write(bytes) {
            Ok(0) => return Err(io::Error::new(io::ErrorKind::WriteZero, "guardian request truncated")),
            Ok(count) => bytes = &bytes[count..],
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                wait_for_io(stream.as_raw_fd(), libc::POLLOUT, deadline)
                    .map_err(|error| stage_error("poll_write", error))?;
            }
            Err(error) => return Err(stage_error("write_syscall", error)),
        }
    }
    remaining(deadline).map(|_| ())
}

fn wait_for_io(socket: libc::c_int, events: libc::c_short, deadline: Instant) -> io::Result<()> {
    loop {
        let budget = remaining(deadline)?;
        let mut descriptor = libc::pollfd { fd: socket, events, revents: 0 };
        let millis = budget.as_nanos().div_ceil(1_000_000).min(libc::c_int::MAX as u128) as libc::c_int;
        // SAFETY: descriptor is writable for the single advertised poll item.
        let ready = unsafe { libc::poll(&raw mut descriptor, 1, millis) };
        if ready < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted { continue; }
            return Err(error);
        }
        if ready == 0 { return Err(timeout_error()); }
        remaining(deadline)?;
        if descriptor.revents & libc::POLLNVAL != 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "guardian poll descriptor is invalid"));
        }
        if descriptor.revents & (events | libc::POLLHUP | libc::POLLERR) != 0 {
            // HUP may accompany a complete queued reply. Let the next read
            // drain it; only an actual zero-byte read proves truncation.
            return Ok(());
        }
    }
}

fn remaining(deadline: Instant) -> io::Result<Duration> {
    deadline.checked_duration_since(Instant::now()).filter(|value| !value.is_zero())
        .ok_or_else(timeout_error)
}

fn timeout_error() -> io::Error { io::Error::new(io::ErrorKind::TimedOut, "guardian RPC deadline exceeded") }

fn is_startup_pending(error: &io::Error) -> bool {
    let Some(stage) = error.get_ref().and_then(|source| source.downcast_ref::<RpcStageError>()) else { return false; };
    matches!(stage.stage, "socket_path" | "connect")
        && matches!(stage.error.raw_os_error(), Some(libc::ENOENT | libc::ECONNREFUSED))
}

#[derive(Debug)]
struct RpcStageError { stage: &'static str, error: io::Error }

impl std::fmt::Display for RpcStageError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.stage, self.error)
    }
}

impl std::error::Error for RpcStageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> { Some(&self.error) }
}

fn stage_error(stage: &'static str, error: io::Error) -> io::Error {
    io::Error::new(error.kind(), RpcStageError { stage, error })
}

impl GuardianLease {
    async fn shutdown(&mut self) -> Result<(), String> {
        if self.configured && !self.helper_proven {
            return Err("CEF guardian helper cleanup proof is unavailable; lease retained".to_owned());
        }
        let process = self.process.as_mut()
            .ok_or_else(|| "CEF guardian managed child is unavailable".to_owned())?;
        tokio::time::timeout(OWNED_CLEANUP_BUDGET, process.shutdown()).await
            .map_err(|_| "CEF guardian owned cleanup exceeded five seconds".to_owned())?
            .map_err(|error| format!("CEF guardian owned cleanup: {error}"))?;
        self.process.take();
        self.directory.take();
        Ok(())
    }

    fn authorize_drop_cleanup(&mut self) -> Result<(), String> {
        if !self.configured || self.helper_proven { return Ok(()); }
        let socket = self.directory.as_ref()
            .ok_or_else(|| "CEF guardian private directory is unavailable".to_owned())?
            .path().join("control.sock");
        let guardian = self.guardian.as_ref()
            .ok_or_else(|| "CEF guardian exact identity is unavailable".to_owned())?;
        let reply = rpc(&socket, Some(guardian), &Request::Status, Instant::now() + RPC_TIMEOUT)
            .map_err(|error| format!("CEF guardian dropped owner status: {error}"))?;
        ensure_ok(&reply)?;
        if !complete_helper_receipt(&reply) {
            return Err("CEF guardian dropped owner lacks complete helper cleanup; lease retained".to_owned());
        }
        let reply = rpc(&socket, Some(guardian), &Request::Stop, Instant::now() + RPC_TIMEOUT)
            .map_err(|error| format!("CEF guardian dropped owner Stop: {error}"))?;
        ensure_ok(&reply)?;
        if !complete_helper_receipt(&reply) {
            return Err("CEF guardian dropped owner Stop lacks helper cleanup proof".to_owned());
        }
        self.helper_proven = true;
        Ok(())
    }
}

fn settle_startup_failure(lease: &mut GuardianLease) -> Result<(), String> {
    // Startup occurs outside async runtime execution on the native main thread.
    // A dedicated cleanup runtime also keeps this path safe if its caller changes.
    let process = lease.process.take();
    let directory = lease.directory.take();
    let guardian = lease.guardian.take();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    dispatch_cleanup(GuardianLease { process, directory, guardian,
        configured: lease.configured, helper_proven: lease.helper_proven }, Some(sender));
    receiver.recv_timeout(OWNED_CLEANUP_BUDGET + Duration::from_secs(1))
        .map_err(|_| "guardian cleanup worker retains its lease".to_owned())?
}

// Irrecoverable OS/runtime errors prove nothing. Preserve authority and its
// private path rather than deleting them or inventing a successful receipt.
static RETAINED_LEASES: OnceLock<Mutex<Vec<GuardianLease>>> = OnceLock::new();

fn retain_lease(lease: GuardianLease) {
    RETAINED_LEASES.get_or_init(|| Mutex::new(Vec::new())).lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner).push(lease);
}

fn dispatch_cleanup(lease: GuardianLease, result: Option<std::sync::mpsc::SyncSender<Result<(), String>>>) {
    let pending = Arc::new(Mutex::new(Some((lease, result))));
    let worker = Arc::clone(&pending);
    let spawned = std::thread::Builder::new().name("nomifun-cef-guardian-cleanup".to_owned()).spawn(move || {
        let (mut lease, result) = worker.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
            .take().expect("guardian cleanup job owned once");
        let cleanup = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            lease.authorize_drop_cleanup()?;
            match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(runtime) => runtime.block_on(lease.shutdown()),
                Err(error) => Err(format!("CEF guardian cleanup runtime: {error}")),
            }
        })).unwrap_or_else(|_| Err("CEF guardian cleanup worker panicked".to_owned()));
        if let Err(error) = &cleanup {
            tracing::error!(%error, "CEF guardian cleanup failed; retaining managed child and private directory");
            retain_lease(lease);
        }
        if let Some(sender) = result { let _ = sender.send(cleanup); }
    });
    if let Err(error) = spawned {
        let (lease, result) = pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
            .take().expect("failed cleanup thread retains its job");
        retain_lease(lease);
        tracing::error!(%error, "CEF guardian cleanup worker unavailable; retaining managed child and private directory");
        if let Some(sender) = result { let _ = sender.send(Err(format!("CEF guardian cleanup thread: {error}"))); }
    }
}

impl Drop for GuardianLease {
    fn drop(&mut self) {
        if self.process.is_none() { return; }
        let lease = Self { process: self.process.take(), directory: self.directory.take(),
            guardian: self.guardian.take(), configured: self.configured, helper_proven: self.helper_proven };
        dispatch_cleanup(lease, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guardian::CleanupReceipt;

    #[test]
    fn launch_capabilities_are_random_lowercase_32_byte_hex() {
        let first = launch_nonce().expect("Darwin cryptographic entropy");
        let second = launch_nonce().expect("Darwin cryptographic entropy");
        assert_eq!(first.len(), 64);
        assert!(first.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
        assert_ne!(first, second);
        for role in ["renderer", "gpu-process", "utility"] { assert!(validate_role(role).is_ok()); }
        for role in ["main", "gpu", "plugin", "alerts", "", "unknown"] { assert!(validate_role(role).is_err()); }
    }

    #[test]
    fn partial_reads_share_one_deadline() {
        let (mut client, mut peer) = UnixStream::pair().unwrap();
        let writer = std::thread::spawn(move || {
            for _ in 0..32 {
                if peer.write_all(&[1]).is_err() { break; }
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        let started = Instant::now();
        let mut payload = [0_u8; 32];
        let error = read_until(&mut client, &mut payload, started + Duration::from_millis(60)).unwrap_err();
        assert!(matches!(error.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock));
        assert!(started.elapsed() < Duration::from_millis(250), "partial frames must not renew the deadline");
        drop(client);
        writer.join().unwrap();
    }

    #[test]
    fn oversized_reply_is_rejected_before_body_read() {
        use std::os::unix::net::UnixListener;
        let directory = tempfile::Builder::new().prefix(".nfg-test-").tempdir_in("/private/tmp").unwrap();
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let socket = directory.path().join("control.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = std::thread::spawn(move || {
            let (mut peer, _) = listener.accept().unwrap();
            let mut prefix = [0_u8; 4];
            peer.read_exact(&mut prefix).unwrap();
            let mut body = vec![0; u32::from_be_bytes(prefix) as usize];
            peer.read_exact(&mut body).unwrap();
            peer.write_all(&((MAX_FRAME_BYTES + 1) as u32).to_be_bytes()).unwrap();
            // No body follows: accepting this prefix would fail as truncated,
            // demonstrating why the size check must precede allocation/read.
        });
        let identity = probe_process_identity(std::process::id()).unwrap().unwrap();
        let error = rpc(&socket, Some(&identity), &Request::Status, Instant::now() + RPC_TIMEOUT).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("frame exceeds limit"));
        server.join().unwrap();
    }

    #[test]
    fn queued_reply_body_survives_actual_peer_close() {
        use std::os::unix::net::UnixListener;
        let directory = tempfile::Builder::new().prefix(".nfg-test-").tempdir_in("/private/tmp").unwrap();
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let socket = directory.path().join("control.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let expected = probe_process_identity(std::process::id()).unwrap().unwrap();
        let reply = Reply { status: "ok".to_owned(), phase: "ready".to_owned(),
            native_running: false, deadline_unix_ms: None, receipt: None, error: None };
        let reply_payload = serde_json::to_vec(&reply).unwrap();
        let outgoing_reply = reply_payload.clone();
        let (close_sender, close_receiver) = std::sync::mpsc::sync_channel(1);
        let (closed_sender, closed_receiver) = std::sync::mpsc::sync_channel(1);
        let deadline = Instant::now() + RPC_TIMEOUT;
        let server = std::thread::spawn(move || {
            let (mut peer, _) = listener.accept().unwrap();
            let mut prefix = [0_u8; 4];
            read_until(&mut peer, &mut prefix, deadline).unwrap();
            let mut request = vec![0; u32::from_be_bytes(prefix) as usize];
            read_until(&mut peer, &mut request, deadline).unwrap();
            assert_eq!(serde_json::from_slice::<Request>(&request).unwrap(), Request::Status);
            write_until(&mut peer, &(outgoing_reply.len() as u32).to_be_bytes(), deadline).unwrap();
            write_until(&mut peer, &outgoing_reply, deadline).unwrap();
            close_receiver.recv_timeout(remaining(deadline).unwrap()).unwrap();
            drop(peer);
            closed_sender.send(()).unwrap();
        });
        let mut client = connect_until(&socket, deadline).unwrap();
        let admitted = verify_peer(&client, Some(&expected), true).unwrap();
        let request = serde_json::to_vec(&Request::Status).unwrap();
        write_until(&mut client, &(request.len() as u32).to_be_bytes(), deadline).unwrap();
        write_until(&mut client, &request, deadline).unwrap();
        let mut prefix = [0_u8; 4];
        read_until(&mut client, &mut prefix, deadline).unwrap();
        assert_eq!(u32::from_be_bytes(prefix) as usize, reply_payload.len());
        close_sender.send(()).unwrap();
        closed_receiver.recv_timeout(remaining(deadline).unwrap()).unwrap();
        let mut body = vec![0; reply_payload.len()];
        read_until(&mut client, &mut body, deadline).expect("a queued valid body must survive the actual peer close");
        assert_eq!(body, reply_payload);
        verify_reply_peer(&client, &admitted, Some(&expected), &Request::Status).unwrap();
        let mut changed_birth = expected.clone();
        changed_birth.platform_start_key = changed_birth.platform_start_key.checked_add(1).unwrap();
        let rejected = verify_reply_peer(&client, &admitted, Some(&changed_birth), &Request::Status).unwrap_err();
        assert_eq!(rejected.kind(), io::ErrorKind::PermissionDenied,
            "reply completion must still reject a changed exact birth identity");
        server.join().unwrap();
    }

    #[test]
    fn helper_proof_does_not_claim_native_return() {
        let mut reply = Reply { status: "ok".to_owned(), phase: "cleanup_complete".to_owned(),
            native_running: true, deadline_unix_ms: None, error: None,
            receipt: Some(CleanupReceipt { complete: true, generation_absence_only: true,
                registered: 2, absent: 2, unresolved_declarations: 0, errors: Vec::new() }) };
        assert!(complete_helper_receipt(&reply));
        assert!(reply.native_running, "exact helper absence does not prove the native CEF call returned");
        reply.receipt.as_mut().unwrap().unresolved_declarations = 1;
        assert!(!complete_helper_receipt(&reply));
        reply.receipt.as_mut().unwrap().unresolved_declarations = 0;
        reply.receipt.as_mut().unwrap().absent = 1;
        assert!(!complete_helper_receipt(&reply));
        reply.receipt.as_mut().unwrap().absent = 2;
        reply.receipt.as_mut().unwrap().errors.push("probe unavailable".to_owned());
        assert!(!complete_helper_receipt(&reply));
    }
}
