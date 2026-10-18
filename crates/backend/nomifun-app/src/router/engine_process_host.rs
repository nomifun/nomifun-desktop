//! Platform-owned, Session-scoped interactive process sessions. Called only after
//! Wave2/Kernel authority and canonical workspace checks, never by the engine.
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::engine_journal::{EngineJournalWrite, EngineTurnJournal};
use super::engine_process_recovery::ProcessWitness;
use super::agent_wave2_host::PreparedWave2Invocation;
use futures_util::{FutureExt, StreamExt};
use nomifun_agent_contracts::StrictJsonValue;
use nomifun_agent_domain_wave2::Wave2HostPortError;
use nomifun_engine_core::{
    EngineExecutionIdentity, EngineProcessPoll, EngineProcessRequest, EngineProcessSession, EngineProcessStartError, EngineProcessTransport,
    MAX_PTY_DIMENSION, ManagedEngineProcessOwner,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

const MAX_STDIN_WRITE_BYTES: usize = 1024 * 1024;
const COMPONENT_PREPARATION_WAIT: Duration = Duration::from_secs(11 * 60);

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
        ToolError::NotPrepared => "COMPONENT_UNAVAILABLE",
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
    Start,
    Poll,
    Stdin,
    CloseStdin,
    Resize,
    Cancel,
}

impl Operation {
    fn action_id(self) -> nomifun_agent_contracts::ActionId {
        format!("workspace.process/{}", match self { Self::Start => "start", Self::Poll => "poll", Self::Stdin => "input",
            Self::CloseStdin => "close_stdin", Self::Resize => "resize", Self::Cancel => "cancel" }).into()
    }
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
    #[serde(default)]
    execution_identity: EngineExecutionIdentity,
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
    Duration::from_millis(params.wait_ms.unwrap_or(if params.operation == Operation::Start { 1000 } else { 0 }))
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
    turn: std::sync::Mutex<Option<Arc<ProcessTurn>>>,
    cancellation: CancellationToken,
    closed: AtomicBool,
    session_id: String,
    owner_id: String,
    owner_generation: String,
    canonical: std::sync::OnceLock<nomifun_agent_session::AgentSessionStore>,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ProcessAuthority {
    pub snapshot: nomifun_agent_contracts::ResolvedSnapshotRef,
    pub actions: BTreeSet<nomifun_agent_contracts::ActionId>,
    pub resources: nomifun_agent_contracts::TypedResourceBindings,
}

struct ProcessTurn {
    operation: String,
    journal: EngineTurnJournal,
    authority: Option<ProcessAuthority>,
    operations: std::sync::Mutex<BTreeSet<String>>,
    closed: AtomicBool,
    cancellation: CancellationToken,
}

#[derive(Default)]
struct ProcessState {
    sessions: HashMap<String, ProcessEntry>,
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
    creator_operation: String,
    creator_call: String,
    authority: Option<ProcessAuthority>,
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

async fn next_process_change(mut receiver: tokio::sync::watch::Receiver<u64>)
    -> Result<tokio::sync::watch::Receiver<u64>, tokio::sync::watch::error::RecvError> {
    receiver.changed().await?;
    Ok(receiver)
}

async fn prepare_while_turn_open<T>(session:&CancellationToken,turn:&CancellationToken,preparation:impl std::future::Future<Output=T>) ->Option<T> {
    tokio::select! {biased;
        _=session.cancelled()=>None,
        _=turn.cancelled()=>None,
        result=preparation=>Some(result),
    }
}

impl EngineProcessScope {
    /// Wait on retained owner output/terminals. No tool admission, stdout reader,
    /// process lease renewal or change to the original deadline occurs here.
    pub(crate) async fn wait_for_changes(&self, processes: &[nomifun_agent_runtime::AgentWaitProcess],
        wake_policy: nomifun_agent_runtime::AgentWaitWakePolicy,
        cancellation: CancellationToken) -> Result<Vec<nomifun_agent_runtime::AgentOwnedProcessObservation>, Wave2HostPortError> {
        if processes.is_empty() || processes.len() > 64 { return Err(error("wait requires exact owned process identities")); }
        let include_output = wake_policy == nomifun_agent_runtime::AgentWaitWakePolicy::OutputOrTerminal;
        let mut changes = futures_util::stream::FuturesUnordered::new();
        let mut has_system = false;
        let mut notification_closed = false;
        {
            let state = self.state.lock().await;
            let mut seen = BTreeSet::new();
            for process in processes {
                if !seen.insert(&process.process_id) { return Err(error("duplicate process wait identity")); }
                let entry = state.sessions.get(&process.process_id).ok_or_else(|| error("wait process is not owned by this exact Session"))?;
                if let Some(receiver) = entry.session.terminal_changes() {
                    changes.push(next_process_change(receiver));
                } else { has_system = true; }
                if include_output {
                    if let Some(receiver) = self.owner.output_changes(&entry.session).map_err(outcome_unknown)? {
                        changes.push(next_process_change(receiver));
                    }
                }
            }
        }
        loop {
            if cancellation.is_cancelled() || self.closed.load(Ordering::Acquire) { return Err(error("process wait cancelled")); }
            let mut observations = Vec::new();
            {
                let mut state = self.state.lock().await;
                for process in processes {
                    let entry = state.sessions.get_mut(&process.process_id).ok_or_else(|| error("wait process ownership changed"))?;
                    if let Some(poll) = self.owner.observe_if_changed(&mut entry.session, process.output_cursor,
                        include_output, self.cancellation.clone()).await.map_err(outcome_unknown)? {
                        if poll.is_terminal() && !cleanup_is_proven(&poll) {
                            return Err(outcome_unknown("the original process wait has no proven cleanup receipt"));
                        }
                        if cleanup_is_proven(&poll) { entry.terminal = Some(poll.clone()); }
                        observations.push(nomifun_agent_runtime::AgentOwnedProcessObservation { process_id: process.process_id.clone(), poll });
                    }
                }
            }
            if !observations.is_empty() { return Ok(observations); }
            if notification_closed { return Err(outcome_unknown("original process lifecycle closed without a terminal observation")); }
            tokio::select! { biased;
                _ = cancellation.cancelled() => return Err(error("process wait cancelled")),
                _ = self.cancellation.cancelled() => return Err(error("process scope closed")),
                receiver = changes.next(), if !changes.is_empty() => {
                    match receiver {
                        Some(Ok(receiver)) => changes.push(next_process_change(receiver)),
                        // A retired registry can close its output watch after
                        // freezing the terminal. Inspect its retained witness
                        // once before classifying this as an owner loss.
                        Some(Err(_)) => notification_closed = true,
                        None => return Err(outcome_unknown("original process lifecycle owner is unavailable")),
                    }
                },
                _ = tokio::time::sleep(Duration::from_millis(250)), if has_system => {},
            }
        }
    }

    pub(crate) fn close_admission(&self) {
        self.closed.store(true, Ordering::Release);
        self.cancellation.cancel();
        if let Some(turn)=self.turn.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref(){turn.cancellation.cancel();}
    }

    pub(crate) fn workspace_root(&self) -> &Path {
        self.owner.workspace_root()
    }
    pub(crate) fn owner_generation(&self) -> &str { &self.owner_generation }
    pub(crate) fn new_session(root: &Path, owner_id: &str, session_id: &str) -> Result<Self, Wave2HostPortError> {
        Ok(Self {
            owner: ManagedEngineProcessOwner::new(root, Default::default()).map_err(error)?,
            state: Mutex::new(ProcessState::default()),
            turn: std::sync::Mutex::new(None),
            cancellation: CancellationToken::new(),
            closed: false.into(),
            session_id: session_id.into(), owner_id: owner_id.into(), owner_generation: uuid::Uuid::now_v7().to_string(),
            canonical: std::sync::OnceLock::new(),
        })
    }

    pub(crate) fn open_turn(&self, operation: &str, journal: EngineTurnJournal, authority: Option<ProcessAuthority>)
        -> Result<(), Wave2HostPortError> {
        let mut turn = self.turn.lock().map_err(error)?;
        if self.closed.load(Ordering::Acquire) || turn.is_some() { return Err(error("previous process Turn is not closed")); }
        let _ = self.canonical.set(journal.canonical_store());
        *turn = Some(Arc::new(ProcessTurn { operation: operation.into(), journal, authority,
            operations: Default::default(), closed: false.into(), cancellation:CancellationToken::new() }));
        Ok(())
    }

    pub(crate) fn finish_turn(&self, operation: &str) -> Result<(), Wave2HostPortError> {
        let mut turn = self.turn.lock().map_err(error)?;
        let Some(current) = turn.as_ref() else { return Ok(()); };
        if current.operation != operation { return Err(error("process Turn closure targets a different owner")); }
        current.closed.store(true, Ordering::Release);
        current.cancellation.cancel();
        *turn = None;
        Ok(())
    }

    pub(crate) fn require_turn(&self, operation: &str) -> Result<(), Wave2HostPortError> {
        self.admitted_turn(Some(operation)).map(|_| ())
    }

    /// This proves live ownership, never process success or tree cleanup.
    /// Unregistered starts and native unknown cleanup keep the Session fenced.
    pub(crate) async fn ensure_retained(&self) -> Result<(), Wave2HostPortError> {
        let mut state = self.state.lock().await;
        if state.unregistered_start || state.cleanup_panicked { return Err(outcome_unknown("Session process ownership is unproven")); }
        for entry in state.sessions.values_mut() {
            if let Some(poll) = self.owner.terminal_observation_if_ready(&mut entry.session, self.cancellation.clone()).await.map_err(outcome_unknown)? {
                if !cleanup_is_proven(&poll) { return Err(outcome_unknown(format!("creator {} process cleanup is unproven", entry.creator_call))); }
                entry.terminal = Some(poll);
            }
        }
        Ok(())
    }

    pub(crate) async fn retention(&self) -> Result<nomifun_agent_contracts::NativeSessionProcessRetention, Wave2HostPortError> {
        let mut state = self.state.lock().await;
        if state.unregistered_start || state.cleanup_panicked { return Err(outcome_unknown("Session process ownership is unproven")); }
        self.retire_confirmed_terminals(&mut state).await?;
        let mut processes = state.sessions.iter().map(|(id, entry)| {
            let authority = entry.authority.as_ref().ok_or_else(|| error("retained process has no original frozen authority"))?;
            Ok(nomifun_agent_contracts::NativeRetainedProcess { process_id: id.clone(), pid: entry.session.pid(),
                creator_turn_id: entry.creator_operation.as_str().into(), creator_call_id: entry.creator_call.as_str().into(),
                resolved_snapshot_ref: authority.snapshot.clone(), resource_binding_digest: nomifun_agent_contracts::digest_payload(&authority.resources).map_err(error)? })
        }).collect::<Result<Vec<_>,Wave2HostPortError>>()?;
        processes.sort_by(|left,right|left.process_id.cmp(&right.process_id));
        let retention = nomifun_agent_contracts::NativeSessionProcessRetention { agent_session_id:self.session_id.as_str().into(),
            owner_ref:nomifun_agent_contracts::PrincipalRef {principal_kind:"user".into(),principal_id:self.owner_id.clone()},
            owner_generation:self.owner_generation.clone(), processes };
        retention.validate().map_err(error)?;
        Ok(retention)
    }

    /// Native terminal handles are only a cache once the original first-party
    /// result proves both their identity and reap in the canonical Session.
    /// A missing/failed durable settlement keeps the handle and its quota.
    async fn retire_confirmed_terminals(&self, state: &mut ProcessState) -> Result<(), Wave2HostPortError> {
        let Some(store) = self.canonical.get() else { return Ok(()); };
        let processes = state.sessions.iter().map(|(id, entry)| {
            if !entry.terminal.as_ref().is_some_and(cleanup_is_proven) { return Ok(None); }
            let Some(authority) = entry.authority.as_ref() else { return Ok(None); };
            Ok(Some(nomifun_agent_contracts::NativeRetainedProcess { process_id: id.clone(), pid: entry.session.pid(),
                creator_turn_id: entry.creator_operation.as_str().into(), creator_call_id: entry.creator_call.as_str().into(),
                resolved_snapshot_ref: authority.snapshot.clone(),
                resource_binding_digest: nomifun_agent_contracts::digest_payload(&authority.resources).map_err(error)? }))
        }).collect::<Result<Vec<_>,Wave2HostPortError>>()?.into_iter().flatten().collect::<Vec<_>>();
        if processes.is_empty() { return Ok(()); }
        let retention = nomifun_agent_contracts::NativeSessionProcessRetention { agent_session_id: self.session_id.as_str().into(),
            owner_ref: nomifun_agent_contracts::PrincipalRef { principal_kind: "user".into(), principal_id: self.owner_id.clone() },
            owner_generation: self.owner_generation.clone(), processes };
        let confirmed = store.confirmed_reaped_session_processes(&retention).await.map_err(error)?;
        state.sessions.retain(|id, _| !confirmed.contains(id));
        Ok(())
    }

    pub(crate) async fn release_receipt(&self) -> Result<nomifun_agent_contracts::SessionProcessOwnerReleasedV1,Wave2HostPortError> {
        if !self.closed.load(Ordering::Acquire) || !self.state.lock().await.is_quiescent() {
            return Err(outcome_unknown("Session process owner has not proved tree cleanup"));
        }
        let receipt=nomifun_agent_contracts::SessionProcessOwnerReleasedV1 {retention:self.retention().await?,reaped:true};
        Ok(receipt)
    }

    fn admitted_turn(&self, operation: Option<&str>) -> Result<Arc<ProcessTurn>, Wave2HostPortError> {
        self.turn.lock().map_err(error)?.as_ref().filter(|turn| !turn.closed.load(Ordering::Acquire)
            && operation.is_none_or(|operation| turn.operation == operation)).cloned()
            .ok_or_else(|| error("no admitted process Turn for this exact operation"))
    }

    pub(crate) fn authorize(&self, context: &nomifun_agent_domain_wave2::Wave2HostContext) -> Result<(), Wave2HostPortError> {
        let turn = self.admitted_turn(Some(context.turn_id.as_ref()))?;
        let authority = turn.authority.as_ref().ok_or_else(|| error("process Turn has no frozen authority"))?;
        if authority.snapshot != context.resolved_snapshot_ref || !authority.actions.contains(&context.action_id)
            || authority.resources != context.resource_bindings { return Err(error("process invocation differs from the frozen Session authority")); }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn new(root: &Path, journal: EngineTurnJournal) -> Result<Self, Wave2HostPortError> {
        let scope = Self::new_session(root,"0190f5fe-7c00-7a00-8000-000000000001","0190f5fe-7c00-7a00-8000-000000000002")?;
        scope.open_turn("fixture-turn", journal, None)?;
        Ok(scope)
    }

    #[cfg(test)]
    pub(crate) async fn invoke(
        &self,
        input: StrictJsonValue,
        operation_id: &str,
    ) -> Result<StrictJsonValue, Wave2HostPortError> {
        self.invoke_for_turn(input, operation_id, None).await
    }

    pub(crate) async fn invoke_for_turn(&self, input: StrictJsonValue, operation_id: &str, turn_id: Option<&str>)
        -> Result<StrictJsonValue, Wave2HostPortError> {
        let turn = self.admitted_turn(turn_id)?;
        let params: Params = serde_json::from_value(input.0)
            .map_err(|e| Wave2HostPortError::new("INVALID_PAYLOAD", e.to_string()))?;
        if params.component.is_some() {
            return match self.prepare_component_params(params,&turn.cancellation).await? {
                PreparedWave2Invocation::Invoke(prepared) => self.invoke_params(prepared.params, Some((prepared.component, prepared.prepared)), operation_id, turn).await,
                PreparedWave2Invocation::Return(observation) => Ok(observation),
            };
        }
        self.invoke_params(params, None, operation_id, turn).await
    }

    /// Run only after the original Effect reader has ruled out replay/unknown results.
    /// No process intent or workspace Effect is reserved while the supplier works.
    pub(crate) async fn prepare_component(
        &self,
        input: StrictJsonValue,
        turn_id: &str,
    ) -> Result<PreparedWave2Invocation<PreparedProcessInvocation>, Wave2HostPortError> {
        let turn=self.admitted_turn(Some(turn_id))?;
        let params = serde_json::from_value(input.0)
            .map_err(|e| Wave2HostPortError::new("INVALID_PAYLOAD", e.to_string()))?;
        self.prepare_component_params(params,&turn.cancellation).await
    }

    async fn prepare_component_params(
        &self,
        params: Params,
        cancellation: &CancellationToken,
    ) -> Result<PreparedWave2Invocation<PreparedProcessInvocation>, Wave2HostPortError> {
        if params.operation != Operation::Start
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
        if self.closed.load(Ordering::Acquire) || self.cancellation.is_cancelled() || cancellation.is_cancelled() {
            return Ok(PreparedWave2Invocation::Return(component_not_started(
                name, "CANCELLED", "Component preparation cancelled before the requested program started.",
            )));
        }
        let mut request = nomifun_engine_core::EngineProcessRequest::pipe(name);
        request.args = params.args.clone();
        request.cwd = params.cwd.clone();
        request.env = params.env.clone();
        request.execution_identity = params.execution_identity;
        request.timeout_ms = params.timeout_ms;
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
        let cwd = self.workspace_root().join(params.cwd.as_deref().unwrap_or("."));
        let Some(preparation) = prepare_while_turn_open(&self.cancellation,cancellation,
            tokio::time::timeout(COMPONENT_PREPARATION_WAIT,
                nomifun_runtime::prepare_component_launch(component, &environment, &cwd))).await else {
            return Ok(PreparedWave2Invocation::Return(component_not_started(
                name, "CANCELLED", "Component preparation cancelled before the requested program started.",
            )));
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
        if self.closed.load(Ordering::Acquire) || self.cancellation.is_cancelled() || cancellation.is_cancelled() {
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

    pub(crate) async fn invoke_prepared_for_turn(&self, invocation: PreparedProcessInvocation, operation_id: &str, turn_id: Option<&str>)
        -> Result<StrictJsonValue, Wave2HostPortError> {
        let turn = self.admitted_turn(turn_id)?;
        self.invoke_params(invocation.params, Some((invocation.component, invocation.prepared)), operation_id, turn).await
    }

    async fn invoke_params(
        &self,
        params: Params,
        component: Option<(nomifun_runtime::ToolId, nomifun_runtime::PreparedComponentLaunch)>,
        operation_id: &str,
        turn: Arc<ProcessTurn>,
    ) -> Result<StrictJsonValue, Wave2HostPortError> {
        if turn.authority.as_ref().is_some_and(|authority| !authority.actions.contains(&params.operation.action_id())) {
            return Err(error("process action is outside the current Turn's frozen grant ceiling"));
        }
        if params.wait_ms.is_some_and(|wait| wait > 30_000)
            || stdin_input_exceeds_budget(&params)
            || pty_dimension_out_of_range(&params)
        {
            return Err(error("process wait/input/PTY dimension budget exceeded"));
        }
        let mut state = self.state.lock().await;
        if self.closed.load(Ordering::Acquire) || turn.closed.load(Ordering::Acquire) || state.unregistered_start || state.cleanup_panicked
        {
            if let Some((id, _)) = &component {
                if !state.unregistered_start && !state.cleanup_panicked {
                    return Ok(component_not_started(id.as_str(), "CANCELLED", "The requested program did not start; the process turn is closed."));
                }
            }
            return Err(error("process turn is closed"));
        }
        let ordinal = {
        let mut operations = turn.operations.lock().map_err(error)?;
        if operation_id.is_empty()
            || operation_id.len() > 1024
            || operation_id.chars().any(char::is_control)
            || operations.len() >= 512
            || !operations.insert(operation_id.to_owned())
        {
            return Err(error("invalid, duplicate or excessive process operation"));
        }
        operations.len() as u16
        };
        let intent = serde_json::to_string(&ProcessWitness::Dispatch {
            operation_id: operation_id.to_owned(),
            ordinal,
        })
        .map_err(error)?;
        // Hold the same scope lock through effect and barrier. A later spawn
        // cannot race ahead of an earlier all-handles-reaped observation.
        if let Err(cause) = turn.journal
            .append(intent, None, EngineJournalWrite::Progress)
            .await
        {
            self.close_admission();
            return Err(error(cause));
        }
        let result = self.invoke_owned(params, component, &mut state, &turn, operation_id).await;
        if state.is_quiescent() {
            let witness = serde_json::to_string(&ProcessWitness::Quiescent {
                operation_id: operation_id.to_owned(),
                ordinal,
                process_count: state.sessions.len(),
            })
            .map_err(error)?;
            if let Err(cause) = turn.journal
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
        turn: &ProcessTurn,
        operation_id: &str,
    ) -> Result<StrictJsonValue, Wave2HostPortError> {
        let poll_delay = poll_wait(&params);
        let launch = params.operation == Operation::Start;
        if launch { self.retire_confirmed_terminals(state).await?; }
        let sessions = &mut state.sessions;
        if params.cursor.is_some() && params.operation != Operation::Poll {
            return Err(error("cursor is only valid for poll"));
        }
        if params.append_newline.is_some() && params.operation != Operation::Stdin {
            return Err(error("append_newline is only valid for stdin"));
        }
        let id = if launch {
            if sessions.len() >= 64 {
                return Err(error("Session process limit reached (64 live or unconfirmed native handles)"));
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
                if self.cancellation.is_cancelled() || turn.cancellation.is_cancelled() {
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
            request.execution_identity = params.execution_identity;
            request.timeout_ms = params.timeout_ms;
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
            resolve_managed_program(&mut request, &turn.cancellation, self.workspace_root()).await?;
            state.unregistered_start = true;
            let session = match self
                .owner
                .start_with_evidence(request, turn.cancellation.clone())
                .await
            {
                Ok(session) => session,
                Err(failure) => return process_start_failure(&mut state.unregistered_start, failure),
            };
            let id = session.session_id();
            // No await between owned spawn and registration in the scope.
            sessions.insert(
                id.clone(),
                ProcessEntry {
                    session,
                    terminal: None,
                    component_identity: identity,
                    creator_operation: turn.operation.clone(),
                    creator_call: operation_id.into(),
                    authority: turn.authority.clone(),
                },
            );
            state.unregistered_start = false;
            id
        } else {
            if params.command.is_some() || params.cmd.is_some() || params.component.is_some()
                || !params.args.is_empty()
                || params.cwd.is_some()
                || !params.env.is_empty()
                || params.execution_identity != EngineExecutionIdentity::CurrentUser
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
                "message":"The process_id is not available in this exact Session. No process control was applied. Copy the exact process_id from start_process or its later receipt, then correct this call. Process capability remains available. Do not start a replacement merely to recover a mistyped reference; the original process may still be running."
            }))),
            None => return Err(error("process_id is not owned by this exact Session")),
        };
        let input_control = matches!(params.operation, Operation::Stdin | Operation::CloseStdin | Operation::Resize);
        if !launch && entry.creator_operation != turn.operation {
            match (&entry.authority, &turn.authority) {
                (Some(creator), Some(current)) if creator.resources == current.resources
                    && creator.actions.contains(&params.operation.action_id()) => {},
                _ => return Err(error("process control exceeds its creator's frozen resource or action ceiling")),
            }
        }
        if input_control && entry.terminal.is_none() {
            // A deadline can settle in the owner between model steps. Read its
            // frozen terminal before attempting input; do not consume running
            // output or turn a missing/uncertain owner into a successful stop.
            if let Some(poll) = self.owner.terminal_observation_if_ready(&mut entry.session, self.cancellation.clone()).await.map_err(outcome_unknown)? {
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
                    .write_stdin(session, &bytes, turn.cancellation.clone())
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
        let observed = async { match params.operation {
            Operation::Start => self.owner.initial_observation(session, poll_delay, self.cancellation.clone()).await,
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
        }};
        let poll = if params.operation == Operation::Cancel { observed.await } else {
            let settled=tokio::select! {biased;
                _=turn.cancellation.cancelled()=>None,
                result=observed=>Some(result),
            };
            match settled {Some(result)=>result,None=>{
                let cursor=params.cursor.unwrap_or(session.cursor());
                self.owner.poll_from(session,cursor,Duration::ZERO,self.cancellation.clone()).await
            }}
        }.map_err(outcome_unknown)?;
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

/// Prepare each launch with the same command discovery and user tool environment
/// as Terminal and MCP, before entering the uncertain process-start window.
async fn resolve_managed_program(
    request: &mut EngineProcessRequest,
    cancellation: &CancellationToken,
    workspace_root: &Path,
) -> Result<(), Wave2HostPortError> {
    if cancellation.is_cancelled() {
        return Err(Wave2HostPortError::new("CANCELLED", "Process preparation cancelled before user code started"));
    }
    let env = request.env.iter().map(|(name, value)| (name.clone(), value.clone())).collect();
    // Explicit shell text stays shell text; the owner selects the platform shell.
    // Its child environment still receives newly prepared runtimes and user tools.
    if request.shell_script.is_some() {
        request.env = nomifun_runtime::process_environment_for(nomifun_runtime::ProcessEnvironmentTarget::platform_shell(), &env).map_err(|cause| runtime_resolution_error(&cause))?
            .into_iter().collect();
        return Ok(());
    }
    let cwd = workspace_root.join(request.cwd.as_deref().unwrap_or("."));
    let prepared = tokio::select! {
        biased;
        _ = cancellation.cancelled() => return Err(Wave2HostPortError::new("CANCELLED", "Process preparation cancelled before user code started")),
        result = nomifun_runtime::prepare_process_launch(&request.command, &request.args, &cwd, &env) => result.map_err(|cause| runtime_resolution_error(&cause))?,
    };
    if cancellation.is_cancelled() {
        return Err(Wave2HostPortError::new("CANCELLED", "Process preparation cancelled before user code started"));
    }
    request.command = prepared.program;
    request.args = prepared.args;
    request.env = prepared.environment.into_iter().collect();
    request.validate().map_err(error)
}

fn runtime_resolution_error(cause: &nomifun_runtime::ToolError) -> Wave2HostPortError {
    match cause {
        nomifun_runtime::ToolError::Shared(cause) => runtime_resolution_error(cause),
        nomifun_runtime::ToolError::Cancelled => Wave2HostPortError::new("CANCELLED", cause.to_string()),
        cause => Wave2HostPortError::new("COMPONENT_UNAVAILABLE", nomi_redact::redact_secrets_owned(cause.to_string())),
    }
}

fn process_start_failure(
    unregistered_start: &mut bool,
    failure: EngineProcessStartError,
) -> Result<StrictJsonValue, Wave2HostPortError> {
    *unregistered_start = !failure.no_live_process_proven;
    if failure.user_code_not_started {
        return Ok(StrictJsonValue(serde_json::json!({
            "schema":"nomifun.process-start-observation.v1", "state":"not_started",
            "success":false, "user_code_started":false, "code":"PROCESS_NOT_STARTED",
            "message":nomi_redact::redact_secrets_owned(failure.error.to_string()),
            "guidance":"Use command with separate literal args for native executables; use cmd for explicit shell syntax. Check the executable path, cwd and the operating system error before deciding how to recover."
        })));
    }
    // Reaping the native tree proves cleanup only. Without a real terminal
    // result, a start-lost program may already have changed the system.
    Err(outcome_unknown(failure.error))
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

    #[tokio::test]
    async fn turn_stop_releases_component_preparation_wait_without_closing_session() {
        let root=tempfile::tempdir().unwrap();
        let(journal,pool)=super::super::engine_journal::test_fixture().await;
        let scope=Arc::new(EngineProcessScope::new(root.path(),journal).unwrap());
        let turn=scope.admitted_turn(None).unwrap();
        let waiting=scope.clone();let token=turn.cancellation.clone();
        let entered=CancellationToken::new();let enter=entered.clone();
        let worker=tokio::spawn(async move {
            prepare_while_turn_open(&waiting.cancellation,&token,async {enter.cancel();std::future::pending::<()>().await}).await
        });
        entered.cancelled().await;
        scope.finish_turn("fixture-turn").unwrap();
        assert!(tokio::time::timeout(Duration::from_secs(1),worker).await.unwrap().unwrap().is_none());
        assert!(!scope.cancellation.is_cancelled());
        assert!(scope.state.lock().await.sessions.is_empty());
        scope.cleanup().await.unwrap();drop(turn);drop(scope);pool.close().await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn turn_stop_interrupts_initial_observation_but_retains_the_registered_child() {
        let root=tempfile::tempdir().unwrap();
        let(journal,pool)=super::super::engine_journal::test_fixture().await;
        let scope=Arc::new(EngineProcessScope::new(root.path(),journal).unwrap());
        let launching=scope.clone();
        let worker=tokio::spawn(async move {launching.invoke(StrictJsonValue(serde_json::json!({
            "operation":"start","cmd":"printf 'READY' > initial-observation-ready; IFS= read -r answer","wait_ms":30000
        })),"stop-during-initial-observation").await});
        // The real child reaches its pipe read while the owner remains inside
        // the long initial observation. A Turn stop must not send it a signal.
        tokio::time::timeout(Duration::from_secs(5),async {
            while !root.path().join("initial-observation-ready").exists(){tokio::time::sleep(Duration::from_millis(5)).await;}
        }).await.unwrap();
        scope.finish_turn("fixture-turn").unwrap();
        let observed=tokio::time::timeout(Duration::from_secs(5),worker).await.unwrap().unwrap().unwrap().0;
        assert_eq!(observed["state"],"running");
        assert!(observed["process_id"].as_str().is_some());
        assert!(!scope.cancellation.is_cancelled());
        assert!(!scope.state.lock().await.is_quiescent());
        scope.cleanup().await.unwrap();drop(scope);pool.close().await;
    }

    #[cfg(unix)]
    fn session_process_authority(actions: &[&str]) -> ProcessAuthority {
        ProcessAuthority {
            snapshot: nomifun_agent_contracts::ResolvedSnapshotRef { snapshot_id: "snapshot".into(), snapshot_digest: "b".repeat(64).into() },
            actions: actions.iter().map(|action| format!("workspace.process/{action}").into()).collect(),
            resources: vec![nomifun_agent_contracts::TypedResourceBinding {
                binding_id: "process-binding".into(), resource_kind: "process_session".into(), resource_id: "managed-process-session".into(),
                owner_id: "owner".into(), operations: BTreeSet::from(["execute".into()]), connection_config_ref: None, typed_parameters: BTreeMap::new(),
            }],
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn more_than_sixty_four_short_commands_retire_only_canonical_reaped_handles() {
        let root = tempfile::tempdir().unwrap();
        let (journal, pool) = super::super::engine_journal::test_fixture().await;
        let scope = EngineProcessScope::new_session(root.path(),
            "0190f5fe-7c00-7a00-8000-000000000001", "0190f5fe-7c00-7a00-8000-000000000002").unwrap();
        scope.open_turn("turn", journal.clone(), Some(session_process_authority(&["start", "poll"]))).unwrap();
        let mut identities = BTreeSet::new();
        let mut last = String::new();
        for index in 0..70 {
            let operation = format!("short-command-{index}");
            let call = format!("short-call-{index}");
            journal.append(serde_json::json!({"event":"host_tool_dispatch", "dispatch":{
                "operation_id":operation, "call_id":call, "capability_id":"workspace.process",
                "action_id":"workspace.process/start", "model_name":"start_process"}}).to_string(),
                None, EngineJournalWrite::Progress).await.unwrap();
            let output = scope.invoke_for_turn(StrictJsonValue(serde_json::json!({"operation":"start",
                "command":"/usr/bin/printf", "args":["short-result"], "wait_ms":1000})), &operation, Some("turn")).await.unwrap();
            assert_eq!(output.0["state"], "exited");
            assert_eq!(output.0["cleanup"]["reaped"], true);
            assert_eq!(output.0["output"]["text"], "short-result");
            last = output.0["process_id"].as_str().unwrap().to_owned();
            assert!(identities.insert(last.clone()), "a retired terminal cannot be replaced by a replayed process");
            assert_eq!(scope.state.lock().await.sessions.len(), 1,
                "the previous canonical terminal must not consume a lifetime Session quota");
            // A native exit alone is not a durable receipt. Until the real
            // result is committed the handle must remain owned and retained.
            assert!(scope.retention().await.unwrap().processes.iter().any(|process| process.process_id == last));
            let result = nomifun_engine_core::EngineToolResult::text(call.clone().into(), output.0.to_string(), false);
            journal.append(serde_json::json!({"event":"host_tool_settled", "operation_id":operation,
                "call_id":call, "result":result, "error":null}).to_string(), None, EngineJournalWrite::Settlement).await.unwrap();
        }
        assert_eq!(identities.len(), 70);
        assert!(scope.retention().await.unwrap().processes.is_empty());
        let retired = scope.invoke_for_turn(StrictJsonValue(serde_json::json!({"operation":"poll", "process_id":last})),
            "observe-retired-command", Some("turn")).await.unwrap();
        assert_eq!(retired.0["code"], "PROCESS_REFERENCE_INVALID");
        assert!(scope.state.lock().await.sessions.is_empty(), "a retired reference cannot start or recreate a process");
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='host_tool_settled'")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(count, 70, "every retired result stays in the original canonical history");
        scope.cleanup().await.unwrap();
        drop(scope);
        drop(journal);
        pool.close().await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn same_session_process_survives_turn_handoff_and_accepts_original_handle_in_successor() {
        let root = tempfile::tempdir().unwrap();
        let (first, first_pool) = super::super::engine_journal::test_fixture().await;
        let scope = EngineProcessScope::new_session(root.path(),"0190f5fe-7c00-7a00-8000-000000000001","0190f5fe-7c00-7a00-8000-000000000002").unwrap();
        let authority = session_process_authority(&["start", "poll", "input", "cancel"]);
        scope.open_turn("first", first, Some(authority.clone())).unwrap();
        let launched = scope.invoke_for_turn(StrictJsonValue(serde_json::json!({
            "operation":"start", "cmd":"printf 'READY\\n'; IFS= read -r answer; printf 'ANSWER:%s\\n' \"$answer\"", "wait_ms":0
        })), "original-launch", Some("first")).await.unwrap().0;
        let id = launched["process_id"].as_str().unwrap().to_owned();
        let pid = launched["pid"].as_u64().unwrap();
        scope.finish_turn("first").unwrap();
        scope.ensure_retained().await.unwrap();
        assert!(!scope.state.lock().await.is_quiescent());
        assert!(scope.invoke_for_turn(StrictJsonValue(serde_json::json!({"operation":"cancel","process_id":id})),
            "late-old-call", Some("first")).await.is_err());
        let (second, second_pool) = super::super::engine_journal::test_fixture().await;
        scope.open_turn("second", second, Some(authority)).unwrap();
        let observed = scope.invoke_for_turn(StrictJsonValue(serde_json::json!({"operation":"poll","process_id":id})),
            "successor-observation", Some("second")).await.unwrap().0;
        assert_eq!(observed["process_id"], id);
        assert_eq!(observed["pid"], pid);
        assert_eq!(observed["state"], "running");
        let sent = scope.invoke_for_turn(StrictJsonValue(serde_json::json!({"operation":"stdin","process_id":id,
            "input":"accepted", "append_newline":true})), "successor-answer", Some("second")).await.unwrap().0;
        let (terminal,text)=tokio::time::timeout(Duration::from_secs(5),async {
            let mut observed=sent;let mut text=String::new();let mut ordinal=0;
            loop {
                text.push_str(observed["output"]["text"].as_str().unwrap());
                if observed["state"]!="running" {break (observed,text);}
                let cursor=observed["output"]["next_cursor"].as_u64().unwrap();
                observed=scope.invoke_for_turn(StrictJsonValue(serde_json::json!({"operation":"poll","process_id":id,"cursor":cursor,"wait_ms":1000})),
                    &format!("successor-terminal-{ordinal}"),Some("second")).await.unwrap().0;
                ordinal+=1;
            }
        }).await.unwrap();
        assert_eq!(terminal["state"], "exited");
        assert!(text.contains("ANSWER:accepted"));
        let state = scope.state.lock().await;
        assert_eq!(state.sessions.len(), 1, "a successor must never relaunch the command");
        assert_eq!(state.sessions[&id].creator_operation, "first");
        assert_eq!(state.sessions[&id].creator_call, "original-launch");
        drop(state);
        scope.cleanup().await.unwrap();
        drop(scope); first_pool.close().await; second_pool.close().await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn successor_process_controls_keep_creator_grants_and_session_teardown_reaps_background() {
        for rejected_ceiling in ["creator_grant", "current_grant", "resource"] {
        let root = tempfile::tempdir().unwrap();
        let (first, first_pool) = super::super::engine_journal::test_fixture().await;
        let scope = EngineProcessScope::new_session(root.path(),"0190f5fe-7c00-7a00-8000-000000000001","0190f5fe-7c00-7a00-8000-000000000002").unwrap();
        let creator = session_process_authority(if rejected_ceiling == "creator_grant" { &["start"] } else { &["start","input"] });
        scope.open_turn("first", first, Some(creator.clone())).unwrap();
        let launched = scope.invoke_for_turn(StrictJsonValue(serde_json::json!({"operation":"start", "cmd":"IFS= read -r answer", "wait_ms":0})),
            "launch-grant-only", Some("first")).await.unwrap().0;
        let id = launched["process_id"].as_str().unwrap().to_owned();
        scope.finish_turn("first").unwrap();
        let (second, second_pool) = super::super::engine_journal::test_fixture().await;
        let mut current = session_process_authority(if rejected_ceiling == "current_grant" { &["start","poll"] } else { &["start","poll","input","cancel"] });
        if rejected_ceiling == "resource" { current.resources[0].resource_id = "different-process-resource".into(); }
        scope.open_turn("second", second, Some(current)).unwrap();
        assert!(scope.invoke_for_turn(StrictJsonValue(serde_json::json!({"operation":"stdin","process_id":id,"input":"forbidden"})),
            "reject-expanded-control", Some("second")).await.is_err());
        scope.ensure_retained().await.unwrap();
        scope.finish_turn("second").unwrap();
        scope.cleanup().await.unwrap();
        let state = scope.state.lock().await;
        assert!(state.is_quiescent());
        assert!(matches!(state.sessions[&id].terminal, Some(EngineProcessPoll::Cancelled { .. })));
        drop(state); drop(scope); first_pool.close().await; second_pool.close().await;
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn background_exit_between_turns_only_retains_terminal_until_next_explicit_observation() {
        let root = tempfile::tempdir().unwrap();
        let (first, first_pool) = super::super::engine_journal::test_fixture().await;
        let scope = EngineProcessScope::new_session(root.path(),"0190f5fe-7c00-7a00-8000-000000000001","0190f5fe-7c00-7a00-8000-000000000002").unwrap();
        let authority = session_process_authority(&["start","poll"]);
        scope.open_turn("first", first, Some(authority.clone())).unwrap();
        let launched = scope.invoke_for_turn(StrictJsonValue(serde_json::json!({"operation":"start", "cmd":"sleep 0.1; printf 'BACKGROUND_DONE'", "wait_ms":0})),
            "finite-third-party-work", Some("first")).await.unwrap().0;
        let id = launched["process_id"].as_str().unwrap().to_owned();
        scope.finish_turn("first").unwrap();
        let original_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events").fetch_one(&first_pool).await.unwrap();
        tokio::time::sleep(Duration::from_millis(250)).await;
        assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM agent_events").fetch_one(&first_pool).await.unwrap(), original_count,
            "background output and exit cannot synthesize a Turn/model event");
        let (second, second_pool) = super::super::engine_journal::test_fixture().await;
        scope.open_turn("second", second, Some(authority)).unwrap();
        let observed = scope.invoke_for_turn(StrictJsonValue(serde_json::json!({"operation":"poll","process_id":id,"wait_ms":1000})),
            "observe-real-terminal", Some("second")).await.unwrap().0;
        assert_eq!(observed["state"], "exited");
        assert_eq!(observed["output"]["text"], "BACKGROUND_DONE");
        assert_eq!(observed["cleanup"]["reaped"], true);
        scope.cleanup().await.unwrap(); drop(scope); first_pool.close().await; second_pool.close().await;
    }
    use nomifun_engine_core::{EngineCleanupReport, EngineProcessOutput};
    include!("engine_process_component_tests.rs");

    #[tokio::test]
    async fn managed_program_preserves_shell_text_and_explicit_user_executables() {
        for mut request in [EngineProcessRequest::shell("bun test"), EngineProcessRequest::pipe("/user/tools/bun"), EngineProcessRequest::pipe("bun test")] {
            let original = request.clone();
            resolve_managed_program(&mut request, &CancellationToken::new(), Path::new(".")).await.unwrap();
            assert_eq!(request.command, original.command);
            assert_eq!(request.args, original.args);
            assert_eq!(request.shell_script, original.shell_script);
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
    fn reaped_start_failure_retains_unknown_effect_when_user_code_may_have_run() {
        for reaped in [true, false] {
            let mut state = ProcessState::default();
            let result = process_start_failure(&mut state.unregistered_start, EngineProcessStartError {
                error: nomifun_engine_core::EngineProcessError::Process("owner lost its startup result after user code may have run".into()),
                no_live_process_proven: reaped, user_code_not_started: false,
            }).unwrap_err();
            assert_eq!(result.code, "EFFECT_OUTCOME_UNKNOWN");
            assert_eq!(state.unregistered_start, !reaped);
            assert_eq!(state.is_quiescent(), reaped, "cleanup and effect outcome are independent facts");
        }
    }

    #[test]
    fn runtime_failure_keeps_the_preparation_reason() {
        let failure = runtime_resolution_error(&nomifun_runtime::ToolError::NotPrepared);
        assert_eq!(failure.code, "COMPONENT_UNAVAILABLE");
        assert!(!failure.message.contains("choose to download"));
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
        for (key, value) in original_env {
            if key.eq_ignore_ascii_case("PATH") {
                let actual = request.env.iter().find(|(name, _)| name.eq_ignore_ascii_case("PATH")).unwrap().1;
                assert_eq!(std::env::split_paths(actual).next(), std::env::split_paths(&value).next(), "explicit user command precedence must remain first");
            } else { assert_eq!(request.env.get(&key), Some(&value)); }
        }
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn unsplit_executable_failure_keeps_literal_argv_recovery_available() {
        let root = tempfile::tempdir().unwrap();
        let (journal, pool) = super::super::engine_journal::test_fixture().await;
        let scope = EngineProcessScope::new(root.path(), journal).unwrap();
        for (index, command) in ["/bin/ls -a", "ls -a"].into_iter().enumerate() {
            let rejected = scope.invoke(StrictJsonValue(serde_json::json!({
                "operation":"start", "wait_ms":1000, "command":command, "env":{"PATH":"/usr/bin:/bin"}, "timeout_ms":5000
            })), &format!("reject-unsplit-command-{index}")).await.unwrap().0;
            assert_eq!(rejected["code"], "PROCESS_NOT_STARTED");
            assert_eq!(rejected["state"], "not_started");
            assert_eq!(rejected["success"], false);
            assert_eq!(rejected["user_code_started"], false);
            assert!(scope.state.lock().await.is_quiescent());
            let message = rejected["message"].as_str().unwrap();
            assert!(!message.is_empty());
            assert!(rejected["guidance"].as_str().unwrap().contains("literal args"));
        }

        let corrected = scope.invoke(StrictJsonValue(serde_json::json!({
            "operation":"start", "wait_ms":1000, "command":"/bin/ls", "args":["-a"], "timeout_ms":5000
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
    async fn owner_lifecycle_closed_without_a_terminal_is_not_requeued() {
        let (sender,receiver)=tokio::sync::watch::channel(0u64);
        drop(sender);
        assert!(tokio::time::timeout(Duration::from_millis(100),next_process_change(receiver)).await.unwrap().is_err(),
            "lost owner notification must return unknown, not become a perpetually ready wait future");
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn bounded_launch_finishes_short_commands_and_yields_interactive_output_without_relaunch() {
        let root = tempfile::tempdir().unwrap();
        let (journal, pool) = super::super::engine_journal::test_fixture().await;
        let scope = EngineProcessScope::new(root.path(), journal).unwrap();
        let short = scope.invoke(StrictJsonValue(serde_json::json!({
            "command":"/bin/sh", "args":["-c","printf short"]
        })), "short-launch").await.unwrap().0;
        assert_eq!(short["state"], "exited");
        assert_eq!(short["success"], true);
        let began = tokio::time::Instant::now();
        let interactive = tokio::time::timeout(Duration::from_secs(3), scope.invoke(StrictJsonValue(serde_json::json!({
            "command":"/bin/sh", "args":["-c","printf 'verification-link'; IFS= read -r answer; printf 'confirmed:%s' \"$answer\""],
            "timeout_ms":5000
        })), "interactive-launch")).await.unwrap().unwrap().0;
        assert!(began.elapsed() < Duration::from_secs(3));
        assert_eq!(interactive["state"], "running");
        assert!(interactive["success"].is_null());
        assert_eq!(interactive["output"]["text"], "verification-link");
        let id = interactive["process_id"].as_str().unwrap();
        scope.invoke(StrictJsonValue(serde_json::json!({
            "operation":"stdin", "process_id":id,"input":"accepted","append_newline":true
        })), "real-user-input").await.unwrap();
        let processes = [nomifun_agent_runtime::AgentWaitProcess {process_id:id.into(),output_cursor:0}];
        let observed = scope.wait_for_changes(&processes, nomifun_agent_runtime::AgentWaitWakePolicy::TerminalOnly,
            CancellationToken::new()).await.unwrap();
        assert!(matches!(&observed[0].poll, EngineProcessPoll::Exited {exit_code:Some(0),output,cleanup,..}
            if cleanup.reaped && output.text == "verification-linkconfirmed:accepted"));
        assert_eq!(scope.state.lock().await.sessions.len(),2,"only the original two launches ran");
        assert!(scope.invoke(StrictJsonValue(serde_json::json!({"operation":"exec","command":"/bin/true"})), "retired-exec").await.is_err());
        scope.cleanup().await.unwrap();drop(scope);pool.close().await;
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn owner_wait_observes_delayed_output_but_human_wait_ignores_later_logs() {
        use nomifun_agent_runtime::{AgentWaitProcess, AgentWaitWakePolicy};
        let root = tempfile::tempdir().unwrap();
        let (journal, pool) = super::super::engine_journal::test_fixture().await;
        let scope = EngineProcessScope::new(root.path(), journal).unwrap();
        let started = scope.invoke(StrictJsonValue(serde_json::json!({"operation":"start","wait_ms":0,
            "command":"/bin/sh","args":["-c","sleep 0.1; printf 'verification-link'; sleep 0.1; printf 'still-waiting'; IFS= read -r answer"],
            "timeout_ms":5000})), "delayed-output-launch").await.unwrap().0;
        assert_eq!(started["state"],"running");
        assert_eq!(started["output"]["text"],"");
        let id = started["process_id"].as_str().unwrap();
        let mut processes = [AgentWaitProcess {process_id:id.into(),output_cursor:0}];
        let observed = tokio::time::timeout(Duration::from_secs(2),scope.wait_for_changes(&processes,
            AgentWaitWakePolicy::OutputOrTerminal,CancellationToken::new())).await.unwrap().unwrap();
        let EngineProcessPoll::Running {output,..} = &observed[0].poll else {panic!("delayed output must wake a live owner")};
        assert!(output.text.contains("verification-link"));
        processes[0].output_cursor = output.next_cursor;
        assert!(tokio::time::timeout(Duration::from_millis(350),scope.wait_for_changes(&processes,
            AgentWaitWakePolicy::TerminalOnly,CancellationToken::new())).await.is_err(),
            "later logs cannot wake the model while it is waiting for a human");
        let state = scope.state.lock().await;
        assert_eq!(state.sessions[id].session.cursor(),0,"read-only owner observations do not consume tool output");
        drop(state);
        scope.invoke(StrictJsonValue(serde_json::json!({"operation":"stdin","process_id":id,"input":"done\n"})),"answer-original-owner").await.unwrap();
        let finished = tokio::time::timeout(Duration::from_secs(2),scope.wait_for_changes(&processes,
            AgentWaitWakePolicy::TerminalOnly,CancellationToken::new())).await.unwrap().unwrap();
        assert!(matches!(&finished[0].poll,EngineProcessPoll::Exited {exit_code:Some(0),cleanup,..} if cleanup.reaped));
        scope.cleanup().await.unwrap();drop(scope);pool.close().await;
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn parked_owner_wait_wakes_for_any_process_and_preserves_the_original_deadline() {
        let root = tempfile::tempdir().unwrap();
        let (journal, pool) = super::super::engine_journal::test_fixture().await;
        let scope = EngineProcessScope::new(root.path(), journal).unwrap();
        let mut processes = Vec::new();
        for (index, timeout) in [5000, 150].into_iter().enumerate() {
            let started = scope.invoke(StrictJsonValue(serde_json::json!({
                "operation":"start","command":"/bin/sh","args":["-c","IFS= read -r line"],
                "wait_ms":0,"timeout_ms":timeout
            })), &format!("start-parked-{index}")).await.unwrap().0;
            processes.push(nomifun_agent_runtime::AgentWaitProcess {process_id:started["process_id"].as_str().unwrap().into(),output_cursor:0});
        }
        let observed = tokio::time::timeout(Duration::from_secs(3), scope.wait_for_changes(&processes,nomifun_agent_runtime::AgentWaitWakePolicy::TerminalOnly,CancellationToken::new())).await.unwrap().unwrap();
        assert!(observed.iter().any(|event|event.process_id==processes[1].process_id && matches!(event.poll,EngineProcessPoll::TimedOut {..})));
        assert!(scope.state.lock().await.sessions[&processes[0].process_id].terminal.is_none(),"unrelated live process must remain owned");
        let already_finished=scope.wait_for_changes(&processes[1..],nomifun_agent_runtime::AgentWaitWakePolicy::TerminalOnly,CancellationToken::new()).await.unwrap();
        assert_eq!(observed,already_finished,"terminal-before-registration remains observable");
        let cancellation=CancellationToken::new();cancellation.cancel();
        assert!(scope.wait_for_changes(&processes[..1],nomifun_agent_runtime::AgentWaitWakePolicy::TerminalOnly,cancellation).await.is_err());
        scope.cleanup().await.unwrap();drop(scope);pool.close().await;
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
                    let ready=scope.owner.terminal_observation_if_ready(&mut entry.session, scope.cancellation.clone()).await.unwrap().is_some();
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
            "operation":"start", "wait_ms":1000, "command":"bun", "args":["-e",script], "timeout_ms":5000
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
            "operation":"start", "wait_ms":1000,"command":"cmd.exe","args":["/d","/s","/c","start \"\" /b bun child.mjs & bun await-child.mjs"],"timeout_ms":10000
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
            "operation":"start", "wait_ms":1000, "command":"net.exe", "args":["session"], "timeout_ms":5000
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
                    let ready = scope.owner.terminal_observation_if_ready(&mut entry.session, scope.cancellation.clone()).await.unwrap().is_some();
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
            "operation":"start", "command":command, "args":args, "wait_ms":0
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
            execution_identity: EngineExecutionIdentity::CurrentUser,
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
        let interrupted_launch = process_output("process-1", &poll, Operation::Start)
            .unwrap()
            .0;
        assert_eq!(interrupted_launch["success"], false);
    }
}
