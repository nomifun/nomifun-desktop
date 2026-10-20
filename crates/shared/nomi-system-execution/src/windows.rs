//! A UAC elevated, application-lifetime runner, with one existing supervisor
//! per logical client. No service installation, credentials, or retry ledger.
//! Every RPC authenticates both pipe endpoints by retained process identity.
use crate::{Runner, SystemExecutionError};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    ffi::OsStr,
    io, mem,
    os::windows::ffi::OsStrExt,
    path::PathBuf,
    ptr,
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_IO_PENDING, ERROR_PIPE_CONNECTED, FILETIME, HANDLE,
        INVALID_HANDLE_VALUE, LocalFree, WAIT_OBJECT_0, WAIT_TIMEOUT,
    },
    Security::{
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        },
        GetTokenInformation, SECURITY_ATTRIBUTES, TOKEN_ELEVATION, TOKEN_QUERY, TOKEN_USER,
        TokenElevation, TokenUser,
    },
    Storage::FileSystem::{
        CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, FILE_GENERIC_READ,
        FILE_GENERIC_WRITE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX, ReadFile, SECURITY_IDENTIFICATION,
        SECURITY_SQOS_PRESENT, WriteFile,
    },
    System::{
        Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize},
        IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED},
        Pipes::{
            ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeClientProcessId,
            GetNamedPipeServerProcessId, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE,
            PIPE_UNLIMITED_INSTANCES,
        },
        RemoteDesktop::ProcessIdToSessionId,
        Threading::{
            CreateEventW, GetCurrentProcess, GetProcessId, GetProcessTimes, OpenProcess,
            OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
            QueryFullProcessImageNameW, SetEvent, WaitForMultipleObjects, WaitForSingleObject,
        },
    },
    UI::{
        Shell::{SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW},
        WindowsAndMessaging::SW_HIDE,
    },
};

const BUILD: &str = env!("NOMIFUN_SYSTEM_EXECUTION_BUILD_ID");
const MODE: &str = "--nomifun-system-runner";
const MAX_REQUEST: usize = 2 * 1024 * 1024;
// A supported 16 MiB retained output can contain JSON-escaped text, its legacy
// raw chunks and an opt-in exact decoder/raw replay receipt. Keep all original
// facts without truncating them or reducing the user's output limit.
const MAX_REPLY: usize = 256 * 1024 * 1024;
const RPC_DEADLINE: Duration = Duration::from_secs(45);
const BUSINESS_RPC_SLOTS: usize = 64;
static HOST_SUPPORTED: AtomicBool = AtomicBool::new(false);

/// Keep the existing business dispatch bound after reading the authenticated
/// request. The listener must remain available to the exact owner's Stop even
/// when every ordinary native call is waiting on its real result.
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

fn validate_reply_size(length:usize)->io::Result<()> {
    if length>MAX_REPLY {return Err(io::Error::other("Native reply exceeds its envelope"));}
    Ok(())
}

#[derive(Debug)]
struct Handle(HANDLE);
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
impl Handle {
    fn new(raw: HANDLE) -> io::Result<Self> {
        if raw.is_null() || raw == INVALID_HANDLE_VALUE {
            Err(io::Error::last_os_error())
        } else {
            Ok(Self(raw))
        }
    }
    fn running(&self) -> bool {
        unsafe { WaitForSingleObject(self.0, 0) == WAIT_TIMEOUT }
    }
}
fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain(Some(0)).collect()
}
fn before(code: &str, message: impl Into<String>) -> SystemExecutionError {
    SystemExecutionError {
        code: code.into(),
        message: message.into(),
        details: json!({"user_code_not_started":true,"cleanup_proven":true}),
    }
}
fn before_io(code: &str, operation: &str, error: io::Error) -> SystemExecutionError {
    let mut failure = before(code, format!("{operation}: {error}"));
    failure.details["kind"] = format!("{:?}", error.kind()).into();
    failure.details["raw_os_error"] = json!(error.raw_os_error());
    failure
}

fn launch_failure(error: io::Error) -> SystemExecutionError {
    // ERROR_CANCELLED describes the OS outcome, not who dismissed the native
    // prompt. Other launch errors (including access denied) are insufficient
    // evidence of an authentication rejection.
    let code = match error.raw_os_error() {
        Some(1223) => "SYSTEM_EXECUTION_AUTHENTICATION_CANCELLED",
        _ => "SYSTEM_EXECUTION_NOT_AVAILABLE",
    };
    before_io(code, "Windows UAC runner was not started", error)
}
pub(super) fn unknown(message: impl Into<String>) -> SystemExecutionError {
    SystemExecutionError {
        code: "SYSTEM_EXECUTION_RESULT_UNKNOWN".into(),
        message: message.into(),
        details: json!({"user_code_not_started":false,"cleanup_proven":false}),
    }
}
fn unknown_io(operation: &str, error: io::Error) -> SystemExecutionError {
    let mut failure = unknown(format!("{operation}: {error}"));
    failure.details["kind"] = format!("{:?}", error.kind()).into();
    failure.details["raw_os_error"] = json!(error.raw_os_error());
    failure
}
fn process(pid: u32) -> io::Result<Handle> {
    Handle::new(unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            0,
            pid,
        )
    })
}
fn image(handle: HANDLE) -> io::Result<PathBuf> {
    let mut bytes = vec![0u16; 32768];
    let mut length = bytes.len() as u32;
    if unsafe { QueryFullProcessImageNameW(handle, 0, bytes.as_mut_ptr(), &mut length) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(PathBuf::from(
        String::from_utf16(&bytes[..length as usize]).map_err(io::Error::other)?,
    ))
}
fn created(handle: HANDLE) -> io::Result<u64> {
    let (mut creation, mut exit, mut kernel, mut user): (FILETIME, FILETIME, FILETIME, FILETIME) =
        unsafe { mem::zeroed() };
    if unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(((creation.dwHighDateTime as u64) << 32) | creation.dwLowDateTime as u64)
}
fn session(pid: u32) -> io::Result<u32> {
    let mut id = 0;
    if unsafe { ProcessIdToSessionId(pid, &mut id) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(id)
    }
}
fn token(handle: HANDLE) -> io::Result<Handle> {
    let mut raw = ptr::null_mut();
    if unsafe { OpenProcessToken(handle, TOKEN_QUERY, &mut raw) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Handle::new(raw)
}
fn elevated() -> io::Result<bool> {
    let token = token(unsafe { GetCurrentProcess() })?;
    let mut result: TOKEN_ELEVATION = unsafe { mem::zeroed() };
    let mut length = 0;
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenElevation,
            (&mut result as *mut TOKEN_ELEVATION).cast(),
            mem::size_of_val(&result) as u32,
            &mut length,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(result.TokenIsElevated != 0)
}
fn sid(handle: HANDLE) -> io::Result<String> {
    let token = token(handle)?;
    let mut length = 0;
    unsafe {
        GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut length);
    }
    if length == 0 {
        return Err(io::Error::last_os_error());
    }
    // usize storage gives TOKEN_USER its native alignment.
    let mut storage = vec![0usize; (length as usize).div_ceil(mem::size_of::<usize>())];
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            storage.as_mut_ptr().cast(),
            length,
            &mut length,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let user = unsafe { &*storage.as_ptr().cast::<TOKEN_USER>() };
    let mut raw = ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut raw) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut count = 0;
    while unsafe { *raw.add(count) } != 0 {
        count += 1;
    }
    let value = String::from_utf16(unsafe { std::slice::from_raw_parts(raw, count) })
        .map_err(io::Error::other);
    unsafe {
        LocalFree(raw.cast());
    }
    value
}
pub(super) fn identity() -> Result<Value, String> {
    Ok(json!({"elevated":elevated().map_err(|e|e.to_string())?,
        "user_sid":sid(unsafe{GetCurrentProcess()}).map_err(|e|e.to_string())?,
        "session_id":session(std::process::id()).map_err(|e|e.to_string())?,"pid":std::process::id()}))
}

#[derive(Debug)]
struct Facility {
    pipe: String,
    process: Handle,
    stopped: AtomicBool,
}
static FACILITY: OnceLock<Mutex<Option<Arc<Facility>>>> = OnceLock::new();
impl Facility {
    fn launch() -> Result<Arc<Self>, SystemExecutionError> {
        if !HOST_SUPPORTED.load(Ordering::Acquire) {
            return Err(before(
                "SYSTEM_EXECUTION_NOT_AVAILABLE",
                "The current executable has not installed the native runner entry point",
            ));
        }
        let executable = std::env::current_exe()
            .map_err(|e| before_io("SYSTEM_EXECUTION_NOT_AVAILABLE", "Locating the native executable", e))?;
        let key = Uuid::now_v7();
        let pipe = format!(r"\\.\pipe\NomiFun.SystemExecution.{key}");
        let stamp = created(unsafe { GetCurrentProcess() })
            .map_err(|e| before_io("SYSTEM_EXECUTION_NOT_AVAILABLE", "Reading the native host identity", e))?;
        let arguments = wide(format!("{MODE} {key} {} {stamp}", std::process::id()));
        let executable = wide(executable);
        let verb = wide("runas");
        let initialized = unsafe {
            CoInitializeEx(
                ptr::null(),
                (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
            )
        } >= 0;
        let mut info: SHELLEXECUTEINFOW = unsafe { mem::zeroed() };
        info.cbSize = mem::size_of_val(&info) as u32;
        info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
        info.lpVerb = verb.as_ptr();
        info.lpFile = executable.as_ptr();
        info.lpParameters = arguments.as_ptr();
        info.nShow = SW_HIDE;
        let launched = unsafe { ShellExecuteExW(&mut info) } != 0;
        let error = io::Error::last_os_error();
        if initialized {
            unsafe {
                CoUninitialize();
            }
        }
        if !launched {
            return Err(launch_failure(error));
        }
        let process = Handle::new(info.hProcess)
            .map_err(|e| unknown(format!("UAC returned no runner process handle: {e}")))?;
        Ok(Arc::new(Self {
            pipe,
            process,
            stopped: AtomicBool::new(false),
        }))
    }
    fn exchange(&self, client: Uuid, request: Value) -> Result<Value, SystemExecutionError> {
        if self.stopped.load(Ordering::Acquire) || !self.process.running() {
            return Err(unknown(
                "The original elevated runner has stopped; its operations cannot be replayed",
            ));
        }
        let id = Uuid::now_v7();
        let envelope = json!({"build":BUILD,"connection":client.to_string(),"id":id.to_string(),"request":request});
        let bytes = serde_json::to_vec(&envelope).expect("JSON");
        if bytes.len() > MAX_REQUEST {
            return Err(before(
                "invalid_system_request",
                "Native request exceeds its bounded envelope",
            ));
        }
        let deadline = Instant::now() + RPC_DEADLINE;
        let pipe = loop {
            let handle = unsafe {
                CreateFileW(
                    wide(&self.pipe).as_ptr(),
                    FILE_GENERIC_READ | FILE_GENERIC_WRITE,
                    0,
                    ptr::null(),
                    OPEN_EXISTING,
                    FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                    ptr::null_mut(),
                )
            };
            if handle != INVALID_HANDLE_VALUE {
                break Handle::new(handle).map_err(|e| unknown_io("Retaining the original administrator pipe", e))?;
            }
            let error = io::Error::last_os_error();
            if !matches!(error.raw_os_error(), Some(2 | 231))
                || Instant::now() >= deadline
                || !self.process.running()
            {
                return Err(unknown_io("Cannot connect to the original administrator runner", error));
            }
            thread::sleep(Duration::from_millis(10));
        };
        let mut server = 0;
        if unsafe { GetNamedPipeServerProcessId(pipe.0, &mut server) } == 0 {
            return Err(unknown_io("Reading the administrator pipe server identity", io::Error::last_os_error()));
        }
        if server != unsafe { GetProcessId(self.process.0) } {
            return Err(unknown(
                "Administrator pipe server identity differs from the retained UAC process",
            ));
        }
        write_frame(pipe.0, &bytes, deadline, &self.process)
            .map_err(|e| unknown_io("Administrator request dispatch is unknown", e))?;
        let response = read_frame(pipe.0, MAX_REPLY, deadline, &self.process)
            .map_err(|e| unknown_io("Administrator reply is unknown", e))?;
        // DisconnectNamedPipe discards unread buffered data. A receipt ACK lets
        // the server close only after the entire bounded reply was consumed.
        write_frame(pipe.0, &[1], deadline, &self.process)
            .map_err(|e| unknown_io("Administrator reply acknowledgement failed", e))?;
        let reply: Value = serde_json::from_slice(&response)
            .map_err(|e| unknown(format!("Invalid administrator reply: {e}")))?;
        if reply["build"] != BUILD
            || reply["id"] != id.to_string()
            || reply["connection"] != client.to_string()
        {
            return Err(unknown(
                "Administrator reply does not match the exact dispatch identity",
            ));
        }
        let response = &reply["response"];
        if response["ok"] == true {
            Ok(response["result"].clone())
        } else {
            Err(SystemExecutionError::from_value(&response["error"]))
        }
    }
}

#[derive(Debug)]
pub(super) struct Client {
    facility: Arc<Facility>,
    id: Uuid,
}
impl Client {
    pub(super) fn connect() -> Result<Self, SystemExecutionError> {
        let mut cached = FACILITY
            .get_or_init(|| Mutex::new(None))
            .lock()
            .expect("runner facility mutex");
        let facility = match cached.as_ref() {
            Some(value) => value.clone(),
            None => {
                let value = Facility::launch()?;
                *cached = Some(value.clone());
                value
            }
        };
        drop(cached);
        let client = Self {
            facility,
            id: Uuid::now_v7(),
        };
        let identity = client.call(json!({"op":"identity"}))?;
        if identity["build_identity"] != BUILD
            || identity["protocol_version"] != 1
            || identity["windows"]["elevated"] != true
        {
            return Err(unknown(
                "Administrator runner did not prove its build, protocol, and elevated token",
            ));
        }
        Ok(client)
    }
    pub(super) fn call(&self, request: Value) -> Result<Value, SystemExecutionError> {
        self.facility.exchange(self.id, request)
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        let facility = self.facility.clone();
        let id = self.id;
        // Explicit lifecycle calls carry reviewable cleanup receipts. This last
        // reference fallback only requests the same shutdown; host death also
        // closes every supervisor independently of GUI destructors.
        let _ = thread::Builder::new()
            .name("system-client-close".into())
            .spawn(move || {
                let _ = facility.exchange(id, json!({"op":"release"}));
            });
    }
}

fn event() -> io::Result<Handle> {
    Handle::new(unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) })
}
/// OVERLAPPED and its buffers remain live until completion, including the
/// cancellation join. No timed-out kernel IO retains stack pointers or bytes.
fn finish_io(
    handle: HANDLE,
    operation: &mut OVERLAPPED,
    peer: &Handle,
    stop: Option<&Handle>,
    deadline: Instant,
) -> io::Result<u32> {
    let mut handles = vec![operation.hEvent, peer.0];
    if let Some(stop) = stop {
        handles.push(stop.0);
    }
    let wait = deadline
        .saturating_duration_since(Instant::now())
        .as_millis()
        .min(u32::MAX as u128 - 1) as u32;
    let result = unsafe { WaitForMultipleObjects(handles.len() as u32, handles.as_ptr(), 0, wait) };
    let mut count = 0;
    if result == WAIT_OBJECT_0 {
        if unsafe { GetOverlappedResult(handle, operation, &mut count, 0) } != 0 {
            return Ok(count);
        }
        return Err(io::Error::last_os_error());
    }
    let error = if result == WAIT_TIMEOUT {
        io::Error::new(io::ErrorKind::TimedOut, "Native IPC deadline")
    } else if result == WAIT_OBJECT_0 + 1 || (stop.is_some() && result == WAIT_OBJECT_0 + 2) {
        io::Error::new(
            io::ErrorKind::Interrupted,
            "Native IPC peer exited or facility stopped",
        )
    } else {
        io::Error::last_os_error()
    };
    unsafe {
        CancelIoEx(handle, operation);
        GetOverlappedResult(handle, operation, &mut count, 1);
    }
    Err(error)
}
fn connect_pipe(
    pipe: &Handle,
    peer: &Handle,
    stop: Option<&Handle>,
    deadline: Instant,
) -> io::Result<()> {
    let event = event()?;
    let mut operation: OVERLAPPED = unsafe { mem::zeroed() };
    operation.hEvent = event.0;
    if unsafe { ConnectNamedPipe(pipe.0, &mut operation) } != 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(ERROR_PIPE_CONNECTED as i32) {
        return Ok(());
    }
    if error.raw_os_error() != Some(ERROR_IO_PENDING as i32) {
        return Err(error);
    }
    finish_io(pipe.0, &mut operation, peer, stop, deadline).map(|_| ())
}
fn io_exact(
    handle: HANDLE,
    bytes: &mut [u8],
    writing: bool,
    deadline: Instant,
    peer: &Handle,
) -> io::Result<()> {
    let mut offset = 0;
    while offset < bytes.len() {
        if Instant::now() >= deadline || !peer.running() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Native IPC deadline or peer exit",
            ));
        }
        let event = event()?;
        let mut operation: OVERLAPPED = unsafe { mem::zeroed() };
        operation.hEvent = event.0;
        let mut count = 0;
        let size = (bytes.len() - offset).min(64 * 1024) as u32;
        let ok = unsafe {
            if writing {
                WriteFile(
                    handle,
                    bytes[offset..].as_ptr(),
                    size,
                    &mut count,
                    &mut operation,
                )
            } else {
                ReadFile(
                    handle,
                    bytes[offset..].as_mut_ptr(),
                    size,
                    &mut count,
                    &mut operation,
                )
            }
        };
        if ok == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_IO_PENDING as i32) {
                return Err(error);
            }
            count = finish_io(handle, &mut operation, peer, None, deadline)?;
        } else if unsafe { GetOverlappedResult(handle, &mut operation, &mut count, 0) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if count == 0 {
            return Err(io::Error::new(
                if writing {
                    io::ErrorKind::WriteZero
                } else {
                    io::ErrorKind::UnexpectedEof
                },
                "Native pipe closed before its frame completed",
            ));
        }
        offset += count as usize;
    }
    Ok(())
}
fn write_frame(handle: HANDLE, bytes: &[u8], deadline: Instant, peer: &Handle) -> io::Result<()> {
    let mut prefix = (bytes.len() as u32).to_le_bytes();
    io_exact(handle, &mut prefix, true, deadline, peer)?;
    let mut bytes = bytes.to_owned();
    io_exact(handle, &mut bytes, true, deadline, peer)
}
fn read_frame(
    handle: HANDLE,
    limit: usize,
    deadline: Instant,
    peer: &Handle,
) -> io::Result<Vec<u8>> {
    let mut prefix = [0; 4];
    io_exact(handle, &mut prefix, false, deadline, peer)?;
    let length = u32::from_le_bytes(prefix) as usize;
    if length == 0 || length > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Native IPC frame exceeds its envelope",
        ));
    }
    let mut bytes = vec![0; length];
    io_exact(handle, &mut bytes, false, deadline, peer)?;
    Ok(bytes)
}

struct PipeSecurity(*mut std::ffi::c_void);
impl Drop for PipeSecurity {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}
#[derive(Debug, Default)]
struct FacilityAdmission {
    closing: AtomicBool,
    gate: std::sync::RwLock<()>,
}
struct RunnerCleanup {
    runners: Arc<Mutex<HashMap<Uuid, Arc<Runner>>>>,
    admission: Arc<FacilityAdmission>,
}
impl Drop for RunnerCleanup {
    fn drop(&mut self) {
        self.admission.closing.store(true, Ordering::Release);
        let _fence = self
            .admission
            .gate
            .write()
            .unwrap_or_else(|poison| poison.into_inner());
        let all = self
            .runners
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for runner in all {
            let _ = runner.call(br#"{"op":"shutdown"}"#);
        }
    }
}
fn listener(name: &str, security: &PipeSecurity, first: bool) -> io::Result<Handle> {
    let attributes = SECURITY_ATTRIBUTES {
        nLength: mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: security.0,
        bInheritHandle: 0,
    };
    Handle::new(unsafe {
        CreateNamedPipeW(
            wide(name).as_ptr(),
            PIPE_ACCESS_DUPLEX
                | FILE_FLAG_OVERLAPPED
                | if first {
                    FILE_FLAG_FIRST_PIPE_INSTANCE
                } else {
                    0
                },
            PIPE_TYPE_BYTE | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            64 * 1024,
            64 * 1024,
            0,
            &attributes,
        )
    })
}
fn serve(key: Uuid, parent_pid: u32, stamp: u64) -> Result<(), String> {
    if !elevated().map_err(|e| e.to_string())? {
        return Err("The native system runner requires a UAC elevated token".into());
    }
    let parent = Arc::new(process(parent_pid).map_err(|e| e.to_string())?);
    if created(parent.0).map_err(|e| e.to_string())? != stamp
        || !parent.running()
        || image(parent.0).map_err(|e| e.to_string())?
            != image(unsafe { GetCurrentProcess() }).map_err(|e| e.to_string())?
        || session(parent_pid).map_err(|e| e.to_string())?
            != session(std::process::id()).map_err(|e| e.to_string())?
    {
        return Err(
            "The native runner parent image, creation identity or login session does not match"
                .into(),
        );
    }
    let owner_sid = sid(parent.0).map_err(|e| e.to_string())?;
    // Medium mandatory label permits the actual unelevated parent to write.
    // The DACL rejects other users and remote clients; exact PID is checked on
    // every accepted handle, including over-the-shoulder administrator login.
    let sddl = wide(format!(
        "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;{owner_sid})S:(ML;;NW;;;ME)"
    ));
    let mut raw = ptr::null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut raw,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error().to_string());
    }
    let security = PipeSecurity(raw);
    let name = format!(r"\\.\pipe\NomiFun.SystemExecution.{key}");
    let runners = Arc::new(Mutex::new(HashMap::<Uuid, Arc<Runner>>::new()));
    let admission = Arc::new(FacilityAdmission::default());
    let cleanup = RunnerCleanup {
        runners: runners.clone(),
        admission: admission.clone(),
    };
    let stopping = Arc::new(AtomicBool::new(false));
    let stop_event = Arc::new(event().map_err(|e| e.to_string())?);
    let cleaned = Arc::new(event().map_err(|e| e.to_string())?);
    {
        let parent = parent.clone();
        let stop = stop_event.clone();
        let stopping = stopping.clone();
        let cleaned = cleaned.clone();
        thread::Builder::new().name("system-parent-watchdog".into()).spawn(move || {
            if unsafe{WaitForSingleObject(parent.0,u32::MAX)}!=WAIT_OBJECT_0 {return;}
            stopping.store(true,Ordering::Release);unsafe{SetEvent(stop.0);}
            if unsafe{WaitForSingleObject(cleaned.0,10_000)}!=WAIT_OBJECT_0 {
                // A stalled cleanup must not keep an elevated orphan or
                // temporary Job alive after the original host died. Exiting
                // this runner closes its non-inheritable kill-on-close Job
                // handles. There is no receipt: original effects stay unknown.
                eprintln!("Native system runner parent exited; cleanup did not finish before its watchdog deadline");
                std::process::exit(74);
            }
        }).map_err(|e|e.to_string())?;
    }
    let mut pending = listener(&name, &security, true).map_err(|e| e.to_string())?;
    let slots = Arc::new(RpcDispatchSlots::new());
    let mut workers: Vec<thread::JoinHandle<()>> = Vec::new();
    while parent.running() && !stopping.load(Ordering::Acquire) {
        workers.retain(|worker| !worker.is_finished());
        match connect_pipe(
            &pending,
            &parent,
            Some(&stop_event),
            Instant::now() + Duration::from_secs(86400),
        ) {
            Ok(()) => {}
            Err(_error) if !parent.running() || stopping.load(Ordering::Acquire) => break,
            Err(error) if error.kind() == io::ErrorKind::TimedOut => continue,
            Err(error) => return Err(error.to_string()),
        }
        let pipe = pending;
        pending = listener(&name, &security, false).map_err(|e| e.to_string())?;
        let mut client_pid = 0;
        if unsafe { GetNamedPipeClientProcessId(pipe.0, &mut client_pid) } == 0
            || client_pid != parent_pid
        {
            unsafe {
                DisconnectNamedPipe(pipe.0);
            }
            continue;
        }
        let parent = parent.clone();
        let runners = runners.clone();
        let stopping = stopping.clone();
        let stop_event = stop_event.clone();
        let admission = admission.clone();
        let slots = slots.clone();
        workers.push(thread::spawn(move || {
            let _ = handle_rpc(&pipe, &parent, &runners, &stopping, &stop_event, &admission, &slots);
            unsafe {
                DisconnectNamedPipe(pipe.0);
            }
        }));
    }
    // Parent death is independent of Rust/GUI destructors. The existing Job
    // supervisor first closes admission, then cancels, joins and reaps tasks.
    drop(cleanup);
    for worker in workers {
        let _ = worker.join();
    }
    unsafe {
        SetEvent(cleaned.0);
    }
    Ok(())
}
fn handle_rpc(
    pipe: &Handle,
    parent: &Handle,
    runners: &Mutex<HashMap<Uuid, Arc<Runner>>>,
    stopping: &AtomicBool,
    stop_event: &Handle,
    admission: &FacilityAdmission,
    slots: &RpcDispatchSlots,
) -> io::Result<()> {
    let deadline = Instant::now() + RPC_DEADLINE;
    let bytes = read_frame(pipe.0, MAX_REQUEST, deadline, parent)?;
    let envelope: Value = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    if envelope["build"] != BUILD {
        return Err(io::Error::other("Native build mismatch"));
    }
    let client: Uuid = envelope["connection"]
        .as_str()
        .ok_or_else(|| io::Error::other("Missing native connection identity"))?
        .parse()
        .map_err(io::Error::other)?;
    let id: Uuid = envelope["id"]
        .as_str()
        .ok_or_else(|| io::Error::other("Missing native request identity"))?
        .parse()
        .map_err(io::Error::other)?;
    let request = &envelope["request"];
    let op = request["op"].as_str().unwrap_or("");
    // Admission owns the complete original request and lives through its
    // result/ACK. Waiting for an ordinary slot does not block the pipe listener
    // or drop an already-dispatched native future.
    let _slot = match slots.admit(op, || !parent.running() || stopping.load(Ordering::Acquire) || Instant::now() >= deadline) {
        Ok(slot) => slot,
        // An authenticated request received after the original facility Stop
        // still owns a reply/ACK. Let the existing admission/Runner owner
        // return its known closing rejection or original terminal receipt;
        // do not turn a deliberate closed admission into transport-unknown.
        Err(_) if parent.running() && stopping.load(Ordering::Acquire) && Instant::now() < deadline => None,
        Err(error) => return Err(error),
    };
    let response = if op == "stop_facility" {
        admission.closing.store(true, Ordering::Release);
        let _fence = admission
            .gate
            .write()
            .unwrap_or_else(|poison| poison.into_inner());
        let all = runners
            .lock()
            .unwrap()
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let mut exact = true;
        for runner in all {
            exact &= runner.call(br#"{"op":"shutdown"}"#)["result"]["exact"] == true;
        }
        if exact {
            stopping.store(true, Ordering::Release);
            unsafe {
                SetEvent(stop_event.0);
            }
        }
        json!({"ok":true,"result":{"exact":exact}})
    } else {
        let _admission = if matches!(op, "identity" | "start" | "acquire_read") {
            Some(
                admission
                    .gate
                    .read()
                    .unwrap_or_else(|poison| poison.into_inner()),
            )
        } else {
            None
        };
        if _admission.is_some() && admission.closing.load(Ordering::Acquire) {
            let response = json!({"build":BUILD,"connection":client.to_string(),"id":envelope["id"],"response":{"ok":false,"error":{"code":"SYSTEM_EXECUTION_CLOSING","message":"Native runner admission is closed","details":{"user_code_not_started":true,"cleanup_proven":true}}}});
            let bytes = serde_json::to_vec(&response).map_err(io::Error::other)?;
            drop(_admission);
            write_frame(pipe.0, &bytes, deadline, parent)?;
            if read_frame(pipe.0, 1, deadline, parent)? != [1] {
                return Err(io::Error::other("Invalid native receipt acknowledgement"));
            }
            return Ok(());
        }
        let runner = {
            let mut active = runners.lock().unwrap();
            if op == "identity" && !active.contains_key(&client) && active.len() < 128 {
                active.insert(client, Arc::new(Runner::new()));
            }
            active.get(&client).cloned()
        };
        match runner {
            Some(runner) if op == "release" => {
                let response = runner.call(br#"{"op":"shutdown"}"#);
                if response["result"]["exact"] == true {
                    runners.lock().unwrap().remove(&client);
                }
                response
            }
            Some(runner) => runner.call(&serde_json::to_vec(request).map_err(io::Error::other)?),
            None => {
                json!({"ok":false,"error":{"code":"SYSTEM_EXECUTION_CONNECTION_CLOSED","message":"The exact native connection is absent or closed","details":{"user_code_not_started":true,"cleanup_proven":true}}})
            }
        }
    };
    reply_rpc(pipe, parent, client, id, response, deadline)
}

fn reply_rpc(pipe: &Handle, parent: &Handle, client: Uuid, id: Uuid, response: Value, deadline: Instant) -> io::Result<()> {
    let response = json!({"build":BUILD,"connection":client.to_string(),"id":id.to_string(),"response":response});
    let bytes = serde_json::to_vec(&response).map_err(io::Error::other)?;
    validate_reply_size(bytes.len())?;
    write_frame(pipe.0, &bytes, deadline, parent)?;
    if read_frame(pipe.0, 1, deadline, parent)? != [1] {
        return Err(io::Error::other("Invalid native receipt acknowledgement"));
    }
    Ok(())
}

/// Must be called before runtime, single-instance routing, or GUI startup.
/// The child runs the same signed executable that Windows UAC approved.
pub fn helper_entry() -> Option<std::process::ExitCode> {
    HOST_SUPPORTED.store(true, Ordering::Release);
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    if arguments
        .first()
        .is_some_and(|arg| arg == "--readonly-acquire")
    {
        if arguments.len() != 1 || !elevated().unwrap_or(false) {
            return Some(std::process::ExitCode::from(77));
        }
        return Some(std::process::ExitCode::from(
            crate::readonly::nomifun_system_readonly_worker() as u8,
        ));
    }
    if !arguments.first().is_some_and(|arg| arg == MODE) {
        return None;
    }
    let result = (|| {
        if arguments.len() != 4 {
            return Err("Invalid native runner arguments".to_owned());
        }
        let key = arguments[1]
            .to_str()
            .and_then(|v| v.parse::<Uuid>().ok())
            .ok_or("Invalid runner key")?;
        let pid = arguments[2]
            .to_str()
            .and_then(|v| v.parse::<u32>().ok())
            .ok_or("Invalid parent PID")?;
        let stamp = arguments[3]
            .to_str()
            .and_then(|v| v.parse::<u64>().ok())
            .ok_or("Invalid parent identity")?;
        serve(key, pid, stamp)
    })();
    Some(if let Err(error) = result {
        eprintln!("Native system runner: {error}");
        std::process::ExitCode::from(77)
    } else {
        std::process::ExitCode::SUCCESS
    })
}
pub(super) fn unregister() -> Result<(), SystemExecutionError> {
    let cached = FACILITY
        .get_or_init(|| Mutex::new(None))
        .lock()
        .expect("runner facility mutex");
    if let Some(facility) = cached.as_ref() {
        if !facility.stopped.load(Ordering::Acquire) {
            let result = facility.exchange(Uuid::now_v7(), json!({"op":"stop_facility"}))?;
            if result["exact"] != true {
                return Err(unknown(
                    "Administrator runner shutdown did not prove exact cleanup",
                ));
            }
            facility.stopped.store(true, Ordering::Release);
        }
        let wait = unsafe { WaitForSingleObject(facility.process.0, 5000) };
        if wait != WAIT_OBJECT_0 {
            return Err(unknown(format!(
                "Administrator runner proved task cleanup but actual process exit is unconfirmed (wait={wait}, error={}); its executable cannot be replaced",
                io::Error::last_os_error()
            )));
        }
    }
    // This runner has no persistent system registration to uninstall.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_output_archive_envelope_carries_supported_raw_and_escaped_text_and_keeps_exact_boundary() {
        let supported_raw=16*1024*1024;
        // JSON can use six bytes per decoded control character and up to four
        // per u8 in each of the legacy raw array and the original frozen raw
        // array. Leave framing/chunk/decoder metadata room as well.
        let supported_capture=supported_raw*(6+4+4)+1024*1024;
        assert!(validate_reply_size(supported_capture).is_ok());
        assert!(validate_reply_size(MAX_REPLY).is_ok());
        assert!(validate_reply_size(MAX_REPLY+1).is_err());
        assert!(validate_reply_size(usize::MAX).is_err());
        let error=unknown("Native reply exceeds its envelope");
        assert_eq!(error.details["cleanup_proven"],false);
        assert_eq!(error.details["user_code_not_started"],false);
    }

    #[test]
    fn uac_launch_failure_preserves_os_facts_without_guessing_who_cancelled() {
        for raw in [1223, 5, 2, 1460] {
            let native = io::Error::from_raw_os_error(raw);
            let kind = format!("{:?}", native.kind());
            let failure = launch_failure(native);
            assert_eq!(failure.code, if raw == 1223 { "SYSTEM_EXECUTION_AUTHENTICATION_CANCELLED" } else { "SYSTEM_EXECUTION_NOT_AVAILABLE" });
            assert_eq!(failure.details["raw_os_error"], raw);
            assert_eq!(failure.details["kind"], kind);
            assert_eq!(failure.details["user_code_not_started"], true);
            assert_eq!(failure.details["cleanup_proven"], true);
        }
        let failure = launch_failure(io::Error::other("no OS code; os error 1223 is only prose"));
        assert_eq!(failure.code, "SYSTEM_EXECUTION_NOT_AVAILABLE");
        assert!(failure.details["raw_os_error"].is_null());
        let uncertain = unknown("disconnected after dispatch; UAC 1223 is only prose");
        assert_eq!(uncertain.code, "SYSTEM_EXECUTION_RESULT_UNKNOWN");
        assert_eq!(uncertain.details["user_code_not_started"], false);
        assert_eq!(uncertain.details["cleanup_proven"], false);
        assert!(uncertain.details.get("raw_os_error").is_none());
        let disconnected = unknown_io("reply failed after dispatch", io::Error::from_raw_os_error(1223));
        assert_eq!(disconnected.code, "SYSTEM_EXECUTION_RESULT_UNKNOWN");
        assert_eq!(disconnected.details["raw_os_error"], 1223);
        assert_eq!(disconnected.details["user_code_not_started"], false);
        assert_eq!(disconnected.details["cleanup_proven"], false);
    }
    // Exercise the real named-pipe framing and Runner without UAC. These are
    // transport tests, never evidence of an administrator token or product UI.
    pub(super) fn security() -> PipeSecurity {
        let sddl = wide(format!(
            "D:P(A;;GA;;;{})",
            sid(unsafe { GetCurrentProcess() }).unwrap()
        ));
        let mut raw = ptr::null_mut();
        assert_ne!(
            unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    sddl.as_ptr(),
                    1,
                    &mut raw,
                    ptr::null_mut(),
                )
            },
            0
        );
        PipeSecurity(raw)
    }
    #[test]
    fn named_pipe_framing_preserves_large_typed_receipt_and_ack() {
        let name = format!(r"\\.\pipe\NomiFun.SystemExecution.Test.{}", Uuid::now_v7());
        let pipe = listener(&name, &security(), true).unwrap();
        let expected = vec![b'x'; 2 * 1024 * 1024];
        let source = expected.clone();
        let worker = thread::spawn(move || {
            let pipe = pipe;
            let peer = process(std::process::id()).unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            connect_pipe(&pipe, &peer, None, deadline).unwrap();
            let request = read_frame(pipe.0, 32, deadline, &peer).unwrap();
            assert_eq!(request, b"typed-request");
            write_frame(pipe.0, &source, deadline, &peer).unwrap();
            assert_eq!(read_frame(pipe.0, 1, deadline, &peer).unwrap(), [1]);
        });
        let peer = process(std::process::id()).unwrap();
        let pipe = Handle::new(unsafe {
            CreateFileW(
                wide(name).as_ptr(),
                FILE_GENERIC_READ | FILE_GENERIC_WRITE,
                0,
                ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                ptr::null_mut(),
            )
        })
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        write_frame(pipe.0, b"typed-request", deadline, &peer).unwrap();
        assert_eq!(
            read_frame(pipe.0, MAX_REPLY, deadline, &peer).unwrap(),
            expected
        );
        write_frame(pipe.0, &[1], deadline, &peer).unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn lost_native_reply_after_dispatch_remains_unknown_with_actual_pipe_error() {
        let name = format!(r"\\.\pipe\NomiFun.SystemExecution.LostReply.{}", Uuid::now_v7());
        let pipe = listener(&name, &security(), true).unwrap();
        let (sent, received) = std::sync::mpsc::channel();
        let worker = thread::spawn(move || {
            let peer = process(std::process::id()).unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            connect_pipe(&pipe, &peer, None, deadline).unwrap();
            let bytes = read_frame(pipe.0, MAX_REQUEST, deadline, &peer).unwrap();
            let envelope: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(envelope["build"], BUILD);
            assert_eq!(envelope["request"]["op"], "acquire_read");
            sent.send(envelope).unwrap();
            // The actual Windows pipe disappears after accepting the whole
            // original request. There is intentionally no result or ACK.
            drop(pipe);
        });
        let facility = Facility { pipe:name, process:process(std::process::id()).unwrap(),
            stopped:AtomicBool::new(false), };
        let connection = Uuid::now_v7();
        let failure = facility.exchange(connection, json!({"op":"acquire_read","request":{"kind":"fixture"}})).unwrap_err();
        let original = received.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(original["connection"], connection.to_string());
        worker.join().unwrap();
        assert_eq!(failure.code, "SYSTEM_EXECUTION_RESULT_UNKNOWN");
        assert_eq!(failure.details["user_code_not_started"], false);
        assert_eq!(failure.details["cleanup_proven"], false);
        let raw = failure.details["raw_os_error"].as_i64().expect("the real disconnected Windows pipe supplies its native error");
        assert!(matches!(raw, 109 | 232 | 233), "unexpected real pipe error: {failure:?}");
        assert_eq!(failure.details["kind"], format!("{:?}", io::Error::from_raw_os_error(raw as i32).kind()));
        assert!(received.try_recv().is_err(), "the lost request was never replayed");
    }
    #[test]
    fn native_windows_runner_shell_cancel_terminal_and_cleanup() {
        let runner = Runner::new();
        let root = tempfile::tempdir().unwrap();
        let base = |op: &str| json!({"op":op,"invocation_id":Uuid::nil().to_string(),"call_id":Uuid::nil().to_string()});
        let mut start = base("start");
        start["cwd"] = json!(root.path());
        start["command"] = json!({"kind":"shell","script":"[Console]::Out.WriteLine('native-pipe-ready'); Start-Sleep -Seconds 60"});
        let started = runner.call(&serde_json::to_vec(&start).unwrap());
        assert_eq!(started["ok"], true, "{started}");
        let mut request = base("terminal");
        request["session_id"] = started["result"]["session_id"].clone();
        let terminal = runner.call(&serde_json::to_vec(&request).unwrap());
        assert!(terminal["result"]["terminal"].is_null(), "{terminal}");
        request["op"] = json!("poll");
        request["yield_ms"] = json!(3000);
        let poll = runner.call(&serde_json::to_vec(&request).unwrap());
        assert_eq!(poll["result"]["state"], "running", "{poll}");
        request["op"] = json!("cancel");
        let stopped = runner.call(&serde_json::to_vec(&request).unwrap());
        assert_eq!(stopped["result"]["state"], "cancelled", "{stopped}");
        assert_eq!(stopped["result"]["cleanup"]["reaped"], true, "{stopped}");
        let closed = runner.call(br#"{"op":"shutdown"}"#);
        assert_eq!(closed["result"]["exact"], true, "{closed}");
    }
    #[test]
    fn full_business_dispatch_still_accepts_exact_stop_over_the_original_pipe() {
        let slots = Arc::new(RpcDispatchSlots::new());
        let permits = (0..BUSINESS_RPC_SLOTS).map(|_| slots.admit("poll", || false).unwrap().unwrap()).collect::<Vec<_>>();
        let runner = Arc::new(Runner::new());
        let root = tempfile::tempdir().unwrap();
        let owner = |op: &str| json!({"op":op,"invocation_id":Uuid::nil().to_string(),"call_id":Uuid::nil().to_string()});
        let mut start = owner("start");
        start["cwd"] = json!(root.path());
        start["command"] = json!({"kind":"program","program":"cmd.exe","args":["/d","/c","set /p fixture="]});
        let a = runner.call(&serde_json::to_vec(&start).unwrap());
        let b = runner.call(&serde_json::to_vec(&start).unwrap());
        assert_eq!(a["ok"], true, "{a}"); assert_eq!(b["ok"], true, "{b}");
        let name = format!(r"\\.\pipe\NomiFun.SystemExecution.FullDispatch.{}", Uuid::now_v7());
        let pipe = listener(&name, &security(), true).unwrap();
        let client = Uuid::now_v7();
        let native = runner.clone(); let dispatch = slots.clone();
        let worker = thread::spawn(move || {
            let peer = process(std::process::id()).unwrap();
            connect_pipe(&pipe, &peer, None, Instant::now() + Duration::from_secs(15)).unwrap();
            let runners = Mutex::new(HashMap::from([(client, native)]));
            let stopping = AtomicBool::new(false);
            handle_rpc(&pipe, &peer, &runners, &stopping, &event().unwrap(), &FacilityAdmission::default(), &dispatch).unwrap();
            assert!(!stopping.load(Ordering::Acquire), "one Stop cannot close the shared facility");
            assert_eq!(runners.lock().unwrap().len(), 1);
        });
        let facility = Facility { pipe: name, process: process(std::process::id()).unwrap(),
            stopped: AtomicBool::new(false), };
        let mut cancel = owner("cancel"); cancel["session_id"] = a["result"]["session_id"].clone();
        let stopped = facility.exchange(client, cancel).unwrap();
        assert_eq!(stopped["cleanup"]["reaped"], true, "{stopped}");
        assert_eq!(*slots.active.lock().unwrap(), BUSINESS_RPC_SLOTS, "Stop does not release another original request's permit");
        let mut sibling = owner("poll"); sibling["session_id"] = b["result"]["session_id"].clone();
        assert_eq!(runner.call(&serde_json::to_vec(&sibling).unwrap())["result"]["state"], "running");
        worker.join().unwrap();
        drop(permits);
        assert_eq!(runner.call(br#"{"op":"shutdown"}"#)["result"]["exact"], true);
    }

    #[test]
    fn full_business_dispatch_keeps_ordinary_requests_until_their_original_slot_is_available() {
        let slots = Arc::new(RpcDispatchSlots::new());
        let mut permits = (0..BUSINESS_RPC_SLOTS).map(|_| slots.admit("poll", || false).unwrap().unwrap()).collect::<Vec<_>>();
        let dispatch = slots.clone(); let (sent, received) = std::sync::mpsc::channel();
        let worker = thread::spawn(move || {
            let _permit = dispatch.admit("write", || false).unwrap().unwrap(); sent.send(()).unwrap();
        });
        assert!(received.recv_timeout(Duration::from_millis(50)).is_err());
        for op in ["cancel", "terminate", "timeout", "quiesce", "shutdown", "release", "stop_facility",
            "observe", "terminal", "wait_terminal", "output_archive", "release_terminal"] {
            assert!(slots.admit(op, || false).unwrap().is_none(), "{op} retains an independent ingress");
        }
        drop(permits.pop());
        received.recv_timeout(Duration::from_secs(2)).unwrap(); worker.join().unwrap();
    }
    #[test]
    fn unregistered_executable_cannot_trigger_uac_or_fallback() {
        assert!(!HOST_SUPPORTED.load(Ordering::Acquire));
        assert_eq!(
            Client::connect().unwrap_err().code,
            "SYSTEM_EXECUTION_NOT_AVAILABLE"
        );
    }
    #[test]
    fn pending_native_pipe_read_deadline_cancels_and_joins_kernel_io() {
        let name = format!(
            r"\\.\pipe\NomiFun.SystemExecution.Timeout.{}",
            Uuid::now_v7()
        );
        let pipe = listener(&name, &security(), true).unwrap();
        let worker = thread::spawn(move || {
            let pipe = pipe;
            let peer = process(std::process::id()).unwrap();
            connect_pipe(&pipe, &peer, None, Instant::now() + Duration::from_secs(5)).unwrap();
            thread::sleep(Duration::from_millis(250));
        });
        let peer = process(std::process::id()).unwrap();
        let pipe = Handle::new(unsafe {
            CreateFileW(
                wide(name).as_ptr(),
                FILE_GENERIC_READ | FILE_GENERIC_WRITE,
                0,
                ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                ptr::null_mut(),
            )
        })
        .unwrap();
        let start = Instant::now();
        let error = read_frame(pipe.0, 1024, start + Duration::from_millis(80), &peer).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(start.elapsed() < Duration::from_secs(2));
        drop(pipe);
        worker.join().unwrap();
    }
    #[test]
    fn separate_native_clients_keep_independent_supervisors_and_cleanup() {
        let name = format!(
            r"\\.\pipe\NomiFun.SystemExecution.Clients.{}",
            Uuid::now_v7()
        );
        let pipe = listener(&name, &security(), true).unwrap();
        let server_name = name.clone();
        let worker = thread::spawn(move || {
            let mut pipe = pipe;
            let peer = process(std::process::id()).unwrap();
            let runners = Mutex::new(HashMap::new());
            let stopping = AtomicBool::new(false);
            let stop = event().unwrap();
            let admission = FacilityAdmission::default();
            for index in 0..10 {
                connect_pipe(&pipe, &peer, None, Instant::now() + Duration::from_secs(20)).unwrap();
                handle_rpc(&pipe, &peer, &runners, &stopping, &stop, &admission, &RpcDispatchSlots::new()).unwrap();
                unsafe {
                    DisconnectNamedPipe(pipe.0);
                }
                if index < 9 {
                    pipe = listener(&server_name, &security(), false).unwrap();
                }
            }
            assert!(runners.lock().unwrap().is_empty());
        });
        let facility = Facility {
            pipe: name,
            process: process(std::process::id()).unwrap(),
            stopped: AtomicBool::new(false),
        };
        let a = Uuid::now_v7();
        let b = Uuid::now_v7();
        facility.exchange(a, json!({"op":"identity"})).unwrap();
        facility.exchange(b, json!({"op":"identity"})).unwrap();
        let root = tempfile::tempdir().unwrap();
        let owner = |op: &str| json!({"op":op,"invocation_id":Uuid::nil().to_string(),"call_id":Uuid::nil().to_string()});
        let mut start = owner("start");
        start["cwd"] = json!(root.path());
        start["command"] = json!({"kind":"shell","script":"Start-Sleep -Seconds 60"});
        let started = facility.exchange(a, start).unwrap();
        assert_eq!(
            facility.exchange(b, json!({"op":"shutdown"})).unwrap()["exact"],
            true
        );
        let mut poll = owner("poll");
        poll["session_id"] = started["session_id"].clone();
        assert_eq!(
            facility.exchange(a, poll.clone()).unwrap()["state"],
            "running"
        );
        poll["op"] = json!("cancel");
        let stopped = facility.exchange(a, poll).unwrap();
        assert_eq!(stopped["cleanup"]["reaped"], true);
        assert_eq!(
            facility.exchange(a, json!({"op":"release"})).unwrap()["exact"],
            true
        );
        assert_eq!(
            facility.exchange(b, json!({"op":"release"})).unwrap()["exact"],
            true
        );
        assert_eq!(
            facility.exchange(a, json!({"op":"stop_facility"})).unwrap()["exact"],
            true
        );
        let refused = facility
            .exchange(Uuid::now_v7(), json!({"op":"identity"}))
            .unwrap_err();
        assert_eq!(refused.code, "SYSTEM_EXECUTION_CLOSING");
        assert_eq!(refused.details["user_code_not_started"], true);
        worker.join().unwrap();
    }
}
