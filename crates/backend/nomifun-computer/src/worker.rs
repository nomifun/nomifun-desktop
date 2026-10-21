//! A single retained native owner with per-Session round-robin admission.
//! Cancellation of queued work is independent of this actor's in-flight I/O.
use crate::{
    permissions,
    protocol::*,
    surface::{self, PreparedCall, RouteTier},
};
use nomi_process_runtime::{ChildProcessBuilder, ManagedChildProcess};
use nomifun_agent_kernel::InvocationControl;
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(test)]
use std::ffi::OsString;
use std::{
    collections::{BTreeSet, HashMap, VecDeque},
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{ChildStdin, ChildStdout},
    sync::{mpsc, oneshot, watch},
    time::Instant,
};
use tokio_util::sync::CancellationToken;

const MAX_FRAME_BYTES: usize = 32 * 1024 * 1024;
const QUEUED: u8 = 0;
const DISPATCHED: u8 = 1;
const CANCELLED: u8 = 2;
const FINISHED: u8 = 3;
#[cfg(any(target_os = "macos", test))]
// The native worker uses the upstream 10-second direct-capture budget; leave
// one second for its private-channel response, then reap the exact owner.
const SCREEN_CAPTURE_REQUEST_TIMEOUT: Duration = Duration::from_secs(11);

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ScreenCaptureProbeStatus { Ready, Unavailable, TimedOut, ProbeFailed }

#[cfg(any(target_os = "macos", test))]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScreenCaptureProbeResponse { status: ScreenCaptureProbeStatus }

#[derive(Debug, Clone)]
pub struct WorkerConfig {
    pub executable: PathBuf,
    /// The application's data root, not an already suffixed driver directory.
    pub data_dir: PathBuf,
    pub host_bundle_id: String,
    pub startup_timeout: Duration,
    pub shutdown_timeout: Duration,
    pub idle_timeout: Duration,
    #[cfg(test)]
    prefix_args: Vec<OsString>,
    #[cfg(test)]
    call_timeout: Option<Duration>,
    #[cfg(test)]
    screen_capture_granted: Option<bool>,
    #[cfg(test)]
    screen_capture_timeout: Option<Duration>,
}

#[cfg(test)]
#[path = "worker_tests.rs"]
mod tests;
impl WorkerConfig {
    pub fn new(executable: impl Into<PathBuf>, data_dir: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
            data_dir: data_dir.into(),
            host_bundle_id: "com.nomifun.desktop".into(),
            startup_timeout: Duration::from_secs(10),
            shutdown_timeout: Duration::from_secs(2),
            idle_timeout: Duration::from_secs(300),
            #[cfg(test)]
            prefix_args: Vec::new(),
            #[cfg(test)]
            call_timeout: None,
            #[cfg(test)]
            screen_capture_granted: None,
            #[cfg(test)]
            screen_capture_timeout: None,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallErrorKind {
    BeforeDispatch,
    Cancelled,
    Unknown,
    Unavailable,
}
#[derive(Debug, Clone, thiserror::Error, Serialize)]
#[error("{code}: {message}")]
pub struct CallError {
    pub kind: CallErrorKind,
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_required: Option<Value>,
}
impl CallError {
    fn new(kind: CallErrorKind, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind,
            code: code.into(),
            message: message.into(),
            permission_required: None,
        }
    }
    fn cancelled() -> Self {
        Self::new(
            CallErrorKind::Cancelled,
            "COMPUTER_CALL_CANCELLED",
            "Computer action was withdrawn before dispatch",
        )
    }
    fn unknown(message: impl Into<String>) -> Self {
        Self::new(CallErrorKind::Unknown, "COMPUTER_RESULT_UNKNOWN", message)
    }
    fn unavailable(message: impl Into<String>) -> Self {
        Self::new(CallErrorKind::Unavailable, "COMPUTER_UNAVAILABLE", message)
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct DriverResult {
    pub result: Value,
    pub completion: ActionCompletion,
    pub generation: String,
    pub foreground: bool,
}
#[derive(Debug, Clone, Serialize)]
pub struct WorkerStatus {
    pub worker_version: Option<String>,
    pub platform: String,
    pub backend: Option<String>,
    pub session_type: Option<String>,
    pub wayland: Option<bool>,
    pub atspi: Option<bool>,
    pub running: bool,
    pub phase: String,
    pub generation: Option<String>,
    pub process_id: Option<u32>,
    pub bound_sessions: usize,
    pub queued: usize,
    pub metadata: Option<Value>,
    pub last_error: Option<String>,
}
impl Default for WorkerStatus {
    fn default() -> Self {
        Self {
            worker_version: None,
            platform: surface::platform().into(),
            backend: None,
            session_type: if cfg!(target_os = "linux") {
                std::env::var("XDG_SESSION_TYPE").ok()
            } else {
                None
            },
            wayland: if cfg!(target_os = "linux") {
                Some(std::env::var_os("WAYLAND_DISPLAY").is_some())
            } else {
                None
            },
            atspi: None,
            running: false,
            phase: "stopped".into(),
            generation: None,
            process_id: None,
            bound_sessions: 0,
            queued: 0,
            metadata: None,
            last_error: None,
        }
    }
}
pub struct Computer {
    sender: mpsc::UnboundedSender<Command>,
    status: watch::Receiver<WorkerStatus>,
    accepting: Arc<AtomicBool>,
    shutdown_requested: CancellationToken,
}
struct QueuedCall {
    session: String,
    name: String,
    prepared: PreparedCall,
    control: InvocationControl,
    state: Arc<AtomicU8>,
    reply: oneshot::Sender<Result<DriverResult, CallError>>,
}
enum Command {
    Call(QueuedCall),
    #[cfg(any(target_os = "macos", test))]
    RequestScreenCapture(oneshot::Sender<Result<(), CallError>>),
    Close(String, oneshot::Sender<Result<(), CallError>>),
    Shutdown(oneshot::Sender<Result<(), CallError>>),
}
impl Computer {
    /// Creates only a lightweight owner; no driver process or native operation.
    pub fn new(config: WorkerConfig) -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        let (status_tx, status) = watch::channel(WorkerStatus::default());
        let accepting = Arc::new(AtomicBool::new(true));
        let shutdown_requested = CancellationToken::new();
        tokio::spawn(Actor::new(config, receiver, status_tx, shutdown_requested.clone()).run());
        Self {
            sender,
            status,
            accepting,
            shutdown_requested,
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub async fn call(
        &self,
        session_id: &str,
        agent_name: &str,
        action: &str,
        arguments: Value,
        authorized_actions: &BTreeSet<String>,
        image_enabled: bool,
        control: InvocationControl,
    ) -> Result<DriverResult, CallError> {
        if !self.accepting.load(Ordering::Acquire) {
            return Err(CallError::unavailable("Computer owner is shutting down"));
        }
        let tier = if image_enabled {
            RouteTier::Full
        } else {
            RouteTier::NoImage
        };
        let prepared =
            surface::prepare(action, arguments, authorized_actions, tier).map_err(|error| {
                CallError::new(CallErrorKind::BeforeDispatch, error.code, error.message)
            })?;
        if control.is_cancelled() {
            return Err(CallError::cancelled());
        }
        let state = Arc::new(AtomicU8::new(QUEUED));
        let (reply, mut receiver) = oneshot::channel();
        self.sender
            .send(Command::Call(QueuedCall {
                session: session_id.into(),
                name: agent_name.into(),
                prepared,
                control: control.clone(),
                state: state.clone(),
                reply,
            }))
            .map_err(|_| CallError::unavailable("Computer owner has stopped"))?;
        tokio::select! {
            result=&mut receiver=>result.unwrap_or_else(|_|Err(if matches!(state.load(Ordering::Acquire),QUEUED|CANCELLED){CallError::cancelled()}else{CallError::unknown("Computer owner stopped before settling the request")})),
            _=control.cancelled()=>{
                if state.compare_exchange(QUEUED,CANCELLED,Ordering::AcqRel,Ordering::Acquire).is_ok(){return Err(CallError::cancelled());}
                receiver.await.unwrap_or_else(|_|Err(if matches!(state.load(Ordering::Acquire),QUEUED|CANCELLED){CallError::cancelled()}else{CallError::unknown("Computer owner stopped while an operation was in flight")}))
            },
            _=self.shutdown_requested.cancelled()=>{
                if state.compare_exchange(QUEUED,CANCELLED,Ordering::AcqRel,Ordering::Acquire).is_ok(){return Err(CallError::cancelled());}
                receiver.await.unwrap_or_else(|_|Err(CallError::unknown("Computer owner stopped during app shutdown")))
            }
        }
    }
    pub async fn close_session(&self, session_id: &str) -> Result<(), CallError> {
        let (reply, receiver) = oneshot::channel();
        if self
            .sender
            .send(Command::Close(session_id.into(), reply))
            .is_err()
        {
            return if self.status().phase == "stopped" {
                Ok(())
            } else {
                Err(CallError::unavailable("Computer owner has stopped"))
            };
        }
        receiver
            .await
            .map_err(|_| CallError::unavailable("Session cleanup owner stopped"))?
    }
    /// Only an explicit host permission request may call this. Status, caches,
    /// model tools and Session reconstruction never start this native probe.
    pub async fn request_screen_capture(&self) -> Result<(), CallError> {
        #[cfg(target_os = "macos")]
        {
            if !self.accepting.load(Ordering::Acquire) {
                return Err(CallError::unavailable("Computer owner is shutting down"));
            }
            let (reply, receiver) = oneshot::channel();
            self.sender.send(Command::RequestScreenCapture(reply))
                .map_err(|_| CallError::unavailable("Computer owner has stopped"))?;
            receiver.await.map_err(|_| CallError::unavailable("Screen capture request owner stopped"))?
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err(CallError::new(CallErrorKind::BeforeDispatch, "COMPUTER_PLATFORM_UNSUPPORTED",
                "Explicit screen capture consent is only supported on macOS"))
        }
    }
    pub async fn shutdown(&self) -> Result<(), CallError> {
        self.accepting.store(false, Ordering::Release);
        self.shutdown_requested.cancel();
        let (reply, receiver) = oneshot::channel();
        if self.sender.send(Command::Shutdown(reply)).is_err() {
            return if self.status().phase == "stopped" {
                Ok(())
            } else {
                Err(CallError::unavailable("Computer cleanup was not proven"))
            };
        }
        receiver
            .await
            .map_err(|_| CallError::unavailable("Computer cleanup owner stopped"))?
    }
    pub fn status(&self) -> WorkerStatus {
        self.status.borrow().clone()
    }
}
impl Drop for Computer {
    fn drop(&mut self) {
        self.accepting.store(false, Ordering::Release);
        self.shutdown_requested.cancel();
    }
}

struct SessionBinding {
    handle: String,
    bound: Instant,
    last_used: Instant,
}
struct LiveProcess {
    owner: ManagedChildProcess,
    input: Option<ChildStdin>,
    output: BufReader<ChildStdout>,
    generation: String,
    next_id: u64,
    stderr: Option<tokio::task::JoinHandle<std::io::Result<()>>>,
    stderr_error: Option<String>,
}
impl LiveProcess {
    async fn exchange(
        &mut self,
        operation: &str,
        name: Option<&str>,
        arguments: Option<Value>,
        handle: Option<&str>,
    ) -> Result<ChannelResponse, CallError> {
        let request_id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| CallError::unknown("Worker request identity exhausted"))?;
        let request = ChannelRequest {
            protocol_version: PRIVATE_WORKER_PROTOCOL_VERSION,
            request_id,
            generation: self.generation.clone(),
            operation: operation.into(),
            name: name.map(str::to_owned),
            arguments,
            session_handle: handle.map(str::to_owned),
        };
        let mut frame =
            serde_json::to_vec(&request).map_err(|error| CallError::unknown(error.to_string()))?;
        frame.push(b'\n');
        let input = self
            .input
            .as_mut()
            .ok_or_else(|| CallError::unknown("Worker stdin is closed"))?;
        input.write_all(&frame).await.map_err(|error| {
            CallError::unknown(format!("Worker request transport failed: {error}"))
        })?;
        input
            .flush()
            .await
            .map_err(|error| CallError::unknown(format!("Worker request flush failed: {error}")))?;
        let mut frame = Vec::new();
        loop {
            let bytes = self.output.fill_buf().await.map_err(|error| {
                CallError::unknown(format!("Worker response transport failed: {error}"))
            })?;
            if bytes.is_empty() {
                return Err(CallError::unknown(
                    "Worker exited before returning a complete response",
                ));
            }
            let count = bytes
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(bytes.len(), |end| end + 1);
            if frame.len().saturating_add(count) > MAX_FRAME_BYTES {
                return Err(CallError::unknown(
                    "Worker response exceeds the protocol frame limit",
                ));
            }
            frame.extend_from_slice(&bytes[..count]);
            self.output.consume(count);
            if frame.last() == Some(&b'\n') {
                break;
            }
        }
        let response: ChannelResponse = serde_json::from_slice(&frame).map_err(|error| {
            CallError::unknown(format!("Worker returned an invalid response: {error}"))
        })?;
        if response.protocol_version != PRIVATE_WORKER_PROTOCOL_VERSION
            || response.request_id != request_id
            || response.generation != self.generation
        {
            return Err(CallError::unknown("Worker response identity mismatch"));
        }
        Ok(response)
    }
}
struct Actor {
    config: WorkerConfig,
    receiver: mpsc::UnboundedReceiver<Command>,
    status: watch::Sender<WorkerStatus>,
    live: Option<LiveProcess>,
    sessions: HashMap<String, SessionBinding>,
    queues: HashMap<String, VecDeque<QueuedCall>>,
    ready: VecDeque<String>,
    last_activity: Instant,
    permissions_seen: Option<permissions::PermissionStatus>,
    cleanup_pending: bool,
    shutdown_requested: CancellationToken,
    last_served: Option<String>,
}
impl Actor {
    fn new(
        config: WorkerConfig,
        receiver: mpsc::UnboundedReceiver<Command>,
        status: watch::Sender<WorkerStatus>,
        shutdown_requested: CancellationToken,
    ) -> Self {
        Self {
            config,
            receiver,
            status,
            live: None,
            sessions: HashMap::new(),
            queues: HashMap::new(),
            ready: VecDeque::new(),
            last_activity: Instant::now(),
            permissions_seen: None,
            cleanup_pending: false,
            shutdown_requested,
            last_served: None,
        }
    }
    fn publish(&self, phase: &str, error: Option<String>) {
        self.status.send_modify(|status| {
            status.phase = phase.into();
            status.process_id = self.live.as_ref().and_then(|live| live.owner.id());
            status.generation = self.live.as_ref().map(|live| live.generation.clone());
            status.running = self.live.is_some();
            if self.live.is_none() {
                status.backend = None;
                status.atspi = None;
            }
            status.bound_sessions = self.sessions.len();
            status.queued = self.queues.values().map(VecDeque::len).sum();
            if error.is_some() {
                status.last_error = error;
            }
        });
    }
    async fn run(mut self) {
        loop {
            for _ in 0..64 {
                match self.receiver.try_recv() {
                    Ok(command) => {
                        if self.command(command).await {
                            return;
                        }
                    }
                    Err(_) => break,
                }
            }
            if self.ready.len() > 1 && self.ready.front() == self.last_served.as_ref() {
                let first = self.ready.pop_front().unwrap();
                self.ready.push_back(first);
            }
            if let Some(session) = self.ready.pop_front() {
                let request = self.queues.get_mut(&session).and_then(VecDeque::pop_front);
                if self
                    .queues
                    .get(&session)
                    .is_some_and(|queue| !queue.is_empty())
                {
                    self.ready.push_back(session.clone());
                } else {
                    self.queues.remove(&session);
                }
                if let Some(request) = request {
                    if request.state.load(Ordering::Acquire) == CANCELLED
                        || request.control.is_cancelled()
                    {
                        let _ = request.reply.send(Err(CallError::cancelled()));
                        continue;
                    }
                    let result = self.perform(&request).await;
                    self.last_served = Some(session);
                    request.state.store(FINISHED, Ordering::Release);
                    let _ = request.reply.send(result);
                    self.last_activity = Instant::now();
                    self.publish(
                        if self.cleanup_pending {
                            "cleanup_pending"
                        } else if self.live.is_some() {
                            "ready"
                        } else {
                            "stopped"
                        },
                        None,
                    );
                }
                continue;
            }
            let retirement = self.last_activity + self.config.idle_timeout;
            tokio::select! {
                command=self.receiver.recv()=>match command{
                    Some(command)=>if self.command(command).await{return;},
                    None=>{
                        // The exact owner remains retained on a cleanup failure.
                        loop {match self.retire().await{Ok(())=>return,Err(error)=>{self.publish("cleanup_pending",Some(error.to_string()));tokio::time::sleep(Duration::from_secs(1)).await;}}}
                    }
                },
                _=tokio::time::sleep_until(retirement),if self.live.is_some() && self.sessions.is_empty()=>{
                    if let Err(error)=self.retire().await{self.publish("cleanup_pending",Some(error.to_string()));}
                    self.last_activity=Instant::now();
                }
                _=async {self.live.as_mut().unwrap().owner.child_mut().wait().await},if self.live.is_some()=>{
                    let error="Computer worker exited; no action will be replayed".to_owned();
                    let _=self.retire().await;self.publish(if self.cleanup_pending{"cleanup_pending"}else{"failed"},Some(error));
                }
            }
        }
    }
    async fn command(&mut self, command: Command) -> bool {
        match command {
            Command::Call(request) => {
                let session = request.session.clone();
                let queue = self.queues.entry(session.clone()).or_default();
                if queue.is_empty() {
                    self.ready.push_back(session);
                }
                queue.push_back(request);
            }
            #[cfg(any(target_os = "macos", test))]
            Command::RequestScreenCapture(reply) => {
                // A dropped HTTP observer has no authority to start a later
                // prompt. An already dispatched probe remains with this owner.
                if !reply.is_closed() {
                    let result = self.request_screen_capture(&reply).await;
                    self.last_activity = Instant::now();
                    self.publish(if self.cleanup_pending { "cleanup_pending" }
                        else if self.live.is_some() { "ready" } else { "stopped" },
                        result.as_ref().err().map(ToString::to_string));
                    let _ = reply.send(result);
                }
            }
            Command::Close(session, reply) => {
                if let Some(queue) = self.queues.remove(&session) {
                    for request in queue {
                        request.state.store(CANCELLED, Ordering::Release);
                        let _ = request.reply.send(Err(CallError::cancelled()));
                    }
                }
                self.ready.retain(|id| id != &session);
                let result = if let Some(binding) = self.sessions.remove(&session) {
                    if let Some(live) = self.live.as_mut() {
                        match tokio::time::timeout(
                            self.config.shutdown_timeout,
                            live.exchange("close_session", None, None, Some(&binding.handle)),
                        )
                        .await
                        {
                            Ok(Ok(response)) if response.ok => Ok(()),
                            _ => self.retire().await,
                        }
                    } else {
                        Ok(())
                    }
                } else if self.cleanup_pending {
                    self.retire().await
                } else {
                    Ok(())
                };
                self.last_activity = Instant::now();
                self.publish(
                    if self.cleanup_pending {
                        "cleanup_pending"
                    } else if self.live.is_some() {
                        "ready"
                    } else {
                        "stopped"
                    },
                    None,
                );
                let _ = reply.send(result);
            }
            Command::Shutdown(reply) => {
                for (_, queue) in self.queues.drain() {
                    for request in queue {
                        request.state.store(CANCELLED, Ordering::Release);
                        let _ = request.reply.send(Err(CallError::cancelled()));
                    }
                }
                self.ready.clear();
                let result = self.retire().await;
                let done = result.is_ok();
                let _ = reply.send(result);
                if done {
                    return true;
                }
            }
        }
        false
    }
    #[cfg(any(target_os = "macos", test))]
    async fn request_screen_capture(&mut self, reply: &oneshot::Sender<Result<(), CallError>>) -> Result<(), CallError> {
        if self.shutdown_requested.is_cancelled() || reply.is_closed() {
            return Err(CallError::cancelled());
        }
        #[cfg(test)]
        let granted = self.config.screen_capture_granted.or_else(permissions::screen_recording_granted);
        #[cfg(not(test))]
        let granted = permissions::screen_recording_granted();
        if granted != Some(true) {
            return Err(CallError::new(CallErrorKind::BeforeDispatch, "SYSTEM_PERMISSION_REQUIRED",
                "Screen Recording is not granted; no verification capture was dispatched"));
        }
        let already_running = self.live.is_some();
        self.start().await?;
        if self.shutdown_requested.is_cancelled() || reply.is_closed() {
            if !already_running { self.retire().await?; }
            return Err(CallError::cancelled());
        }
        self.publish("busy", None);
        let budget = SCREEN_CAPTURE_REQUEST_TIMEOUT;
        #[cfg(test)]
        let budget = self.config.screen_capture_timeout.unwrap_or(budget);
        let result = {
            let response = self.live.as_mut().unwrap().exchange("request_screen_capture", None, None, None);
            tokio::pin!(response);
            tokio::select! {
                result = tokio::time::timeout(budget, &mut response) => result,
                _ = self.shutdown_requested.cancelled() => tokio::time::timeout(self.config.shutdown_timeout, &mut response).await,
            }
        };
        let response = match result {
            Ok(Ok(response)) => response,
            other => {
                let message = match other {
                    Ok(Err(error)) => error.to_string(),
                    Err(_) => "Explicit screen capture verification exceeded its native probe deadline".into(),
                    _ => unreachable!(),
                };
                self.live.as_mut().unwrap().input.take();
                self.retire().await?;
                return Err(CallError::unavailable(message));
            }
        };
        let probe = response.result.clone().and_then(|value| serde_json::from_value::<ScreenCaptureProbeResponse>(value).ok());
        if probe.as_ref().is_some_and(|probe| probe.status == ScreenCaptureProbeStatus::TimedOut) {
            // Dropping a spawn_blocking future does not end the native call.
            // Skip graceful IPC and reap this exact worker before replying.
            self.live.as_mut().unwrap().input.take();
        }
        // Explicit consent can change while AX/CG Boolean grants stay true.
        // Retire this exact generation even then; no old snapshot or negative
        // consent cache may survive into a subsequent Agent operation.
        self.retire().await?;
        if self.shutdown_requested.is_cancelled() {
            return Err(CallError::new(CallErrorKind::Cancelled, "COMPUTER_SCREEN_CAPTURE_REQUEST_CANCELLED",
                "Application shutdown retired the screen capture verification worker"));
        }
        if !response.ok || response.completion != ActionCompletion::Completed
            || !probe.as_ref().is_some_and(|probe| probe.status == ScreenCaptureProbeStatus::Ready) {
            let diagnostic = match probe.map(|probe| probe.status) {
                Some(ScreenCaptureProbeStatus::TimedOut) => "The native verification capture timed out; its worker was retired",
                Some(ScreenCaptureProbeStatus::ProbeFailed) => "The native verification capture could not complete",
                Some(ScreenCaptureProbeStatus::Unavailable) => "The driver could not capture a verification frame; the native permission state remains unchanged",
                _ => "The worker returned an invalid screen capture verification response",
            };
            return Err(CallError::new(CallErrorKind::Unavailable, "COMPUTER_SCREEN_CAPTURE_REQUEST_FAILED",
                response.error.unwrap_or_else(|| diagnostic.into())));
        }
        Ok(())
    }
    async fn start(&mut self) -> Result<(), CallError> {
        if self.cleanup_pending {
            self.retire().await?;
        }
        if self.live.is_some() {
            return Ok(());
        }
        self.publish("starting", None);
        let home = self.config.data_dir.join("computer-driver");
        tokio::fs::create_dir_all(home.join("tmp"))
            .await
            .map_err(|error| CallError::unavailable(error.to_string()))?;
        let generation = uuid::Uuid::new_v4().to_string();
        let mut builder = ChildProcessBuilder::new(&self.config.executable);
        builder.env_clear();
        for key in [
            "PATH",
            "SYSTEMROOT",
            "WINDIR",
            "COMSPEC",
            "PATHEXT",
            "LANG",
            "LC_ALL",
            "LC_CTYPE",
            "USER",
            "USERNAME",
            "LOGNAME",
            "DISPLAY",
            "WAYLAND_DISPLAY",
            "XAUTHORITY",
            "DBUS_SESSION_BUS_ADDRESS",
            "XDG_RUNTIME_DIR",
            "XDG_SESSION_TYPE",
            "XDG_CURRENT_DESKTOP",
            "XDG_SESSION_DESKTOP",
            "DESKTOP_SESSION",
        ] {
            if let Some(value) = std::env::var_os(key) {
                builder.env(key, value);
            }
        }
        // Resolve the inherited login user's authority before HOME is isolated.
        #[cfg(target_os = "linux")]
        if std::env::var_os("XAUTHORITY").is_none() {
            if let Some(user_home) = std::env::var_os("HOME") {
                let authority = PathBuf::from(user_home).join(".Xauthority");
                if authority.is_file() {
                    builder.env("XAUTHORITY", authority);
                }
            }
        }
        for key in [
            "HOME",
            "USERPROFILE",
            "APPDATA",
            "LOCALAPPDATA",
            "XDG_CONFIG_HOME",
            "XDG_STATE_HOME",
            "XDG_DATA_HOME",
            "XDG_CACHE_HOME",
            "CUA_DRIVER_RS_HOME",
        ] {
            builder.env(key, &home);
        }
        for key in ["TMPDIR", "TMP", "TEMP"] {
            builder.env(key, home.join("tmp"));
        }
        builder
            .env("CUA_DRIVER_CURSOR_THEME_DIR", home.join("cursor-themes"))
            .env("CUA_DRIVER_RS_ENABLE_WAYLAND", "0");
        for key in [
            "CUA_DRIVER_RS_WINDOW_CHANGE_WAIT_MS",
            "CUA_DRIVER_RS_KEY_INTERVAL_MS",
            "CUA_DRIVER_RS_KEY_DELAY_MS",
        ] {
            if let Some(value) = std::env::var_os(key) {
                builder.env(key, value);
            }
        }
        #[cfg(test)]
        builder.args(&self.config.prefix_args);
        builder
            .args(["__private-worker", "--generation", &generation])
            .current_dir(&home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut owner = builder.spawn_managed().map_err(|error| {
            CallError::unavailable(format!("Cannot start Computer worker: {error}"))
        })?;
        let input = owner
            .child_mut()
            .stdin
            .take()
            .ok_or_else(|| CallError::unavailable("Worker stdin unavailable"))?;
        let output = owner
            .child_mut()
            .stdout
            .take()
            .ok_or_else(|| CallError::unavailable("Worker stdout unavailable"))?;
        let mut stderr = owner
            .child_mut()
            .stderr
            .take()
            .ok_or_else(|| CallError::unavailable("Worker stderr unavailable"))?;
        let stderr = tokio::spawn(async move {
            let mut buffer = [0u8; 4096];
            loop {
                if stderr.read(&mut buffer).await? == 0 {
                    return Ok(());
                }
            }
        });
        self.live = Some(LiveProcess {
            owner,
            input: Some(input),
            output: BufReader::new(output),
            generation,
            next_id: 0,
            stderr: Some(stderr),
            stderr_error: None,
        });
        let initialization = serde_json::to_value(WorkerInitialization::standard(
            self.config.host_bundle_id.clone(),
        ))
        .expect("worker initialization serializes");
        let response = tokio::select! {
            response=tokio::time::timeout(self.config.startup_timeout,self.live.as_mut().unwrap().exchange("initialize",None,Some(initialization),None))=>response,
            _=self.shutdown_requested.cancelled()=>{
                self.live.as_mut().unwrap().input.take();let _=self.retire().await;return Err(CallError::cancelled());
            }
        };
        match response {
            Ok(Ok(response)) if response.ok => {
                self.status.send_modify(|status| {
                    let result = response.result.as_ref();
                    status.metadata = result.and_then(|result| result.get("metadata").cloned());
                    let diagnostics = result.and_then(|result| result.get("diagnostics"));
                    status.backend = diagnostics
                        .and_then(|diagnostics| diagnostics.get("backend"))
                        .and_then(Value::as_str)
                        .filter(|backend| !backend.is_empty())
                        .map(str::to_owned);
                    status.atspi = diagnostics
                        .and_then(|diagnostics| diagnostics.get("atspi"))
                        .and_then(Value::as_bool);
                    status.worker_version = status
                        .metadata
                        .as_ref()
                        .and_then(|metadata| metadata.get("driver_version"))
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    status.last_error = None;
                });
                self.publish("ready", None);
                Ok(())
            }
            other => {
                let message = match other {
                    Ok(Ok(response)) => response
                        .error
                        .unwrap_or_else(|| "Worker initialization was refused".into()),
                    Ok(Err(error)) => error.to_string(),
                    Err(_) => "Worker startup timed out".into(),
                };
                let cleanup = self.retire().await;
                self.publish("failed", Some(message.clone()));
                cleanup?;
                Err(CallError::unavailable(message))
            }
        }
    }
    async fn bind(&mut self, session: &str, name: &str) -> Result<String, CallError> {
        if self.sessions.get(session).is_some_and(|binding| {
            binding.bound.elapsed() >= Duration::from_secs(86_400)
                || binding.last_used.elapsed() >= Duration::from_secs(3_600)
        }) {
            if let Some(binding) = self.sessions.remove(session) {
                let _ = tokio::time::timeout(
                    self.config.startup_timeout,
                    self.live.as_mut().unwrap().exchange(
                        "close_session",
                        None,
                        None,
                        Some(&binding.handle),
                    ),
                )
                .await;
            }
        }
        if let Some(binding) = self.sessions.get_mut(session) {
            binding.last_used = Instant::now();
            return Ok(binding.handle.clone());
        }
        let options = serde_json::to_value(TrustedSessionOptions::standard(name.into())).unwrap();
        let response = tokio::select! {
            result=tokio::time::timeout(self.config.startup_timeout,self.live.as_mut().unwrap().exchange("bind_session",None,Some(options),None))=>result.map_err(|_|CallError::unavailable("Worker session binding timed out"))??,
            _=self.shutdown_requested.cancelled()=>return Err(CallError::cancelled()),
        };
        if !response.ok {
            return Err(CallError::new(
                CallErrorKind::BeforeDispatch,
                response
                    .error_code
                    .unwrap_or_else(|| "COMPUTER_SESSION_REJECTED".into()),
                response.error.unwrap_or_default(),
            ));
        }
        let handle = response
            .result
            .and_then(|result| {
                result
                    .get("session_handle")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .ok_or_else(|| CallError::unavailable("Worker omitted its session handle"))?;
        self.sessions.insert(
            session.into(),
            SessionBinding {
                handle: handle.clone(),
                bound: Instant::now(),
                last_used: Instant::now(),
            },
        );
        Ok(handle)
    }
    async fn perform(&mut self, request: &QueuedCall) -> Result<DriverResult, CallError> {
        if self.shutdown_requested.is_cancelled() {
            return Err(CallError::cancelled());
        }
        let current = permissions::permission_status();
        if self
            .permissions_seen
            .is_some_and(|previous| previous != current)
        {
            self.retire().await?;
        }
        self.permissions_seen = Some(current);
        if let Some(carrier) = permissions::required_carrier(
            &request.prepared.tool.action_id,
            &request.prepared.arguments,
            current,
        ) {
            return Err(CallError {
                kind: CallErrorKind::BeforeDispatch,
                code: "SYSTEM_PERMISSION_REQUIRED".into(),
                message: "The required system permission is missing".into(),
                permission_required: Some(carrier),
            });
        }
        self.start().await?;
        let handle = match self.bind(&request.session, &request.name).await {
            Ok(handle) => handle,
            Err(mut error) => {
                self.live.as_mut().unwrap().input.take();
                let _ = self.retire().await;
                // No call request was written: a lost bind response is not an
                // unknown desktop effect.
                if error.kind == CallErrorKind::Unknown {
                    error.kind = CallErrorKind::Unavailable;
                }
                return Err(error);
            }
        };
        if request.control.is_cancelled()
            || self.shutdown_requested.is_cancelled()
            || request
                .state
                .compare_exchange(QUEUED, DISPATCHED, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
        {
            return Err(CallError::cancelled());
        }
        if request.control.is_cancelled() {
            return Err(CallError::cancelled());
        }
        self.publish("busy", None);
        let budget = request.prepared.budget;
        #[cfg(test)]
        let budget = self.config.call_timeout.unwrap_or(budget);
        let deadline = Instant::now() + budget;
        let status = self.status.clone();
        let result = {
            let live = self.live.as_mut().unwrap();
            let response = live.exchange(
                "call",
                Some(&request.prepared.tool.driver_tool),
                Some(surface::driver_arguments(&request.prepared)),
                Some(&handle),
            );
            tokio::pin!(response);
            let mut stopping = false;
            loop {
                tokio::select! {
                    result=tokio::time::timeout_at(deadline,&mut response)=>break result,
                    _=request.control.cancelled(),if !stopping=>{stopping=true;status.send_modify(|status|status.phase="waiting_for_current_operation".into());},
                    _=self.shutdown_requested.cancelled()=>{
                        let shutdown_deadline=deadline.min(Instant::now()+self.config.shutdown_timeout);
                        break tokio::time::timeout_at(shutdown_deadline,&mut response).await;
                    }
                }
            }
        };
        let response = match result {
            Ok(Ok(response)) => response,
            other => {
                let error = match other {
                    Ok(Err(error)) => error,
                    Err(_) => CallError::unknown(
                        "The worker exceeded the operation budget; the action must not be replayed",
                    ),
                    _ => unreachable!(),
                };
                self.publish("stopping", Some(error.to_string()));
                self.live.as_mut().unwrap().input.take();
                let _ = self.retire().await;
                return Err(error);
            }
        };
        if !response.ok {
            let kind = if response.completion == ActionCompletion::NotStarted {
                CallErrorKind::BeforeDispatch
            } else {
                CallErrorKind::Unknown
            };
            return Err(CallError::new(
                kind,
                response
                    .error_code
                    .unwrap_or_else(|| "COMPUTER_WORKER_FAILED".into()),
                response.error.unwrap_or_default(),
            ));
        }
        let generation = self.live.as_ref().unwrap().generation.clone();
        Ok(DriverResult {
            result: response.result.unwrap_or(Value::Null),
            completion: response.completion,
            generation,
            foreground: request.prepared.foreground,
        })
    }
    async fn retire(&mut self) -> Result<(), CallError> {
        let Some(live) = self.live.as_mut() else {
            self.sessions.clear();
            self.publish("stopped", None);
            return Ok(());
        };
        self.cleanup_pending = true;
        self.status
            .send_modify(|status| status.phase = "stopping".into());
        if live.input.is_some() {
            let _ = tokio::time::timeout(
                self.config.shutdown_timeout,
                live.exchange("shutdown", None, None, None),
            )
            .await;
            live.input.take();
            let _ =
                tokio::time::timeout(self.config.shutdown_timeout, live.owner.child_mut().wait())
                    .await;
        }
        live.owner.shutdown().await.map_err(|error| {
            CallError::unavailable(format!("Computer worker cleanup remains unproven: {error}"))
        })?;
        // The Linux native guardian and Windows one-shot user-app launcher
        // deliberately live outside the worker's process tree. Both retain a
        // non-inheritable stderr lifetime. Direct PID/Job/group exit alone is
        // insufficient; EOF never settles an unknown action's business effect.
        if let Some(error) = &live.stderr_error {
            return Err(CallError::unavailable(error.clone()));
        }
        if let Some(stderr) = live.stderr.as_mut() {
            match tokio::time::timeout(self.config.shutdown_timeout, stderr).await {
                Ok(Ok(Ok(()))) => {
                    live.stderr.take();
                }
                Ok(error) => {
                    let error = format!("Worker stderr cleanup could not be proven: {error:?}");
                    live.stderr.take();
                    live.stderr_error = Some(error.clone());
                    return Err(CallError::unavailable(error));
                }
                Err(_) => {
                    return Err(CallError::unavailable(
                        "Worker exited, but native cleanup still owns stderr; waiting for its EOF",
                    ));
                }
            }
        }
        self.live.take();
        self.cleanup_pending = false;
        self.sessions.clear();
        self.publish("stopped", None);
        Ok(())
    }
}
