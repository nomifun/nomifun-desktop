//! A per-owner polkit runner transports the existing supervisor over inherited
//! pipes. There is no listening service, password field, or authorization DB.
use std::{
    collections::HashMap,
    ffi::CStr,
    io::{self, Read, Write},
    os::{fd::AsRawFd, unix::{fs::MetadataExt, process::ExitStatusExt}},
    process::{Child, ChildStdin, Command, ExitStatus, Stdio},
    sync::{Arc, Condvar, Mutex, atomic::{AtomicBool, AtomicU64, Ordering}},
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use tokio::sync::{oneshot, watch};

use crate::{Runner, SystemExecutionError, effective_uid};

const HELPER: &str = "/usr/libexec/nomifun-system-runner";
const MAX_REQUEST: usize = 1024 * 1024;
// Product output retains up to 16 MiB. Control characters expand when encoded
// as JSON text; include that expansion and metadata without truncating facts.
const MAX_FRAME: usize = 192 * 1024 * 1024;
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(180);
const PROTOCOL: u64 = 1;
const AUTHENTICATION_REAP_GRACE: Duration = Duration::from_millis(250);
const PUBLIC_DIAGNOSTIC_BYTES: usize = 2048;
const RETAINED_DIAGNOSTIC_BYTES: usize = 64 * 1024;
const BUSINESS_RPC_SLOTS: usize = 128;

/// Bound only ordinary native dispatch, after the inherited-pipe reader has
/// retained the complete request. Exact Stop and original-owner observation
/// remain receivable while those calls wait for their real results.
struct RpcDispatchSlots {
    active: Mutex<usize>,
    available: Condvar,
}
struct RpcDispatchPermit<'a>(&'a RpcDispatchSlots);
impl RpcDispatchSlots {
    fn new() -> Self { Self { active: Mutex::new(0), available: Condvar::new() } }

    fn admit(&self, op: &str, interrupted: impl Fn() -> bool) -> io::Result<Option<RpcDispatchPermit<'_>>> {
        if matches!(op, "cancel" | "terminate" | "timeout" | "interrupt" | "quiesce" | "shutdown"
            | "release" | "stop_facility" | "observe" | "terminal" | "wait_terminal"
            | "output_archive" | "release_terminal") { return Ok(None); }
        let mut active = self.active.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            if interrupted() { return Err(io::Error::new(io::ErrorKind::Interrupted, "original native RPC admission closed before dispatch")); }
            if *active < BUSINESS_RPC_SLOTS {
                *active += 1;
                return Ok(Some(RpcDispatchPermit(self)));
            }
            active = self.available.wait_timeout(active, Duration::from_millis(25))
                .unwrap_or_else(std::sync::PoisonError::into_inner).0;
        }
    }
}
impl Drop for RpcDispatchPermit<'_> {
    fn drop(&mut self) {
        let mut active = self.0.active.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        *active = active.checked_sub(1).expect("native business dispatch permit returned once");
        self.0.available.notify_one();
    }
}

fn dispatch_received_rpc(runner: &Runner, admission_lock: Arc<tokio::sync::RwLock<()>>,
    slots: &RpcDispatchSlots, closing: &AtomicBool, request: Value) -> io::Result<Value> {
    let _slot = slots.admit(request["op"].as_str().unwrap_or(""), || closing.load(Ordering::Acquire))?;
    // Do not wait for a start/whole-client fence on the frame reader. A user
    // cancel behind that fence must still reach its exact native process.
    let _guard = runner.runtime.block_on(admission(admission_lock, &request));
    if closing.load(Ordering::Acquire) {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "original native pipe closed before dispatch"));
    }
    Ok(runner.call(request.to_string().as_bytes()))
}

fn unknown(message: impl Into<String>) -> SystemExecutionError {
    SystemExecutionError {
        code: "system_result_unknown".into(), message: message.into(),
        details: json!({"user_code_not_started":false,"cleanup_proven":false}),
    }
}

fn not_started(code: &str, message: impl Into<String>) -> SystemExecutionError {
    SystemExecutionError {
        code: code.into(), message: message.into(),
        details: json!({"user_code_not_started":true,"cleanup_proven":true}),
    }
}

fn wait_for_authentication_exit(child: &mut Child, grace: Duration) -> Option<ExitStatus> {
    let deadline = Instant::now() + grace;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Err(_) => return None,
            Ok(None) if Instant::now() >= deadline => return None,
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

fn reap_failed_authentication(child: &mut Child, timed_out: bool) -> Option<ExitStatus> {
    // Closing stdout can precede the native 126/127 exit. Preserve that real
    // status instead of racing it with SIGKILL and reporting a generic EOF.
    if !timed_out {
        if let Some(status) = wait_for_authentication_exit(child, AUTHENTICATION_REAP_GRACE) {
            return Some(status);
        }
    }
    match child.try_wait() {
        Ok(Some(status)) => return Some(status),
        Err(_) => return None,
        Ok(None) => { let _ = child.kill(); }
    }
    // A failed kill or an uninterruptible child cannot block the reply. The
    // original Child remains owned and its final drop schedules real reaping.
    wait_for_authentication_exit(child, AUTHENTICATION_REAP_GRACE)
}

fn drain_authentication_stderr(reader: &mut impl Read, retained: &Mutex<Vec<u8>>) {
    let mut buffer = [0; 4096];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(length) => {
                let mut bytes = retained.lock().expect("system diagnostic mutex");
                let retain = length.min(RETAINED_DIAGNOSTIC_BYTES.saturating_sub(bytes.len()));
                bytes.extend_from_slice(&buffer[..retain]);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                let message = format!("\nNative authentication stderr read failed: {error}");
                let mut bytes = retained.lock().expect("system diagnostic mutex");
                // Preserve the read error inside the same bounded diagnostic.
                let message = message.as_bytes();
                bytes.truncate(RETAINED_DIAGNOSTIC_BYTES.saturating_sub(message.len()));
                let retain = message.len().min(RETAINED_DIAGNOSTIC_BYTES.saturating_sub(bytes.len()));
                bytes.extend_from_slice(&message[..retain]);
                break;
            }
        }
    }
}

fn write_frame(writer: &mut impl Write, value: &Value, limit: usize) -> io::Result<()> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > limit {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Native RPC frame exceeds its envelope"));
    }
    writer.write_all(&(bytes.len() as u32).to_be_bytes())?;
    writer.write_all(&bytes)?;
    writer.flush()
}

fn read_frame(reader: &mut impl Read, limit: usize) -> io::Result<Option<Value>> {
    let mut header = [0; 4];
    match reader.read(&mut header[..1])? {
        0 => return Ok(None),
        _ => reader.read_exact(&mut header[1..])?,
    }
    let length = u32::from_be_bytes(header) as usize;
    if length == 0 || length > limit {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid native RPC frame length"));
    }
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes)?;
    Ok(Some(serde_json::from_slice(&bytes)?))
}

struct ApprovalReader<R> { inner: R, deadline: Instant }
impl<R: Read + AsRawFd> Read for ApprovalReader<R> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        loop {
            let remaining = self.deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() { return Err(io::ErrorKind::TimedOut.into()); }
            let mut descriptor = libc::pollfd { fd: self.inner.as_raw_fd(), events: libc::POLLIN, revents: 0 };
            let result = unsafe { libc::poll(&mut descriptor, 1, remaining.as_millis().min(i32::MAX as u128) as i32) };
            if result == 0 { return Err(io::ErrorKind::TimedOut.into()); }
            if result < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted { continue; }
                return Err(error);
            }
            return self.inner.read(bytes);
        }
    }
}

enum Admission {
    Start { _guard: tokio::sync::OwnedRwLockReadGuard<()> },
    Fence { _guard: tokio::sync::OwnedRwLockWriteGuard<()> },
    None,
}

async fn admission(lock: Arc<tokio::sync::RwLock<()>>, request: &Value) -> Admission {
    match request["op"].as_str() {
        Some("start" | "acquire_read") => Admission::Start { _guard: lock.read_owned().await },
        Some("quiesce" | "shutdown") => Admission::Fence { _guard: lock.write_owned().await },
        _ => Admission::None,
    }
}

struct Pending {
    reply: oneshot::Sender<Result<Value, SystemExecutionError>>,
    // A cancelled waiting future does not release the dispatch fence. The
    // original response/connection loss settles the admitted request instead.
    _admission: Admission,
}

struct ClientInner {
    stdin: Mutex<Option<ChildStdin>>,
    child: Arc<Mutex<Option<Child>>>,
    pending: Mutex<HashMap<u64, Pending>>,
    admission: Arc<tokio::sync::RwLock<()>>,
    sequence: AtomicU64,
    approved: AtomicBool,
}

impl std::fmt::Debug for ClientInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinuxSystemTransport").finish_non_exhaustive()
    }
}

impl Drop for ClientInner {
    fn drop(&mut self) {
        // EOF is the runner's shutdown fence. It invokes supervisor.shutdown
        // before exiting; the existing Linux watchdog also covers host death.
        self.stdin.get_mut().expect("system stdin mutex").take();
        if let Some(child) = self.child.lock().expect("system child mutex").as_mut() {
            // During authentication pkexec still has the caller's real UID.
            // No operation was dispatched; end the native prompt on cancel.
            if !self.approved.load(Ordering::Acquire) { let _ = child.kill(); }
        }
        // Its dedicated creating thread owns the Child until actual exit and
        // reaping. Do not retire that parent while the root runner is cleaning.
    }
}

#[derive(Debug)]
pub(crate) struct LinuxClient {
    inner: Arc<ClientInner>,
    ready: watch::Receiver<Option<Result<(), SystemExecutionError>>>,
}

impl LinuxClient {
    pub(crate) fn connect() -> Result<Self, SystemExecutionError> {
        // Fixed installed components only. Never select a privileged executable
        // from PATH, a user-writable resource folder, or a runtime env override.
        for path in ["/usr", "/usr/libexec", HELPER] {
            let metadata = std::fs::symlink_metadata(path).map_err(|error| SystemExecutionError::unavailable(
                format!("Linux system runner component is unavailable at {path}: {error}")))?;
            if metadata.file_type().is_symlink() || metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
                return Err(SystemExecutionError::unavailable(format!("Linux system component must be root-owned and protected against replacement: {path}")));
            }
        }
        let mut command = Command::new("/usr/bin/pkexec");
        command.args(["--disable-internal-agent", HELPER, "--stdio"])
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        Self::spawn(command)
    }

    fn spawn(command: Command) -> Result<Self, SystemExecutionError> {
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        std::thread::Builder::new().name("nomifun-system-parent".into()).spawn(move || {
            let connected = Self::spawn_on_parent_thread(command);
            let child = connected.as_ref().ok().map(|client| client.inner.child.clone());
            // Returning the client does not retire this creating thread.
            // pkexec's PR_SET_PDEATHSIG is tied to this exact thread, not the
            // whole App; transient Tokio blocking workers cannot own it.
            let _ = send.send(connected);
            if let Some(child) = child {
                loop {
                    let finished = {
                        let mut child = child.lock().expect("system child mutex");
                        match child.as_mut().map(Child::try_wait) {
                            Some(Ok(None)) => false,
                            Some(Err(error)) if error.kind() == io::ErrorKind::Interrupted => false,
                            _ => true,
                        }
                    };
                    if finished { break; }
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        }).map_err(|error| SystemExecutionError::unavailable(format!("Linux system owner thread could not start: {error}")))?;
        receive.recv().map_err(|error| SystemExecutionError::unavailable(format!("Linux system owner stopped before connecting: {error}")))?
    }

    fn spawn_on_parent_thread(mut command: Command) -> Result<Self, SystemExecutionError> {
        let mut child = command.spawn().map_err(|error| SystemExecutionError::unavailable(
            format!("Linux native polkit authentication could not start: {error}")))?;
        let stdin = child.stdin.take().expect("piped system stdin");
        let stdout = child.stdout.take().expect("piped system stdout");
        let stderr = child.stderr.take().expect("piped system stderr");
        let diagnostic = Arc::new(Mutex::new(Vec::new()));
        let retained = diagnostic.clone();
        let (drained, drain_finished) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            // Keep stderr drained after the diagnostic envelope fills.
            let mut reader = stderr;
            drain_authentication_stderr(&mut reader, &retained);
            let _ = drained.send(());
        });
        let inner = Arc::new(ClientInner {
            stdin: Mutex::new(Some(stdin)), child: Arc::new(Mutex::new(Some(child))),
            pending: Mutex::new(HashMap::new()),
            admission: Arc::new(tokio::sync::RwLock::new(())), sequence: AtomicU64::new(1),
            approved: AtomicBool::new(false),
        });
        let weak = Arc::downgrade(&inner);
        let (ready, state) = watch::channel(None);
        std::thread::spawn(move || {
            let mut reader = ApprovalReader { inner: stdout, deadline: Instant::now() + APPROVAL_TIMEOUT };
            let handshake = read_frame(&mut reader, MAX_REQUEST).and_then(|value| {
                let value = value.ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "Native authentication ended before the runner was ready"))?;
                validate_handshake(&value, unsafe { libc::getuid() })
            });
            if let Err(error) = handshake {
                if let Some(inner) = weak.upgrade() {
                    inner.stdin.lock().expect("system stdin mutex").take();
                    let status = inner.child.lock().expect("system child mutex").as_mut()
                        .and_then(|child| reap_failed_authentication(child, error.kind() == io::ErrorKind::TimedOut));
                    // Keep draining stderr, but do not hang readiness if an
                    // inherited descriptor remains open in another process.
                    let _ = drain_finished.recv_timeout(Duration::from_millis(100));
                    let diagnostic = String::from_utf8_lossy(&diagnostic.lock().expect("system diagnostic mutex")).into_owned();
                    let failure = authentication_failure_with_status(status, &diagnostic, &error);
                    let _ = ready.send(Some(Err(failure)));
                }
                return;
            }
            if let Some(inner) = weak.upgrade() { inner.approved.store(true, Ordering::Release); }
            if ready.send(Some(Ok(()))).is_err() { return; }
            let mut reader = reader.inner;
            let failure = loop {
                let frame = match read_frame(&mut reader, MAX_FRAME) {
                    Ok(Some(value)) => value,
                    Ok(None) => break unknown("Linux system runner connection closed; admitted operation results remain unknown"),
                    Err(error) => break unknown(format!("Invalid Linux system runner response: {error}")),
                };
                let Some(id) = frame["id"].as_u64() else { break unknown("Linux system runner omitted RPC identity"); };
                let Some(inner) = weak.upgrade() else { return; };
                let Some(pending) = inner.pending.lock().expect("system pending mutex").remove(&id) else {
                    break unknown("Linux system runner returned an unrecognized RPC identity");
                };
                let result = if frame["reply"]["ok"] == true {
                    Ok(frame["reply"]["result"].clone())
                } else { Err(SystemExecutionError::from_value(&frame["reply"]["error"])) };
                let _ = pending.reply.send(result);
            };
            if let Some(inner) = weak.upgrade() {
                inner.stdin.lock().expect("system stdin mutex").take();
                for (_, pending) in inner.pending.lock().expect("system pending mutex").drain() {
                    let _ = pending.reply.send(Err(failure.clone()));
                }
                let _ = ready.send(Some(Err(failure)));
            }
        });
        Ok(Self { inner, ready: state })
    }

    pub(crate) async fn wait_ready(&self) -> Result<(), SystemExecutionError> {
        let mut ready = self.ready.clone();
        loop {
            if let Some(result) = ready.borrow_and_update().clone() { return result; }
            ready.changed().await.map_err(|_| SystemExecutionError::unavailable("Linux authentication transport closed before readiness"))?;
        }
    }

    pub(crate) async fn call(&self, request: Value) -> Result<Value, SystemExecutionError> {
        self.wait_ready().await?;
        let guard = admission(self.inner.admission.clone(), &request).await;
        let id = self.inner.sequence.fetch_add(1, Ordering::Relaxed);
        let frame = json!({"id":id,"request":request});
        // Validate before admission to the pipe: this result proves no dispatch.
        let bytes = serde_json::to_vec(&frame).expect("JSON serialization");
        if bytes.len() > MAX_REQUEST {
            return Err(not_started("invalid_system_request", "Linux system request exceeds its envelope"));
        }
        let (reply, receive) = oneshot::channel();
        self.inner.pending.lock().expect("system pending mutex").insert(id, Pending { reply, _admission: guard });
        let inner = self.inner.clone();
        let dispatched = tokio::task::spawn_blocking(move || {
            let mut stdin = inner.stdin.lock().expect("system stdin mutex");
            match stdin.as_mut() {
                Some(stdin) => write_frame(stdin, &frame, MAX_REQUEST),
                None => Err(io::ErrorKind::BrokenPipe.into()),
            }
        }).await.map_err(|error| unknown(format!("Linux system dispatch worker stopped: {error}")))?;
        if let Err(error) = dispatched {
            // Even a partial frame write is never proof that the operation did
            // not execute. Close admission and leave actual cleanup to the owner.
            self.inner.stdin.lock().expect("system stdin mutex").take();
            self.inner.pending.lock().expect("system pending mutex").remove(&id);
            return Err(unknown(format!("Linux system request transport failed: {error}")));
        }
        match tokio::time::timeout(Duration::from_secs(60), receive).await {
            Ok(result) => result.map_err(|_| unknown("Linux system request lost its original reply"))?,
            Err(_) => {
                // Close this original connection. A new runner cannot settle
                // any admitted effects from this one or certify its cleanup.
                self.inner.stdin.lock().expect("system stdin mutex").take();
                Err(unknown("Linux system request timed out; its result and cleanup remain unknown"))
            }
        }
    }
}

fn authentication_failure(status: Option<i32>, diagnostic: &str, error: &io::Error) -> SystemExecutionError {
    let code = if error.kind() == io::ErrorKind::TimedOut { "SYSTEM_EXECUTION_AUTHENTICATION_TIMED_OUT" }
        else if status == Some(126) { "SYSTEM_EXECUTION_AUTHENTICATION_CANCELLED" }
        else if diagnostic.to_ascii_lowercase().contains("authentication agent") || diagnostic.contains("org.freedesktop.PolicyKit1") {
            "SYSTEM_EXECUTION_AUTHENTICATION_UNAVAILABLE"
        } else if status == Some(127) { "SYSTEM_EXECUTION_AUTHENTICATION_DENIED" }
        else { "SYSTEM_EXECUTION_NOT_AVAILABLE" };
    let mut end = diagnostic.len().min(PUBLIC_DIAGNOSTIC_BYTES);
    while !diagnostic.is_char_boundary(end) { end -= 1; }
    let excerpt = &diagnostic[..end];
    let suffix = if end < diagnostic.len() { " [truncated]" } else { "" };
    // AppError consumers preserve this message even where they do not carry
    // native details. This is pkexec/runner stderr, never the agent's password
    // stream; keep its public excerpt bounded while retaining the 64 KiB drain.
    let mut result = not_started(code, format!("Linux system authentication did not produce a valid runner: {error}; native_exit_code={status:?}; native_diagnostic={excerpt}{suffix}"));
    result.details["native_exit_code"] = json!(status);
    result.details["native_diagnostic"] = json!(diagnostic);
    result
}

fn authentication_failure_with_status(status: Option<ExitStatus>, diagnostic: &str, error: &io::Error) -> SystemExecutionError {
    let signal = status.as_ref().and_then(ExitStatusExt::signal);
    let mut failure = authentication_failure(status.and_then(|status| status.code()), diagnostic, error);
    failure.details["native_signal"] = json!(signal);
    failure.message.push_str(&format!("; native_signal={signal:?}"));
    failure
}

fn validate_handshake(value: &Value, uid: u32) -> io::Result<()> {
    if value["protocol_version"] != PROTOCOL || value["effective_uid"] != 0
        || value["caller_uid"] != uid
        || value["build_identity"] != option_env!("NOMIFUN_SYSTEM_EXECUTION_BUILD_ID").unwrap_or("unbuilt") {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Installed Linux runner identity/protocol differs from this application"));
    }
    Ok(())
}

fn caller_identity() -> io::Result<(u32, String, String)> {
    let uid: u32 = std::env::var("PKEXEC_UID").map_err(|_| io::Error::new(io::ErrorKind::PermissionDenied, "Runner requires the native pkexec caller identity"))?
        .parse().map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Invalid PKEXEC_UID"))?;
    let mut entry = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let mut found = std::ptr::null_mut();
    let mut buffer = vec![0u8; 64 * 1024];
    let result = unsafe { libc::getpwuid_r(uid, entry.as_mut_ptr(), buffer.as_mut_ptr().cast(), buffer.len(), &mut found) };
    if result != 0 { return Err(io::Error::from_raw_os_error(result)); }
    if found.is_null() { return Err(io::Error::new(io::ErrorKind::NotFound, "Native caller has no passwd identity")); }
    let entry = unsafe { entry.assume_init() };
    let home = unsafe { CStr::from_ptr(entry.pw_dir) }.to_str().map_err(|_| io::ErrorKind::InvalidData)?.to_owned();
    let name = unsafe { CStr::from_ptr(entry.pw_name) }.to_str().map_err(|_| io::ErrorKind::InvalidData)?.to_owned();
    Ok((uid, home, name))
}

/// Entry of the packaged root runner. The GUI remains the login user; its
/// inherited private pipes alone own this runner and its temporary tasks.
pub fn run_stdio_helper() -> io::Result<()> {
    if effective_uid() != 0 { return Err(io::Error::new(io::ErrorKind::PermissionDenied, "System runner must be launched through native administrator authentication")); }
    let (uid, home, name) = caller_identity()?;
    let runner = Arc::new(Runner::new());
    let admission_lock = Arc::new(tokio::sync::RwLock::new(()));
    let slots = Arc::new(RpcDispatchSlots::new());
    let closing = Arc::new(AtomicBool::new(false));
    let writer = Arc::new(Mutex::new(io::stdout()));
    write_frame(&mut *writer.lock().expect("runner stdout mutex"), &json!({
        "protocol_version":PROTOCOL,"effective_uid":effective_uid(),"caller_uid":uid,
        "build_identity":option_env!("NOMIFUN_SYSTEM_EXECUTION_BUILD_ID").unwrap_or("unbuilt")
    }), MAX_REQUEST)?;
    let mut reader = io::stdin().lock();
    let mut workers: Vec<std::thread::JoinHandle<()>> = Vec::new();
    let result = loop {
        let frame = match read_frame(&mut reader, MAX_REQUEST) {
            Ok(Some(frame)) => frame,
            Ok(None) => break Ok(()),
            Err(error) => break Err(error),
        };
        let Some(id) = frame["id"].as_u64() else { break Err(io::ErrorKind::InvalidData.into()); };
        let mut request = frame["request"].clone();
        if request["op"] == "start" {
            if !request["env"].is_object() { request["env"] = json!({}); }
            let env = request["env"].as_object_mut().expect("environment object");
            env.entry("HOME").or_insert_with(|| json!(home));
            env.entry("USER").or_insert_with(|| json!(name));
            env.entry("LOGNAME").or_insert_with(|| json!(name));
        }
        // Bound retained finished thread handles; no detached request may race
        // the EOF shutdown fence or keep a supervisor alive invisibly.
        let mut active = Vec::new();
        for worker in workers.drain(..) {
            if worker.is_finished() { let _ = worker.join(); } else { active.push(worker); }
        }
        workers = active;
        let runner = runner.clone();
        let writer = writer.clone();
        let admission_lock = admission_lock.clone();
        let slots = slots.clone();
        let closing = closing.clone();
        workers.push(std::thread::spawn(move || {
            let Ok(reply) = dispatch_received_rpc(&runner, admission_lock, &slots, &closing, request) else { return; };
            if write_frame(&mut *writer.lock().expect("runner stdout mutex"), &json!({"id":id,"reply":reply}), MAX_FRAME).is_err() {
                let _ = runner.runtime.block_on(runner.supervisor.shutdown());
                // Never leave an omitted response on a live connection: its
                // original pending call and admission fence would hang. Native
                // shutdown runs first; closing all pipes settles calls unknown.
                std::process::exit(74);
            }
        }));
    };
    closing.store(true, Ordering::Release);
    let _fence = runner.runtime.block_on(admission_lock.write_owned());
    let report = runner.runtime.block_on(runner.supervisor.shutdown());
    // Native admission is now irreversibly closed. Release the transport fence
    // so queued original requests can observe EOF and finish without executing.
    drop(_fence);
    for worker in workers { let _ = worker.join(); }
    if !report.is_exact() { return Err(io::Error::other("System runner could not prove exact shutdown cleanup")); }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_preserve_literals_and_reject_partial_oversize_and_invalid_json() {
        let value = json!({"args":["", "中文 空格", "$(false); & | < > *"]});
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &value, MAX_REQUEST).unwrap();
        assert_eq!(read_frame(&mut bytes.as_slice(), MAX_REQUEST).unwrap(), Some(value));
        assert!(read_frame(&mut &bytes[..bytes.len()-1], MAX_REQUEST).is_err());
        assert!(read_frame(&mut &(u32::MAX.to_be_bytes())[..], MAX_REQUEST).is_err());
        assert!(read_frame(&mut &[0, 0, 0, 1, b'x'][..], MAX_REQUEST).is_err());
    }

    #[test]
    fn full_business_dispatch_still_accepts_exact_stop_and_preserves_the_sibling() {
        let slots = RpcDispatchSlots::new();
        let permits = (0..BUSINESS_RPC_SLOTS).map(|_| slots.admit("poll", || false).unwrap().unwrap()).collect::<Vec<_>>();
        let runner = Runner::new(); let root = tempfile::tempdir().unwrap();
        let owner = |op: &str| json!({"op":op,"invocation_id":uuid::Uuid::nil().to_string(),"call_id":uuid::Uuid::nil().to_string()});
        let mut start = owner("start"); start["cwd"] = json!(root.path());
        start["command"] = json!({"kind":"program","program":"/bin/sh","args":["-c","IFS= read -r fixture"]});
        let a = runner.call(&serde_json::to_vec(&start).unwrap()); let b = runner.call(&serde_json::to_vec(&start).unwrap());
        assert_eq!(a["ok"], true, "{a}"); assert_eq!(b["ok"], true, "{b}");
        let mut cancel = owner("cancel"); cancel["session_id"] = a["result"]["session_id"].clone();
        // The same complete frame parser and dispatch owner used by the
        // inherited transport; no polkit, installed helper or user data.
        let mut frame = Vec::new(); write_frame(&mut frame, &json!({"id":1,"request":cancel}), MAX_REQUEST).unwrap();
        let received = read_frame(&mut frame.as_slice(), MAX_REQUEST).unwrap().unwrap();
        let closing = AtomicBool::new(false);
        let stopped = dispatch_received_rpc(&runner, Arc::new(tokio::sync::RwLock::new(())), &slots, &closing, received["request"].clone()).unwrap();
        assert_eq!(stopped["result"]["cleanup"]["reaped"], true, "{stopped}");
        assert!(!closing.load(Ordering::Acquire), "one exact Stop cannot close the original pipe");
        assert_eq!(*slots.active.lock().unwrap(), BUSINESS_RPC_SLOTS);
        let mut sibling = owner("poll"); sibling["session_id"] = b["result"]["session_id"].clone();
        assert_eq!(runner.call(&serde_json::to_vec(&sibling).unwrap())["result"]["state"], "running");
        drop(permits);
        assert_eq!(runner.call(br#"{"op":"shutdown"}"#)["result"]["exact"], true);
    }

    #[test]
    fn full_business_dispatch_keeps_queued_original_requests_and_eof_prevents_new_dispatch() {
        let slots = Arc::new(RpcDispatchSlots::new());
        let mut permits = (0..BUSINESS_RPC_SLOTS).map(|_| slots.admit("poll", || false).unwrap().unwrap()).collect::<Vec<_>>();
        let dispatch = slots.clone(); let (sent, received) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _permit = dispatch.admit("write", || false).unwrap().unwrap(); sent.send(()).unwrap();
        });
        assert!(received.recv_timeout(Duration::from_millis(50)).is_err());
        for op in ["cancel", "terminate", "timeout", "quiesce", "shutdown", "release", "stop_facility",
            "observe", "terminal", "wait_terminal", "output_archive", "release_terminal"] {
            assert!(slots.admit(op, || false).unwrap().is_none(), "{op} retains an independent ingress");
        }
        drop(permits.pop()); received.recv_timeout(Duration::from_secs(2)).unwrap(); worker.join().unwrap();
        permits.push(slots.admit("poll", || false).unwrap().unwrap());
        let closing = Arc::new(AtomicBool::new(false)); let eof = closing.clone(); let dispatch = slots.clone();
        let (sent, received) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let result = dispatch.admit("start", || eof.load(Ordering::Acquire));
            sent.send(result.err().map(|error|error.kind())).unwrap();
        });
        closing.store(true, Ordering::Release);
        assert_eq!(received.recv_timeout(Duration::from_secs(2)).unwrap(), Some(io::ErrorKind::Interrupted)); worker.join().unwrap();
        assert_eq!(*slots.active.lock().unwrap(), BUSINESS_RPC_SLOTS, "EOF does not invent another original request's completion");
    }

    #[test]
    fn approval_results_keep_cancel_denial_no_agent_and_timeout_distinct() {
        let eof = io::Error::from(io::ErrorKind::UnexpectedEof);
        assert_eq!(authentication_failure(Some(126), "", &eof).code, "SYSTEM_EXECUTION_AUTHENTICATION_CANCELLED");
        assert_eq!(authentication_failure(Some(127), "", &eof).code, "SYSTEM_EXECUTION_AUTHENTICATION_DENIED");
        assert_eq!(authentication_failure(Some(127), "No authentication agent found", &eof).code, "SYSTEM_EXECUTION_AUTHENTICATION_UNAVAILABLE");
        assert_eq!(authentication_failure(None, "", &io::ErrorKind::TimedOut.into()).code, "SYSTEM_EXECUTION_AUTHENTICATION_TIMED_OUT");
        assert_eq!(authentication_failure(Some(127), "", &eof).details["user_code_not_started"], true);
        assert_eq!(authentication_failure(Some(127), "native denial", &io::ErrorKind::TimedOut.into()).code, "SYSTEM_EXECUTION_AUTHENTICATION_TIMED_OUT");
    }

    #[test]
    fn public_authentication_message_keeps_exit_and_bounded_utf8_diagnostic() {
        let diagnostic = "原生诊断".repeat(10000);
        let failure = authentication_failure(Some(127), &diagnostic, &io::ErrorKind::UnexpectedEof.into());
        assert!(failure.message.contains("native_exit_code=Some(127)"));
        assert!(failure.message.contains("原生诊断"));
        assert!(failure.message.ends_with(" [truncated]"));
        assert!(failure.message.len() < PUBLIC_DIAGNOSTIC_BYTES + 512);
        assert_eq!(failure.details["native_diagnostic"], diagnostic);
    }

    #[tokio::test]
    async fn stdout_eof_preserves_later_native_denial_and_dismissal_exit() {
        for (code, expected) in [(127, "SYSTEM_EXECUTION_AUTHENTICATION_DENIED"), (126, "SYSTEM_EXECUTION_AUTHENTICATION_CANCELLED")] {
            let mut command = Command::new("/bin/sh");
            command.args(["-c", &format!("exec 1>&-; /bin/sleep 0.05; printf 'fixture native rejection' >&2; exit {code}")])
                .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
            let client = LinuxClient::spawn(command).unwrap();
            let failure = tokio::time::timeout(Duration::from_secs(2), client.wait_ready()).await.unwrap().unwrap_err();
            assert_eq!(failure.code, expected, "{failure:?}");
            assert_eq!(failure.details["native_exit_code"], code);
            assert!(failure.details["native_signal"].is_null());
            assert_eq!(failure.details["native_diagnostic"], "fixture native rejection");
            assert!(failure.message.contains("fixture native rejection"));
            assert_eq!(failure.details["user_code_not_started"], true);
        }
    }

    #[test]
    fn stderr_drain_retries_interruption_and_preserves_real_read_error() {
        struct InterruptedThenFailed { step: usize }
        impl Read for InterruptedThenFailed {
            fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
                self.step += 1;
                match self.step {
                    1 => Err(io::ErrorKind::Interrupted.into()),
                    2 => { bytes[..7].copy_from_slice(b"fixture"); Ok(7) }
                    _ => Err(io::Error::new(io::ErrorKind::BrokenPipe, "fixture pipe failure")),
                }
            }
        }
        let retained = Mutex::new(Vec::new());
        drain_authentication_stderr(&mut InterruptedThenFailed { step: 0 }, &retained);
        let bytes = retained.into_inner().unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(text.starts_with("fixture"));
        assert!(text.contains("fixture pipe failure"));
    }

    #[tokio::test]
    async fn native_signal_is_reported_from_the_actual_child_exit_status() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "exec 1>&-; /bin/sleep 0.05; kill -TERM $$"])
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let client = LinuxClient::spawn(command).unwrap();
        let failure = tokio::time::timeout(Duration::from_secs(2), client.wait_ready()).await.unwrap().unwrap_err();
        assert!(failure.details["native_exit_code"].is_null());
        assert_eq!(failure.details["native_signal"], libc::SIGTERM);
        assert!(failure.message.contains("native_signal=Some(15)"));
        assert_ne!(failure.code, "SYSTEM_EXECUTION_AUTHENTICATION_DENIED");
    }

    #[tokio::test]
    async fn authentication_stderr_keeps_draining_after_its_retained_envelope() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "exec 1>&-; printf '%131072s' x >&2; exit 127"])
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let client = LinuxClient::spawn(command).unwrap();
        let failure = tokio::time::timeout(Duration::from_secs(2), client.wait_ready()).await.unwrap().unwrap_err();
        assert_eq!(failure.code, "SYSTEM_EXECUTION_AUTHENTICATION_DENIED");
        assert_eq!(failure.details["native_exit_code"], 127);
        assert_eq!(failure.details["native_diagnostic"].as_str().unwrap().len(), 64 * 1024);
        assert!(failure.message.len() < PUBLIC_DIAGNOSTIC_BYTES + 512);
    }

    #[test]
    fn handshake_requires_exact_caller_root_protocol_and_build() {
        let mut value = json!({"protocol_version":PROTOCOL,"effective_uid":0,"caller_uid":123,
            "build_identity":option_env!("NOMIFUN_SYSTEM_EXECUTION_BUILD_ID").unwrap_or("unbuilt")});
        assert!(validate_handshake(&value, 123).is_ok());
        assert!(validate_handshake(&value, 124).is_err());
        value["effective_uid"] = json!(123);
        assert!(validate_handshake(&value, 123).is_err());
    }

    #[tokio::test]
    async fn cancelled_waiter_retains_dispatch_fence_until_original_receipt() {
        let lock = Arc::new(tokio::sync::RwLock::new(()));
        let guard = admission(lock.clone(), &json!({"op":"start"})).await;
        let (reply, receive) = oneshot::channel();
        let pending = Pending { reply, _admission: guard };
        drop(receive);
        assert!(lock.try_write().is_err());
        drop(pending);
        assert!(lock.try_write().is_ok());
    }

    #[tokio::test]
    async fn dropping_pending_authentication_ends_the_original_attempt() {
        let mut command = Command::new("/bin/sleep");
        command.arg("60").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let client = LinuxClient::spawn(command).unwrap();
        let pid = client.inner.child.lock().unwrap().as_ref().unwrap().id();
        assert!(tokio::time::timeout(Duration::from_millis(20), client.wait_ready()).await.is_err());
        drop(client);
        let deadline = Instant::now() + Duration::from_secs(2);
        while std::path::Path::new(&format!("/proc/{pid}")).exists() && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists(), "cancelled authentication process remained alive");
    }

    #[test]
    fn native_child_outlives_the_connecting_tokio_blocking_thread() {
        use std::os::unix::process::CommandExt;
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all()
            .thread_keep_alive(Duration::from_millis(20)).build().unwrap();
        runtime.block_on(async {
            let client = tokio::task::spawn_blocking(|| {
                let mut command = Command::new("/bin/sleep");
                command.arg("60").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
                // pkexec itself sets this native protection before asking its
                // authentication agent. Reproduce it without PAM or passwords.
                unsafe { command.pre_exec(|| {
                    if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) == 0 { Ok(()) }
                    else { Err(io::Error::last_os_error()) }
                }); }
                LinuxClient::spawn(command)
            }).await.unwrap().unwrap();
            let pid = client.inner.child.lock().unwrap().as_ref().unwrap().id();
            tokio::time::sleep(Duration::from_millis(100)).await;
            let readiness = tokio::time::timeout(Duration::from_millis(20), client.wait_ready()).await;
            assert!(readiness.is_err(), "native child died when the transient connecting thread retired: {readiness:?}");
            drop(client);
            let deadline = Instant::now() + Duration::from_secs(2);
            while std::path::Path::new(&format!("/proc/{pid}")).exists() && Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists(), "dropped authentication child was not reaped");
        });
    }

    #[test]
    fn creating_thread_survives_approved_client_drop_until_native_eof_cleanup() {
        use std::os::unix::process::CommandExt;
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("eof-cleanup-completed");
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "/bin/cat >/dev/null; /bin/sleep 0.1; printf completed > \"$1\"", "fixture"])
            .arg(&marker).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        unsafe { command.pre_exec(|| {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) == 0 { Ok(()) }
            else { Err(io::Error::last_os_error()) }
        }); }
        let client = LinuxClient::spawn(command).unwrap();
        let pid = client.inner.child.lock().unwrap().as_ref().unwrap().id();
        // This is only a transport lifecycle fixture, not an authentication
        // grant. Model the already-approved state so Drop must allow EOF work.
        client.inner.approved.store(true, Ordering::Release);
        drop(client);
        let deadline = Instant::now() + Duration::from_secs(2);
        while std::path::Path::new(&format!("/proc/{pid}")).exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists(), "EOF cleanup child was not reaped");
        assert_eq!(std::fs::read_to_string(marker).unwrap(), "completed");
    }
}
