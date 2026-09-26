//! Platform-owned, turn-scoped interactive process sessions. Called only after
//! Wave2/Kernel authority and canonical workspace checks, never by the engine.
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::engine_journal::{EngineJournalWrite, EngineTurnJournal};
use super::engine_process_recovery::ProcessWitness;
use futures_util::FutureExt;
use nomifun_agent_contracts::StrictJsonValue;
use nomifun_agent_domain_wave2::Wave2HostPortError;
use nomifun_engine_core::{
    EngineProcessPoll, EngineProcessRequest, EngineProcessSession, EngineProcessTransport,
    ManagedEngineProcessOwner,
};
use serde::Deserialize;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

fn error(value: impl std::fmt::Display) -> Wave2HostPortError {
    Wave2HostPortError::new("CAPABILITY_UNAVAILABLE", value.to_string())
}

fn outcome_unknown(value: impl std::fmt::Display) -> Wave2HostPortError {
    Wave2HostPortError::new("EFFECT_OUTCOME_UNKNOWN", value.to_string())
}

#[derive(Clone, Copy, Default, Deserialize, PartialEq, Eq)]
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
    #[serde(default)]
    args: Vec<String>,
    cwd: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    timeout_ms: Option<u64>,
    process_id: Option<String>,
    input: Option<String>,
    wait_ms: Option<u64>,
    #[serde(default)]
    tty: bool,
    cols: Option<u16>,
    rows: Option<u16>,
}

fn poll_wait(params: &Params) -> Duration {
    Duration::from_millis(params.wait_ms.unwrap_or(0))
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
    pub(crate) async fn is_quiescent(&self) -> bool {
        self.state.lock().await.is_quiescent()
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
        if params.wait_ms.is_some_and(|wait| wait > 30_000)
            || params
                .input
                .as_ref()
                .is_some_and(|text| text.len() > 1024 * 1024)
        {
            return Err(error("process wait/input budget exceeded"));
        }
        let mut state = self.state.lock().await;
        if self.closed.load(Ordering::Acquire) || state.unregistered_start || state.cleanup_panicked
        {
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
        let result = self.invoke_owned(params, &mut state).await;
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
        state: &mut ProcessState,
    ) -> Result<StrictJsonValue, Wave2HostPortError> {
        let poll_delay = poll_wait(&params);
        let sessions = &mut state.sessions;
        let launch = matches!(params.operation, Operation::Exec | Operation::Start);
        let id = if launch {
            if sessions.len() >= 64 {
                return Err(error("turn process limit reached (64 launches)"));
            }
            if params.process_id.is_some() || params.input.is_some() {
                return Err(error(
                    "launch cannot address an existing process or write stdin",
                ));
            }
            let mut request = match (params.cmd, params.command) {
                (Some(script), None) if params.args.is_empty() => {
                    EngineProcessRequest::shell(script)
                },
                (None, Some(command)) => {
                    let mut request=EngineProcessRequest::pipe(command);
                    request.args=params.args;
                    request
                },
                _ => return Err(error("launch requires cmd OR command plus args; do not mix them")),
            };
            request.cwd = params.cwd;
            request.env = params.env;
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
                            "message":"The requested executable did not start. Use cmd for a shell command, or command for the executable only with a separate args array. Check the executable and cwd, then correct this request. Process capability remains available; this failed launch did not invalidate earlier file/check observations."
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
                },
            );
            state.unregistered_start = false;
            id
        } else {
            if params.command.is_some() || params.cmd.is_some()
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
        let entry = sessions
            .get_mut(&id)
            .ok_or_else(|| error("process_id is not owned by this exact turn"))?;
        if let Some(poll) = &entry.terminal {
            if matches!(params.operation, Operation::Poll | Operation::Cancel) {
                return process_output(&id, poll);
            }
            return Err(error(
                "process already terminated; stdin/resize cannot restart it",
            ));
        }
        let session = &mut entry.session;
        match params.operation {
            Operation::Stdin => self
                .owner
                .write_stdin(
                    session,
                    params
                        .input
                        .as_deref()
                        .ok_or_else(|| error("stdin requires input"))?
                        .as_bytes(),
                    self.cancellation.clone(),
                )
                .await
                .map_err(outcome_unknown)?,
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
        process_output(&id, &poll)
    }

    /// Close admission before waiting for any running launch/poll. The token
    /// wakes exec, poll and stdin, while retained handles prove tree cleanup.
    pub(crate) async fn cleanup(&self) -> Result<(), Wave2HostPortError> {
        self.close_admission();
        let mut state = self.state.lock().await;
        let mut failures = Vec::new();
        if state.unregistered_start {
            failures
                .push("a process start has no retained handle; cleanup cannot be certified".into());
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

fn process_output(
    id: &str,
    poll: &EngineProcessPoll,
) -> Result<StrictJsonValue, Wave2HostPortError> {
    let success = match poll {
        EngineProcessPoll::Running { .. } => None,
        EngineProcessPoll::Exited {
            exit_code, cleanup, ..
        } => Some(*exit_code == Some(0) && cleanup.reaped),
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

    #[test]
    fn omitted_wait_matches_explicit_zero_wire_contract() {
        let omitted: Params = serde_json::from_value(serde_json::json!({
            "operation": "poll",
            "process_id": "process-1"
        }))
        .unwrap();
        let explicit: Params = serde_json::from_value(serde_json::json!({
            "operation": "poll",
            "process_id": "process-1",
            "wait_ms": 0
        }))
        .unwrap();
        assert_eq!(poll_wait(&omitted), Duration::ZERO);
        assert_eq!(poll_wait(&explicit), Duration::ZERO);
    }
}
