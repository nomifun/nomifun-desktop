//! Native system execution is a transport for the existing process supervisor.
//! It adds no Agent state, authorization store, shell wrapper, or retry policy.
use nomi_process_runtime::{
    CleanupReport, CommandSpec, OutputCursor, OutputSnapshot, PollResult, ProcessOutcome,
    ProcessOwner, ProcessPolicy, ProcessRequest, ProcessSupervisor, SessionId, ShellKind,
    SupervisorConfig, Transport, normalize_request,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    ffi::{CString, c_char, c_void},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use uuid::Uuid;

mod readonly;
pub use readonly::{SystemReadReceipt, nomifun_system_readonly_worker};
#[cfg(target_os = "macos")]
use std::ffi::CStr;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::run_stdio_helper;

#[derive(Clone, Debug, thiserror::Error)]
#[error("{code}: {message}")]
pub struct SystemExecutionError {
    pub code: String,
    pub message: String,
    pub details: Value,
}

impl SystemExecutionError {
    fn unavailable(message: impl Into<String>) -> Self {
        Self {
            code: "SYSTEM_EXECUTION_NOT_AVAILABLE".into(),
            message: message.into(),
            details: json!({"user_code_not_started":true,"cleanup_proven":true}),
        }
    }
    fn from_value(value: &Value) -> Self {
        Self {
            code: value["code"]
                .as_str()
                .unwrap_or("system_result_unknown")
                .into(),
            message: value["message"]
                .as_str()
                .unwrap_or("Native system execution returned an invalid error")
                .into(),
            details: value.get("details").cloned().unwrap_or(Value::Null),
        }
    }
}

/// One authenticated XPC connection owns one independent supervisor in the
/// privileged service. Dropping the last client invalidates that connection;
/// the helper shuts down and proves cleanup for every admitted temporary task.
#[derive(Debug)]
pub struct SystemExecutionClient {
    #[cfg(target_os = "linux")]
    linux: linux::LinuxClient,
    #[cfg(target_os = "macos")]
    native: Arc<NativeClient>,
    #[cfg(target_os = "macos")]
    admission: Arc<tokio::sync::RwLock<()>>,
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
struct NativeClient(usize);
#[cfg(target_os = "macos")]
impl Drop for NativeClient {
    fn drop(&mut self) {
        unsafe { nomifun_system_client_drop(self.0 as *mut c_void) };
    }
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn nomifun_system_client_new(error: *mut *mut c_char) -> *mut c_void;
    fn nomifun_system_client_call(
        client: *mut c_void,
        request: *const u8,
        length: usize,
    ) -> *mut c_char;
    fn nomifun_system_client_drop(client: *mut c_void);
    fn nomifun_system_client_free(pointer: *mut c_char);
    fn nomifun_system_service_unregister() -> *mut c_char;
}

impl SystemExecutionClient {
    pub fn connect() -> Result<Self, SystemExecutionError> {
        #[cfg(target_os = "macos")]
        {
            let mut error = std::ptr::null_mut();
            let native = unsafe { nomifun_system_client_new(&mut error) };
            if native.is_null() {
                if error.is_null() {
                    return Err(SystemExecutionError::unavailable(
                        "Native administrator connection could not be created",
                    ));
                }
                let message = unsafe { CStr::from_ptr(error) }
                    .to_string_lossy()
                    .into_owned();
                unsafe { nomifun_system_client_free(error) };
                return Err(serde_json::from_str::<Value>(&message)
                    .ok()
                    .map(|value| SystemExecutionError::from_value(&value))
                    .unwrap_or_else(|| SystemExecutionError::unavailable(message)));
            }
            Ok(Self {
                native: Arc::new(NativeClient(native as usize)),
                admission: Arc::new(tokio::sync::RwLock::new(())),
            })
        }
        #[cfg(target_os = "linux")]
        {
            Ok(Self { linux: linux::LinuxClient::connect()? })
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            Err(SystemExecutionError::unavailable(
                "Native administrator execution awaits this platform's implementation and native validation",
            ))
        }
    }

    /// Linux authentication is an asynchronous OS interaction. Dropping this
    /// client while approval is pending closes its transport and cancels that
    /// attempt before any user operation is dispatched.
    pub async fn wait_ready(&self) -> Result<(), SystemExecutionError> {
        #[cfg(target_os = "linux")]
        { self.linux.wait_ready().await }
        #[cfg(not(target_os = "linux"))]
        { Ok(()) }
    }

    pub async fn call(&self, request: Value) -> Result<Value, SystemExecutionError> {
        #[cfg(target_os = "macos")]
        {
            let native = self.native.clone();
            // Fence starts at the client transport as well as in the native
            // supervisor. A cancelled awaiting future may leave a blocking RPC
            // queued in Tokio; quiesce must wait for that queued start to finish
            // before requesting the helper's exact process-tree fence.
            enum Admission {
                Start {
                    _guard: tokio::sync::OwnedRwLockReadGuard<()>,
                },
                Fence {
                    _guard: tokio::sync::OwnedRwLockWriteGuard<()>,
                },
                None,
            }
            let admission = match request["op"].as_str() {
                Some("start" | "acquire_read") => Admission::Start {
                    _guard: self.admission.clone().read_owned().await,
                },
                Some("quiesce" | "shutdown") => Admission::Fence {
                    _guard: self.admission.clone().write_owned().await,
                },
                _ => Admission::None,
            };
            // Retain the native connection inside the blocking worker too;
            // cancelling the awaiting future cannot invalidate its pointer.
            let result = tokio::task::spawn_blocking(move || {
                let _admission = admission;
                let bytes =
                    serde_json::to_vec(&request).expect("JSON value serialization cannot fail");
                let response = unsafe {
                    nomifun_system_client_call(native.0 as *mut c_void, bytes.as_ptr(), bytes.len())
                };
                if response.is_null() {
                    return Err(SystemExecutionError {
                        code: "system_result_unknown".into(),
                        message: "Native administrator service returned no response".into(),
                        details: json!({"user_code_not_started":false,"cleanup_proven":false}),
                    });
                }
                let bytes = unsafe { CStr::from_ptr(response) }.to_bytes().to_owned();
                unsafe { nomifun_system_client_free(response) };
                let value: Value =
                    serde_json::from_slice(&bytes).map_err(|error| SystemExecutionError {
                        code: "system_result_unknown".into(),
                        message: format!("Invalid native administrator response: {error}"),
                        details: json!({"user_code_not_started":false,"cleanup_proven":false}),
                    })?;
                if value["ok"] == true {
                    Ok(value["result"].clone())
                } else {
                    Err(SystemExecutionError::from_value(&value["error"]))
                }
            })
            .await
            .map_err(|error| SystemExecutionError {
                code: "system_result_unknown".into(),
                message: format!("Native administrator transport worker stopped: {error}"),
                details: json!({"user_code_not_started":false,"cleanup_proven":false}),
            })?;
            result
        }
        #[cfg(target_os = "linux")]
        {
            self.linux.call(request).await
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            let _ = request;
            Err(SystemExecutionError::unavailable(
                "Native administrator execution is not implemented on this platform",
            ))
        }
    }

    /// Called by the application's uninstall action, while its signed bundle
    /// still exists. This removes the operating-system registration itself.
    pub fn unregister() -> Result<(), SystemExecutionError> {
        #[cfg(target_os = "macos")]
        {
            let error = unsafe { nomifun_system_service_unregister() };
            if error.is_null() {
                return Ok(());
            }
            let message = unsafe { CStr::from_ptr(error) }
                .to_string_lossy()
                .into_owned();
            unsafe { nomifun_system_client_free(error) };
            Err(SystemExecutionError::unavailable(message))
        }
        #[cfg(target_os = "linux")]
        {
            Err(SystemExecutionError::unavailable(
                "Remove the Linux system-execution package component with its native package manager",
            ))
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            Err(SystemExecutionError::unavailable(
                "Native administrator execution is not implemented on this platform",
            ))
        }
    }
}

struct Runner {
    runtime: tokio::runtime::Runtime,
    supervisor: Arc<ProcessSupervisor>,
}

fn field<'a>(request: &'a Value, key: &str) -> Result<&'a str, String> {
    request[key]
        .as_str()
        .ok_or_else(|| format!("{key} must be a string"))
}
fn owner(request: &Value) -> Result<ProcessOwner, String> {
    Ok(ProcessOwner::new(
        Uuid::parse_str(field(request, "invocation_id")?).map_err(|e| e.to_string())?,
        Uuid::parse_str(field(request, "call_id")?).map_err(|e| e.to_string())?,
    ))
}
fn session(request: &Value) -> Result<SessionId, String> {
    field(request, "session_id")?
        .parse()
        .map_err(|e: uuid::Error| e.to_string())
}
fn cursor(request: &Value) -> OutputCursor {
    OutputCursor::new(request["cursor"].as_u64().unwrap_or(0))
}
fn output(value: &OutputSnapshot) -> Value {
    json!({"chunks":value.chunks.iter().map(output_chunk).collect::<Vec<_>>(),"next_cursor":value.next_cursor.offset(),"retained_bytes":value.retained_bytes,"dropped_bytes":value.dropped_bytes,"encoding":{"source_encoding":value.encoding.source_encoding,"decode_errors":value.encoding.decode_errors}})
}
fn output_chunk(chunk: &nomi_process_runtime::OutputChunk) -> Value {
    let value = json!({"seq":chunk.seq,"start":chunk.start,"stream":match chunk.stream {
        nomi_process_runtime::OutputStream::Stdout=>"stdout",nomi_process_runtime::OutputStream::Stderr=>"stderr",nomi_process_runtime::OutputStream::Pty=>"pty"
    },"text":chunk.text});
    // The Linux engine consumes text and its original encoding/counts. Do not
    // materialize an unused second JSON byte array for every output chunk.
    // Readonly acquisition reads native bytes directly inside its supervisor.
    #[cfg(not(target_os = "linux"))]
    {
        let mut value = value;
        value["bytes"] = json!(chunk.bytes);
        value
    }
    #[cfg(target_os = "linux")]
    { value }
}
fn cleanup(value: &CleanupReport) -> Value {
    json!({"interrupt_attempted":value.interrupt_attempted,"terminate_attempted":value.terminate_attempted,"force_kill_attempted":value.force_kill_attempted,"reaped":value.reaped,"elapsed_ms":value.elapsed.as_millis(),"errors":value.errors})
}
fn startup_reports(values: &[nomi_process_runtime::StartupCleanupReport]) -> Value {
    json!(values.iter().map(|value| json!({"session_id":value.session_id.to_string(),"invocation_id":value.owner.invocation_id.to_string(),"call_id":value.owner.call_id.to_string(),"failure":{"code":value.failure.code,"message":value.failure.message},"cleanup":cleanup(&value.cleanup),"user_code_not_started":value.user_code_not_started})).collect::<Vec<_>>())
}
fn outcome(value: &ProcessOutcome) -> Value {
    match value {
        ProcessOutcome::Exited {
            code,
            signal,
            output: out,
            cleanup: report,
        } => {
            json!({"state":"exited","code":code,"signal":signal,"output":output(out),"cleanup":cleanup(report)})
        }
        ProcessOutcome::Cancelled {
            output: out,
            cleanup: report,
        } => json!({"state":"cancelled","output":output(out),"cleanup":cleanup(report)}),
        ProcessOutcome::TimedOut {
            output: out,
            cleanup: report,
        } => json!({"state":"timed_out","output":output(out),"cleanup":cleanup(report)}),
        ProcessOutcome::Lost {
            last_known,
            output: out,
            cleanup: report,
        } => {
            json!({"state":"lost","pid":last_known.pid,"output":output(out),"cleanup":cleanup(report)})
        }
        ProcessOutcome::SpawnFailed(failure) => {
            json!({"state":"spawn_failed","failure":{"code":failure.code,"message":failure.message}})
        }
    }
}

impl Runner {
    fn new() -> Self {
        Self {
            runtime: tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("system execution runtime"),
            supervisor: ProcessSupervisor::new(SupervisorConfig::default()),
        }
    }
    async fn perform(&self, request: Value) -> Result<Value, (String, String, Value)> {
        let invalid = |message: String| {
            (
                "invalid_system_request".into(),
                message,
                json!({"user_code_not_started":true,"cleanup_proven":true}),
            )
        };
        let process_error = |error: nomi_process_runtime::ProcessError| {
            let details = match &error {
                nomi_process_runtime::ProcessError::StartLost {
                    failure,
                    last_known,
                    cleanup: report,
                } => {
                    json!({"failure":{"code":failure.code,"message":failure.message},"last_known_pid":last_known.as_ref().map(|v|v.pid),"cleanup":cleanup(report),"user_code_not_started":false,"cleanup_proven":report.reaped})
                }
                nomi_process_runtime::ProcessError::SpawnFailed { failure } => {
                    json!({"failure":{"code":failure.code,"message":failure.message},"user_code_not_started":true,"cleanup_proven":true})
                }
                nomi_process_runtime::ProcessError::InvalidWorkingDirectory { .. }
                | nomi_process_runtime::ProcessError::InvalidCommand { .. }
                | nomi_process_runtime::ProcessError::InvalidTransport { .. }
                | nomi_process_runtime::ProcessError::CapacityExhausted { .. } => {
                    json!({"user_code_not_started":true,"cleanup_proven":true})
                }
                _ => json!({"user_code_not_started":false,"cleanup_proven":false}),
            };
            (error.code().into(), error.to_string(), details)
        };
        match field(&request, "op").map_err(invalid)? {
            "acquire_read" => {
                let request = serde_json::from_value(request["request"].clone()).map_err(|error|invalid(error.to_string()))?;
                self.acquire_read(request).await
            }
            "identity" => Ok(
                json!({"effective_uid":effective_uid(),"protocol_version":1,"build_identity":option_env!("NOMIFUN_SYSTEM_EXECUTION_BUILD_ID").unwrap_or("platform-not-implemented")}),
            ),
            "start" => {
                let command = match field(&request["command"], "kind").map_err(invalid)? {
                    "program" => CommandSpec::Program {
                        program: field(&request["command"], "program")
                            .map_err(invalid)?
                            .into(),
                        args: request["command"]["args"]
                            .as_array()
                            .ok_or_else(|| invalid("args must be an array".into()))?
                            .iter()
                            .map(|arg| {
                                arg.as_str()
                                    .map(Into::into)
                                    .ok_or_else(|| invalid("args must contain strings".into()))
                            })
                            .collect::<Result<_, _>>()?,
                    },
                    "shell" => CommandSpec::Shell {
                        shell: ShellKind::Posix,
                        script: field(&request["command"], "script")
                            .map_err(invalid)?
                            .into(),
                    },
                    _ => return Err(invalid("unsupported command kind".into())),
                };
                let cwd = PathBuf::from(field(&request, "cwd").map_err(invalid)?);
                if !cwd.is_absolute() {
                    return Err(invalid("system process cwd must be resolved to an absolute native path by the caller".into()));
                }
                let mut env = BTreeMap::new();
                if let Some(values) = request["env"].as_object() {
                    for (key, value) in values {
                        env.insert(
                            key.into(),
                            value
                                .as_str()
                                .ok_or_else(|| {
                                    invalid("environment values must be strings".into())
                                })?
                                .into(),
                        );
                    }
                }
                let transport = match request["transport"]["kind"].as_str().unwrap_or("pipe") {
                    "pipe" => Transport::Pipe,
                    "pty" => Transport::Pty {
                        cols: u16::try_from(request["transport"]["cols"].as_u64().unwrap_or(80))
                            .map_err(|_| invalid("invalid PTY cols".into()))?,
                        rows: u16::try_from(request["transport"]["rows"].as_u64().unwrap_or(24))
                            .map_err(|_| invalid("invalid PTY rows".into()))?,
                    },
                    _ => return Err(invalid("unsupported process transport".into())),
                };
                let mut policy = ProcessPolicy::default();
                policy.expire_on_idle = request["expire_on_idle"].as_bool().unwrap_or(true);
                policy.output_limit_bytes = request["output_limit_bytes"]
                    .as_u64()
                    .map(|value| usize::try_from(value).unwrap_or(usize::MAX))
                    .unwrap_or(policy.output_limit_bytes);
                if let Some(value) = request.get("timeout_ms").filter(|value|!value.is_null()) {
                    let timeout=value.as_u64().filter(|timeout|*timeout>0).ok_or_else(||invalid("process timeout must be a positive integer".into()))?;
                    policy.deadline=Some(Instant::now().checked_add(Duration::from_millis(timeout))
                        .ok_or_else(||invalid("process deadline overflows the native monotonic clock".into()))?);
                }
                let normalized = normalize_request(
                    ProcessRequest {
                        owner: owner(&request).map_err(invalid)?,
                        command,
                        cwd: cwd.clone(),
                        env,
                        transport,
                        policy,
                    },
                    &cwd,
                )
                .map_err(process_error)?;
                let handle = self
                    .supervisor
                    .start(normalized)
                    .await
                    .map_err(process_error)?;
                Ok(json!({"session_id":handle.session_id.to_string(),"pid":handle.pid}))
            }
            "quiesce" => {
                let report = self.supervisor.quiesce().await;
                Ok(
                    json!({"exact":report.is_exact(),"sessions":report.sessions.iter().map(|session|json!({"session_id":session.session_id.to_string(),"invocation_id":session.owner.invocation_id.to_string(),"call_id":session.owner.call_id.to_string(),"outcome":outcome(&session.outcome)})).collect::<Vec<_>>(),"startups":startup_reports(&report.startups),"startup_count":report.startups.len(),"errors":report.errors}),
                )
            }
            "shutdown" => {
                let report = self.supervisor.shutdown().await;
                Ok(
                    json!({"exact":report.is_exact(),"sessions":report.sessions.iter().map(|session|json!({"session_id":session.session_id.to_string(),"invocation_id":session.owner.invocation_id.to_string(),"call_id":session.owner.call_id.to_string(),"outcome":outcome(&session.outcome)})).collect::<Vec<_>>(),"startups":startup_reports(&report.startups),"startup_count":report.startups.len()}),
                )
            }
            op => {
                let owner = owner(&request).map_err(invalid)?;
                let session = session(&request).map_err(invalid)?;
                match op {
                    "observe" => {
                        let observation = self.supervisor.observe_if_changed(
                            &owner, &session, cursor(&request), request["include_output"].as_bool().unwrap_or(false),
                        ).map_err(process_error)?;
                        let observation = observation.map(|value| match value {
                            PollResult::Running { snapshot, output: out } =>
                                json!({"state":"running","pid":snapshot.pid,"output":output(&out)}),
                            PollResult::Finished(value) => outcome(&value),
                        });
                        Ok(json!({"observation":observation}))
                    }
                    "terminal" => {
                        let terminal = self
                            .supervisor
                            .terminal_outcome_if_ready(&owner, &session, cursor(&request))
                            .map_err(process_error)?;
                        Ok(json!({"terminal":terminal.as_ref().map(outcome)}))
                    }
                    "poll" => {
                        let until = Instant::now()
                            + Duration::from_millis(
                                request["yield_ms"].as_u64().unwrap_or(0).min(30_000),
                            );
                        let polled = if request["break_on_output"].as_bool().unwrap_or(true) {
                            self.supervisor.poll_until_activity(&owner, &session, cursor(&request), until).await
                        } else {
                            self.supervisor.poll(&owner, &session, cursor(&request), until).await
                        }.map_err(process_error)?;
                        match polled {
                            PollResult::Running {
                                snapshot,
                                output: out,
                            } => Ok(
                                json!({"state":"running","pid":snapshot.pid,"output":output(&out)}),
                            ),
                            PollResult::Finished(value) => Ok(outcome(&value)),
                        }
                    }
                    "write" => {
                        let bytes: Vec<u8> = serde_json::from_value(request["bytes"].clone())
                            .map_err(|error| invalid(error.to_string()))?;
                        self.supervisor
                            .write(&owner, &session, &bytes)
                            .await
                            .map_err(process_error)?;
                        Ok(json!({}))
                    }
                    "close_stdin" => {
                        self.supervisor
                            .close_stdin(&owner, &session)
                            .await
                            .map_err(process_error)?;
                        Ok(json!({}))
                    }
                    "resize" => {
                        let cols = u16::try_from(request["cols"].as_u64().unwrap_or(0))
                            .map_err(|_| invalid("invalid cols".into()))?;
                        let rows = u16::try_from(request["rows"].as_u64().unwrap_or(0))
                            .map_err(|_| invalid("invalid rows".into()))?;
                        self.supervisor
                            .resize(&owner, &session, cols, rows)
                            .await
                            .map_err(process_error)?;
                        Ok(json!({}))
                    }
                    "interrupt" => {
                        self.supervisor
                            .interrupt(&owner, &session)
                            .await
                            .map_err(process_error)?;
                        Ok(json!({}))
                    }
                    "cancel" => Ok(outcome(
                        &self
                            .supervisor
                            .cancel(&owner, &session)
                            .await
                            .map_err(process_error)?,
                    )),
                    "terminate" => Ok(outcome(
                        &self
                            .supervisor
                            .terminate(&owner, &session)
                            .await
                            .map_err(process_error)?,
                    )),
                    "timeout" => Ok(outcome(
                        &self
                            .supervisor
                            .timeout(&owner, &session)
                            .await
                            .map_err(process_error)?,
                    )),
                    _ => Err(invalid(format!(
                        "unsupported native system execution operation {op}"
                    ))),
                }
            }
        }
    }
    fn call(&self, bytes: &[u8]) -> Value {
        let request: Value = match serde_json::from_slice(bytes) {
            Ok(value) => value,
            Err(error) => {
                return json!({"ok":false,"error":{"code":"invalid_system_request","message":error.to_string(),"details":{"user_code_not_started":true,"cleanup_proven":true}}});
            }
        };
        match self.runtime.block_on(self.perform(request)) {
            Ok(result) => json!({"ok":true,"result":result}),
            Err((code, message, details)) => {
                json!({"ok":false,"error":{"code":code,"message":message,"details":details}})
            }
        }
    }
}

fn effective_uid() -> u32 {
    #[cfg(unix)]
    {
        unsafe extern "C" {
            fn geteuid() -> u32;
        }
        unsafe { geteuid() }
    }
    #[cfg(not(unix))]
    {
        u32::MAX
    }
}

#[cfg(target_os = "macos")]
#[unsafe(no_mangle)]
pub extern "C" fn nomifun_system_execution_build_identity() -> *const c_char {
    concat!(env!("NOMIFUN_SYSTEM_EXECUTION_BUILD_ID"), "\0")
        .as_ptr()
        .cast()
}

#[unsafe(no_mangle)]
pub extern "C" fn nomifun_system_runner_new() -> *mut c_void {
    std::panic::catch_unwind(|| Box::into_raw(Box::new(Runner::new())).cast())
        .unwrap_or(std::ptr::null_mut())
}
/// Safety: pointer must come from runner_new, remain alive throughout this
/// call, and bytes must address length readable bytes. Multiple calls may run
/// concurrently; the supervisor provides the admission and shutdown fences.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nomifun_system_runner_call(
    pointer: *mut c_void,
    bytes: *const u8,
    length: usize,
) -> *mut c_char {
    let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let runner=unsafe{&*pointer.cast::<Runner>()};
        let request=unsafe{std::slice::from_raw_parts(bytes,length)};
        runner.call(request)
    })).unwrap_or_else(|_|json!({"ok":false,"error":{"code":"system_result_unknown","message":"Native process supervisor panicked; the operation result is unknown","details":{"user_code_not_started":false,"cleanup_proven":false}}}));
    CString::new(result.to_string())
        .expect("JSON has no NUL")
        .into_raw()
}
/// Safety: pointer must be a live runner_new result and no calls may remain.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nomifun_system_runner_drop(pointer: *mut c_void) {
    if pointer.is_null() {
        return;
    }
    let runner = unsafe { Box::from_raw(pointer.cast::<Runner>()) };
    let _ = runner.runtime.block_on(runner.supervisor.shutdown());
}
/// Safety: pointer must be an unfreed runner_call result.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nomifun_system_runner_free(pointer: *mut c_char) {
    if !pointer.is_null() {
        drop(unsafe { CString::from_raw(pointer) });
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    fn base(op: &str) -> Value {
        json!({"op":op,"invocation_id":"00000000-0000-0000-0000-000000000001","call_id":"00000000-0000-0000-0000-000000000002"})
    }
    #[test]
    fn native_runner_rejects_invalid_deadlines_before_spawn_and_accepts_explicit_long_workloads() {
        let runner=Runner::new();let dir=tempfile::tempdir().unwrap();
        let marker=dir.path().join("must-not-start");
        let mut invalid_values=vec![json!(0),json!(-1),json!("not-a-duration")];
        if Instant::now().checked_add(Duration::from_millis(u64::MAX)).is_none(){invalid_values.push(json!(u64::MAX));}
        for timeout in invalid_values {
            let mut request=base("start");request["cwd"]=json!(dir.path());request["timeout_ms"]=timeout;
            request["command"]=json!({"kind":"program","program":"/usr/bin/touch","args":[marker]});
            let result=runner.call(request.to_string().as_bytes());
            assert_eq!(result["ok"],false,"{result}");
            assert_eq!(result["error"]["details"]["user_code_not_started"],true);
            assert!(!marker.exists());
        }
        let mut request=base("start");request["cwd"]=json!(dir.path());request["timeout_ms"]=json!(3_600_000);
        request["command"]=json!({"kind":"program","program":"/bin/sh","args":["-c","exit 0"]});
        let result=runner.call(request.to_string().as_bytes());assert_eq!(result["ok"],true,"{result}");
        let mut stop=base("cancel");stop["session_id"]=result["result"]["session_id"].clone();
        assert_eq!(runner.call(stop.to_string().as_bytes())["result"]["cleanup"]["reaped"],true);
    }
    #[test]
    fn native_runner_retains_literal_arguments_and_real_user_environment() {
        let runner = Runner::new();
        let dir = tempfile::tempdir().unwrap();
        let mut start = base("start");
        start["cwd"] = json!(dir.path());
        start["command"] = json!({"kind":"program","program":"/usr/bin/printf","args":["%s|%s|%s","","中文 空格","$(false);x"]});
        start["transport"] = json!({"kind":"pipe"});
        start["env"] = json!({"HOME":dir.path()});
        let response = runner.call(start.to_string().as_bytes());
        assert_eq!(response["ok"], true, "{response}");
        let mut poll = base("poll");
        poll["session_id"] = response["result"]["session_id"].clone();
        poll["yield_ms"] = json!(2000);
        let mut text = String::new();
        for _ in 0..10 {
            let result = runner.call(poll.to_string().as_bytes());
            assert_eq!(result["ok"], true, "{result}");
            for chunk in result["result"]["output"]["chunks"].as_array().unwrap() {
                text.push_str(chunk["text"].as_str().unwrap());
            }
            poll["cursor"] = result["result"]["output"]["next_cursor"].clone();
            if result["result"]["state"] != "running" {
                assert_eq!(result["result"]["code"], 0);
                assert_eq!(result["result"]["cleanup"]["reaped"], true);
                break;
            }
        }
        assert_eq!(text, "|中文 空格|$(false);x");
        let _ = runner.call(br#"{"op":"shutdown"}"#);
    }
    #[test]
    fn native_runner_cancel_quiesce_and_stdin_use_shared_supervision() {
        let runner = Runner::new();
        let dir = tempfile::tempdir().unwrap();
        let mut start = base("start");
        start["cwd"] = json!(dir.path());
        start["command"] = json!({"kind":"program","program":"/bin/cat","args":[]});
        start["transport"] = json!({"kind":"pipe"});
        let response = runner.call(start.to_string().as_bytes());
        assert_eq!(response["ok"], true, "{response}");
        let mut write = base("write");
        write["session_id"] = response["result"]["session_id"].clone();
        write["bytes"] = json!(b"fixture\n".to_vec());
        assert_eq!(runner.call(write.to_string().as_bytes())["ok"], true);
        let mut cancel = write;
        cancel["op"] = json!("cancel");
        let response = runner.call(cancel.to_string().as_bytes());
        assert_eq!(response["result"]["state"], "cancelled");
        assert_eq!(response["result"]["cleanup"]["reaped"], true, "{response}");
        let response = runner.call(br#"{"op":"quiesce"}"#);
        assert_eq!(response["result"]["exact"], true, "{response}");
    }
    #[test]
    fn native_runner_pty_resize_timeout_and_user_home_are_preserved() {
        let runner = Runner::new();
        let dir = tempfile::tempdir().unwrap();
        let mut start = base("start");
        start["cwd"] = json!(dir.path());
        start["env"] = json!({"HOME": dir.path()});
        start["command"] = json!({"kind":"shell","script":"printf 'HOME=%s\\n' \"$HOME\"; read line; printf 'INPUT=%s\\n' \"$line\"; exec /bin/sleep 60"});
        start["transport"] = json!({"kind":"pty","cols":80,"rows":24});
        let started = runner.call(start.to_string().as_bytes());
        assert_eq!(started["ok"], true, "{started}");
        let mut request = base("resize");
        request["session_id"] = started["result"]["session_id"].clone();
        request["cols"] = json!(100);
        request["rows"] = json!(40);
        assert_eq!(runner.call(request.to_string().as_bytes())["ok"], true);
        request["op"] = json!("write");
        request["bytes"] = json!(b"native-pty\n".to_vec());
        assert_eq!(runner.call(request.to_string().as_bytes())["ok"], true);
        request["op"] = json!("poll");
        request["yield_ms"] = json!(2000);
        let mut text = String::new();
        for _ in 0..10 {
            let value = runner.call(request.to_string().as_bytes());
            assert_eq!(value["ok"], true, "{value}");
            for chunk in value["result"]["output"]["chunks"].as_array().unwrap() {
                text.push_str(chunk["text"].as_str().unwrap());
            }
            request["cursor"] = value["result"]["output"]["next_cursor"].clone();
            if text.contains("INPUT=native-pty") {
                break;
            }
        }
        assert!(
            text.contains(&format!("HOME={}", dir.path().display())),
            "{text}"
        );
        assert!(text.contains("INPUT=native-pty"), "{text}");
        request["op"] = json!("timeout");
        let value = runner.call(request.to_string().as_bytes());
        assert_eq!(value["result"]["state"], "timed_out", "{value}");
        assert_eq!(value["result"]["cleanup"]["reaped"], true, "{value}");
    }
    #[test]
    fn native_runner_invalid_cwd_proves_no_task_started() {
        let runner = Runner::new();
        let dir = tempfile::tempdir().unwrap();
        let mut start = base("start");
        start["cwd"] = json!(dir.path().join("missing"));
        start["command"] = json!({"kind":"program","program":"/bin/true","args":[]});
        let value = runner.call(start.to_string().as_bytes());
        assert_eq!(value["ok"], false);
        assert_eq!(value["error"]["code"], "invalid_working_directory");
        assert_eq!(value["error"]["details"]["user_code_not_started"], true);
        assert_eq!(value["error"]["details"]["cleanup_proven"], true);
    }
    #[test]
    fn native_terminal_observation_preserves_output_without_poll_or_lease_renewal() {
        let runner = Runner::new();
        let dir = tempfile::tempdir().unwrap();
        let mut start = base("start");
        start["cwd"] = json!(dir.path());
        start["command"] =
            json!({"kind":"shell","script":"printf 'final-output'; /bin/sleep 0.15; exit 7"});
        let started = runner.call(start.to_string().as_bytes());
        assert_eq!(started["ok"], true, "{started}");
        let mut request = base("terminal");
        request["session_id"] = started["result"]["session_id"].clone();
        let first = runner.call(request.to_string().as_bytes());
        assert_eq!(first["ok"], true);
        assert!(first["result"]["terminal"].is_null(), "{first}");
        let until = Instant::now() + Duration::from_secs(5);
        let terminal = loop {
            let value = runner.call(request.to_string().as_bytes());
            assert_eq!(value["ok"], true, "{value}");
            if !value["result"]["terminal"].is_null() {
                break value["result"]["terminal"].clone();
            }
            assert!(
                Instant::now() < until,
                "native terminal observation never completed"
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(terminal["state"], "exited");
        assert_eq!(terminal["code"], 7);
        assert_eq!(terminal["cleanup"]["reaped"], true);
        let text = terminal["output"]["chunks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|chunk| chunk["text"].as_str().unwrap())
            .collect::<String>();
        assert_eq!(text, "final-output");
        let again = runner.call(request.to_string().as_bytes());
        assert_eq!(again["result"]["terminal"], terminal);
    }

    #[test]
    fn native_owner_observation_exposes_running_output_and_preserves_hard_deadline() {
        let runner = Runner::new();
        let dir = tempfile::tempdir().unwrap();
        let mut start = base("start");
        start["cwd"] = json!(dir.path());
        start["timeout_ms"] = json!(200);
        start["command"] = json!({"kind":"shell","script":"printf 'verification-link'; IFS= read -r answer"});
        let started = runner.call(start.to_string().as_bytes());
        assert_eq!(started["ok"], true, "{started}");
        let mut request = base("observe");
        request["session_id"] = started["result"]["session_id"].clone();
        request["include_output"] = json!(true);
        let until = Instant::now() + Duration::from_secs(3);
        let mut saw_running = false;
        loop {
            let value = runner.call(request.to_string().as_bytes());
            assert_eq!(value["ok"], true, "{value}");
            let observed = &value["result"]["observation"];
            if observed["state"] == "running" {
                saw_running = true;
                assert!(observed["output"]["next_cursor"].as_u64().unwrap() > 0);
                let mut terminal_only = request.clone();
                terminal_only["include_output"] = json!(false);
                let hidden = runner.call(terminal_only.to_string().as_bytes());
                assert!(hidden["result"]["observation"].is_null() || hidden["result"]["observation"]["state"] == "timed_out");
            } else if observed["state"] == "timed_out" {
                assert!(saw_running, "the first output must be visible before expiry");
                assert_eq!(observed["cleanup"]["reaped"], true);
                assert!(observed["output"]["chunks"].as_array().unwrap().iter().any(|chunk| chunk["text"] == "verification-link"));
                break;
            }
            assert!(Instant::now() < until, "read-only observations must not renew the hard deadline");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
