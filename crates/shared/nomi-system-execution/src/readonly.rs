//! Read-only acquisition runs in an owned native child of the existing
//! supervisor. Slow filesystem IO never occupies an unkillable daemon worker.
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::PathBuf,
    time::{Duration, Instant},
};

use nomi_native_files::{NativeReadAcquisition, NativeReadRequest};
use nomi_process_runtime::{
    CommandSpec, OutputCursor, OutputStream, PollResult, ProcessOutcome, ProcessOwner,
    ProcessPolicy, ProcessRequest, Transport, normalize_request,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{Runner, SystemExecutionClient, SystemExecutionError, cleanup};

const MAX_REQUEST_BYTES: usize = 64 * 1024;
// 8 MiB source is 11.19 MiB in base64, plus bounded path/scope/JSON metadata.
const MAX_REPLY_BYTES: usize = 16 * 1024 * 1024;
const READ_DEADLINE: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemReadReceipt {
    pub acquisition: NativeReadAcquisition,
    pub worker: Value,
}

impl SystemExecutionClient {
    /// Registration/approval is the same OS component used for process work.
    /// The caller must retain this future in its ordinary owned task group.
    pub async fn connect_for_read() -> Result<Self, SystemExecutionError> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
        loop {
            let connected = tokio::task::spawn_blocking(Self::connect)
                .await
                .map_err(|error| SystemExecutionError {
                    code: "SYSTEM_READ_RESULT_UNKNOWN".into(),
                    message: format!("Native readonly connection worker stopped: {error}"),
                    details: json!({"cleanup_proven":false}),
                })?;
            match connected {
                Ok(client) => return Ok(client),
                Err(error)
                    if error.code == "SYSTEM_EXECUTION_APPROVAL_REQUIRED"
                        && tokio::time::Instant::now() < deadline =>
                {
                    tokio::time::sleep(Duration::from_millis(750)).await
                }
                Err(error) => return Err(error),
            }
        }
    }

    pub async fn acquire_read(
        &self,
        request: NativeReadRequest,
    ) -> Result<SystemReadReceipt, SystemExecutionError> {
        let reply = self
            .call(json!({"op":"acquire_read","request":request}))
            .await?;
        let receipt: SystemReadReceipt =
            serde_json::from_value(reply).map_err(|error| SystemExecutionError {
                code: "SYSTEM_READ_RESULT_UNKNOWN".into(),
                message: format!("Native readonly receipt is invalid: {error}"),
                details: json!({"cleanup_proven":false}),
            })?;
        if receipt.worker["state"] != "exited"
            || receipt.worker["exit_code"] != 0
            || !receipt.worker["signal"].is_null()
            || receipt
                .worker
                .pointer("/cleanup/reaped")
                .and_then(Value::as_bool)
                != Some(true)
        {
            return Err(SystemExecutionError {
                code: "SYSTEM_READ_RESULT_UNKNOWN".into(),
                message: "Native readonly owner did not prove its worker completed and was reaped"
                    .into(),
                details: json!({"worker":receipt.worker,"cleanup_proven":false}),
            });
        }
        Ok(receipt)
    }
}

type Failure = (String, String, Value);

fn unknown(message: impl Into<String>, worker: Value) -> Failure {
    let proven = worker
        .pointer("/cleanup/reaped")
        .or_else(|| worker.pointer("/stop/cleanup/reaped"))
        .and_then(Value::as_bool)
        == Some(true);
    (
        "SYSTEM_READ_RESULT_UNKNOWN".into(),
        message.into(),
        json!({"worker":worker,"cleanup_proven":proven,"result_known":false}),
    )
}

impl Runner {
    pub(crate) async fn acquire_read(&self, request: NativeReadRequest) -> Result<Value, Failure> {
        let program = std::env::current_exe().map_err(|error| {
            unknown(
                format!("Cannot locate signed native readonly worker: {error}"),
                Value::Null,
            )
        })?;
        self.acquire_read_with_command(
            request,
            CommandSpec::Program {
                program: program.into_os_string(),
                args: vec!["--readonly-acquire".into()],
            },
            READ_DEADLINE,
        )
        .await
    }

    async fn acquire_read_with_command(
        &self,
        request: NativeReadRequest,
        command: CommandSpec,
        deadline: Duration,
    ) -> Result<Value, Failure> {
        self.acquire_read_observed(request, command, deadline, |_| {})
            .await
    }

    async fn acquire_read_observed<O: FnOnce(nomi_process_runtime::ProcessHandle) + Send>(
        &self,
        request: NativeReadRequest,
        command: CommandSpec,
        deadline: Duration,
        on_ready: O,
    ) -> Result<Value, Failure> {
        let payload = serde_json::to_vec(&request)
            .map_err(|error| unknown(error.to_string(), Value::Null))?;
        if payload.len() > MAX_REQUEST_BYTES {
            return Err((
                "invalid_system_read_request".into(),
                "Native read request exceeds its envelope".into(),
                json!({"user_code_not_started":true,"cleanup_proven":true}),
            ));
        }
        let owner = ProcessOwner::new(Uuid::now_v7(), Uuid::now_v7());
        let policy = ProcessPolicy {
            output_limit_bytes: MAX_REPLY_BYTES,
            deadline: Some(Instant::now() + deadline),
            expire_on_idle: false,
            ..Default::default()
        };
        let normalized = normalize_request(
            ProcessRequest {
                owner: owner.clone(),
                command,
                cwd: PathBuf::from("/"),
                env: BTreeMap::new(),
                transport: Transport::Pipe,
                policy,
            },
            std::path::Path::new("/"),
        )
        .map_err(|error| {
            (
                error.code().into(),
                error.to_string(),
                json!({"user_code_not_started":true,"cleanup_proven":true}),
            )
        })?;
        let handle = self.supervisor.start(normalized).await.map_err(|error| {
            let details = match &error {
                nomi_process_runtime::ProcessError::StartLost { cleanup:report,.. } => json!({"cleanup":cleanup(report),"cleanup_proven":report.reaped,"user_code_not_started":false}),
                nomi_process_runtime::ProcessError::SpawnFailed { .. }
                | nomi_process_runtime::ProcessError::InvalidWorkingDirectory { .. }
                | nomi_process_runtime::ProcessError::InvalidCommand { .. }
                | nomi_process_runtime::ProcessError::InvalidTransport { .. }
                | nomi_process_runtime::ProcessError::CapacityExhausted { .. } => json!({"user_code_not_started":true,"cleanup_proven":true}),
                _ => json!({"user_code_not_started":false,"cleanup_proven":false}),
            };
            (error.code().into(),error.to_string(),details)
        })?;
        let io = async {
            self.supervisor
                .write(&owner, &handle.session_id, &payload)
                .await?;
            self.supervisor
                .close_stdin(&owner, &handle.session_id)
                .await
        }
        .await;
        if let Err(error) = io {
            let stopped = self.supervisor.cancel(&owner, &handle.session_id).await;
            let worker = json!({"session_id":handle.session_id.to_string(),"pid":handle.pid,"stop":stopped.as_ref().map(crate::outcome).ok(),"stop_error":stopped.as_ref().err().map(ToString::to_string)});
            return Err(unknown(
                format!("Native readonly worker input failed: {error}"),
                worker,
            ));
        }
        on_ready(handle.clone());
        let terminal = match self
            .supervisor
            .poll(
                &owner,
                &handle.session_id,
                OutputCursor::START,
                Instant::now() + Duration::from_secs(20),
            )
            .await
        {
            Ok(PollResult::Finished(outcome)) => outcome,
            Ok(PollResult::Running { .. }) => self
                .supervisor
                .timeout(&owner, &handle.session_id)
                .await
                .map_err(|error| {
                    unknown(
                        error.to_string(),
                        json!({"session_id":handle.session_id.to_string(),"pid":handle.pid}),
                    )
                })?,
            Err(error) => {
                let stopped = self.supervisor.cancel(&owner, &handle.session_id).await;
                return Err(unknown(
                    format!("Native readonly worker observation failed: {error}"),
                    json!({"session_id":handle.session_id.to_string(),"pid":handle.pid,"stop":stopped.as_ref().map(crate::outcome).ok()}),
                ));
            }
        };
        let (output, report, exited) = match &terminal {
            ProcessOutcome::Exited {
                code,
                signal,
                output,
                cleanup,
            } => (output, cleanup, *code == Some(0) && signal.is_none()),
            ProcessOutcome::Cancelled { output, cleanup }
            | ProcessOutcome::TimedOut { output, cleanup }
            | ProcessOutcome::Lost {
                output, cleanup, ..
            } => (output, cleanup, false),
            ProcessOutcome::SpawnFailed(failure) => {
                return Err(unknown(
                    format!("Native readonly worker did not start: {}", failure.message),
                    Value::Null,
                ));
            }
        };
        let (exit_code, signal) = match &terminal {
            ProcessOutcome::Exited { code, signal, .. } => (*code, *signal),
            _ => (None, None),
        };
        let stderr = output
            .chunks
            .iter()
            .filter(|chunk| chunk.stream == OutputStream::Stderr)
            .flat_map(|chunk| chunk.bytes.iter().copied())
            .collect::<Vec<_>>();
        let worker = json!({"session_id":handle.session_id.to_string(),"pid":handle.pid,"state":match &terminal { ProcessOutcome::Exited { .. }=>"exited",ProcessOutcome::Cancelled { .. }=>"cancelled",ProcessOutcome::TimedOut { .. }=>"timed_out",_=>"lost" },"exit_code":exit_code,"signal":signal,"cleanup":cleanup(report),"stderr":String::from_utf8_lossy(&stderr[..stderr.len().min(4096)]),"stderr_total_bytes":stderr.len(),"output_dropped_bytes":output.dropped_bytes});
        if !report.reaped {
            return Err(unknown(
                "Native readonly worker cleanup is unresolved",
                worker,
            ));
        }
        if !exited {
            return Err((
                "SYSTEM_READ_FAILED".into(),
                "Native readonly worker stopped before returning a complete acquisition".into(),
                json!({"worker":worker,"cleanup_proven":true}),
            ));
        }
        if output.dropped_bytes != 0 || output.retained_bytes > MAX_REPLY_BYTES {
            return Err(unknown(
                "Native readonly worker output was truncated",
                worker,
            ));
        }
        let stdout = output
            .chunks
            .iter()
            .filter(|chunk| chunk.stream == OutputStream::Stdout)
            .flat_map(|chunk| chunk.bytes.iter().copied())
            .collect::<Vec<_>>();
        let reply: Value = serde_json::from_slice(&stdout).map_err(|error| {
            unknown(
                format!("Native readonly worker returned invalid JSON: {error}"),
                worker.clone(),
            )
        })?;
        if reply["ok"] != true {
            let error = &reply["error"];
            return Err((
                error["code"]
                    .as_str()
                    .unwrap_or("SYSTEM_READ_FAILED")
                    .into(),
                error["message"]
                    .as_str()
                    .unwrap_or("Native readonly acquisition failed")
                    .into(),
                json!({"native_file_error":error,"worker":worker,"cleanup_proven":true}),
            ));
        }
        let acquisition: NativeReadAcquisition =
            serde_json::from_value(reply["acquisition"].clone()).map_err(|error| {
                unknown(
                    format!("Native readonly acquisition is invalid: {error}"),
                    worker.clone(),
                )
            })?;
        Ok(json!({"acquisition":acquisition,"worker":worker}))
    }
}

/// Entry for the signed helper's internal child mode. This is not a user shell
/// or an elevation method; its parent already has the native system identity.
#[unsafe(no_mangle)]
pub extern "C" fn nomifun_system_readonly_worker() -> i32 {
    let result = std::panic::catch_unwind(|| {
        let mut bytes = Vec::new();
        std::io::stdin()
            .lock()
            .take((MAX_REQUEST_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        if bytes.len() > MAX_REQUEST_BYTES {
            return Err("Native read request exceeds its envelope".to_owned());
        }
        worker_reply(&bytes)
    });
    let bytes=match result {
        Ok(Ok(bytes)) if bytes.len()<=MAX_REPLY_BYTES=>bytes,
        Ok(Err(message))=>json!({"ok":false,"error":{"code":"invalid_system_read_request","message":message}}).to_string().into_bytes(),
        _=>json!({"ok":false,"error":{"code":"SYSTEM_READ_RESULT_UNKNOWN","message":"Native readonly worker could not produce a bounded result"}}).to_string().into_bytes(),
    };
    // This function is called from Swift, which exits through libc rather
    // than Rust's lang_start cleanup. Stdout's LineWriter may retain a whole
    // JSON reply with no newline; explicit flush is part of the IPC receipt.
    let mut stdout = std::io::stdout().lock();
    match stdout.write_all(&bytes).and_then(|()| stdout.flush()) {
        Ok(()) => 0,
        Err(_) => 74,
    }
}

fn worker_reply(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let request: NativeReadRequest =
        serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    let reply = match nomi_native_files::acquire(request) {
        Ok(acquisition) => json!({"ok":true,"acquisition":acquisition}),
        Err(error) => json!({"ok":false,"error":error.observation()}),
    };
    serde_json::to_vec(&reply).map_err(|error| error.to_string())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn readonly_wire_preserves_near_eight_mib_source_and_rejects_oversize() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("large.txt");
        let source = vec![b'x'; nomi_native_files::MAX_SOURCE_BYTES - 1];
        std::fs::write(&path, &source).unwrap();
        let request = NativeReadRequest::Source {
            path: path.to_str().unwrap().into(),
            max_bytes: nomi_native_files::MAX_SOURCE_BYTES,
        };
        let reply = worker_reply(&serde_json::to_vec(&request).unwrap()).unwrap();
        assert!(reply.len() > 4 * 1024 * 1024);
        assert!(reply.len() < MAX_REPLY_BYTES);
        let value: Value = serde_json::from_slice(&reply).unwrap();
        assert_eq!(value["ok"], true);
        let NativeReadAcquisition::Source {
            source: Some(acquired),
        } = serde_json::from_value(value["acquisition"].clone()).unwrap()
        else {
            panic!("wrong acquisition")
        };
        assert_eq!(acquired.bytes, source);
        assert_eq!(acquired.sha256.len(), 64);
        std::fs::write(&path, vec![b'x'; nomi_native_files::MAX_SOURCE_BYTES + 1]).unwrap();
        let reply = worker_reply(&serde_json::to_vec(&request).unwrap()).unwrap();
        let value: Value = serde_json::from_slice(&reply).unwrap();
        assert_eq!(value["ok"], false);
        assert_eq!(value["error"]["code"], "INVALID_PAYLOAD");
        assert!(value.get("acquisition").is_none());
    }

    #[test]
    fn readonly_worker_timeout_and_quiesce_join_the_existing_process_owner() {
        let runner = Runner::new();
        let request = || NativeReadRequest::Source {
            path: "/unused-test-source".into(),
            max_bytes: 16384,
        };
        let command = || CommandSpec::Program {
            program: "/bin/sleep".into(),
            args: vec!["60".into()],
        };
        let (ready, started) = tokio::sync::oneshot::channel();
        let (result, timed_out) = runner.runtime.block_on(async {
            tokio::join!(
                runner.acquire_read_observed(
                    request(),
                    command(),
                    Duration::from_secs(30),
                    move |handle| {
                        let _ = ready.send(handle);
                    }
                ),
                async {
                    let handle = started.await.unwrap();
                    runner
                        .supervisor
                        .timeout(&handle.owner, &handle.session_id)
                        .await
                        .unwrap()
                }
            )
        });
        assert!(matches!(
            timed_out,
            ProcessOutcome::TimedOut {
                cleanup: nomi_process_runtime::CleanupReport { reaped: true, .. },
                ..
            }
        ));
        let (_, _, details) = result.unwrap_err();
        assert_eq!(details["cleanup_proven"], true, "{details}");
        assert_eq!(
            details.pointer("/worker/cleanup/reaped"),
            Some(&Value::Bool(true))
        );
        let (ready, started) = tokio::sync::oneshot::channel();
        let (result, report) = runner.runtime.block_on(async {
            tokio::join!(
                runner.acquire_read_observed(
                    request(),
                    command(),
                    Duration::from_secs(30),
                    move |handle| {
                        let _ = ready.send(handle);
                    }
                ),
                async {
                    let _handle = started.await.unwrap();
                    runner.supervisor.quiesce().await
                }
            )
        });
        assert!(report.is_exact());
        let (_, _, details) = result.unwrap_err();
        assert_eq!(details["cleanup_proven"], true, "{details}");
        assert_eq!(details.pointer("/worker/state"), Some(&json!("cancelled")));
    }
}
