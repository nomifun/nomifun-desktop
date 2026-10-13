//! Platform-owned, turn-scoped interactive process sessions. Called only after
//! Wave2/Kernel authority and canonical workspace checks, never by the engine.
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::engine_journal::{EngineJournalWrite, EngineTurnJournal};
use super::engine_process_recovery::ProcessWitness;
use super::agent_wave2_host::PreparedWave2Invocation;
use futures_util::FutureExt;
use nomifun_agent_contracts::StrictJsonValue;
use nomifun_agent_domain_wave2::Wave2HostPortError;
use nomifun_engine_core::{
    EngineProcessPoll, EngineProcessRequest, EngineProcessSession, EngineProcessTransport,
    MAX_PTY_DIMENSION, ManagedEngineProcessOwner,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

const MAX_STDIN_WRITE_BYTES: usize = 1024 * 1024;
const COMPONENT_PREPARATION_WAIT: Duration = Duration::from_secs(60);

fn error(value: impl std::fmt::Display) -> Wave2HostPortError {
    Wave2HostPortError::new("CAPABILITY_UNAVAILABLE", value.to_string())
}

fn outcome_unknown(value: impl std::fmt::Display) -> Wave2HostPortError {
    Wave2HostPortError::new("EFFECT_OUTCOME_UNKNOWN", value.to_string())
}

fn bounded_component_detail(value: &str, limit: usize) -> &str {
    let mut end = value.len().min(limit);
    while !value.is_char_boundary(end) { end -= 1; }
    &value[..end]
}

fn component_not_started(component: &str, code: &str, message: &str) -> StrictJsonValue {
    StrictJsonValue(serde_json::json!({
        "schema":"nomifun.process-start-observation.v1", "state":"not_started",
        "success":false, "user_code_started":false,
        "component":bounded_component_detail(component,128),
        "code":bounded_component_detail(code,96),
        "message":bounded_component_detail(message,1024),
    }))
}

fn component_resolution_error(id: nomifun_runtime::ToolId, cause: &nomifun_runtime::ToolError) -> Wave2HostPortError {
    use nomifun_runtime::ToolError;
    let code = match cause {
        ToolError::Shared(cause) => return component_resolution_error(id, cause),
        ToolError::NotPrepared => "COMPONENT_PREPARATION_REQUIRED",
        ToolError::Unavailable { .. } => "COMPONENT_UNAVAILABLE",
        ToolError::EnvironmentConflict { .. } => "COMPONENT_ENVIRONMENT_CONFLICT",
        ToolError::Verification { .. } => "COMPONENT_VERIFICATION_FAILED",
        ToolError::InvalidOverride { .. } => "COMPONENT_OVERRIDE_INVALID",
        ToolError::Changed { .. } => "COMPONENT_CHANGED",
        ToolError::Cancelled => "CANCELLED",
        ToolError::Cleanup(_) => "COMPONENT_CLEANUP_UNPROVEN",
        ToolError::Download(_) => "COMPONENT_DOWNLOAD_FAILED",
        ToolError::TooLarge => "COMPONENT_SIZE_LIMIT",
        ToolError::InvalidAsset(_) => "COMPONENT_ASSET_INVALID",
        ToolError::Extract(_) | ToolError::Io(_) => "COMPONENT_CACHE_FAILED",
        ToolError::Worker(_) => "COMPONENT_PREPARATION_FAILED",
    };
    let reason = nomi_redact::redact_secrets_owned(cause.to_string());
    Wave2HostPortError::new(code, format!("{}: {}", id.as_str(), bounded_component_detail(&reason,768)))
}

fn with_component_identity(
    result: Result<StrictJsonValue, Wave2HostPortError>,
    identity: Option<&StrictJsonValue>,
) -> Result<StrictJsonValue, Wave2HostPortError> {
    result.map(|mut output| {
        if let Some(identity) = identity { output.0["component_identity"] = identity.0.clone(); }
        output
    })
}

#[derive(Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Operation {
    #[default]
    Exec,
    Start,
    Poll,
    Stdin,
    CloseStdin,
    Resize,
    Cancel,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Params {
    #[serde(default)]
    operation: Operation,
    command: Option<String>,
    cmd: Option<String>,
    component: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    cwd: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    timeout_ms: Option<u64>,
    process_id: Option<String>,
    input: Option<String>,
    append_newline: Option<bool>,
    cursor: Option<u64>,
    wait_ms: Option<u64>,
    #[serde(default)]
    tty: bool,
    cols: Option<u16>,
    rows: Option<u16>,
}

fn poll_wait(params: &Params) -> Duration {
    Duration::from_millis(params.wait_ms.unwrap_or(0))
}

fn stdin_bytes(
    input: Option<&str>,
    append_newline: bool,
) -> Result<Vec<u8>, Wave2HostPortError> {
    let input = input.ok_or_else(|| error("stdin requires input"))?;
    let mut bytes = input.as_bytes().to_vec();
    if append_newline {
        bytes.push(b'\n');
    }
    Ok(bytes)
}

fn stdin_input_exceeds_budget(params: &Params) -> bool {
    params.input.as_ref().is_some_and(|text| {
        text.len() > MAX_STDIN_WRITE_BYTES
            || (params.append_newline == Some(true) && text.len() == MAX_STDIN_WRITE_BYTES)
    })
}

fn pty_dimension_out_of_range(params: &Params) -> bool {
    params
        .cols
        .is_some_and(|value| !(1..=MAX_PTY_DIMENSION).contains(&value))
        || params
            .rows
            .is_some_and(|value| !(1..=MAX_PTY_DIMENSION).contains(&value))
}

pub(crate) struct EngineProcessScope {
    owner: ManagedEngineProcessOwner,
    state: Mutex<ProcessState>,
    journal: EngineTurnJournal,
    cancellation: CancellationToken,
    closed: AtomicBool,
}

#[derive(Default)]
struct ProcessState {
    sessions: HashMap<String, ProcessEntry>,
    operations: BTreeSet<String>,
    /// Set across spawn until the returned handle is registered. An error or
    /// panic cannot turn an unaccounted spawn into proof from an empty map.
    unregistered_start: bool,
    /// An unwind may have invalidated an owner's cleanup witness. Retain
    /// this uncertainty even if a later cleanup invocation returns normally.
    cleanup_panicked: bool,
}

impl ProcessState {
    fn is_quiescent(&self) -> bool {
        !self.unregistered_start
            && !self.cleanup_panicked
            && self
                .sessions
                .values()
                .all(|entry| entry.terminal.as_ref().is_some_and(cleanup_is_proven))
    }
}

struct ProcessEntry {
    session: EngineProcessSession,
    terminal: Option<EngineProcessPoll>,
    component_identity: Option<StrictJsonValue>,
}

/// A single invocation's local preparation, never a Session binding or checkpoint.
pub(crate) struct PreparedProcessInvocation {
    params: Params,
    component: nomifun_runtime::ToolId,
    prepared: nomifun_runtime::PreparedComponentLaunch,
}

fn cleanup_is_proven(poll: &EngineProcessPoll) -> bool {
    matches!(poll, EngineProcessPoll::Exited { cleanup, .. } | EngineProcessPoll::Cancelled { cleanup, .. }
        | EngineProcessPoll::TimedOut { cleanup, .. } | EngineProcessPoll::Lost { cleanup, .. } if cleanup.reaped)
}

impl EngineProcessScope {
    pub(crate) fn close_admission(&self) {
        self.closed.store(true, Ordering::Release);
        self.cancellation.cancel();
    }

    pub(crate) fn workspace_root(&self) -> &Path {
        self.owner.workspace_root()
    }
    pub(crate) fn new(root: &Path, journal: EngineTurnJournal) -> Result<Self, Wave2HostPortError> {
        Ok(Self {
            owner: ManagedEngineProcessOwner::new(root, Default::default()).map_err(error)?,
            state: Mutex::new(ProcessState::default()),
            journal,
            cancellation: CancellationToken::new(),
            closed: false.into(),
        })
    }

    pub(crate) async fn invoke(
        &self,
        input: StrictJsonValue,
        operation_id: &str,
    ) -> Result<StrictJsonValue, Wave2HostPortError> {
        let params: Params = serde_json::from_value(input.0)
            .map_err(|e| Wave2HostPortError::new("INVALID_PAYLOAD", e.to_string()))?;
        if params.component.is_some() {
            return match self.prepare_component_params(params).await? {
                PreparedWave2Invocation::Invoke(prepared) => self.invoke_prepared(prepared, operation_id).await,
                PreparedWave2Invocation::Return(observation) => Ok(observation),
            };
        }
        self.invoke_params(params, None, operation_id).await
    }

    /// Run only after the original Effect reader has ruled out replay/unknown results.
    /// No process intent or workspace Effect is reserved while the supplier works.
    pub(crate) async fn prepare_component(
        &self,
        input: StrictJsonValue,
    ) -> Result<PreparedWave2Invocation<PreparedProcessInvocation>, Wave2HostPortError> {
        let params = serde_json::from_value(input.0)
            .map_err(|e| Wave2HostPortError::new("INVALID_PAYLOAD", e.to_string()))?;
        self.prepare_component_params(params).await
    }

    async fn prepare_component_params(
        &self,
        params: Params,
    ) -> Result<PreparedWave2Invocation<PreparedProcessInvocation>, Wave2HostPortError> {
        if !matches!(params.operation, Operation::Exec | Operation::Start)
            || params.command.is_some() || params.cmd.is_some()
            || params.process_id.is_some() || params.input.is_some()
            || params.cursor.is_some() || params.append_newline.is_some()
        {
            return Err(Wave2HostPortError::new("INVALID_PAYLOAD", "component is an exclusive launch target"));
        }
        let name = params.component.as_deref().ok_or_else(||
            Wave2HostPortError::new("INVALID_PAYLOAD", "component launch requires an exact approved component id"))?;
        let Some(component) = nomifun_runtime::ToolId::parse(name) else {
            return Ok(PreparedWave2Invocation::Return(component_not_started(
                name, "COMPONENT_NOT_DECLARED", "Choose an exact approved component id: officecli, bun or node.",
            )));
        };
        if self.closed.load(Ordering::Acquire) || self.cancellation.is_cancelled() {
            return Ok(PreparedWave2Invocation::Return(component_not_started(
                name, "CANCELLED", "Component preparation cancelled before the requested program started.",
            )));
        }
        let mut request = nomifun_engine_core::EngineProcessRequest::pipe(name);
        request.args = params.args.clone();
        request.cwd = params.cwd.clone();
        request.env = params.env.clone();
        request.timeout_ms = params.timeout_ms.unwrap_or(30_000);
        request.transport = if params.tty {
            EngineProcessTransport::Pty { cols: params.cols.unwrap_or(120), rows: params.rows.unwrap_or(40) }
        } else { EngineProcessTransport::Pipe };
        if let Err(cause) = request.validate() {
            return Ok(PreparedWave2Invocation::Return(component_not_started(
                name, "INVALID_PAYLOAD", &cause.to_string(),
            )));
        }
        if params.wait_ms.is_some_and(|wait| wait > 30_000) || pty_dimension_out_of_range(&params) {
            return Ok(PreparedWave2Invocation::Return(component_not_started(
                name, "INVALID_PAYLOAD", "Process wait/PTY dimension budget exceeded.",
            )));
        }
        let environment = params.env.iter().map(|(key, value)| (key.clone(), value.clone())).collect();
        let preparation = tokio::select! {
            biased;
            _ = self.cancellation.cancelled() => return Ok(PreparedWave2Invocation::Return(component_not_started(
                name, "CANCELLED", "Component preparation cancelled before the requested program started.",
            ))),
            result = tokio::time::timeout(COMPONENT_PREPARATION_WAIT,
                nomifun_runtime::prepare_component_launch(component, &environment)) => result,
        };
        let prepared = match preparation {
            Ok(Ok(prepared)) => prepared,
            Ok(Err(cause)) => {
                let failure = component_resolution_error(component, &cause);
                return Ok(PreparedWave2Invocation::Return(component_not_started(name, &failure.code, &failure.message)));
            }
            Err(_) => return Ok(PreparedWave2Invocation::Return(component_not_started(
                name, "COMPONENT_PREPARATION_TIMEOUT", "The component preparation wait expired; the requested program did not start.",
            ))),
        };
        if self.closed.load(Ordering::Acquire) || self.cancellation.is_cancelled() {
            return Ok(PreparedWave2Invocation::Return(component_not_started(
                name, "CANCELLED", "Component preparation cancelled before the requested program started.",
            )));
        }
        let Some(path) = prepared.tool.executable_path.to_str() else {
            return Ok(PreparedWave2Invocation::Return(component_not_started(
                name, "COMPONENT_PATH_INVALID", "The verified executable path cannot be represented by the process protocol.",
            )));
        };
        request.command = path.to_owned();
        request.env = prepared.environment.iter().map(|(key, value)| (key.clone(), value.clone())).collect();
        if let Err(cause) = request.validate() {
            return Ok(PreparedWave2Invocation::Return(component_not_started(
                name, "INVALID_PAYLOAD", &cause.to_string(),
            )));
        }
        Ok(PreparedWave2Invocation::Invoke(PreparedProcessInvocation { params, component, prepared }))
    }

    pub(crate) async fn invoke_prepared(
        &self,
        invocation: PreparedProcessInvocation,
        operation_id: &str,
    ) -> Result<StrictJsonValue, Wave2HostPortError> {
        self.invoke_params(invocation.params, Some((invocation.component, invocation.prepared)), operation_id).await
    }

    async fn invoke_params(
        &self,
        params: Params,
        component: Option<(nomifun_runtime::ToolId, nomifun_runtime::PreparedComponentLaunch)>,
        operation_id: &str,
    ) -> Result<StrictJsonValue, Wave2HostPortError> {
        if params.wait_ms.is_some_and(|wait| wait > 30_000)
            || stdin_input_exceeds_budget(&params)
            || pty_dimension_out_of_range(&params)
        {
            return Err(error("process wait/input/PTY dimension budget exceeded"));
        }
        let mut state = self.state.lock().await;
        if self.closed.load(Ordering::Acquire) || state.unregistered_start || state.cleanup_panicked
        {
            if let Some((id, _)) = &component {
                if !state.unregistered_start && !state.cleanup_panicked {
                    return Ok(component_not_started(id.as_str(), "CANCELLED", "The requested program did not start; the process turn is closed."));
                }
            }
            return Err(error("process turn is closed"));
        }
        if operation_id.is_empty()
            || operation_id.len() > 1024
            || operation_id.chars().any(char::is_control)
            || state.operations.len() >= 512
            || !state.operations.insert(operation_id.to_owned())
        {
            return Err(error("invalid, duplicate or excessive process operation"));
        }
        let ordinal = state.operations.len() as u16;
        let intent = serde_json::to_string(&ProcessWitness::Dispatch {
            operation_id: operation_id.to_owned(),
            ordinal,
        })
        .map_err(error)?;
        // Hold the same scope lock through effect and barrier. A later spawn
        // cannot race ahead of an earlier all-handles-reaped observation.
        if let Err(cause) = self
            .journal
            .append(intent, None, EngineJournalWrite::Progress)
            .await
        {
            self.close_admission();
            return Err(error(cause));
        }
        let result = self.invoke_owned(params, component, &mut state).await;
        if state.is_quiescent() {
            let witness = serde_json::to_string(&ProcessWitness::Quiescent {
                operation_id: operation_id.to_owned(),
                ordinal,
                process_count: state.sessions.len(),
            })
            .map_err(error)?;
            if let Err(cause) = self
                .journal
                .append(witness, None, EngineJournalWrite::Settlement)
                .await
            {
                self.close_admission();
                return Err(outcome_unknown(cause));
            }
        }
        result
    }

    async fn invoke_owned(
        &self,
        params: Params,
        component: Option<(nomifun_runtime::ToolId, nomifun_runtime::PreparedComponentLaunch)>,
        state: &mut ProcessState,
    ) -> Result<StrictJsonValue, Wave2HostPortError> {
        let poll_delay = poll_wait(&params);
        let sessions = &mut state.sessions;
        let launch = matches!(params.operation, Operation::Exec | Operation::Start);
        if params.cursor.is_some() && params.operation != Operation::Poll {
            return Err(error("cursor is only valid for poll"));
        }
        if params.append_newline.is_some() && params.operation != Operation::Stdin {
            return Err(error("append_newline is only valid for stdin"));
        }
        let id = if launch {
            if sessions.len() >= 64 {
                return Err(error("turn process limit reached (64 launches)"));
            }
            if params.process_id.is_some() || params.input.is_some() {
                return Err(error(
                    "launch cannot address an existing process or write stdin",
                ));
            }
            let identity = component.as_ref().map(|(id, prepared)| StrictJsonValue(serde_json::json!({
                "component":id.as_str(), "version":prepared.tool.version,
                "digest":prepared.tool.digest, "source":prepared.tool.source,
            })));
            let mut request = if let Some((id, prepared)) = &component {
                if self.cancellation.is_cancelled() {
                    return Ok(component_not_started(id.as_str(), "CANCELLED", "The requested program did not start."));
                }
                if let Err(cause) = prepared.validate_for_start() {
                    let failure = component_resolution_error(*id, &cause);
                    return Ok(component_not_started(id.as_str(), &failure.code, &failure.message));
                }
                let Some(path) = prepared.tool.executable_path.to_str() else {
                    return Ok(component_not_started(id.as_str(), "COMPONENT_PATH_INVALID", "The verified executable path cannot be represented by the process protocol."));
                };
                let mut request = EngineProcessRequest::pipe(path);
                request.args = params.args;
                request.env = prepared.environment.iter().map(|(key, value)| (key.clone(), value.clone())).collect();
                request
            } else { match (params.cmd, params.command) {
                (Some(script), None) if params.args.is_empty() => {
                    EngineProcessRequest::shell(script)
                },
                (None, Some(command)) => {
                    let mut request=EngineProcessRequest::pipe(command);
                    request.args=params.args;
                    request
                },
                _ => return Err(error("launch requires cmd OR command plus args; do not mix them")),
            }};
            request.cwd = params.cwd;
            if component.is_none() { request.env = params.env; }
            request.timeout_ms = params.timeout_ms.unwrap_or(30_000);
            request.output_limit_bytes = 256 * 1024;
            request.transport = if params.tty {
                EngineProcessTransport::Pty {
                    cols: params.cols.unwrap_or(120),
                    rows: params.rows.unwrap_or(40),
                }
            } else {
                EngineProcessTransport::Pipe
            };
            // Obvious input errors are rejected before the uncertain spawn
            // window. Errors returned by the owner may still require repair.
            request.validate().map_err(error)?;
            if component.is_none() {
                resolve_managed_program(&mut request, &self.cancellation, self.workspace_root()).await?;
            }
            state.unregistered_start = true;
            let session = match self
                .owner
                .start_with_evidence(request, self.cancellation.clone())
                .await
            {
                Ok(session) => session,
                Err(failure) => {
                    state.unregistered_start = !failure.no_live_process_proven;
                    if failure.user_code_not_started {
                        return Ok(StrictJsonValue(serde_json::json!({
                            "schema":"nomifun.process-start-observation.v1", "state":"not_started",
                            "success":false, "user_code_started":false,
                            "code":"PROCESS_NOT_STARTED",
                            "message":"The requested executable did not start. For an ordinary executable, use command for the executable name or path only, with separate literal tokens in an args JSON array, and omit cmd. The owner never splits command or evaluates it as shell text. Use cmd only when shell syntax is required; then omit command and args. Check the executable and workspace-relative cwd, then correct this request. Process capability remains available; this failed launch did not invalidate earlier file/check observations."
                        })));
                    }
                    return Err(if failure.no_live_process_proven {
                        error(failure.error)
                    } else {
                        outcome_unknown(failure.error)
                    });
                }
            };
            let id = session.session_id();
            // No await between owned spawn and registration in the scope.
            sessions.insert(
                id.clone(),
                ProcessEntry {
                    session,
                    terminal: None,
                    component_identity: identity,
                },
            );
            state.unregistered_start = false;
            id
        } else {
            if params.command.is_some() || params.cmd.is_some() || params.component.is_some()
                || !params.args.is_empty()
                || params.cwd.is_some()
                || !params.env.is_empty()
                || params.timeout_ms.is_some()
                || params.tty
            {
                return Err(error("process controls cannot change launch parameters"));
            }
            params
                .process_id
                .clone()
                .ok_or_else(|| error("process control requires process_id"))?
        };
        let entry = match sessions.get_mut(&id) {
            Some(entry) => entry,
            None if !launch => return Ok(StrictJsonValue(serde_json::json!({
                "schema":"nomifun.process-control-observation.v1",
                "state":"not_executed", "success":false, "control_applied":false,
                "operation":params.operation, "code":"PROCESS_REFERENCE_INVALID",
                "message":"The process_id is not available in this exact turn. No process control was applied. Copy the exact process_id from start_process or its later receipt, then correct this call. Process capability remains available. Do not start a replacement merely to recover a mistyped reference; the original process may still be running."
            }))),
            None => return Err(error("process_id is not owned by this exact turn")),
        };
        let input_control = matches!(params.operation, Operation::Stdin | Operation::CloseStdin | Operation::Resize);
        if input_control && entry.terminal.is_none() {
            // A deadline can settle in the owner between model steps. Read its
            // frozen terminal before attempting input; do not consume running
            // output or turn a missing/uncertain owner into a successful stop.
            if let Some(poll) = self.owner.terminal_if_ready(&mut entry.session).map_err(outcome_unknown)? {
                if !cleanup_is_proven(&poll) { return Err(outcome_unknown("the original owner terminal has no proven cleanup receipt")); }
                entry.terminal = Some(poll.clone());
                return with_component_identity(rejected_terminal_control(&id, &poll, params.operation), entry.component_identity.as_ref());
            }
        }
        if let Some(poll) = &entry.terminal {
            if params.operation == Operation::Cancel {
                return with_component_identity(process_output(&id, poll, params.operation), entry.component_identity.as_ref());
            }
            if params.operation == Operation::Poll {
                // Re-poll the retained terminal session so the caller's output
                // cursor is honored even when another interaction observed exit.
            } else {
                return with_component_identity(rejected_terminal_control(&id, poll, params.operation), entry.component_identity.as_ref());
            }
        }
        let session = &mut entry.session;
        match params.operation {
            Operation::Stdin => {
                let bytes = stdin_bytes(
                    params.input.as_deref(),
                    params.append_newline.unwrap_or(false),
                )?;
                self.owner
                    .write_stdin(session, &bytes, self.cancellation.clone())
                    .await
                    .map_err(outcome_unknown)?
            }
            Operation::CloseStdin => self
                .owner
                .close_stdin(session)
                .await
                .map_err(outcome_unknown)?,
            Operation::Resize => self
                .owner
                .resize(
                    session,
                    params
                        .cols
                        .filter(|v| *v > 0)
                        .ok_or_else(|| error("resize requires cols"))?,
                    params
                        .rows
                        .filter(|v| *v > 0)
                        .ok_or_else(|| error("resize requires rows"))?,
                )
                .await
                .map_err(outcome_unknown)?,
            _ => {}
        }
        let poll = match params.operation {
            Operation::Exec => self.owner.wait(session, self.cancellation.clone()).await,
            Operation::Cancel => self.owner.cancel(session).await,
            Operation::Poll => {
                self.owner
                    .poll_from(
                        session,
                        params.cursor.unwrap_or(0),
                        poll_delay,
                        self.cancellation.clone(),
                    )
                    .await
            }
            _ => {
                self.owner
                    .poll(
                        session,
                        poll_delay,
                        self.cancellation.clone(),
                    )
                    .await
            }
        }
        .map_err(outcome_unknown)?;
        if cleanup_is_proven(&poll) {
            entry.terminal = Some(poll.clone());
        }
        with_component_identity(process_output(&id, &poll, params.operation), entry.component_identity.as_ref())
    }

    /// Close admission before waiting for any running launch/poll. The token
    /// wakes exec, poll and stdin, while retained handles prove tree cleanup.
    pub(crate) async fn cleanup(&self) -> Result<(), Wave2HostPortError> {
        self.close_admission();
        let mut state = self.state.lock().await;
        let mut failures = Vec::new();
        if let Err(cause) = reconcile_unregistered_start(&mut state, self.owner.quiesce_unregistered_starts()).await {
            failures.push(cause);
        }
        if state.cleanup_panicked {
            failures
                .push("an earlier process cleanup panicked; cleanup cannot be certified".into());
        }
        let ProcessState {
            sessions,
            cleanup_panicked,
            ..
        } = &mut *state;
        for (id, entry) in sessions.iter_mut() {
            if entry.terminal.as_ref().is_some_and(cleanup_is_proven) {
                continue;
            }
            let outcome = AssertUnwindSafe(async { self.owner.cancel(&mut entry.session).await })
                .catch_unwind()
                .await;
            match outcome {
                Ok(Ok(poll)) if cleanup_is_proven(&poll) => {
                    entry.terminal = Some(poll);
                }
                Ok(Ok(_)) => failures.push(format!("{id}: cleanup not proven")),
                Ok(Err(cause)) => failures.push(format!("{id}: {cause}")),
                Err(_) => {
                    // Store before the next await, including if the cleanup
                    // waiter is dropped while another handle is settling.
                    *cleanup_panicked = true;
                    failures.push(format!("{id}: cleanup panicked; settlement unknown"));
                }
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(error(failures.join("; ")))
        }
    }
}

fn rejected_terminal_control(id: &str, poll: &EngineProcessPoll, operation: Operation) -> Result<StrictJsonValue, Wave2HostPortError> {
    if !cleanup_is_proven(poll) { return Err(outcome_unknown("the original owner terminal has no proven cleanup receipt")); }
    let mut output = process_output(id, poll, operation)?;
    output.0["success"] = serde_json::json!(false);
    output.0["control_applied"] = serde_json::json!(false);
    output.0["schema"] = serde_json::json!("nomifun.process-control-observation.v1");
    output.0["operation"] = serde_json::to_value(operation).map_err(error)?;
    output.0["code"] = serde_json::json!("PROCESS_ALREADY_TERMINATED");
    output.0["message"] = serde_json::json!("The original process already terminated. This control was not applied. Its actual terminal output and cleanup receipt are retained; do not restart or replay input merely to repair the report.");
    Ok(output)
}

/// Resolve system Bun or an existing verified download before entering the
/// uncertain process-start window. Missing components require a user choice;
/// an Agent execution request never starts their download.
async fn resolve_managed_program(
    request: &mut EngineProcessRequest,
    cancellation: &CancellationToken,
    workspace_root: &Path,
) -> Result<(), Wave2HostPortError> {
    if request.shell_script.is_some() || nomifun_runtime::managed_command_tool(&request.command).is_none() {
        return Ok(());
    }
    if cancellation.is_cancelled() {
        return Err(Wave2HostPortError::new("CANCELLED", "Process resolution cancelled before user code started"));
    }
    let env = request.env.iter().map(|(name, value)| (name.clone(), value.clone())).collect();
    let cwd = workspace_root.join(request.cwd.as_deref().unwrap_or("."));
    let prepared = tokio::select! {
        biased;
        _ = cancellation.cancelled() => return Err(Wave2HostPortError::new("CANCELLED", "Process resolution cancelled before user code started")),
        result = nomifun_runtime::prepare_managed_command_with_env(&request.command, &request.args, &env, Some(&cwd)) => result.map_err(|cause| runtime_resolution_error(&cause))?,
    };
    if cancellation.is_cancelled() {
        return Err(Wave2HostPortError::new("CANCELLED", "Process resolution cancelled before user code started"));
    }
    let prepared = prepared.ok_or_else(|| runtime_resolution_error(&nomifun_runtime::ToolError::NotPrepared))?;
    request.command = prepared.executable_path.to_str().ok_or_else(|| error("Bun path cannot be represented by the process protocol"))?.to_owned();
    request.args = prepared.args;
    request.validate().map_err(error)
}

fn runtime_resolution_error(cause: &nomifun_runtime::ToolError) -> Wave2HostPortError {
    // Ordinary command discovery keeps its existing response contract.
    match cause {
        nomifun_runtime::ToolError::Shared(cause) => runtime_resolution_error(cause),
        nomifun_runtime::ToolError::NotPrepared | nomifun_runtime::ToolError::Unavailable { .. } => {
            Wave2HostPortError::new(
                "COMPONENT_PREPARATION_REQUIRED",
                "Bun is unavailable. Install Bun on the system or choose to download it in NomiFun, then retry.",
            )
        }
        cause => Wave2HostPortError::new("COMPONENT_UNAVAILABLE", cause.to_string()),
    }
}

async fn reconcile_unregistered_start(
    state: &mut ProcessState,
    proof: impl std::future::Future<Output = bool>,
) -> Result<(), String> {
    if state.unregistered_start {
        match AssertUnwindSafe(proof).catch_unwind().await {
            Ok(true) => state.unregistered_start = false,
            Ok(false) => return Err("a process start has no exact owner cleanup proof".into()),
            Err(_) => {
                state.cleanup_panicked = true;
                return Err("startup cleanup fence panicked; settlement unknown".into());
            }
        }
    }
    Ok(())
}

fn process_output(
    id: &str,
    poll: &EngineProcessPoll,
    operation: Operation,
) -> Result<StrictJsonValue, Wave2HostPortError> {
    let success = match poll {
        EngineProcessPoll::Running { .. } => None,
        EngineProcessPoll::Exited {
            exit_code, cleanup, ..
        } => Some(*exit_code == Some(0) && cleanup.reaped),
        EngineProcessPoll::Cancelled { cleanup, .. } => {
            Some(operation == Operation::Cancel && cleanup.reaped)
        },
        _ => Some(false),
    };
    let mut output = serde_json::to_value(poll).map_err(error)?;
    output["process_id"] = id.into();
    output["success"] = serde_json::to_value(success).map_err(error)?;
    Ok(StrictJsonValue(output))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_engine_core::{EngineCleanupReport, EngineProcessOutput};
    include!("engine_process_component_tests.rs");

    #[tokio::test]
    async fn managed_program_preserves_shell_text_and_explicit_user_executables() {
        for mut request in [EngineProcessRequest::shell("bun test"), EngineProcessRequest::pipe("/user/tools/bun"), EngineProcessRequest::pipe("bun test")] {
            let original = request.clone();
            resolve_managed_program(&mut request, &CancellationToken::new(), Path::new(".")).await.unwrap();
            assert_eq!(request, original);
        }
    }

    #[tokio::test]
    async fn managed_program_cancelled_before_resolution_cannot_enter_user_start() {
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let mut request = EngineProcessRequest::pipe("bunx");
        request.args = vec!["tool-package".into()];
        let original = request.clone();
        let failure = resolve_managed_program(&mut request, &cancellation, Path::new(".")).await.unwrap_err();
        assert_eq!(failure.code, "CANCELLED");
        assert_eq!(request, original);
    }

    #[test]
    fn missing_runtime_is_a_user_choice_before_process_start() {
        let failure = runtime_resolution_error(&nomifun_runtime::ToolError::NotPrepared);
        assert_eq!(failure.code, "COMPONENT_PREPARATION_REQUIRED");
        assert!(failure.message.contains("Install Bun"));
        let shared = nomifun_runtime::ToolError::Shared(std::sync::Arc::new(nomifun_runtime::ToolError::NotPrepared));
        assert_eq!(runtime_resolution_error(&shared).code, "COMPONENT_PREPARATION_REQUIRED");
    }

    #[tokio::test]
    async fn managed_program_uses_child_path_relative_to_workspace_cwd() {
        let root = tempfile::tempdir().unwrap();
        let child = root.path().join("child");
        let tools = root.path().join("tools");
        std::fs::create_dir_all(&child).unwrap();
        std::fs::create_dir_all(&tools).unwrap();
        let bun = tools.join(if cfg!(windows) { "bun.exe" } else { "bun" });
        std::fs::write(&bun, b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bun, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut request = EngineProcessRequest::pipe("bun");
        request.args = vec!["user-script.js".into()];
        request.cwd = Some("child".into());
        request.env.insert(if cfg!(windows) { "PaTh" } else { "PATH" }.into(), "../tools".into());
        let original_env = request.env.clone();
        resolve_managed_program(&mut request, &CancellationToken::new(), root.path()).await.unwrap();
        assert_eq!(std::fs::canonicalize(&request.command).unwrap(), std::fs::canonicalize(bun).unwrap());
        assert_eq!(request.args, vec!["user-script.js"]);
        assert_eq!(request.cwd.as_deref(), Some("child"));
        assert_eq!(request.env, original_env);
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn unsplit_executable_failure_keeps_literal_argv_recovery_available() {
        let root = tempfile::tempdir().unwrap();
        let (journal, pool) = super::super::engine_journal::test_fixture().await;
        let scope = EngineProcessScope::new(root.path(), journal).unwrap();
        for (index, command) in ["/bin/ls -a", "ls -a"].into_iter().enumerate() {
            let rejected = scope.invoke(StrictJsonValue(serde_json::json!({
                "operation":"exec", "command":command, "env":{"PATH":"/usr/bin:/bin"}, "timeout_ms":5000
            })), &format!("reject-unsplit-command-{index}")).await.unwrap().0;
            assert_eq!(rejected["code"], "PROCESS_NOT_STARTED");
            assert_eq!(rejected["state"], "not_started");
            assert_eq!(rejected["success"], false);
            assert_eq!(rejected["user_code_started"], false);
            assert!(scope.state.lock().await.is_quiescent());
            let message = rejected["message"].as_str().unwrap();
            assert!(message.contains("For an ordinary executable, use command"));
            assert!(message.contains("never splits command"));
            assert!(message.contains("Use cmd only when shell syntax is required"));
        }

        let corrected = scope.invoke(StrictJsonValue(serde_json::json!({
            "operation":"exec", "command":"/bin/ls", "args":["-a"], "timeout_ms":5000
        })), "correct-literal-argv").await.unwrap().0;
        assert_eq!(corrected["state"], "exited");
        assert_eq!(corrected["exit_code"], 0);
        assert_eq!(corrected["success"], true);
        assert_eq!(corrected["cleanup"]["reaped"], true);
        assert!(scope.state.lock().await.is_quiescent());
        scope.cleanup().await.unwrap();
        drop(scope);
        pool.close().await;
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn expired_pipe_rejects_late_input_with_original_terminal_and_cleanup() {
        let root = tempfile::tempdir().unwrap();
        let (journal, pool) = super::super::engine_journal::test_fixture().await;
        let scope = EngineProcessScope::new(root.path(), journal).unwrap();
        let started = scope.invoke(StrictJsonValue(serde_json::json!({
            "operation":"start", "command":"/bin/sh", "args":["-c", "printf 'READY\\n'; IFS= read -r line; printf 'ECHO:%s\\n' \"$line\""],
            "tty":false, "wait_ms":0, "timeout_ms":1000
        })), "start-expiring-pipe").await.unwrap().0;
        let id = started["process_id"].as_str().unwrap();
        // Observe the original owner, not scope.invoke/poll: the scope has not
        // cached a terminal. This is the same between-step deadline condition.
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                {
                    let mut state = scope.state.lock().await;
                    let entry = state.sessions.get_mut(id).unwrap();
                    let cursor=entry.session.cursor();
                    let ready=scope.owner.terminal_if_ready(&mut entry.session).unwrap().is_some();
                    assert_eq!(entry.session.cursor(),cursor,"terminal inspection must not consume output");
                    if ready { break; }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.unwrap();
        for (index, operation) in ["stdin", "close_stdin", "resize"].into_iter().enumerate() {
            let mut args = serde_json::json!({"operation":operation,"process_id":id});
            if operation == "stdin" { args["input"] = serde_json::json!("must-not-be-sent\n"); }
            if operation == "resize" { args["cols"] = serde_json::json!(80); args["rows"] = serde_json::json!(24); }
            let rejected = scope.invoke(StrictJsonValue(args), &format!("late-control-{index}")).await.unwrap().0;
            assert_eq!(rejected["state"], "timed_out");
            assert_eq!(rejected["success"], false);
            assert_eq!(rejected["control_applied"], false);
            assert_eq!(rejected["code"], "PROCESS_ALREADY_TERMINATED");
            assert_eq!(rejected["cleanup"]["reaped"], true);
            let original_output=format!("{}{}",started["output"]["text"].as_str().unwrap(),rejected["output"]["text"].as_str().unwrap());
            assert!(original_output.contains("READY\n"));
        }
        let replay = scope.invoke(StrictJsonValue(serde_json::json!({
            "operation":"poll", "process_id":id,"cursor":0,"wait_ms":0
        })), "replay-original-output").await.unwrap().0;
        let text = replay["output"]["text"].as_str().unwrap();
        assert!(text.contains("READY\n"));
        assert!(!text.contains("ECHO:") && !text.contains("must-not-be-sent"));
        assert!(scope.state.lock().await.is_quiescent());
        scope.cleanup().await.unwrap();
        drop(scope);
        pool.close().await;
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn windows_expired_pipe_keeps_terminal_truth_before_late_controls() {
        windows_expired_transport_keeps_terminal_truth(false).await;
    }

    #[cfg(windows)]
    #[tokio::test]
    #[ignore = "requires the unchanged Windows ACP 936 fixture and Bun"]
    async fn windows_mixed_code_page_output_keeps_frozen_tool_metadata() {
        // SAFETY: GetACP only reads the host's current ANSI code page.
        assert_eq!(unsafe { windows_sys::Win32::Globalization::GetACP() }, 936);
        let root = tempfile::tempdir().unwrap();
        let (journal, pool) = super::super::engine_journal::test_fixture().await;
        let scope = EngineProcessScope::new(root.path(), journal).unwrap();
        let script = "process.stdout.write(Buffer.from([0xd6,0xd0,0xce,0xc4,10])); await Bun.sleep(40); const e=Buffer.from('UTF8:中文🙂\\n'); process.stderr.write(e.subarray(0,12)); await Bun.sleep(40); process.stderr.write(e.subarray(12));";
        let result = scope.invoke(StrictJsonValue(serde_json::json!({
            "operation":"exec", "command":"bun", "args":["-e",script], "timeout_ms":5000
        })), "mixed-code-page-exec").await.unwrap().0;
        assert_eq!(result["state"], "exited");
        assert_eq!(result["exit_code"], 0);
        assert_eq!(result["success"], true);
        assert_eq!(result["cleanup"]["reaped"], true);
        assert_eq!(result["output"]["source_encoding"], "mixed");
        assert_eq!(result["output"]["decode_errors"], 1);
        assert_eq!(result["output"]["next_cursor"], 21);
        assert_eq!(result["output"]["retained_bytes"], 21);
        assert_eq!(result["output"]["dropped_bytes"], 0);
        let text = result["output"]["text"].as_str().unwrap();
        assert!(text.contains("中文\n") && text.contains("UTF8:中文🙂\n"));
        assert_eq!(text.len(), "中文\nUTF8:中文🙂\n".len());
        let empty = scope.invoke(StrictJsonValue(serde_json::json!({
            "operation":"poll", "process_id":result["process_id"], "cursor":21, "wait_ms":0
        })), "mixed-code-page-frozen-poll").await.unwrap().0;
        assert_eq!(empty["state"], result["state"]);
        assert_eq!(empty["output"]["text"], "");
        for field in ["source_encoding", "decode_errors", "next_cursor", "retained_bytes", "dropped_bytes"] {
            assert_eq!(empty["output"][field], result["output"][field], "frozen {field}");
        }
        assert!(scope.state.lock().await.is_quiescent());
        scope.cleanup().await.unwrap();
        println!("WINDOWS_MIXED_TOOL_EVIDENCE {}", serde_json::json!({"result":result,"empty":empty}));
        drop(scope); pool.close().await;
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn windows_persistent_cmd_remains_owned_and_times_out() {
        let root = tempfile::tempdir().unwrap();
        let (journal, pool) = super::super::engine_journal::test_fixture().await;
        let scope = EngineProcessScope::new(root.path(), journal).unwrap();
        let started = scope.invoke(StrictJsonValue(serde_json::json!({
            "operation":"start","command":"cmd.exe","args":["/d","/k","echo CMD_KEEPER_READY"],
            "timeout_ms":1000,"wait_ms":0
        })), "owned-persistent-cmd").await.unwrap().0;
        assert_eq!(started["state"], "running");
        let mut observations = Vec::new();
        let mut output = started["output"]["text"].as_str().unwrap().to_owned();
        let mut cursor = started["output"]["next_cursor"].as_u64().unwrap();
        // Unread output can complete a poll before the deadline. Advance its cursor
        // and keep observing the same process instead of treating that as terminal.
        let terminal = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let call_id = format!("observe-persistent-cmd-deadline-{}", observations.len());
                let observed = scope.invoke(StrictJsonValue(serde_json::json!({
                    "operation":"poll","process_id":started["process_id"],"cursor":cursor,"wait_ms":1000
                })), &call_id).await.unwrap().0;
                cursor = observed["output"]["next_cursor"].as_u64().unwrap();
                output.push_str(observed["output"]["text"].as_str().unwrap());
                observations.push(observed.clone());
                if observed["state"] != "running" { break observed; }
            }
        }).await.expect("the original one-second deadline must reach a terminal");
        println!("WINDOWS_CMD_OWNER_EVIDENCE {}", serde_json::json!({"kind":"persistent","started":started,"observations":observations,"terminal":terminal}));
        assert_eq!(terminal["state"], "timed_out");
        assert_eq!(terminal["success"], false);
        assert_eq!(terminal["cleanup"]["reaped"], true);
        assert!(output.contains("CMD_KEEPER_READY"));
        assert!(scope.state.lock().await.is_quiescent());
        scope.cleanup().await.unwrap();
        drop(scope);pool.close().await;
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn windows_cmd_start_background_child_is_reaped_before_success() {
        use std::os::windows::process::CommandExt;
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("child.mjs"), "await Bun.write('child.pid', String(process.pid)); setInterval(() => {}, 1000);\n").unwrap();
        std::fs::write(root.path().join("await-child.mjs"), "const until=Date.now()+5000; while(!(await Bun.file('child.pid').exists())){if(Date.now()>until)throw new Error('no child readiness'); await Bun.sleep(10);} console.log('CMD_CHILD_READY');\n").unwrap();
        let (journal, pool) = super::super::engine_journal::test_fixture().await;
        let scope = EngineProcessScope::new(root.path(), journal).unwrap();
        let result = scope.invoke(StrictJsonValue(serde_json::json!({
            "operation":"exec","command":"cmd.exe","args":["/d","/s","/c","start \"\" /b bun child.mjs & bun await-child.mjs"],"timeout_ms":10000
        })), "cmd-background-child").await.unwrap().0;
        assert_eq!(result["state"], "exited");
        assert_eq!(result["exit_code"], 0);
        assert_eq!(result["cleanup"]["reaped"], true);
        assert!(result["output"]["text"].as_str().unwrap().contains("CMD_CHILD_READY"));
        let pid: u32 = std::fs::read_to_string(root.path().join("child.pid")).unwrap().parse().unwrap();
        let check = std::process::Command::new("powershell.exe").args(["-NoProfile","-NonInteractive","-Command",
            &format!("if(Get-Process -Id {pid} -ErrorAction SilentlyContinue){{exit 1}}; exit 0")]).creation_flags(0x08000000).output().unwrap();
        assert!(check.status.success(), "the ready background child must be gone when success is returned");
        assert!(scope.state.lock().await.is_quiescent());
        scope.cleanup().await.unwrap();
        println!("WINDOWS_CMD_OWNER_EVIDENCE {}", serde_json::json!({"kind":"background-child","child_pid":pid,"result":result}));
        drop(scope);pool.close().await;
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn windows_expired_conpty_keeps_terminal_truth_before_late_controls() {
        windows_expired_transport_keeps_terminal_truth(true).await;
    }

    #[cfg(windows)]
    #[tokio::test]
    #[ignore = "requires a standard Windows token and a running LanmanServer service"]
    async fn windows_standard_token_admin_query_returns_a_reaped_nonzero_result() {
        use std::os::windows::process::CommandExt;
        let eligibility = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command",
                "$fixturePrincipal=[Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent()); if($fixturePrincipal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)){exit 10}; if((Get-Service LanmanServer).Status -ne 'Running'){exit 20}; exit 0"])
            .creation_flags(0x08000000)
            .status().unwrap();
        assert_eq!(eligibility.code(), Some(0), "fixture requires the standard token and active service; do not elevate or start services to pass");
        let root = tempfile::tempdir().unwrap();
        let (journal, pool) = super::super::engine_journal::test_fixture().await;
        let scope = EngineProcessScope::new(root.path(), journal).unwrap();
        let result = scope.invoke(StrictJsonValue(serde_json::json!({
            "operation":"exec", "command":"net.exe", "args":["session"], "timeout_ms":5000
        })), "standard-user-admin-query").await.unwrap().0;
        assert_eq!(result["state"], "exited", "permission denial is not a spawn failure");
        assert_eq!(result["exit_code"], 2);
        assert_eq!(result["success"], false);
        assert_eq!(result["cleanup"]["reaped"], true);
        assert_eq!(result["output"]["dropped_bytes"], 0);
        let text = result["output"]["text"].as_str().unwrap();
        assert!(text.contains("5") && (text.contains("Access is denied") || text.contains("拒绝访问") || text.contains("访问被拒绝")),
            "the actual OS output must identify access denied, not service absence");
        assert_eq!(scope.state.lock().await.sessions.len(), 1, "no privileged fallback or command retry");
        assert!(scope.state.lock().await.is_quiescent());
        scope.cleanup().await.unwrap();
        println!("WINDOWS_STANDARD_TOKEN_EVIDENCE {}", result);
        drop(scope);
        pool.close().await;
    }

    #[cfg(windows)]
    async fn windows_expired_transport_keeps_terminal_truth(tty: bool) {
        let root = tempfile::tempdir().unwrap();
        let (journal, pool) = super::super::engine_journal::test_fixture().await;
        let scope = EngineProcessScope::new(root.path(), journal).unwrap();
        let started = scope.invoke(StrictJsonValue(serde_json::json!({
            "operation":"start", "command":"bun",
            "args":["-e", "process.stdout.write('READY\\n'); process.stdin.resume(); process.stdin.on('data', b => process.stdout.write('ECHO:' + b.toString()));"],
            "tty":tty, "wait_ms":0, "timeout_ms":1000
        })), "windows-expiring-receiver").await.unwrap().0;
        assert_eq!(started["state"], "running");
        let id = started["process_id"].as_str().unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let ready = {
                    let mut state = scope.state.lock().await;
                    let entry = state.sessions.get_mut(id).unwrap();
                    assert!(entry.terminal.is_none(), "scope has not observed a terminal yet");
                    let cursor = entry.session.cursor();
                    let ready = scope.owner.terminal_if_ready(&mut entry.session).unwrap().is_some();
                    assert_eq!(entry.session.cursor(), cursor, "lookup must not consume output");
                    ready
                };
                if ready { break; }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.unwrap();
        assert!(!scope.state.lock().await.is_quiescent(), "scope still has no cached terminal");
        let mut receipts: Vec<serde_json::Value> = Vec::new();
        for (index, operation) in ["stdin", "close_stdin", "resize"].into_iter().enumerate() {
            let mut args = serde_json::json!({"operation":operation,"process_id":id});
            if operation == "stdin" { args["input"] = serde_json::json!("WINDOWS_LATE_INPUT_MUST_NOT_RUN\n"); }
            if operation == "resize" { args["cols"] = serde_json::json!(80); args["rows"] = serde_json::json!(24); }
            let receipt = scope.invoke(StrictJsonValue(args), &format!("windows-late-control-{index}")).await.unwrap().0;
            assert_eq!(receipt["state"], "timed_out");
            assert_eq!(receipt["success"], false);
            assert_eq!(receipt["control_applied"], false);
            assert_eq!(receipt["code"], "PROCESS_ALREADY_TERMINATED");
            assert_eq!(receipt["cleanup"]["reaped"], true);
            if let Some(original) = receipts.first() { assert_eq!(&receipt["output"], &original["output"]); }
            receipts.push(receipt);
        }
        let replay = scope.invoke(StrictJsonValue(serde_json::json!({
            "operation":"poll", "process_id":id,"cursor":0,"wait_ms":0
        })), "windows-replay-original-output").await.unwrap().0;
        let text = replay["output"]["text"].as_str().unwrap();
        assert_eq!(text, format!("{}{}", started["output"]["text"].as_str().unwrap(), receipts[0]["output"]["text"].as_str().unwrap()));
        if tty { assert!(text.contains("READY\r\n")); } else { assert_eq!(text, "READY\n"); }
        assert!(!text.contains("ECHO:") && !text.contains("WINDOWS_LATE_INPUT_MUST_NOT_RUN"));
        assert_eq!(replay["cleanup"]["reaped"], true);
        assert!(scope.state.lock().await.is_quiescent());
        scope.cleanup().await.unwrap();
        println!("WINDOWS_EXPIRED_OWNER_EVIDENCE {}", serde_json::json!({"tty":tty,"started":started,"late_controls":receipts,"replay":replay}));
        drop(scope);
        pool.close().await;
    }

    #[tokio::test]
    async fn invalid_process_reference_preserves_the_original_owned_process() {
        let root = tempfile::tempdir().unwrap();
        let (journal, pool) = super::super::engine_journal::test_fixture().await;
        let scope = EngineProcessScope::new(root.path(), journal).unwrap();
        let (command, args) = if cfg!(windows) {
            ("powershell.exe", vec!["-NoProfile", "-Command",
                "$line=[Console]::ReadLine(); [Console]::WriteLine('ORIGINAL:'+$line)"])
        } else {
            ("/bin/sh", vec!["-c", "IFS= read -r line; printf 'ORIGINAL:%s\\n' \"$line\""])
        };
        let started = scope.invoke(StrictJsonValue(serde_json::json!({
            "operation":"start", "command":command, "args":args, "timeout_ms":10000, "wait_ms":0
        })), "start-original").await.unwrap();
        let id = started.0["process_id"].as_str().unwrap().to_owned();
        let rejected = scope.invoke(StrictJsonValue(serde_json::json!({
            "operation":"stdin", "process_id":"not-owned-by-this-turn", "input":"must-not-be-sent", "append_newline":true
        })), "reject-wrong-reference").await;
        if let Err(cause) = &rejected {
            scope.cleanup().await.unwrap();
            panic!("an unowned reference must return an explicit not-executed observation: {cause}");
        }
        let rejected = rejected.unwrap().0;
        assert_eq!(rejected["code"], "PROCESS_REFERENCE_INVALID");
        assert_eq!(rejected["state"], "not_executed");
        assert_eq!(rejected["control_applied"], false);
        assert!(!scope.state.lock().await.is_quiescent(), "rejecting a reference must not claim that the original child was reaped");

        let (other_journal, other_pool) = super::super::engine_journal::test_fixture().await;
        let other = EngineProcessScope::new(root.path(), other_journal).unwrap();
        let foreign = other.invoke(StrictJsonValue(serde_json::json!({
            "operation":"cancel", "process_id":id
        })), "reject-foreign-reference").await.unwrap();
        assert_eq!(foreign.0["code"], "PROCESS_REFERENCE_INVALID");
        assert!(other.state.lock().await.is_quiescent());

        scope.invoke(StrictJsonValue(serde_json::json!({
            "operation":"stdin", "process_id":id, "input":"ok", "append_newline":true
        })), "write-original").await.unwrap();
        let (finished, text) = tokio::time::timeout(Duration::from_secs(5), async {
            let mut cursor = 0;
            let mut text = String::new();
            let mut ordinal = 0;
            loop {
                let observed = scope.invoke(StrictJsonValue(serde_json::json!({
                    "operation":"poll", "process_id":id, "cursor":cursor, "wait_ms":30000
                })), &format!("poll-original-{ordinal}")).await.unwrap();
                text.push_str(observed.0["output"]["text"].as_str().unwrap());
                cursor = observed.0["output"]["next_cursor"].as_u64().unwrap();
                if observed.0["state"] != "running" { break (observed, text); }
                ordinal += 1;
            }
        }).await.expect("original child must finish within the bounded poll");
        assert_eq!(finished.0["state"], "exited");
        assert_eq!(finished.0["exit_code"], 0);
        assert!(text.contains("ORIGINAL:ok"));
        assert!(!text.contains("must-not-be-sent"));
        assert_eq!(finished.0["cleanup"]["reaped"], true);
        assert!(scope.state.lock().await.is_quiescent());
        scope.cleanup().await.unwrap();
        other.cleanup().await.unwrap();
        drop(scope);
        drop(other);
        pool.close().await;
        other_pool.close().await;
    }

    #[tokio::test]
    async fn exact_owner_startup_fence_releases_unregistered_start_uncertainty() {
        let mut state = ProcessState { unregistered_start: true, ..ProcessState::default() };
        let result = reconcile_unregistered_start(&mut state, async { true }).await;
        assert!(result.is_ok(), "late exact owner cleanup must not remain permanently unknown");
        assert!(state.is_quiescent());
    }

    #[tokio::test]
    async fn unproven_startup_fence_cannot_clear_unregistered_start_uncertainty() {
        let mut state = ProcessState { unregistered_start: true, ..ProcessState::default() };
        assert!(reconcile_unregistered_start(&mut state, async { false }).await.is_err());
        assert!(!state.is_quiescent());
    }

    #[tokio::test]
    async fn startup_fence_panic_remains_uncertain_after_a_later_exact_proof() {
        let mut state = ProcessState { unregistered_start: true, ..ProcessState::default() };
        assert!(reconcile_unregistered_start(&mut state, async { panic!("injected startup fence panic") }).await.is_err());
        assert!(state.cleanup_panicked && !state.is_quiescent());
        assert!(reconcile_unregistered_start(&mut state, async { true }).await.is_ok());
        assert!(!state.unregistered_start && state.cleanup_panicked && !state.is_quiescent());
    }

    #[test]
    fn omitted_wait_and_cursor_match_explicit_zero_wire_contract() {
        let omitted: Params = serde_json::from_value(serde_json::json!({
            "operation": "poll",
            "process_id": "process-1"
        }))
        .unwrap();
        let explicit: Params = serde_json::from_value(serde_json::json!({
            "operation": "poll",
            "process_id": "process-1",
            "cursor": 0,
            "wait_ms": 0
        }))
        .unwrap();
        assert_eq!(poll_wait(&omitted), Duration::ZERO);
        assert_eq!(poll_wait(&explicit), Duration::ZERO);
        assert_eq!(omitted.cursor.unwrap_or(0), 0);
        assert_eq!(explicit.cursor.unwrap_or(0), 0);
    }

    #[test]
    fn stdin_append_newline_adds_one_lf_without_normalizing_input() {
        let exact: Params = serde_json::from_value(serde_json::json!({
            "operation": "stdin",
            "process_id": "process-1",
            "input": " payload\r",
            "append_newline": true
        }))
        .unwrap();
        let unchanged: Params = serde_json::from_value(serde_json::json!({
            "operation": "stdin",
            "process_id": "process-1",
            "input": " payload\r"
        }))
        .unwrap();
        assert_eq!(
            stdin_bytes(exact.input.as_deref(), exact.append_newline.unwrap_or(false)).unwrap(),
            b" payload\r\n"
        );
        assert_eq!(
            stdin_bytes(
                unchanged.input.as_deref(),
                unchanged.append_newline.unwrap_or(false)
            )
            .unwrap(),
            b" payload\r"
        );
        assert_eq!(stdin_bytes(Some("你好 MAC-B"), true).unwrap(), "你好 MAC-B\n".as_bytes());
        assert_eq!(stdin_bytes(Some("你好 MAC-B\n"), false).unwrap(), "你好 MAC-B\n".as_bytes());
        assert_eq!(stdin_bytes(Some("你好 MAC-B\n"), true).unwrap(), "你好 MAC-B\n\n".as_bytes(),
            "an existing LF and an appended LF must not be silently deduplicated");
    }

    #[test]
    fn stdin_budget_counts_the_structured_newline_before_dispatch() {
        let params = |bytes: usize, append_newline: Option<bool>| Params {
            operation: Operation::Stdin,
            command: None,
            cmd: None,
            component: None,
            args: Vec::new(),
            cwd: None,
            env: BTreeMap::new(),
            timeout_ms: None,
            process_id: Some("process-1".to_owned()),
            input: Some("x".repeat(bytes)),
            append_newline,
            cursor: None,
            wait_ms: None,
            tty: false,
            cols: None,
            rows: None,
        };
        assert!(!stdin_input_exceeds_budget(&params(
            MAX_STDIN_WRITE_BYTES,
            None
        )));
        assert!(stdin_input_exceeds_budget(&params(
            MAX_STDIN_WRITE_BYTES,
            Some(true)
        )));
        assert!(stdin_input_exceeds_budget(&params(
            MAX_STDIN_WRITE_BYTES + 1,
            Some(false)
        )));
    }

    #[test]
    fn pty_dimensions_use_the_portable_predispatch_limit() {
        let mut params: Params = serde_json::from_value(serde_json::json!({
            "operation": "resize",
            "process_id": "process-1",
            "cols": 32767,
            "rows": 32767
        }))
        .unwrap();
        assert!(!pty_dimension_out_of_range(&params));
        params.cols = Some(32768);
        assert!(pty_dimension_out_of_range(&params));
        params.cols = Some(120);
        params.rows = Some(0);
        assert!(pty_dimension_out_of_range(&params));
    }

    #[test]
    fn reaped_explicit_cancellation_is_a_successful_process_result() {
        let mut poll = EngineProcessPoll::Cancelled {
            output: EngineProcessOutput {
                text: "READY\n".to_owned(),
                next_cursor: 6,
                retained_bytes: 6,
                dropped_bytes: 0,
                source_encoding: "utf-8".to_owned(),
                decode_errors: 0,
            },
            cleanup: EngineCleanupReport {
                interrupt_attempted: true,
                terminate_attempted: true,
                force_kill_attempted: false,
                reaped: true,
                elapsed_ms: 42,
                errors: vec!["interrupt unavailable; terminated instead".to_owned()],
            },
        };

        let output = process_output("process-1", &poll, Operation::Cancel)
            .unwrap()
            .0;
        assert_eq!(output["state"], "cancelled");
        assert_eq!(output["success"], true);
        assert_eq!(output["cleanup"]["reaped"], true);
        assert_eq!(
            output["cleanup"]["errors"][0],
            "interrupt unavailable; terminated instead"
        );

        if let EngineProcessPoll::Cancelled { cleanup, .. } = &mut poll {
            cleanup.reaped = false;
        }
        let unreaped = process_output("process-1", &poll, Operation::Cancel)
            .unwrap()
            .0;
        assert_eq!(unreaped["success"], false);
        assert!(rejected_terminal_control("process-1", &poll, Operation::Stdin).is_err(),
            "unreaped or lost ownership cannot become a proven non-input receipt");

        if let EngineProcessPoll::Cancelled { cleanup, .. } = &mut poll {
            cleanup.reaped = true;
        }
        let interrupted_exec = process_output("process-1", &poll, Operation::Exec)
            .unwrap()
            .0;
        assert_eq!(interrupted_exec["success"], false);
    }
}
