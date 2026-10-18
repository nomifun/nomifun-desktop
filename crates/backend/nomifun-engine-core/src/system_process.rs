//! The system identity uses the same process supervisor through the native
//! platform transport. It does not create Agent state or authorize replay.
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nomi_process_runtime::{OutputCursor, ProcessOwner, SessionId};
use nomi_system_execution::SystemExecutionClient;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::process::{EngineCleanupReport, EngineProcessOutput, EngineProcessPoll,
    EngineProcessRequest, EngineProcessSession, EngineProcessStartError, EngineProcessTransport};
use crate::EngineProcessError;

#[derive(Default)]
pub(crate) struct SystemEngineOwner {
    // Native in-flight starts only, like the local owner's pending_starts.
    // A failed transport never turns an empty handle map into cleanup proof.
    pending: Mutex<HashMap<Uuid, Arc<SystemExecutionClient>>>,
}

pub(crate) struct SystemEngineSession {
    client: Arc<SystemExecutionClient>,
    invocation_id: Uuid,
    call_id: Uuid,
    session_id: String,
    pub(crate) terminal: Option<EngineProcessPoll>,
}

impl std::fmt::Debug for SystemEngineSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SystemEngineSession").field("session_id", &self.session_id).finish_non_exhaustive()
    }
}

fn failure(error: impl std::fmt::Display) -> EngineProcessError {
    EngineProcessError::Process(error.to_string())
}

impl SystemEngineOwner {
    async fn client(&self, cancellation: &CancellationToken) -> Result<Arc<SystemExecutionClient>, EngineProcessError> {
        // Registering a daemon and approving it are separate OS facts. Recheck
        // the native facility while the original task remains cancellable.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
        loop {
            if cancellation.is_cancelled() { return Err(EngineProcessError::Cancelled); }
            let connected = tokio::task::spawn_blocking(SystemExecutionClient::connect).await.map_err(failure)?;
            match connected {
                Ok(client) => {
                    tokio::select! {
                        _ = cancellation.cancelled() => return Err(EngineProcessError::Cancelled),
                        ready = client.wait_ready() => ready.map_err(failure)?,
                    }
                    return Ok(Arc::new(client));
                }
                Err(error) if error.code == "SYSTEM_EXECUTION_APPROVAL_REQUIRED" && tokio::time::Instant::now() < deadline => {
                    tokio::select! {
                        _ = cancellation.cancelled() => return Err(EngineProcessError::Cancelled),
                        _ = tokio::time::sleep(Duration::from_millis(750)) => {},
                    }
                }
                Err(error) => return Err(failure(format!("{}: {}", error.code, error.message))),
            }
        }
    }

    pub(crate) async fn start(&self, request: EngineProcessRequest, default_directory: &Path,
        cancellation: CancellationToken) -> Result<EngineProcessSession, EngineProcessStartError> {
        let before = |error| EngineProcessStartError { error, no_live_process_proven:true, user_code_not_started:true };
        let client = self.client(&cancellation).await.map_err(before)?;
        if cancellation.is_cancelled() { return Err(before(EngineProcessError::Cancelled)); }
        let invocation_id = Uuid::now_v7();
        let call_id = Uuid::now_v7();
        let cwd = request.cwd.as_deref().map(Path::new).unwrap_or(default_directory);
        let cwd = if cwd.is_absolute() { cwd.to_owned() } else { default_directory.join(cwd) };
        // Preserve the real login user's environment and HOME. The helper is
        // a system execution context, never a substitute login session.
        let mut environment: BTreeMap<String, String> = std::env::vars_os()
            .filter(|(key, _)| !nomi_process_runtime::is_inherited_application_environment_key(key))
            .map(|(key, value)| {
                let key = key.into_string().map_err(|_| before(failure("Inherited environment name cannot be represented by the native execution protocol (non-UTF-8)")))?;
                let value = value.into_string().map_err(|_| before(failure("Inherited environment value cannot be represented by the native execution protocol (non-UTF-8)")))?;
                Ok((key,value))
            }).collect::<Result<_,EngineProcessStartError>>()?;
        environment.extend(request.env);
        let command = match request.shell_script {
            Some(script) => json!({"kind":"shell", "script":script}),
            None => json!({"kind":"program", "program":request.command, "args":request.args}),
        };
        let transport = match request.transport {
            EngineProcessTransport::Pipe => json!({"kind":"pipe"}),
            EngineProcessTransport::Pty { cols,rows } => json!({"kind":"pty","cols":cols,"rows":rows}),
        };
        self.pending.lock().expect("system starts mutex poisoned").insert(call_id, client.clone());
        // Do not abandon a started native RPC when cancellation races spawn.
        // The returned handle or the exact quiesce fence owns its settlement.
        let result = client.call(json!({"op":"start","invocation_id":invocation_id.to_string(),"call_id":call_id.to_string(),
            "command":command,"cwd":cwd,"env":environment,"transport":transport,
            "timeout_ms":request.timeout_ms,"expire_on_idle":false,"output_limit_bytes":request.output_limit_bytes})).await;
        let value = result.map_err(|error| {
            let not_started = error.details.get("user_code_not_started").and_then(Value::as_bool) == Some(true);
            let cleanup = not_started || error.details.get("cleanup_proven").and_then(Value::as_bool) == Some(true);
            if cleanup { self.pending.lock().expect("system starts mutex poisoned").remove(&call_id); }
            EngineProcessStartError { error:failure(format!("{}: {}",error.code,error.message)),
                no_live_process_proven:cleanup, user_code_not_started:not_started }
        })?;
        let malformed = |error| EngineProcessStartError { error, no_live_process_proven:false, user_code_not_started:false };
        let native_id = value.get("session_id").and_then(Value::as_str).ok_or_else(|| malformed(failure("system runner omitted session identity")))?;
        let session_id: SessionId = native_id.parse().map_err(|error| malformed(failure(error)))?;
        let pid = value.get("pid").and_then(Value::as_u64).and_then(|pid| u32::try_from(pid).ok())
            .ok_or_else(|| malformed(failure("system runner omitted process identity")))?;
        self.pending.lock().expect("system starts mutex poisoned").remove(&call_id);
        Ok(EngineProcessSession { terminal:None, system:Some(SystemEngineSession {
            client, invocation_id, call_id, session_id:native_id.to_owned(), terminal:None,
        }), owner:ProcessOwner::new(invocation_id,call_id), session_id, pid, cursor:OutputCursor::START })
    }

    pub(crate) async fn quiesce_unregistered_starts(&self) -> Option<bool> {
        let pending = self.pending.lock().expect("system starts mutex poisoned").clone();
        if pending.is_empty() { return None; }
        let mut proven = true;
        for (id, client) in pending {
            match client.call(json!({"op":"quiesce"})).await {
                Ok(receipt) if receipt.get("exact").and_then(Value::as_bool) == Some(true) => {
                    self.pending.lock().expect("system starts mutex poisoned").remove(&id);
                }
                _ => proven = false,
            }
        }
        Some(proven)
    }

}

impl SystemEngineSession {
    fn request(&self, operation: &str, mut fields: Value) -> Value {
        fields["op"] = json!(operation);
        fields["session_id"] = json!(self.session_id);
        fields["invocation_id"] = json!(self.invocation_id.to_string());
        fields["call_id"] = json!(self.call_id.to_string());
        fields
    }

    pub(crate) async fn control(&self, operation: &str, fields: Value,
        cancellation: CancellationToken) -> Result<(),EngineProcessError> {
        if cancellation.is_cancelled() { return Err(EngineProcessError::Cancelled); }
        // Once dispatched, a write/control result is retained instead of being
        // discarded on cancellation and retried with an unknown partial effect.
        self.client.call(self.request(operation,fields)).await.map_err(|error|
            failure(format!("{}: {}",error.code,error.message)))?;
        Ok(())
    }
}

pub(crate) async fn poll(session:&mut EngineProcessSession, cursor:u64, wait:Duration,
    cancellation:CancellationToken) -> Result<EngineProcessPoll,EngineProcessError> {
    poll_with_policy(session, cursor, wait, true, cancellation).await
}

pub(crate) async fn initial_observation(session: &mut EngineProcessSession, wait: Duration,
    cancellation: CancellationToken) -> Result<EngineProcessPoll, EngineProcessError> {
    poll_with_policy(session, 0, wait, false, cancellation).await
}

async fn poll_with_policy(session:&mut EngineProcessSession, cursor:u64, wait:Duration, break_on_output: bool,
    cancellation:CancellationToken) -> Result<EngineProcessPoll,EngineProcessError> {
    if cancellation.is_cancelled() { return cancel(session).await; }
    let system=session.system.as_ref().expect("system session required");
    let call=system.client.call(system.request("poll",json!({"cursor":cursor,"yield_ms":wait.as_millis().min(30000) as u64,"break_on_output":break_on_output})));
    let value=tokio::select! {
        _ = cancellation.cancelled() => return cancel(session).await,
        result=call => result.map_err(|error| failure(format!("{}: {}",error.code,error.message)))?,
    };
    convert(session,value)
}

pub(crate) async fn cancel(session:&mut EngineProcessSession) -> Result<EngineProcessPoll,EngineProcessError> {
    let system=session.system.as_ref().expect("system session required");
    let value=system.client.call(system.request("cancel",json!({"cursor":session.cursor.offset()})))
        .await.map_err(|error| failure(format!("{}: {}",error.code,error.message)))?;
    convert(session,value)
}

pub(crate) async fn terminal_if_ready(session: &mut EngineProcessSession, cancellation: CancellationToken)
    -> Result<Option<EngineProcessPoll>, EngineProcessError> {
    let system = session.system.as_ref().expect("system session required");
    let request = system.request("terminal", json!({"cursor":session.cursor.offset()}));
    let call = system.client.call(request);
    let receipt = tokio::select! {
        _ = cancellation.cancelled() => return Err(EngineProcessError::Cancelled),
        result = call => result.map_err(|error| failure(format!("{}: {}",error.code,error.message)))?,
    };
    let native = receipt.get("terminal").ok_or_else(|| failure("system runner omitted terminal observation"))?;
    if native.is_null() { return Ok(None); }
    let saved_cursor = session.cursor;
    let result = convert(session,native.clone())?;
    session.cursor = saved_cursor;
    if !result.is_terminal() { return Err(failure("system runner returned running state as a terminal receipt")); }
    Ok(Some(result))
}

pub(crate) async fn observe_if_changed(session: &mut EngineProcessSession, cursor: u64,
    include_output: bool, cancellation: CancellationToken) -> Result<Option<EngineProcessPoll>, EngineProcessError> {
    let system = session.system.as_ref().expect("system session required");
    let call = system.client.call(system.request("observe", json!({"cursor":cursor,"include_output":include_output})));
    let receipt = tokio::select! {
        _ = cancellation.cancelled() => return Err(EngineProcessError::Cancelled),
        result = call => result.map_err(|error| failure(format!("{}: {}",error.code,error.message)))?,
    };
    let value = receipt.get("observation").ok_or_else(|| failure("system runner omitted owner observation"))?;
    if value.is_null() { return Ok(None); }
    let saved_cursor = session.cursor;
    let result = convert(session, value.clone())?;
    session.cursor = saved_cursor;
    Ok(Some(result))
}

fn convert(session:&mut EngineProcessSession,value:Value) -> Result<EngineProcessPoll,EngineProcessError> {
    let (result,next_cursor)=parse_poll(value,session.pid)?;
    session.cursor=OutputCursor::new(next_cursor);
    if result.is_terminal() { session.system.as_mut().expect("system session required").terminal=Some(result.clone()); }
    Ok(result)
}

fn parse_poll(value:Value,pid:u32) -> Result<(EngineProcessPoll,u64),EngineProcessError> {
    let output=value.get("output").ok_or_else(|| failure("system runner omitted output receipt"))?;
    let chunks=output.get("chunks").and_then(Value::as_array).ok_or_else(|| failure("system runner omitted output chunks"))?;
    let text=chunks.iter().map(|chunk| chunk.get("text").and_then(Value::as_str)
        .ok_or_else(|| failure("system runner returned an invalid output chunk"))).collect::<Result<String,_>>()?;
    let number=|key| output.get(key).and_then(Value::as_u64).ok_or_else(|| failure(format!("system runner omitted {key}")));
    let next_cursor=number("next_cursor")?;
    let output=EngineProcessOutput { text,next_cursor,retained_bytes:usize::try_from(number("retained_bytes")?).map_err(failure)?,
        dropped_bytes:number("dropped_bytes")?,source_encoding:output.pointer("/encoding/source_encoding")
            .and_then(Value::as_str).ok_or_else(||failure("system runner omitted output encoding"))?.to_owned(),
        decode_errors:output.pointer("/encoding/decode_errors").and_then(Value::as_u64)
            .ok_or_else(||failure("system runner omitted decode error measurement"))? };
    let state=value.get("state").and_then(Value::as_str).ok_or_else(|| failure("system runner omitted terminal state"))?;
    let result=if state=="running" { EngineProcessPoll::Running { pid,output } } else {
        let cleanup:EngineCleanupReport=serde_json::from_value(value.get("cleanup").cloned()
            .ok_or_else(|| failure("system runner omitted cleanup receipt"))?).map_err(failure)?;
        match state {
            "exited" => EngineProcessPoll::Exited { exit_code:value.get("code").and_then(Value::as_i64).and_then(|code|i32::try_from(code).ok()),
                signal:value.get("signal").and_then(Value::as_i64).and_then(|code|i32::try_from(code).ok()),output,cleanup },
            "cancelled"=>EngineProcessPoll::Cancelled { output,cleanup },
            "timed_out"=>EngineProcessPoll::TimedOut { output,cleanup },
            "lost"=>EngineProcessPoll::Lost { pid,output,cleanup },
            _=>return Err(failure(format!("unknown system process state: {state}"))),
        }
    };
    Ok((result,next_cursor))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EngineExecutionIdentity,ManagedEngineProcessOwner};

    fn receipt() -> Value {
        json!({"state":"cancelled","output":{"chunks":[{"text":"已发生的输出\n"}],"next_cursor":25,
            "retained_bytes":25,"dropped_bytes":0,"encoding":{"source_encoding":"utf-8","decode_errors":0}},
            "cleanup":{"interrupt_attempted":true,"terminate_attempted":false,"force_kill_attempted":false,
                "reaped":true,"elapsed_ms":10,"errors":[]}})
    }

    #[test]
    fn system_receipt_preserves_actual_output_and_cleanup_including_unknown_reap() {
        let mut value=receipt();
        value["cleanup"]["reaped"]=json!(false);
        value["cleanup"]["errors"]=json!(["original cleanup failure"]);
        let (poll,cursor)=parse_poll(value,123).unwrap();
        assert_eq!(cursor,25);
        let EngineProcessPoll::Cancelled{output,cleanup}=poll else {panic!("wrong native state")};
        assert_eq!(output.text,"已发生的输出\n");
        assert!(!cleanup.reaped);
        assert_eq!(cleanup.errors,["original cleanup failure"]);
    }

    #[test]
    fn missing_native_measurement_or_cleanup_is_not_synthesized_as_success() {
        for field in ["cleanup","output"] {
            let mut value=receipt();value.as_object_mut().unwrap().remove(field);
            assert!(parse_poll(value,123).is_err());
        }
        let mut value=receipt();value["output"]["encoding"].as_object_mut().unwrap().remove("decode_errors");
        assert!(parse_poll(value,123).is_err());
        let mut value=receipt();value["state"]=json!("authentication_approved");
        assert!(parse_poll(value,123).is_err());
    }

    #[tokio::test]
    async fn an_empty_native_start_map_is_not_a_cleanup_witness() {
        let root=tempfile::tempdir().unwrap();
        let owner=ManagedEngineProcessOwner::new(root.path(),Default::default()).unwrap();
        assert!(!owner.quiesce_unregistered_starts().await);
    }

    #[tokio::test]
    async fn unbundled_system_execution_never_falls_back_to_the_login_user() {
        // This case is specifically the missing-component path. A developer
        // machine with the real Linux component installed must not open an OS
        // authentication prompt or perform root work as a side effect of tests.
        #[cfg(target_os = "linux")]
        if Path::new("/usr/libexec/nomifun-system-runner").exists() { return; }
        let root=tempfile::tempdir().unwrap();
        let owner=ManagedEngineProcessOwner::new(root.path(),Default::default()).unwrap();
        let marker=root.path().join("must-not-run");
        let mut request=EngineProcessRequest::pipe(if cfg!(windows){"cmd.exe"}else{"/usr/bin/touch"});
        request.execution_identity=EngineExecutionIdentity::Administrator;
        request.args.push(marker.display().to_string());
        let failure=owner.start_with_evidence(request,CancellationToken::new()).await.unwrap_err();
        assert!(failure.user_code_not_started);
        assert!(failure.no_live_process_proven);
        assert!(!marker.exists());
    }
}
