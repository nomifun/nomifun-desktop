//! One voice operation path for system shortcuts, subtitle controls and optional hardware.
mod asr_preparation;
mod audio;
mod external_input;
mod key_hold;
mod lifecycle;
mod models;
mod microphone_check;
mod platform;
pub(crate) mod shortcuts;
mod update_pause;
use key_hold::KeyHold;
use microphone_check::{MicrophoneCheck, MicrophoneTestState};
use models::{
    DesktopModels, KeyboardModelSelection, StreamingAsrConfig, StreamingAsrEvent,
    StreamingAsrInput, StreamingAsrSession,
};
use nomifun_keyboard::{
    document::{
        AsrMode, Document, Effect, Identity, Intent, ModelSelection, Phase, Settings, TextSelection,
    },
    service::{Commands, Control, DeviceStatus, Event, Transport},
};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tauri::{Emitter, Manager};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
pub(crate) use update_pause::UpdatePause;
use update_pause::{CaptureSafety, Suspension, UpdateAdmission};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ServiceState {
    running: bool,
    ready: bool,
    detail: Option<String>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AudioState {
    state: String,
    detail: Option<String>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct KeyboardState {
    sequence: u64,
    settings: Settings,
    #[serde(flatten)]
    document: Document,
    device: DeviceStatus,
    service: ServiceState,
    audio_state: AudioState,
    capture_preparing: bool,
    microphone_test: MicrophoneTestState,
    cancel_generation: u64,
    edit_session_id: Option<String>,
    shortcut_state: ShortcutState,
    external_keyboard: ExternalKeyboardState,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExternalKeyboardState { state: String, detail: Option<String> }
impl Default for ExternalKeyboardState {
    fn default() -> Self { Self { state: "disabled".into(), detail: None } }
}
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct ShortcutState { registered: usize, detail: Option<String> }
#[derive(Default, Serialize, Deserialize)]
struct Saved {
    settings: Settings,
}
impl Saved {
    fn validate(&self) -> Result<(), String> {
        self.settings.validate()
    }
}
pub(crate) struct KeyboardRuntime {
    tx: mpsc::Sender<Message>,
    urgent: mpsc::UnboundedSender<Message>,
    snapshot: Arc<Mutex<KeyboardState>>,
    models: DesktopModels,
    stop_flag: Arc<AtomicBool>,
    update_admission: Arc<UpdateAdmission>,
    capture_safety: Arc<CaptureSafety>,
    root: PathBuf,
    transport: Arc<Mutex<Option<OwnedTransport>>>,
    external: external_input::Controller,
}
/// Both joins belong to the GUI runtime. The event bridge never reconnects or
/// starts another process, and shutdown holds this owner until both are gone.
struct OwnedTransport {
    transport: Transport,
    bridge: Option<std::thread::JoinHandle<()>>,
}
impl OwnedTransport {
    fn shutdown(&mut self) -> Result<(), String> {
        let result = self.transport.shutdown().map_err(|error| error.to_string());
        let bridge = self.bridge.take().map(|thread| thread.join());
        if bridge.is_some_and(|result| result.is_err()) {
            return Err("键盘事件线程异常退出".into());
        }
        result
    }
}
impl Drop for OwnedTransport {
    fn drop(&mut self) { let _ = self.shutdown(); }
}
enum Message {
    Shortcut(shortcuts::Action, bool),
    #[cfg(feature = "native-keyboard-validation")]
    ValidationShortcut(shortcuts::Action, bool, oneshot::Sender<Result<KeyboardState, String>>),
    #[cfg(feature = "native-keyboard-validation")]
    ValidationButtonClick(String, oneshot::Sender<Result<KeyboardState, String>>),
    #[cfg(feature = "native-keyboard-validation")]
    ValidationHardware {
        pressed: Option<bool>,
        reset_asr: bool,
        reply: oneshot::Sender<Result<KeyboardState, String>>,
    },
    #[cfg(feature = "native-keyboard-validation")]
    ValidationHoldStop(bool, oneshot::Sender<Result<(), String>>),
    #[cfg(feature = "native-keyboard-validation")]
    ValidationLateCaptureCallback(oneshot::Sender<Result<(), String>>),
    Connected(u64, Commands),
    Disconnected(u64, String),
    Device(u64, Event),
    ExternalSettled(u64, Result<bool, String>),
    LifecycleFailure(String),
    ShortcutCleanup(Result<usize, String>),
    #[cfg(feature = "native-keyboard-validation")]
    ValidationExternalEvent(u64, Event, oneshot::Sender<Result<KeyboardState, String>>),
    #[cfg(feature = "native-keyboard-validation")]
    ValidationExternalTerminal(u64, String, oneshot::Sender<Result<KeyboardState, String>>),
    Suspend(bool),
    Tick,
    MicrophoneTick(Identity),
    #[cfg(feature = "native-keyboard-validation")]
    SyntheticInputReady(Identity),
    Action {
        action: String,
        document: Option<String>,
        revision: Option<u64>,
        cancel_generation: Option<u64>,
        text: Option<String>,
        edit_id: Option<String>,
        selection: Option<TextSelection>,
        test_id: Option<String>,
        audio_device_id: Option<String>,
        reply: oneshot::Sender<Result<KeyboardState, String>>,
    },
    Settings(Settings, oneshot::Sender<Result<KeyboardState, String>>),
    AsrReady(u64, Result<StreamingAsrSession, String>),
    Asr(u64, StreamingAsrEvent),
    Shutdown(oneshot::Sender<Result<(), String>>),
    PrepareUpdate(oneshot::Sender<Result<(), String>>),
    ResumeAfterUpdate(Option<oneshot::Sender<()>>),
    CaptureReady(
        Identity,
        Result<audio::CaptureHandle, audio::CaptureStartFailure>,
    ),
    CaptureStopped(Identity, audio::StopReceipt),
    CaptureError(Identity, String),
    PlatformAsrFinished(Identity, Result<String, String>),
    Candidate(Identity, String),
    Polished(Identity, Result<String, String>),
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum InputSource { #[default] Ui, Hardware, Shortcut }
impl InputSource { fn hardware(self) -> bool { self == Self::Hardware } }
struct PendingStart {
    source: InputSource,
    activation_generation: u64,
    document_id: Option<String>,
    revision: u64,
    started_at: Instant,
}
enum EditorOperation {
    Intent(Intent, InputSource),
    Capture {
        source: InputSource,
        generation: u64,
    },
}
impl EditorOperation {
    fn hardware(&self) -> bool {
        match self {
            Self::Intent(_, source) | Self::Capture { source, .. } => source.hardware(),
        }
    }
}
struct EditorLease {
    id: String,
    document: Option<String>,
    pending: Option<EditorOperation>,
    requested_at: Option<Instant>,
}
struct TextCursor {
    document: String,
    revision: u64,
    selection: TextSelection,
}
struct Actor {
    app: tauri::AppHandle,
    models: DesktopModels,
    root: PathBuf,
    saved: Saved,
    state: KeyboardState,
    snapshot: Arc<Mutex<KeyboardState>>,
    tx: mpsc::Sender<Message>,
    urgent: mpsc::UnboundedSender<Message>,
    commands: Option<Commands>,
    stop_flag: Arc<AtomicBool>,
    suspended: bool,
    suspension: Suspension,
    activation_generation: u64,
    hardware_armed: bool,
    policy_sent: bool,
    key_hold: KeyHold,
    shortcut_hold: KeyHold,
    shortcut_generation: u64,
    shortcuts: Option<shortcuts::Controller>,
    capture_safety: Arc<CaptureSafety>,
    external: external_input::Controller,
    asr: Option<StreamingAsrInput>,
    asr_generation: u64,
    asr_document: Option<Identity>,
    buffered_capture: Option<(Identity, audio::BufferedCapture)>,
    platform_asr: Option<(Identity, CancellationToken)>,
    opening: bool,
    opening_abort: Option<tokio::task::AbortHandle>,
    opening_since: Option<Instant>,
    retry_at: Instant,
    pending_start: Option<PendingStart>,
    editor: Option<EditorLease>,
    text_cursor: Option<TextCursor>,
    dictation_cursor_revision: Option<(String, u64)>,
    microphone_check: Option<MicrophoneCheck>,
    capture: Option<audio::CaptureHandle>,
    capture_start_gate: Option<audio::CaptureStartGate>,
    capture_canceller: Option<audio::CaptureCanceller>,
    capture_id: Option<Identity>,
    capture_input: Option<(Identity, audio::CaptureInput)>,
    last_capture_stats: Option<(Identity, audio::AudioStats)>,
    capture_starting: bool,
    startup_trace: Option<(&'static str, u64, u64, bool)>,
    stop_requested: Option<bool>,
    recording_at: Option<Instant>,
    drain_at: Option<Instant>,
    polish: Option<CancellationToken>,
    close_pending: bool,
    operation_source: InputSource,
    pending_origin: Option<InputSource>,
    shutdown_replies: Vec<oneshot::Sender<Result<(), String>>>,
    display_sent: Option<String>,
    production: bool,
    #[cfg(feature = "native-keyboard-validation")]
    synthetic_input: bool,
    #[cfg(feature = "native-keyboard-validation")]
    synthetic_stop: Option<Arc<std::sync::atomic::AtomicU8>>,
    #[cfg(feature = "native-keyboard-validation")]
    synthetic_stop_hold: Arc<AtomicBool>,
}
fn selection(m: &ModelSelection) -> KeyboardModelSelection {
    KeyboardModelSelection {
        provider_id: m.provider_id.clone(),
        model: m.model.clone(),
    }
}
pub(crate) fn start(
    app: tauri::AppHandle,
    server: Arc<nomifun_app::DesktopServer>,
    root: PathBuf,
    host_stop: Arc<AtomicBool>,
) -> anyhow::Result<()> {
    let models = DesktopModels::new(
        server
            .keyboard_model_service()
            .ok_or_else(|| anyhow::anyhow!("模型服务未就绪"))?,
    );
    start_core(app, models, root, true, host_stop)?;
    Ok(())
}
fn start_core(
    app: tauri::AppHandle,
    models: DesktopModels,
    root: PathBuf,
    production: bool,
    host_stop: Arc<AtomicBool>,
) -> anyhow::Result<()> {
    anyhow::ensure!(app.try_state::<KeyboardRuntime>().is_none(), "语音键盘已经初始化");
    let path = root.join("keyboard-settings.json");
    let (saved, error) = if path.exists() {
        match std::fs::read(path)
            .map_err(|e| e.to_string())
            .and_then(|bytes| serde_json::from_slice::<Saved>(&bytes).map_err(|e| e.to_string()))
            .and_then(|s| {
                s.validate()?;
                Ok(s)
            }) {
            Ok(s) => (s, None),
            Err(e) => (
                Saved::default(),
                Some(format!("设置读取失败，采集保持关闭：{e}")),
            ),
        }
    } else {
        (Saved::default(), None)
    };
    let mut saved = saved;
    if error.is_some() {
        saved.settings.enabled = false;
        saved.settings.external_keyboard_enabled = false;
    }
    let mut state = KeyboardState {
        sequence: 0,
        settings: saved.settings.clone(),
        document: Document::default(),
        device: DeviceStatus::default(),
        service: ServiceState {
            running: false,
            ready: false,
            detail: None,
        },
        audio_state: AudioState {
            state: "stopped".into(),
            detail: None,
        },
        capture_preparing: false,
        microphone_test: MicrophoneTestState::default(),
        cancel_generation: 0,
        edit_session_id: None,
        shortcut_state: ShortcutState::default(),
        external_keyboard: ExternalKeyboardState::default(),
    };
    state.document.error = error;
    let snapshot = Arc::new(Mutex::new(state.clone()));
    let (tx, rx) = mpsc::channel(64);
    // Native stop/lifecycle messages must neither compete with streaming deltas
    // nor disappear when the bounded data queue is full. Their producers are
    // trusted native callbacks; only the actor consumes this priority channel.
    let (urgent, urgent_rx) = mpsc::unbounded_channel();
    let shortcuts = if production {
        match app.plugin(shortcuts::plugin()) {
            Ok(()) => {
                let edges = urgent.clone();
                let mut controller = shortcuts::Controller::new(app.clone(), move |action, pressed| {
                    let _ = edges.send(Message::Shortcut(action, pressed));
                });
                match controller.reconfigure(&saved.settings) {
                    Ok(count) => state.shortcut_state.registered = count,
                    Err(error) => state.shortcut_state.detail = Some(error),
                }
                Some(controller)
            }
            Err(error) => { state.shortcut_state.detail = Some(format!("系统快捷键初始化失败：{error}")); None }
        }
    } else { None };
    *snapshot.lock().unwrap() = state.clone();
    let stop_flag = host_stop;
    let update_admission = Arc::new(UpdateAdmission::default());
    let capture_safety = Arc::new(CaptureSafety::default());
    let transport = Arc::new(Mutex::new(None));
    let external = external_input::Controller::new(transport.clone(), stop_flag.clone(), root.clone(), production, tx.clone(), urgent.clone());
    app.manage(KeyboardRuntime {
        tx: tx.clone(),
        urgent: urgent.clone(),
        snapshot: snapshot.clone(),
        models: models.clone(),
        stop_flag: stop_flag.clone(),
        update_admission: update_admission.clone(),
        capture_safety: capture_safety.clone(),
        root: root.clone(),
        transport: transport.clone(),
        external: external.clone(),
    });
    #[cfg(feature = "native-keyboard-validation")]
    let synthetic_input = !production && root.join(".synthetic-audio-fixture").exists();
    let actor = Actor {
        app: app.clone(),
        models,
        root,
        saved,
        state,
        snapshot,
        tx: tx.clone(),
        urgent: urgent.clone(),
        commands: None,
        stop_flag: stop_flag.clone(),
        suspended: true,
        suspension: Suspension::initial(),
        activation_generation: 1,
        hardware_armed: false,
        policy_sent: false,
        key_hold: KeyHold::default(),
        shortcut_hold: KeyHold::default(),
        shortcut_generation: 1,
        shortcuts,
        capture_safety,
        external: external.clone(),
        asr: None,
        asr_generation: 0,
        asr_document: None,
        buffered_capture: None,
        platform_asr: None,
        opening: false,
        opening_abort: None,
        opening_since: None,
        retry_at: Instant::now(),
        pending_start: None,
        editor: None,
        text_cursor: None,
        dictation_cursor_revision: None,
        microphone_check: None,
        capture: None,
        capture_start_gate: None,
        capture_canceller: None,
        capture_id: None,
        capture_input: None,
        last_capture_stats: None,
        capture_starting: false,
        startup_trace: None,
        stop_requested: None,
        recording_at: None,
        drain_at: None,
        polish: None,
        close_pending: false,
        operation_source: InputSource::Ui,
        pending_origin: None,
        shutdown_replies: Vec::new(),
        display_sent: None,
        production,
        #[cfg(feature = "native-keyboard-validation")]
        synthetic_input,
        #[cfg(feature = "native-keyboard-validation")]
        synthetic_stop: None,
        #[cfg(feature = "native-keyboard-validation")]
        synthetic_stop_hold: Arc::new(AtomicBool::new(false)),
    };
    tauri::async_runtime::spawn(actor.run(rx, urgent_rx));
    if production {
        let observer = urgent.clone();
        match lifecycle::install(&app, move |paused| {
            let _ = observer.send(Message::Suspend(paused));
        }) {
            Ok(guard) => {
                app.manage(guard);
            }
            Err(e) => {
                let _ = urgent.send(Message::LifecycleFailure(format!("系统会话监听失败：{e}")));
            }
        }
    } else {
        let _ = urgent.send(Message::Suspend(false));
    }
    let tick = tx.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            if tick.send(Message::Tick).await.is_err() {
                break;
            }
        }
    });
    Ok(())
}
/// Native conformance examples reuse the production actor, model resolution,
/// OS capture and window. They own only a marked disposable data root, and do
/// not register login items or claim the user's USB device.
#[cfg(feature = "native-keyboard-validation")]
pub(crate) fn validation_start(
    app: tauri::AppHandle,
    invoke: Arc<nomifun_model_invoke::ModelInvokeService>,
    root: PathBuf,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        std::fs::read_to_string(root.join(".native-keyboard-validation"))
            .ok()
            .as_deref()
            == Some("isolated-native-keyboard-v1"),
        "isolated validation marker required"
    );
    start_core(app, DesktopModels::new(invoke), root, false, Arc::new(AtomicBool::new(false)))
}
#[cfg(feature = "native-keyboard-validation")]
pub(crate) fn validation_snapshot(app: &tauri::AppHandle) -> serde_json::Value {
    serde_json::to_value(
        app.state::<KeyboardRuntime>()
            .snapshot
            .lock()
            .unwrap()
            .clone(),
    )
    .unwrap()
}
#[cfg(feature = "native-keyboard-validation")]
pub(crate) async fn validation_request(
    app: &tauri::AppHandle,
    action: Option<&str>,
    settings: Option<Settings>,
) -> Result<serde_json::Value, String> {
    let runtime = app.state::<KeyboardRuntime>();
    let (tx, rx) = oneshot::channel();
    if let Some(settings) = settings {
        runtime
            .tx
            .send(Message::Settings(settings, tx))
            .await
            .map_err(|_| "actor stopped")?;
    } else {
        let snapshot = runtime.snapshot.lock().unwrap().clone();
        runtime
            .tx
            .send(Message::Action {
                action: action.ok_or("action required")?.into(),
                document: snapshot.document.document_id,
                revision: Some(snapshot.document.revision),
                cancel_generation: Some(snapshot.cancel_generation),
                text: None,
                edit_id: None,
                selection: None,
                test_id: None,
                audio_device_id: None,
                reply: tx,
            })
            .await
            .map_err(|_| "actor stopped")?;
    }
    serde_json::to_value(rx.await.map_err(|_| "actor stopped")??).map_err(|e| e.to_string())
}
#[cfg(feature = "native-keyboard-validation")]
pub(crate) async fn validation_microphone_test(
    app: &tauri::AppHandle,
    action: &str,
    test_id: &str,
    device_id: Option<String>,
) -> Result<serde_json::Value, String> {
    let runtime = app.state::<KeyboardRuntime>();
    let (reply, result) = oneshot::channel();
    runtime.tx.send(Message::Action {
        action: action.into(), document: None, revision: None, cancel_generation: None,
        text: None, edit_id: None, selection: None, test_id: Some(test_id.into()),
        audio_device_id: device_id, reply,
    }).await.map_err(|_| "actor stopped")?;
    serde_json::to_value(result.await.map_err(|_| "actor stopped")??).map_err(|error| error.to_string())
}
#[cfg(feature = "native-keyboard-validation")]
pub(crate) async fn validation_suspend(app: &tauri::AppHandle, paused: bool) -> Result<(), String> {
    app.state::<KeyboardRuntime>().urgent.send(Message::Suspend(paused)).map_err(|_| "actor stopped")?;
    // An unrelated targeted no-op acknowledges that the urgent lifecycle edge
    // has been consumed; it cannot cancel the current test by accident.
    validation_microphone_test(app, "microphone_test_stop", "suspension-ack", None).await.map(|_| ())
}
#[cfg(feature = "native-keyboard-validation")]
pub(crate) async fn validation_late_capture_callback(app: &tauri::AppHandle) -> Result<(), String> {
    let (reply, result) = oneshot::channel();
    app.state::<KeyboardRuntime>().urgent.send(Message::ValidationLateCaptureCallback(reply)).map_err(|_| "actor stopped")?;
    result.await.map_err(|_| "actor stopped")?
}
#[cfg(feature = "native-keyboard-validation")]
pub(crate) async fn validation_shortcut(app: &tauri::AppHandle, action: &str, pressed: bool) -> Result<serde_json::Value, String> {
    let action = match action { "toggle" => shortcuts::Action::Toggle, "beautify" => shortcuts::Action::Beautify,
        "cancel" => shortcuts::Action::Cancel, "dictate" => shortcuts::Action::Dictate, _ => return Err("unknown voice shortcut".into()) };
    let (reply, result) = oneshot::channel();
    app.state::<KeyboardRuntime>().urgent.send(Message::ValidationShortcut(action, pressed, reply)).map_err(|_| "actor stopped")?;
    serde_json::to_value(result.await.map_err(|_| "actor stopped")??).map_err(|error| error.to_string())
}
/// Inject only downstream key edges in a marked, synthetic, disposable test.
/// This never opens a serial port or substitutes for firmware/IPC validation.
#[cfg(feature = "native-keyboard-validation")]
pub(crate) async fn validation_hardware(
    app: &tauri::AppHandle,
    pressed: Option<bool>,
) -> Result<serde_json::Value, String> {
    validation_hardware_mode(app, pressed, true).await
}
/// Exercise the optional hardware dispatcher in a disposable synthetic Actor.
#[cfg(feature = "native-keyboard-validation")]
pub(crate) async fn validation_hardware_click(
    app: &tauri::AppHandle,
    button: &str,
) -> Result<serde_json::Value, String> {
    let runtime = app.state::<KeyboardRuntime>();
    let (reply, result) = oneshot::channel();
    runtime
        .tx
        .send(Message::ValidationButtonClick(button.into(), reply))
        .await
        .map_err(|_| "actor stopped")?;
    serde_json::to_value(result.await.map_err(|_| "actor stopped")??).map_err(|e| e.to_string())
}
#[cfg(feature = "native-keyboard-validation")]
pub(crate) async fn validation_external_connection(app: &tauri::AppHandle, enabled: bool) -> Result<serde_json::Value, String> {
    let mut settings = app.state::<KeyboardRuntime>().snapshot.lock().unwrap().settings.clone();
    settings.external_keyboard_enabled = enabled;
    validation_request(app, None, Some(settings)).await?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let state = app.state::<KeyboardRuntime>().snapshot.lock().unwrap().clone();
        if state.external_keyboard.state == "error" { return Err(state.external_keyboard.detail.unwrap_or("external adapter failed".into())); }
        if (enabled && matches!(state.external_keyboard.state.as_str(), "connecting" | "connected"))
            || (!enabled && state.external_keyboard.state == "disabled") {
            return serde_json::to_value(state).map_err(|error| error.to_string());
        }
        if Instant::now() >= deadline { return Err("external adapter did not settle".into()); }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}
#[cfg(feature = "native-keyboard-validation")]
pub(crate) async fn validation_external_frame(app: &tauri::AppHandle, epoch: u64, frame: serde_json::Value) -> Result<serde_json::Value, String> {
    validation_external_event(app, epoch, Event::Frame(frame)).await
}
#[cfg(feature = "native-keyboard-validation")]
pub(crate) async fn validation_external_event(app: &tauri::AppHandle, epoch: u64, event: Event) -> Result<serde_json::Value, String> {
    let (reply, result) = oneshot::channel();
    app.state::<KeyboardRuntime>().urgent.send(Message::ValidationExternalEvent(epoch, event, reply)).map_err(|_| "actor stopped")?;
    serde_json::to_value(result.await.map_err(|_| "actor stopped")??).map_err(|error| error.to_string())
}
#[cfg(feature = "native-keyboard-validation")]
pub(crate) fn validation_external_commands(app: &tauri::AppHandle) -> Result<Commands, String> {
    app.state::<KeyboardRuntime>().transport.lock().map_err(|_| "transport owner poisoned")?
        .as_ref().map(|owner| owner.transport.commands()).ok_or("external owner absent".into())
}
#[cfg(feature = "native-keyboard-validation")]
pub(crate) async fn validation_external_terminal(app: &tauri::AppHandle, epoch: u64, error: &str) -> Result<serde_json::Value, String> {
    let (reply, result) = oneshot::channel();
    app.state::<KeyboardRuntime>().urgent.send(Message::ValidationExternalTerminal(epoch, error.into(), reply)).map_err(|_| "actor stopped")?;
    serde_json::to_value(result.await.map_err(|_| "actor stopped")??).map_err(|error| error.to_string())
}
#[cfg(feature = "native-keyboard-validation")]
pub(crate) async fn validation_external_queued_start(app: &tauri::AppHandle, epoch: u64, commands: Commands) -> Result<serde_json::Value, String> {
    let runtime = app.state::<KeyboardRuntime>();
    runtime.tx.send(Message::Connected(epoch, commands)).await.map_err(|_| "actor stopped")?;
    runtime.tx.send(Message::Device(epoch, Event::Status(DeviceStatus::default()))).await.map_err(|_| "actor stopped")?;
    validation_microphone_test(app, "microphone_test_stop", "stale-start-ack", None).await
}
/// Delay only the explicit synthetic capture owner's release proof. This is
/// unavailable to ordinary application builds and never affects native audio.
#[cfg(feature = "native-keyboard-validation")]
pub(crate) async fn validation_hold_capture_stop(
    app: &tauri::AppHandle,
    hold: bool,
) -> Result<(), String> {
    let runtime = app.state::<KeyboardRuntime>();
    let (reply, result) = oneshot::channel();
    runtime.urgent.send(Message::ValidationHoldStop(hold, reply))
        .map_err(|_| "actor stopped")?;
    result.await.map_err(|_| "actor stopped")?
}
#[cfg(feature = "native-keyboard-validation")]
pub(crate) fn validation_update_paused(app: &tauri::AppHandle) -> bool {
    app.state::<KeyboardRuntime>().update_admission.paused()
}
/// Read the native producer's stop proof without changing maintenance fences.
/// Actor/UI projection delivery may follow this proof and is observed separately.
#[cfg(feature = "native-keyboard-validation")]
pub(crate) fn validation_capture_stopped(app: &tauri::AppHandle) -> bool {
    app.state::<KeyboardRuntime>().capture_safety.stopped()
}
/// Reads the last command actually consumed by the isolated serial worker.
#[cfg(feature = "native-keyboard-validation")]
pub(crate) fn validation_transport_control(app: &tauri::AppHandle) -> Result<Option<Control>, String> {
    let runtime = app.state::<KeyboardRuntime>();
    let owner = runtime.transport.lock().map_err(|_| "transport owner poisoned")?;
    Ok(owner.as_ref().and_then(|owner| owner.transport.validation_last_control()))
}
/// Read the test-owned main window through the same native foreground adapter.
#[cfg(feature = "native-keyboard-validation")]
pub(crate) fn validation_main_window(app: &tauri::AppHandle) -> Result<serde_json::Value, String> {
    let window = app.get_webview_window("main").ok_or("owned main window missing")?;
    Ok(serde_json::json!({
        "visible": window.is_visible().map_err(|error| error.to_string())?,
        "minimized": window.is_minimized().map_err(|error| error.to_string())?,
        "foreground": platform::main_is_foreground(&window).map_err(|error| error.to_string())?,
    }))
}

/// Observes the real fixture's joinable owner and OS lock without USB access.
#[cfg(feature = "native-keyboard-validation")]
pub(crate) fn validation_transport_owner(app: &tauri::AppHandle) -> Result<serde_json::Value, String> {
    let runtime = app.state::<KeyboardRuntime>();
    let owner = runtime.transport.lock().map_err(|_| "transport owner poisoned")?;
    let directory = runtime.root.join("keyboard-transport");
    let worker = owner.as_ref().and_then(|owner| owner.transport.validation_owner());
    let lock_held = Transport::validation_lock_held(&directory).map_err(|error| error.to_string())?;
    Ok(serde_json::json!({
        "processId": std::process::id(), "worker": worker, "lockHeld": lock_held,
        "endpointPublished": directory.join("owner.json").exists(),
        "directoryExists": directory.exists(),
        "epoch": runtime.external.epoch(),
    }))
}
#[cfg(feature = "native-keyboard-validation")]
async fn validation_hardware_mode(
    app: &tauri::AppHandle,
    pressed: Option<bool>,
    reset_asr: bool,
) -> Result<serde_json::Value, String> {
    let runtime = app.state::<KeyboardRuntime>();
    let (reply, result) = oneshot::channel();
    runtime
        .tx
        .send(Message::ValidationHardware {
            pressed,
            reset_asr,
            reply,
        })
        .await
        .map_err(|_| "actor stopped")?;
    serde_json::to_value(result.await.map_err(|_| "actor stopped")??).map_err(|e| e.to_string())
}
fn initialize_transport_events(
    events: &std::sync::mpsc::Receiver<Event>,
    commands: Commands,
    epoch: u64,
    tx: &mpsc::Sender<Message>,
) -> Result<(), String> {
    let initial = events.recv_timeout(Duration::from_secs(1))
        .map_err(|error| format!("键盘串口初始化未确认：{error}"))?;
    if !matches!(initial, Event::Status(_)) {
        return Err("键盘串口首个事件不是连接状态".into());
    }
    tx.try_send(Message::Connected(epoch, commands)).map_err(|_| "键盘初始化队列已关闭或满")?;
    tx.try_send(Message::Device(epoch, initial)).map_err(|_| "键盘初始化队列已关闭或满")?;
    Ok(())
}
fn transport_events(
    events: std::sync::mpsc::Receiver<Event>,
    commands: Commands,
    epoch: u64,
    tx: mpsc::Sender<Message>,
    urgent: mpsc::UnboundedSender<Message>,
) {
    let mut error = "键盘串口线程已停止".to_owned();
    while let Ok(event) = events.recv() {
        if matches!(&event, Event::Failure(_)) {
            if urgent.send(Message::Device(epoch, event)).is_err() { break; }
        } else if tx.try_send(Message::Device(epoch, event)).is_err() {
            let _ = commands.control(Control::Policy { voice_key: "none".into() });
            error = "设备事件队列溢出，键盘输入已撤销".into();
            break;
        }
    }
    let _ = urgent.send(Message::Disconnected(epoch, error));
}
async fn next_message(
    data: &mut mpsc::Receiver<Message>,
    urgent: &mut mpsc::UnboundedReceiver<Message>,
) -> Option<Message> {
    tokio::select! {
        biased;
        message = urgent.recv() => message,
        message = data.recv() => message,
    }
}
#[cfg(test)]
mod priority_tests {
    use super::*;

    #[cfg(feature = "native-keyboard-validation")]
    #[test]
    fn transport_initialization_enters_actor_fifo_before_later_actions() {
        let root = tempfile::tempdir().unwrap();
        let (mut owner, events) = Transport::start_validation(root.path().into()).unwrap();
        let (tx, mut rx) = mpsc::channel(4);
        initialize_transport_events(&events, owner.commands(), 1, &tx).unwrap();
        tx.try_send(Message::Tick).unwrap();
        assert!(matches!(rx.try_recv(), Ok(Message::Connected(1, _))));
        assert!(matches!(rx.try_recv(), Ok(Message::Device(1, Event::Status(_)))));
        assert!(matches!(rx.try_recv(), Ok(Message::Tick)));
        owner.shutdown().unwrap();
        assert!(!Transport::validation_lock_held(root.path()).unwrap());
    }
    #[cfg(feature = "native-keyboard-validation")]
    #[test]
    fn failed_initial_publication_still_joins_and_releases_the_local_owner() {
        let root = tempfile::tempdir().unwrap();
        let (owner, events) = Transport::start_validation(root.path().into()).unwrap();
        let (tx, _rx) = mpsc::channel(1);
        tx.try_send(Message::Tick).unwrap();
        assert!(initialize_transport_events(&events, owner.commands(), 1, &tx).is_err());
        drop(owner);
        assert!(!Transport::validation_lock_held(root.path()).unwrap());
    }
    #[tokio::test]
    async fn stop_and_resume_survive_a_full_streaming_queue_in_order() {
        let (data_tx, mut data) = mpsc::channel(64);
        for _ in 0..64 {
            data_tx.try_send(Message::Tick).unwrap();
        }
        assert!(data_tx.try_send(Message::Tick).is_err());
        let (control_tx, mut control) = mpsc::unbounded_channel();
        control_tx.send(Message::Suspend(true)).unwrap();
        control_tx.send(Message::Suspend(false)).unwrap();
        control_tx.send(Message::Disconnected(1, "stopped".into())).unwrap();
        assert!(matches!(
            next_message(&mut data, &mut control).await,
            Some(Message::Suspend(true))
        ));
        assert!(matches!(
            next_message(&mut data, &mut control).await,
            Some(Message::Suspend(false))
        ));
        assert!(matches!(
            next_message(&mut data, &mut control).await,
            Some(Message::Disconnected(1, _))
        ));
        assert_eq!(data.len(), 64);
    }

    #[tokio::test]
    async fn cancelled_update_preflight_recovers_even_when_streaming_data_queue_is_full() {
        let (data_tx, mut data) = mpsc::channel(64);
        for _ in 0..64 { data_tx.try_send(Message::Tick).unwrap(); }
        let (control_tx, mut control) = mpsc::unbounded_channel();
        let admission = Arc::new(UpdateAdmission::default());
        let (pause, _) = UpdatePause::begin(control_tx, admission.clone(), Arc::new(CaptureSafety::default())).unwrap();
        drop(pause);
        assert!(matches!(next_message(&mut data, &mut control).await, Some(Message::PrepareUpdate(_))));
        assert!(matches!(next_message(&mut data, &mut control).await, Some(Message::ResumeAfterUpdate(None))));
        assert!(!admission.paused());
        assert_eq!(data.len(), 64);
    }
}
impl Actor {
    fn hardware_lost(&mut self, detail: Option<String>) {
        let active_hardware = self.operation_source == InputSource::Hardware
            && (self.capture_id.is_some() || self.state.document.phase != Phase::Idle);
        let pending_hardware = self.pending_start.as_ref().is_some_and(|pending| pending.source == InputSource::Hardware);
        self.rearm_hardware();
        if self.pending_origin == Some(InputSource::Hardware) {
            self.pending_origin = None; self.state.document.pending_action = None;
        }
        if active_hardware || pending_hardware {
            self.abort(false);
            if let Some(error) = &detail { self.state.document.error = Some(error.clone()); }
        }
        if let Some(error) = detail {
            self.state.external_keyboard.detail = Some(error.clone());
            self.state.device.detail = Some(error);
        }
    }
    fn request_external(&mut self, enabled: bool) {
        self.hardware_lost(None);
        self.commands = None; self.display_sent = None;
        self.state.device = DeviceStatus::default();
        self.state.external_keyboard = ExternalKeyboardState {
            state: if enabled { "starting" } else { "stopping" }.into(), detail: None,
        };
        self.external.request(enabled);
    }
    fn external_failed(&mut self, epoch: u64, error: String) {
        if !self.external.accepts(epoch) { return; }
        self.hardware_lost(Some(error.clone()));
        self.commands = None; self.display_sent = None;
        self.state.device.connected = false;
        self.state.external_keyboard = ExternalKeyboardState { state: "error".into(), detail: Some(error) };
        // Terminal failure cannot be revived by queued initial Connected or
        // Status messages from this same instance. Cleanup gets a fresh epoch.
        self.external.request(false);
    }
    fn capture_admitted(&self) -> Result<(), String> {
        if self.stop_flag.load(Ordering::Acquire) || self.capture_safety.fenced() { return Err("语音输入已暂停，应用正在更新或退出".into()); }
        Ok(())
    }
    fn consume_holds(&mut self) {
        self.key_hold.consume();
        self.shortcut_hold.consume();
    }
    fn consume_source(&mut self, source: InputSource) {
        match source { InputSource::Hardware => self.key_hold.consume(), InputSource::Shortcut => self.shortcut_hold.consume(), InputSource::Ui => {} }
    }
    fn source_generation(&self, source: InputSource) -> u64 {
        if source == InputSource::Shortcut { self.shortcut_generation } else { self.activation_generation }
    }
    fn source_authorized(&self, source: InputSource, generation: u64) -> bool {
        match source {
            InputSource::Ui => true,
            InputSource::Hardware => self.saved.settings.external_keyboard_enabled && self.key_hold.waiting() && self.hardware_armed && generation == self.activation_generation,
            InputSource::Shortcut => self.shortcut_hold.waiting() && self.saved.settings.enabled && generation == self.shortcut_generation,
        }
    }
    fn revoke_shortcut_hold(&mut self) {
        self.shortcut_generation = self.shortcut_generation.wrapping_add(1).max(1);
        self.shortcut_hold.consume();
        if self.pending_start.as_ref().is_some_and(|pending| pending.source == InputSource::Shortcut) { self.cancel_asr(); }
        if let Some(editor) = &mut self.editor {
            if matches!(editor.pending, Some(EditorOperation::Capture { source: InputSource::Shortcut, .. })) {
                editor.pending = None; editor.requested_at = None;
            }
        }
    }
    fn shortcut(&mut self, action: shortcuts::Action, pressed: bool) {
        if action == shortcuts::Action::Dictate && !pressed {
            let consumed = self.shortcut_hold.consumed();
            self.shortcut_hold.release();
            self.shortcut_generation = self.shortcut_generation.wrapping_add(1).max(1);
            if let Some(editor) = &mut self.editor {
                if matches!(editor.pending, Some(EditorOperation::Capture { source: InputSource::Shortcut, .. })) {
                    editor.pending = None; editor.requested_at = None;
                }
            }
            if self.pending_start.as_ref().is_some_and(|pending| pending.source == InputSource::Shortcut) { self.cancel_asr(); }
            if consumed && self.operation_source == InputSource::Shortcut && self.state.document.phase == Phase::Recording {
                if self.capture_starting { self.abort(false); }
                else { let effects = self.state.document.stop(); self.effects(effects); }
            }
            return;
        }
        if !pressed || !self.saved.settings.enabled || self.suspended || self.stop_flag.load(Ordering::Acquire) { return; }
        match action {
            shortcuts::Action::Toggle => self.toggle_subtitle(),
            shortcuts::Action::Beautify => self.request_intent(Intent::Beautify, InputSource::Shortcut),
            shortcuts::Action::Cancel => self.cancel_current(),
            shortcuts::Action::Dictate => { if self.shortcut_hold.press() { self.begin_capture(InputSource::Shortcut); } }
        }
    }
    fn toggle_subtitle(&mut self) {
        if self.state.document.visible {
            if self.capture_id.is_some() || self.pending_start.is_some() || self.state.document.phase != Phase::Idle {
                self.state.document.notice = Some("请先结束或取消语音操作再隐藏字幕".into());
            } else if let Some(window) = self.app.get_webview_window("keyboard-subtitle") {
                match platform::hide_subtitle(&window) {
                    Ok(()) => self.state.document.visible = false,
                    Err(error) => self.state.document.error = Some(error.to_string()),
                }
            }
        } else {
            self.close_pending = false; self.state.document.open();
            if let Err(error) = self.show() { self.state.document.error = Some(error); }
        }
    }
    async fn run(
        mut self,
        mut rx: mpsc::Receiver<Message>,
        mut urgent: mpsc::UnboundedReceiver<Message>,
    ) {
        if self.saved.settings.external_keyboard_enabled && !self.stop_flag.load(Ordering::Acquire) { self.request_external(true); }
        while let Some(message) = next_message(&mut rx, &mut urgent).await {
            match message {
                Message::Shortcut(action, pressed) => self.shortcut(action, pressed),
                #[cfg(feature = "native-keyboard-validation")]
                Message::ValidationShortcut(action, pressed, reply) => {
                    let result = if self.production || !self.synthetic_input { Err("synthetic isolated actor required".into()) }
                    else { self.shortcut(action, pressed); self.publish(); Ok(self.state.clone()) };
                    let _ = reply.send(result);
                }
                #[cfg(feature = "native-keyboard-validation")]
                Message::ValidationExternalEvent(epoch, event, reply) => {
                    let result = if self.production || !self.synthetic_input { Err("synthetic isolated actor required".into()) }
                    else {
                        if self.saved.settings.external_keyboard_enabled && self.external.accepts(epoch) { self.device(event); }
                        self.publish(); Ok(self.state.clone())
                    };
                    let _ = reply.send(result);
                }
                #[cfg(feature = "native-keyboard-validation")]
                Message::ValidationExternalTerminal(epoch, error, reply) => {
                    let result = if self.production || !self.synthetic_input { Err("synthetic isolated actor required".into()) }
                    else { self.external_failed(epoch, error); self.publish(); Ok(self.state.clone()) };
                    let _ = reply.send(result);
                }
                #[cfg(feature = "native-keyboard-validation")]
                Message::ValidationButtonClick(button, reply) => {
                    let result = if self.production || !self.synthetic_input {
                        Err("synthetic isolated actor required".into())
                    } else if !matches!(button.as_str(), "k1" | "k2" | "k3") {
                        Err("only K1/K2/K3 fixture clicks are allowed".into())
                    } else {
                        self.device(Event::Frame(serde_json::json!({
                            "event": "button_click", "button": button
                        })));
                        self.publish();
                        Ok(self.state.clone())
                    };
                    let _ = reply.send(result);
                }
                #[cfg(feature = "native-keyboard-validation")]
                Message::ValidationHardware {
                    pressed,
                    reset_asr,
                    reply,
                } => {
                    let result = if self.production || !self.synthetic_input {
                        Err("synthetic isolated actor required".into())
                    } else if !self.saved.settings.external_keyboard_enabled {
                        Err("external input mode is disabled".into())
                    } else {
                        if let Some(pressed) = pressed {
                            self.device(Event::Frame(serde_json::json!({
                                "event": if pressed { "button_down" } else { "button_up" },
                                "button": "k4"
                            })));
                        } else {
                            if reset_asr && self.capture_id.is_none() && self.pending_start.as_ref().is_none_or(|pending| pending.source == InputSource::Hardware) {
                                self.cancel_asr();
                            }
                            self.key_hold.release();
                            self.state.device.id = Some("isolated-synthetic-keyboard".into());
                            self.state.device.connected = true;
                            self.state.device.policy_state = "applied".into();
                            self.state.device.capture_state = "stopped".into();
                            self.hardware_armed = true;
                            self.state.service.ready = true;
                            self.state.external_keyboard = ExternalKeyboardState { state: "connected".into(), detail: None };
                        }
                        Ok(self.state.clone())
                    };
                    self.publish();
                    let _ = reply.send(result);
                }
                #[cfg(feature = "native-keyboard-validation")]
                Message::ValidationHoldStop(hold, reply) => {
                    let result = if self.production || !self.synthetic_input {
                        Err("synthetic isolated actor required".into())
                    } else {
                        self.synthetic_stop_hold.store(hold, Ordering::Release);
                        Ok(())
                    };
                    let _ = reply.send(result);
                }
                #[cfg(feature = "native-keyboard-validation")]
                Message::ValidationLateCaptureCallback(reply) => {
                    let result = if self.production || !self.synthetic_input || self.microphone_check.is_none()
                        || self.stop_requested != Some(true) {
                        Err("cancelled synthetic local owner required".into())
                    } else {
                        // Deterministically represent an input callback that
                        // crossed its native gate before cancellation, then
                        // finishes scalar ingress after the stop request.
                        let append = self.capture_input.as_ref().ok_or("local input missing")
                            .and_then(|(_, input)| input.append_pcm16_16k(&[0; 32]).map_err(|_| "admitted late input was closed early"));
                        if let Some(id) = self.capture_id.clone() {
                            let _ = self.urgent.send(Message::CaptureError(id, "late callback after cancellation fixture".into()));
                        }
                        append.map_err(str::to_owned)
                    };
                    let _ = reply.send(result);
                }
                Message::Shutdown(reply) => {
                    self.suspended = true;
                    self.revoke_shortcut_hold();
                    self.rearm_hardware();
                    self.apply_policy();
                    self.state.service.ready = false;
                    self.state.service.detail =
                        Some("正在结束系统录音并释放语音资源".into());
                    self.abort(false);
                    self.shutdown_replies.push(reply);
                    if let Some(mut controller) = self.shortcuts.take() {
                        let urgent = self.urgent.clone();
                        tauri::async_runtime::spawn_blocking(move || {
                            let result = controller.shutdown().map(|_| controller.registered());
                            let _ = urgent.send(Message::ShortcutCleanup(result));
                        });
                    }
                }
                Message::PrepareUpdate(reply) => {
                    self.revoke_shortcut_hold();
                    self.suspended = self.suspension.update(true);
                    self.rearm_hardware();
                    self.apply_policy();
                    self.state.service.detail = Some("正在暂停语音键盘以准备更新".into());
                    self.abort(false);
                    self.shutdown_replies.push(reply);
                }
                Message::ResumeAfterUpdate(reply) => {
                    self.suspended = self.suspension.update(false);
                    // Resume admission, never the recording or K4 hold that was
                    // cancelled by maintenance. A fresh device release fence
                    // must complete before a new physical press is accepted.
                    self.rearm_hardware();
                    self.state.service.detail = None;
                    if !self.suspended && !self.stop_flag.load(Ordering::Acquire) {
                        self.apply_policy();
                        self.prepare_asr();
                    }
                    if let Some(reply) = reply {
                        let _ = reply.send(());
                    }
                }
                Message::Connected(epoch, commands) => {
                    if !self.saved.settings.external_keyboard_enabled || !self.external.accepts(epoch) { continue; }
                    self.rearm_hardware();
                    self.display_sent = None;
                    self.commands = Some(commands);
                    self.policy_sent = false;
                    self.state.external_keyboard = ExternalKeyboardState { state: "connecting".into(), detail: None };
                    self.apply_policy();
                }
                Message::Disconnected(epoch, error) => self.external_failed(epoch, error),
                Message::ExternalSettled(epoch, result) => {
                    if self.external.epoch() != epoch { continue; }
                    match result {
                        Ok(false) if !self.saved.settings.external_keyboard_enabled => {
                            self.state.external_keyboard = ExternalKeyboardState::default();
                            self.state.device = DeviceStatus::default(); self.commands = None;
                        }
                        Ok(true) if self.saved.settings.external_keyboard_enabled => {
                            self.state.external_keyboard.state = if self.state.device.connected { "connected" } else { "connecting" }.into();
                        }
                        Err(error) => {
                            self.external.invalidate(epoch);
                            self.hardware_lost(Some(error.clone()));
                            self.commands = None;
                            self.state.external_keyboard = ExternalKeyboardState { state: "error".into(), detail: Some(error) };
                        }
                        _ => {}
                    }
                }
                Message::LifecycleFailure(error) => {
                    self.suspended = true; self.abort(false);
                    self.state.service.detail = Some(error);
                }
                Message::ShortcutCleanup(result) => {
                    match result {
                        Ok(count) => self.state.shortcut_state.registered = count,
                        Err(error) => self.state.shortcut_state.detail = Some(error),
                    }
                }
                Message::Suspend(paused) => {
                    self.revoke_shortcut_hold();
                    self.rearm_hardware();
                    self.suspended = self.suspension.system(paused);
                    self.key_hold.release();
                    self.abort(false);
                    self.policy_sent = false;
                    self.apply_policy();
                    if !paused {
                        self.prepare_asr();
                    }
                }
                Message::Tick => {
                    if self.capture_id.is_none() && self.pending_start.is_none() && !self.opening
                        && self.state.document.phase == Phase::Idle && !self.close_pending
                        && !self.key_hold.waiting() && !self.shortcut_hold.waiting()
                        && self.editor.as_ref().is_none_or(|editor| editor.requested_at.is_none())
                        && self.shutdown_replies.is_empty() { continue; }
                    if self
                        .opening_since
                        .is_some_and(|at| at.elapsed() >= Duration::from_secs(20))
                    {
                        self.asr_failed("实时识别准备超时".into(), false);
                    }
                    if self.capture_starting
                        && self.microphone_check.is_none()
                        && self
                            .recording_at
                            .is_some_and(|at| at.elapsed() >= Duration::from_secs(15))
                    {
                        self.abort(false);
                        self.state.document.error =
                            Some("系统麦克风启动超时，已撤销本段；正在等待原生停采回执".into());
                    }
                    if let Some(editor) = &mut self.editor {
                        if editor
                            .requested_at
                            .is_some_and(|at| at.elapsed() >= Duration::from_secs(15))
                        {
                            editor.pending = None;
                            editor.requested_at = None;
                            self.consume_holds();
                            self.state.document.error =
                                Some("等待编辑保存超时，按键操作已撤销；请完成编辑后重试".into());
                        }
                    }
                    if self.pending_start.as_ref().is_some_and(|pending| {
                        pending.started_at.elapsed() >= Duration::from_secs(20)
                    }) {
                        self.asr_failed("实时识别准备超时".into(), false);
                    }
                    if self
                        .capture_id
                        .as_ref()
                        .is_some_and(|id| self.state.document.matches(id))
                        && (self
                            .recording_at
                            .is_some_and(|at| at.elapsed() > Duration::from_secs(600))
                            || self
                                .drain_at
                                .is_some_and(|at| at.elapsed() > Duration::from_secs(15)))
                    {
                        self.abort(false);
                        self.state.document.error = Some("录音或定稿超时，自动操作已撤销".into());
                    }
                    self.prepare_asr();
                }
                Message::MicrophoneTick(id) => self.microphone_tick(&id),
                #[cfg(feature = "native-keyboard-validation")]
                Message::SyntheticInputReady(id) => {
                    if self.microphone_check.as_ref().is_some_and(|check| check.owner == id) {
                        self.microphone_check_ready(true);
                    }
                }
                Message::Device(epoch, event) => {
                    if self.saved.settings.external_keyboard_enabled && self.external.accepts(epoch) {
                        self.device(event);
                    }
                }
                Message::Action {
                    action,
                    document,
                    revision,
                    cancel_generation,
                    text,
                    edit_id,
                    selection,
                    test_id,
                    audio_device_id,
                    reply,
                } => {
                    let result = self.action(&action, document, revision, cancel_generation, text, edit_id, selection, test_id, audio_device_id);
                    self.publish();
                    let _ = reply.send(result.map(|_| self.state.clone()));
                }
                Message::Settings(settings, reply) => {
                    let result = self.settings(settings);
                    self.publish();
                    let _ = reply.send(result.map(|_| self.state.clone()));
                }
                Message::AsrReady(generation, result) => {
                    if generation == self.asr_generation {
                        let result = if self
                            .opening_since
                            .is_some_and(|at| at.elapsed() < Duration::from_secs(20))
                        {
                            result
                        } else {
                            if let Ok(session) = result {
                                session.cancel();
                            }
                            Err("实时识别准备超过20秒，未打开麦克风，请重试".into())
                        };
                        self.opening = false;
                        self.opening_abort = None;
                        self.opening_since = None;
                        match result {
                            Ok(mut session) => {
                                self.asr = Some(session.input());
                                let tx = self.tx.clone();
                                tauri::async_runtime::spawn(async move {
                                    let mut terminal = false;
                                    while let Some(event) = session.recv().await {
                                        terminal |= matches!(
                                            &event,
                                            StreamingAsrEvent::Finished { .. }
                                                | StreamingAsrEvent::Error { .. }
                                                | StreamingAsrEvent::Cancelled
                                        );
                                        if tx.send(Message::Asr(generation, event)).await.is_err() {
                                            session.cancel();
                                            break;
                                        }
                                    }
                                    if !terminal {
                                        let _ = tx
                                            .send(Message::Asr(
                                                generation,
                                                StreamingAsrEvent::Error {
                                                    message: "实时识别连接结束但未完成定稿".into(),
                                                },
                                            ))
                                            .await;
                                    }
                                });
                                if let Some(pending) = self.pending_start.take() {
                                    let timely =
                                        pending.started_at.elapsed() < Duration::from_secs(20);
                                    let current = pending.document_id
                                        == self.state.document.document_id
                                        && pending.revision == self.state.document.revision;
                                    let authorized = self.source_authorized(pending.source, pending.activation_generation);
                                    if !timely {
                                        if current && authorized {
                                            self.consume_source(pending.source);
                                            self.asr_failed("实时识别准备超时".into(), true);
                                        } else {
                                            self.cancel_asr();
                                        }
                                    } else if current && authorized {
                                        self.begin_capture(pending.source);
                                    } else {
                                        self.consume_source(pending.source);
                                        self.state.audio_state = AudioState {
                                            state: "stopped".into(),
                                            detail: None,
                                        };
                                    }
                                }
                            }
                            Err(error) => self.asr_failed(error, false),
                        }
                    } else if let Ok(session) = result {
                        session.cancel();
                    }
                }
                Message::Asr(generation, event) => {
                    if generation == self.asr_generation {
                        match event {
                            StreamingAsrEvent::Partial { text } => {
                                if let Some(id) = self.asr_document.clone() {
                                    self.state.document.partial(&id, text);
                                }
                            }
                            StreamingAsrEvent::Final { .. } => {}
                            StreamingAsrEvent::Finished { text } => {
                                self.asr = None;
                                if let Some(id) = self.asr_document.take() {
                                    let effects = self.state.document.asr_finished(&id, text);
                                    self.drain_at = None;
                                    self.recording_at = None;
                                    self.effects(effects);
                                }
                                self.prepare_asr();
                            }
                            StreamingAsrEvent::Error { message } => self.asr_failed(message, false),
                            StreamingAsrEvent::Cancelled => {
                                self.asr_failed("语音识别会话已终止".into(), false);
                            }
                        }
                    }
                }
                Message::CaptureReady(id, result) => {
                    if self.capture_id.as_ref() == Some(&id) {
                        self.capture_starting = false;
                        match result {
                            Ok(handle) => {
                                let input_ready = handle.input_ready();
                                if self.microphone_check.is_some() {
                                    self.state.microphone_test.callback_observed |= handle.callback_observed();
                                    if input_ready { self.microphone_check_ready(true); }
                                }
                                self.capture_canceller = Some(handle.canceller());
                                self.capture = Some(handle);
                                if let Some(cancel) = self.stop_requested {
                                    self.request_stop(cancel);
                                } else if input_ready {
                                    self.state.audio_state = AudioState {
                                        state: "capturing".into(),
                                        detail: None,
                                    };
                                } else if self.microphone_check.is_some() {
                                    self.stop_microphone_check("failed", Some("系统麦克风未进入聆听状态".into()));
                                } else {
                                    self.abort(false);
                                    self.state.document.error =
                                        Some("系统麦克风未进入聆听状态，已取消本段录音".into());
                                }
                            }
                            Err(error) => self.capture_failed(id, error),
                        }
                    } else if let Ok(handle) = result {
                        tauri::async_runtime::spawn(async move {
                            let _ = handle.cancel().await;
                        });
                    }
                }
                Message::CaptureStopped(id, receipt) => self.capture_stopped(id, receipt),
                Message::CaptureError(id, error) => {
                    if self.microphone_check.as_ref().is_some_and(|check| check.owner == id) {
                        // A callback can have passed native admission just
                        // before cancellation. Its late fault cannot replace
                        // the user's already-owned cancellation result.
                        if self.stop_requested != Some(true) {
                            self.stop_microphone_check("failed", Some(format!("系统麦克风：{error}")));
                        }
                    } else if self.capture_id.as_ref() == Some(&id)
                        && self.stop_requested != Some(true)
                        && self.state.document.matches(&id)
                    {
                        self.abort(false);
                        self.state.document.error = Some(format!("系统麦克风：{error}"));
                    }
                }
                Message::PlatformAsrFinished(id, result) => {
                    if self
                        .platform_asr
                        .as_ref()
                        .is_some_and(|(active, _)| active == &id)
                        && self.state.document.matches(&id)
                        && self.state.document.phase == Phase::Finalizing
                    {
                        self.platform_asr = None;
                        self.drain_at = None;
                        self.recording_at = None;
                        match result {
                            Ok(text) => {
                                let effects = self.state.document.asr_finished(&id, text);
                                self.effects(effects);
                            }
                            Err(error) => {
                                self.state.document.fail(&id, error);
                            }
                        }
                        self.prepare_asr();
                    }
                }
                Message::Candidate(id, text) => self.state.document.candidate(&id, text),
                Message::Polished(id, result) => {
                    if self.state.document.matches(&id) {
                        self.polish = None;
                        match result {
                            Ok(text) => {
                                let effects = self.state.document.beautified(&id, text);
                                self.effects(effects);
                            }
                            Err(error) => self.state.document.fail(&id, error),
                        }
                        self.prepare_asr();
                    }
                }
            }
            if self.capture_safety.stopped() {
                for reply in self.shutdown_replies.drain(..) {
                    let _ = reply.send(Ok(()));
                }
            }
            self.resume_waiting_hold();
            self.publish();
        }
    }
    fn resume_waiting_hold(&mut self) {
        if self.suspended
            || self.stop_flag.load(Ordering::Acquire)
            || self.close_pending
            || self.capture_id.is_some()
            || self.pending_start.is_some()
            || self.state.document.phase != Phase::Idle
            || self.state.audio_state.state != "stopped"
            || self.editor.as_ref().is_some_and(|editor| {
                matches!(editor.pending, Some(EditorOperation::Capture { .. }))
            })
        {
            return;
        }
        let source = if self.shortcut_hold.waiting() && self.saved.settings.enabled {
            InputSource::Shortcut
        } else if self.key_hold.waiting() && self.hardware_armed { InputSource::Hardware }
        else { return; };
        self.sync_text_cursor();
        self.begin_capture(source);
    }
    fn sync_text_cursor(&mut self) {
        if let Some(document) = self.state.document.document_id.clone() {
            if self
                .text_cursor
                .as_ref()
                .is_some_and(|cursor| cursor.document != document)
            {
                self.text_cursor = None;
            }
            let committed = (document.clone(), self.state.document.revision);
            if self.state.document.phase == Phase::Idle
                && self.dictation_cursor_revision.as_ref() != Some(&committed)
            {
                if let Some(selection) = self.state.document.dictation_selection {
                    self.text_cursor = Some(TextCursor {
                        document,
                        revision: committed.1,
                        selection,
                    });
                    self.dictation_cursor_revision = Some(committed);
                }
            }
        } else {
            self.text_cursor = None;
            self.dictation_cursor_revision = None;
        }
    }
    fn publish(&mut self) {
        self.sync_text_cursor();
        self.state.sequence = self.state.sequence.saturating_add(1);
        self.state.edit_session_id = self.editor.as_ref().map(|editor| editor.id.clone());
        self.state.service.running = !self.stop_flag.load(Ordering::Acquire);
        self.state.service.ready = !self.suspended && self.state.service.running;
        // Waiting for the current editor/IME is preparation, not microphone
        // capture. Keep audioState stopped so the edit can still be committed.
        self.state.capture_preparing = !self.suspended
            && !self.stop_flag.load(Ordering::Acquire)
            && (self.key_hold.waiting() || self.shortcut_hold.waiting()
                || self.state.audio_state.state == "starting"
                || self.editor.as_ref().is_some_and(|editor| {
                    matches!(editor.pending, Some(EditorOperation::Capture { .. }))
                }));
        let stage =
            if self.editor.as_ref().is_some_and(|editor| {
                matches!(editor.pending, Some(EditorOperation::Capture { .. }))
            }) {
                "saving_edit"
            } else if self.pending_start.is_some() {
                "connecting_for_hold"
            } else if self.state.audio_state.state == "stopping" {
                "stopping"
            } else if self.capture_starting {
                "starting_microphone"
            } else if self.state.audio_state.state == "capturing" {
                "listening"
            } else if self.opening {
                "connecting_provider"
            } else if self.state.document.phase == Phase::Finalizing {
                "finalizing"
            } else if self.state.document.phase == Phase::Beautifying {
                "polishing"
            } else if self.key_hold.waiting() || self.shortcut_hold.waiting() {
                "waiting_previous"
            } else if self.asr.is_some() {
                "ready"
            } else {
                "idle"
            };
        let trace = (
            stage,
            self.asr_generation,
            self.activation_generation,
            self.key_hold.pressed() || self.shortcut_hold.pressed(),
        );
        if self.startup_trace.as_ref() != Some(&trace) {
            let waiting_ms = self
                .pending_start
                .as_ref()
                .map(|request| request.started_at)
                .or(self.recording_at)
                .map(|at| at.elapsed().as_millis() as u64)
                .unwrap_or(0);
            tracing::info!(target: "nomifun_keyboard", stage, waiting_ms,
                asr_generation = self.asr_generation, activation_generation = self.activation_generation,
                held = self.key_hold.pressed() || self.shortcut_hold.pressed(), microphone_owner = self.capture_id.is_some(),
                error_present = self.state.document.error.is_some(), "capture lifecycle");
            self.startup_trace = Some(trace);
        }
        let display = if self.suspended || self.stop_flag.load(Ordering::Acquire) {
            "sleeping"
        } else if self.state.document.error.is_some() || self.state.audio_state.state == "error" {
            "error"
        } else if self.state.capture_preparing {
            "starting"
        } else if self.state.audio_state.state == "capturing" {
            "listening"
        } else if self.state.document.phase == Phase::Finalizing {
            "recognizing"
        } else if self.state.document.phase == Phase::Beautifying {
            "polishing"
        } else if self.state.audio_state.state == "stopping" {
            "connecting"
        } else {
            "ready"
        };
        if self.state.device.display_available && self.display_sent.as_deref() != Some(display) {
            if self
                .control(Control::Display {
                    state: display.into(),
                })
                .is_ok()
            {
                self.display_sent = Some(display.into());
            }
        }
        *self.snapshot.lock().unwrap() = self.state.clone();
        let _ = self.app.emit("keyboard://state", &self.state);
    }
    fn control(&self, control: Control) -> Result<(), String> {
        self.commands.as_ref().ok_or("键盘连接未就绪")?
            .control(control).map_err(|error| error.to_string())
    }
    fn apply_policy(&mut self) {
        if !self.policy_sent && self.commands.is_some() {
            let control = if !self.saved.settings.external_keyboard_enabled || self.suspended || self.stop_flag.load(Ordering::Acquire) {
                Control::Policy { voice_key: "none".into() }
            } else {
                Control::Rearm { generation: self.activation_generation }
            };
            match self.control(control) {
                Ok(()) => {
                    self.policy_sent = true;
                    self.state.device.policy_state = "pending".into();
                }
                Err(error) => self.state.external_keyboard.detail = Some(error),
            }
        }
    }
    fn rearm_hardware(&mut self) {
        self.revoke_editor_hardware();
        if self
            .pending_start
            .as_ref()
            .is_some_and(|pending| pending.source.hardware())
        {
            self.cancel_asr();
        }
        self.activation_generation = self.activation_generation.wrapping_add(1).max(1);
        self.hardware_armed = false;
        self.state.service.ready = false;
        self.key_hold.release();
        self.policy_sent = false;
    }
    fn prepare_asr(&mut self) {
        // Voice is demand-driven. Merely starting the application, changing a
        // setting or opening the settings page must not open a provider socket
        // or construct a subtitle WebView.
        if self.saved.settings.asr_mode != AsrMode::Streaming
            || self.pending_start.is_none()
            || self.suspended
            || self.stop_flag.load(Ordering::Acquire)
            || self.close_pending
            || Instant::now() < self.retry_at
            || self.asr.is_some()
            || self.opening
            || self.state.document.phase != Phase::Idle
            || self.capture_id.is_some()
        {
            return;
        }
        let Some(model) = self.saved.settings.asr_model.clone() else {
            return;
        };
        self.asr_generation += 1;
        let generation = self.asr_generation;
        self.opening = true;
        self.opening_since = Some(Instant::now());
        let models = self.models.clone();
        let ready_tx = self.urgent.clone();
        let config = StreamingAsrConfig {
            language: Some(self.saved.settings.language.clone()).filter(|s| !s.is_empty()),
            dictionary: self.saved.settings.dictionary.clone(),
        };
        self.opening_abort = Some(asr_preparation::spawn(
            async move {
                tracing::info!(target: "nomifun_keyboard", stage = "asr_task_polled",
                    asr_generation = generation, "capture lifecycle");
                models.open_asr(&selection(&model), config).await
            },
            self.opening_since.unwrap() + Duration::from_secs(20),
            move |result| {
                tracing::info!(target: "nomifun_keyboard", stage = "asr_task_finished",
                    asr_generation = generation, success = result.is_ok(), "capture lifecycle");
                let _ = ready_tx.send(Message::AsrReady(generation, result));
            },
        ));
    }
    fn cancel_asr(&mut self) {
        if let Some(task) = self.opening_abort.take() {
            task.abort();
        }
        self.opening_since = None;
        if let Some((_, input)) = self.buffered_capture.take() {
            input.cancel();
        }
        if let Some((_, cancel)) = self.platform_asr.take() {
            cancel.cancel();
        }
        if self.capture_id.is_none() && self.state.audio_state.state == "starting" {
            self.state.audio_state = AudioState {
                state: "stopped".into(),
                detail: None,
            };
        }
        self.asr_document = None;
        self.asr_generation += 1;
        self.opening = false;
        self.pending_start = None;
        if let Some(input) = self.asr.take() {
            input.cancel();
        }
    }
    fn asr_failed(&mut self, error: String, requested: bool) {
        let active = self.asr_document.is_some();
        let requested_source = self
            .pending_start
            .as_ref()
            .map(|pending| pending.source);
        let requested = requested || self.pending_start.is_some() || active;
        let timed_out = error.to_ascii_lowercase().contains("timed out")
            || error.contains("超时")
            || error.contains("超过20秒");
        let kind = if timed_out {
            "timeout"
        } else if error.contains("queue") || error.contains("backlog") {
            "backpressure"
        } else if error.contains("transport") || error.contains("socket") {
            "transport"
        } else {
            "protocol"
        };
        if active {
            self.abort(false);
        } else {
            self.cancel_asr();
        }
        self.retry_at = Instant::now() + Duration::from_secs(15);
        tracing::warn!(target: "nomifun_keyboard", stage = "asr_failure",
            asr_generation = self.asr_generation, user_requested = requested,
            active_capture = active, timed_out, kind, "recognition lifecycle");
        // A retired connection must not cancel editing/polishing or replace a
        // real operation error after its capture request has been revoked.
        if !requested {
            return;
        }
        if let Some(source) = requested_source { self.consume_source(source); }
        let message = if active && kind == "backpressure" {
            "语音识别处理暂时跟不上，本段未写入，已有文字已保留。请稍候重试。"
        } else { match (active, timed_out) {
            (false, true) => {
                "连接语音识别服务超时，尚未开始录音。请检查网络和所选 ASR 模型设置后重试。"
            }
            (false, false) => {
                "暂时无法连接语音识别服务，尚未开始录音。请检查网络和所选 ASR 模型设置后重试。"
            }
            (true, true) => {
                "语音识别超时，本段未写入，已有文字已保留。请检查网络和所选 ASR 模型设置后重试。"
            }
            (true, false) => {
                "语音识别中断，本段未写入，已有文字已保留。请检查网络和所选 ASR 模型设置后重试。"
            }
        }}
        .to_owned();
        self.state.document.error = Some(message);
    }
    fn capture_failed(&mut self, id: Identity, failure: audio::CaptureStartFailure) {
        if self.microphone_check.as_ref().is_some_and(|check| check.owner == id) {
            self.microphone_check_failed(id, failure);
            return;
        }
        let cancelled = self.stop_requested == Some(true);
        if !cancelled || !failure.capture_stopped {
            self.state.document.fail(&id, failure.message.clone());
        }
        self.cancel_asr();
        self.recording_at = None;
        self.drain_at = None;
        if failure.capture_stopped {
            self.capture_id = None;
            self.capture_input = None;
            self.capture_start_gate = None;
            self.capture_starting = false;
            self.capture_canceller = None;
            self.state.audio_state = AudioState {
                state: "stopped".into(),
                detail: (!cancelled).then_some(failure.message),
            };
            if self.close_pending {
                self.finish_close();
            }
        } else {
            self.state.audio_state = AudioState {
                state: "stopping".into(),
                detail: Some(failure.message),
            };
            if let Some(handle) = failure.pending {
                self.capture_canceller = Some(handle.canceller());
                self.capture = Some(handle);
                self.request_stop(true);
            }
        }
    }
    fn capture_stopped(&mut self, id: Identity, receipt: audio::StopReceipt) {
        if self.microphone_check.as_ref().is_some_and(|check| check.owner == id) {
            self.microphone_check_stopped(id, receipt);
            return;
        }
        if self.capture_id.as_ref() != Some(&id) {
            return;
        }
        if !receipt.capture_stopped {
            self.state.audio_state = AudioState {
                state: "error".into(),
                detail: receipt.error.clone(),
            };
            self.state
                .document
                .fail(&id, receipt.error.unwrap_or("系统麦克风停止未确认".into()));
            self.cancel_asr();
            return;
        }
        let cancelled = receipt.cancelled || self.stop_requested == Some(true);
        if let Some((owner, input)) = self.capture_input.take() {
            if owner == id {
                self.last_capture_stats = Some((owner, input.stats()));
            }
        }
        self.capture_id = None;
        self.capture_start_gate = None;
        #[cfg(feature = "native-keyboard-validation")]
        {
            self.synthetic_stop = None;
        }
        self.capture_starting = false;
        self.capture = None;
        self.capture_canceller = None;
        self.stop_requested = None;
        self.recording_at = None;
        self.state.audio_state = AudioState {
            state: "stopped".into(),
            detail: receipt.error.clone().or_else(|| self.capture_summary(&id)),
        };
        if !cancelled
            && receipt.error.is_none()
            && self
                .last_capture_stats
                .as_ref()
                .is_some_and(|(owner, stats)| {
                    owner == &id && stats.samples > 0 && stats.nonzero == 0
                })
        {
            self.state.document.fail(
                &id,
                "麦克风输入信号全为静音，请检查录音设备、静音开关和输入音量。".into(),
            );
            self.cancel_asr();
            if self.close_pending {
                self.finish_close();
            }
            return;
        }
        if let Some(error) = receipt.error {
            self.state.document.fail(&id, error);
            self.cancel_asr();
        } else if !cancelled && self.state.document.matches(&id) {
            let effects = self.state.document.audio_ended(&id);
            self.effects(effects);
            if self.saved.settings.asr_mode == AsrMode::Platform {
                self.transcribe_buffered(id);
            } else if let Some(input) = self.asr.clone() {
                let tx = self.tx.clone();
                let generation = self.asr_generation;
                tauri::async_runtime::spawn(async move {
                    if let Err(message) = input.finish().await {
                        let _ = tx
                            .send(Message::Asr(
                                generation,
                                StreamingAsrEvent::Error { message },
                            ))
                            .await;
                    }
                });
            } else {
                self.state
                    .document
                    .fail(&id, "实时识别连接已失效，未提交自动操作".into());
            }
        }
        if self.close_pending {
            self.finish_close();
        }
    }
    fn capture_summary(&self, id: &Identity) -> Option<String> {
        let (owner, stats) = self.last_capture_stats.as_ref()?;
        if owner != id {
            return None;
        }
        let duration = stats.samples as f64 / 16000.0;
        let peak = if stats.peak == 0 {
            "无输入电平".into()
        } else {
            format!(
                "峰值 {:.0} dBFS",
                20.0 * (stats.peak as f64 / 32768.0).log10()
            )
        };
        Some(format!("已采集 {duration:.1} 秒系统音频，{peak}"))
    }
    fn transcribe_buffered(&mut self, id: Identity) {
        let Some((owner, buffer)) = self.buffered_capture.take() else {
            self.state.document.fail(&id, "本段录音缓冲区不可用".into());
            return;
        };
        if owner != id {
            buffer.cancel();
            self.state.document.fail(&id, "本段录音身份已失效".into());
            return;
        }
        let pcm = match buffer.finish() {
            Ok(pcm) => pcm,
            Err(error) => {
                self.state.document.fail(&id, error);
                return;
            }
        };
        if pcm.is_empty() {
            let effects = self.state.document.asr_finished(&id, String::new());
            self.effects(effects);
            return;
        }
        let wav = match models::pcm16_mono_16k_wav(&pcm) {
            Ok(wav) => wav,
            Err(error) => {
                self.state.document.fail(&id, error);
                return;
            }
        };
        let Some(model) = self.saved.settings.asr_model.clone() else {
            self.state.document.fail(&id, "普通 ASR 模型未配置".into());
            return;
        };
        let config = StreamingAsrConfig {
            language: Some(self.saved.settings.language.clone()).filter(|s| !s.is_empty()),
            dictionary: self.saved.settings.dictionary.clone(),
        };
        let cancel = CancellationToken::new();
        self.platform_asr = Some((id.clone(), cancel.clone()));
        // Publish the confirmed native stop before a file upload can begin.
        self.publish();
        let models = self.models.clone();
        let tx = self.tx.clone();
        tauri::async_runtime::spawn(async move {
            let result = models
                .transcribe_platform(&selection(&model), wav, config, cancel)
                .await;
            let _ = tx.send(Message::PlatformAsrFinished(id, result)).await;
        });
    }
    fn start_microphone_check(&mut self, test_id: Option<String>, device: Option<String>) -> Result<(), String> {
        self.capture_admitted()?;
        let test_id = test_id.filter(|id| !id.trim().is_empty() && id.len() <= 128)
            .ok_or("缺少有效麦克风检测标识")?;
        if device.as_ref().is_some_and(|id| id.is_empty() || id.len() > 1024) {
            return Err("系统麦克风选择无效".into());
        }
        if self.state.microphone_test.id.as_deref() == Some(&test_id) {
            return Err("麦克风检测标识已使用，请重新开始检测".into());
        }
        if self.suspended || self.stop_flag.load(Ordering::Acquire) {
            return Err("当前系统会话不可录音，请解锁或重新启动服务".into());
        }
        if self.capture_id.is_some() || self.pending_start.is_some() || self.close_pending
            || self.state.document.phase != Phase::Idle || self.state.audio_state.state != "stopped" {
            return Err("请先结束当前录音或处理，再检测麦克风".into());
        }
        // Keep idle provider preparation intact. A local test neither uses it
        // nor cancels it, since cancellation would make the next idle tick
        // reconnect and turn this local action into a new provider request.
        self.consume_holds();
        let owner = Identity { document_id: format!("microphone-test:{test_id}"), revision: 0,
            operation_id: uuid::Uuid::now_v7().to_string() };
        let meter = audio::MeterCapture::new();
        let input = meter.input();
        let timer = CancellationToken::new();
        self.state.microphone_test = MicrophoneTestState { id: Some(test_id.clone()),
            device_id: device.clone(), state: "starting".into(), stopped: false,
            ..Default::default() };
        self.microphone_check = Some(MicrophoneCheck { test_id, owner: owner.clone(), meter,
            started_at: Instant::now(), listening_at: None, timer: timer.clone(), outcome: "completed" });
        self.operation_source = InputSource::Ui;
        self.capture_id = Some(owner.clone());
        self.capture_input = Some((owner.clone(), input.clone()));
        self.capture_starting = true;
        self.stop_requested = None;
        self.state.audio_state = AudioState { state: "starting".into(), detail: None };
        let ticks = self.tx.clone();
        let tick_owner = owner.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::select! {
                    _ = timer.cancelled() => break,
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {
                        tokio::select! {
                            _ = timer.cancelled() => break,
                            result = ticks.send(Message::MicrophoneTick(tick_owner.clone())) => {
                                if result.is_err() { break; }
                            }
                        }
                    }
                }
            }
        });
        self.start_capture_owner(owner, input, device);
        Ok(())
    }
    fn microphone_check_ready(&mut self, callback_observed: bool) {
        let Some(check) = &mut self.microphone_check else { return; };
        self.state.microphone_test.callback_observed |= callback_observed;
        if self.stop_requested.is_none() {
            check.listening_at.get_or_insert_with(Instant::now);
            self.capture_starting = false;
            self.state.microphone_test.state = "listening".into();
            self.state.audio_state.state = "capturing".into();
        }
    }
    fn measure_microphone_check(&mut self) {
        let Some(check) = &self.microphone_check else { return; };
        let stats = check.meter.stats();
        let (peak, rms) = check.meter.levels();
        let test = &mut self.state.microphone_test;
        test.samples = stats.samples;
        test.nonzero = stats.nonzero;
        test.peak = peak;
        test.rms = rms;
        test.elapsed_ms = check.listening_at.map(|at| at.elapsed().as_millis() as u64).unwrap_or(0);
        if let Some(gate) = &self.capture_start_gate {
            test.callback_observed |= gate.callback_observed();
        }
    }
    fn microphone_tick(&mut self, owner: &Identity) {
        if !self.microphone_check.as_ref().is_some_and(|check| &check.owner == owner) { return; }
        self.measure_microphone_check();
        let check = self.microphone_check.as_ref().unwrap();
        if self.stop_requested.is_some() { return; }
        if check.listening_at.is_some_and(|at| at.elapsed() >= Duration::from_secs(5)) {
            self.stop_microphone_check("completed", None);
        } else if check.listening_at.is_none() && check.started_at.elapsed() >= Duration::from_secs(15) {
            self.stop_microphone_check("failed", Some("系统麦克风启动超时，正在等待原生停采回执".into()));
        }
    }
    fn stop_microphone_check(&mut self, outcome: &'static str, detail: Option<String>) {
        let Some(check) = &mut self.microphone_check else { return; };
        // A later cleanup cannot downgrade an actual capture failure.
        if check.outcome != "failed" { check.outcome = outcome; }
        if detail.is_some() { self.state.microphone_test.detail = detail; }
        // Keep scalar ingress open until native stop is proven: a callback may
        // already have passed the gate when cancellation revokes admission.
        // Closing first would turn its admitted append into a synthetic fault.
        self.measure_microphone_check();
        self.request_stop(outcome != "completed");
    }
    fn microphone_check_failed(&mut self, id: Identity, failure: audio::CaptureStartFailure) {
        self.state.microphone_test.detail = Some(failure.message.clone());
        if let Some(check) = &mut self.microphone_check {
            if check.outcome != "cancelled" { check.outcome = "failed"; }
        }
        if failure.capture_stopped {
            self.microphone_check_stopped(id, audio::StopReceipt {
                capture_stopped: true, cancelled: self.stop_requested == Some(true), error: Some(failure.message),
            });
        } else {
            self.capture_starting = false;
            if let Some(handle) = failure.pending {
                self.capture_canceller = Some(handle.canceller());
                self.capture = Some(handle);
            }
            self.request_stop(true);
        }
    }
    fn microphone_check_stopped(&mut self, id: Identity, receipt: audio::StopReceipt) {
        if self.capture_id.as_ref() != Some(&id) { return; }
        self.measure_microphone_check();
        if !receipt.capture_stopped {
            if let Some(check) = &mut self.microphone_check {
                check.outcome = "failed";
            }
            self.state.microphone_test.state = "stopping".into();
            self.state.microphone_test.detail = receipt.error.clone();
            self.state.audio_state = AudioState { state: "error".into(), detail: receipt.error };
            return; // Retain the only owner until its real terminal receipt.
        }
        let Some(check) = self.microphone_check.take() else { return; };
        check.meter.close();
        // Closing drains any in-flight meter ingress before the final sample
        // snapshot, while the receipt independently proves native stop/join.
        let stats = check.meter.stats();
        self.state.microphone_test.samples = stats.samples;
        self.state.microphone_test.nonzero = stats.nonzero;
        self.state.microphone_test.state = if receipt.error.is_some() && check.outcome != "cancelled" {
            "failed"
        } else { check.outcome }.into();
        self.state.microphone_test.stopped = true;
        if receipt.error.is_some() { self.state.microphone_test.detail = receipt.error; }
        self.capture_id = None;
        self.capture_input = None;
        self.capture_start_gate = None;
        self.capture_starting = false;
        self.capture = None;
        self.capture_canceller = None;
        self.stop_requested = None;
        self.recording_at = None;
        self.drain_at = None;
        #[cfg(feature = "native-keyboard-validation")]
        { self.synthetic_stop = None; }
        self.state.audio_state = AudioState { state: "stopped".into(), detail: None };
    }
    fn begin_capture(&mut self, source: InputSource) {
        if let Err(error) = self.capture_admitted() { self.consume_source(source); self.state.document.error = Some(error); return; }
        if source == InputSource::Shortcut && !self.saved.settings.enabled { self.consume_source(source); return; }
        if self.microphone_check.is_some() {
            self.consume_source(source);
            return;
        }
        if source != InputSource::Ui && !self.source_authorized(source, self.source_generation(source)) {
            return;
        }
        if self.saved.settings.asr_model.is_none() {
            self.consume_source(source);
            self.state.document.error = Some("请先在语音键盘设置中选择当前模式的 ASR 模型".into());
            if let Err(error) = self.show() {
                self.state.document.error = Some(format!("请先选择 ASR 模型；字幕窗口打开失败：{error}"));
            }
            return;
        }
        if self.suspended
            || self.stop_flag.load(Ordering::Acquire)
            || self.pending_start.is_some()
            || self.capture_id.is_some()
            || self.state.document.phase != Phase::Idle
        {
            if source == InputSource::Ui {
                self.state.document.notice = Some("请等待当前处理完成或取消".into());
            }
            return;
        }
        if source.hardware()
            && (!self.hardware_armed
                || !self.saved.settings.external_keyboard_enabled
                || !self.saved.settings.k4_enabled
                || self.state.device.policy_state != "applied")
        {
            self.consume_source(source);
            self.state.document.notice = Some("键盘尚未准备就绪，请稍候重试".into());
            return;
        }
        if self.editor.is_some() {
            self.defer_editor(EditorOperation::Capture {
                source,
                generation: self.source_generation(source),
            });
            return;
        }
        let streaming = if self.saved.settings.asr_mode == AsrMode::Streaming {
            let input = self
                .asr
                .as_ref()
                .filter(|input| input.is_accepting())
                .cloned();
            let Some(input) = input else {
                if self.asr.is_some() {
                    self.cancel_asr();
                }
                self.queue_capture_start(source);
                return;
            };
            Some(input)
        } else {
            None
        };
        let selection = self
            .text_cursor
            .as_ref()
            .filter(|cursor| {
                !self.state.document.closed
                    && Some(cursor.document.as_str()) == self.state.document.document_id.as_deref()
                    && cursor.revision == self.state.document.revision
            })
            .map(|cursor| cursor.selection);
        let id = match self.state.document.begin_recording_at(selection) {
            Ok(id) => id,
            Err(error) => {
                self.consume_source(source);
                self.state.document.error = Some(error);
                return;
            }
        };
        self.consume_source(source);
        self.operation_source = source;
        self.last_capture_stats = None;
        self.capture_id = Some(id.clone());
        self.asr_document = streaming.as_ref().map(|_| id.clone());
        let input: audio::CaptureInput = if let Some(input) = streaming {
            input.into()
        } else {
            let buffer = audio::BufferedCapture::new();
            let input = buffer.input();
            self.buffered_capture = Some((id.clone(), buffer));
            input
        };
        self.capture_input = Some((id.clone(), input.clone()));
        self.capture_starting = true;
        self.stop_requested = None;
        self.recording_at = Some(Instant::now());
        self.drain_at = None;
        self.state.audio_state = AudioState {
            state: "starting".into(),
            detail: None,
        };
        if let Err(error) = self.show() {
            self.state.document.visible = false;
            self.capture_failed(
                id.clone(),
                audio::CaptureStartFailure {
                    message: error,
                    capture_stopped: true,
                    pending: None,
                },
            );
            return;
        }
        let device = self.saved.settings.audio_device_id.clone();
        self.start_capture_owner(id, input, device);
    }
    fn start_capture_owner(&mut self, id: Identity, input: audio::CaptureInput, device: Option<String>) {
        let lease = match self.capture_safety.claim_for_host(&self.stop_flag) {
            Ok(lease) => lease,
            Err(message) => {
                self.capture_failed(id, audio::CaptureStartFailure { message, capture_stopped: true, pending: None });
                return;
            }
        };
        #[cfg(feature = "native-keyboard-validation")]
        if self.synthetic_input {
            let control = Arc::new(std::sync::atomic::AtomicU8::new(0));
            self.synthetic_stop = Some(control.clone());
            let local = self.microphone_check.is_some();
            if !local {
                self.capture_starting = false;
                self.state.audio_state.state = "capturing".into();
            }
            let tx = self.tx.clone();
            let stop_hold = self.synthetic_stop_hold.clone();
            let stopped_lease = lease.clone();
            tauri::async_runtime::spawn(async move {
                let mut error = None;
                let mut first = true;
                while control.load(Ordering::Acquire) == 0 {
                    let samples: Vec<i16> = (0..960)
                        .map(|n| ((n as f64 * 0.1).sin() * 2000.0) as i16)
                        .collect();
                    if let Err(message) = input.append_pcm16_16k(&samples) {
                        error = Some(message);
                        break;
                    }
                    if local && first {
                        first = false;
                        let _ = tx.send(Message::SyntheticInputReady(id.clone())).await;
                    }
                    tokio::time::sleep(Duration::from_millis(60)).await;
                }
                // Native capture is not considered released merely because
                // stop was requested. Hold its proof to exercise cancellation,
                // timeout, and late owner release through the production Actor.
                while stop_hold.load(Ordering::Acquire) {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                let receipt = audio::StopReceipt {
                    capture_stopped: true,
                    cancelled: control.load(Ordering::Acquire) == 2,
                    error,
                };
                stopped_lease.confirm_stopped();
                let _ = tx.send(Message::CaptureStopped(id, receipt)).await;
            });
            return;
        }
        let gate = audio::CaptureStartGate::new();
        self.capture_start_gate = Some(gate.clone());
        let ready_tx = self.urgent.clone();
        let error_tx = self.urgent.clone();
        let error_id = id.clone();
        let stopped_tx = self.urgent.clone();
        let stopped_id = id.clone();
        let ready_id = id.clone();
        let stopped_lease = lease.clone();
        let ready_lease = lease.clone();
        if let Err(message) = audio::NativeCapture::spawn_start(
            device,
            gate,
            input,
            move |error| {
                let _ = error_tx.send(Message::CaptureError(error_id.clone(), error));
            },
            move |receipt| {
                if receipt.capture_stopped { stopped_lease.confirm_stopped(); }
                let _ = stopped_tx.send(Message::CaptureStopped(stopped_id.clone(), receipt));
            },
            move |result| {
                if result.as_ref().is_err_and(|failure| failure.capture_stopped) { ready_lease.confirm_stopped(); }
                let _ = ready_tx.send(Message::CaptureReady(ready_id, result));
            },
        ) {
            lease.confirm_stopped(); // Relay thread creation failed before native start.
            self.capture_failed(
                id,
                audio::CaptureStartFailure {
                    message,
                    capture_stopped: true,
                    pending: None,
                },
            );
        }
    }
    fn queue_capture_start(&mut self, source: InputSource) {
        if let Err(error) = self.capture_admitted() { self.consume_source(source); self.state.document.error = Some(error); return; }
        // Connect only for this explicit request. No microphone opens until
        // the same source, hold generation and document remain authorized.
        if self.opening {
            self.cancel_asr();
        }
        if self.state.document.closed {
            self.state.document = Document::default();
        }
        if self.state.document.text.is_empty() && self.state.document.original_text.is_empty() {
            // A cancelled empty hold is not the next empty document. Allocate
            // its identity before showing controls so an old close/cancel
            // cannot be accepted against a later pending hold.
            self.state.document.document_id = Some(uuid::Uuid::now_v7().to_string());
        }
        self.close_pending = false;
        self.state.document.error = None;
        self.state.document.notice = None;
        self.pending_start = Some(PendingStart {
            source,
            activation_generation: self.source_generation(source),
            document_id: self.state.document.document_id.clone(),
            revision: self.state.document.revision,
            started_at: Instant::now(),
        });
        self.state.audio_state = AudioState {
            state: "starting".into(),
            detail: Some("正在连接流式识别，尚未打开系统麦克风".into()),
        };
        if let Err(error) = self.show() {
            self.consume_source(source);
            self.cancel_asr();
            self.state.document.error = Some(error);
            return;
        }
        self.retry_at = Instant::now();
        self.prepare_asr();
    }
    fn request_stop(&mut self, cancel: bool) {
        if self.microphone_check.is_some() {
            self.state.microphone_test.state = "stopping".into();
            self.state.microphone_test.stopped = false;
        }
        if let Some(gate) = &self.capture_start_gate {
            gate.stop(cancel);
        }
        #[cfg(feature = "native-keyboard-validation")]
        if let Some(control) = &self.synthetic_stop {
            let mode = if cancel { 2 } else { 1 };
            control.fetch_max(mode, Ordering::AcqRel);
            self.stop_requested = Some(cancel || self.stop_requested == Some(true));
            self.state.audio_state.state = "stopping".into();
            return;
        }
        if cancel {
            if let Some(canceller) = &self.capture_canceller {
                canceller.cancel();
            }
        }
        self.stop_requested = Some(cancel || self.stop_requested == Some(true));
        self.state.audio_state.state = "stopping".into();
        self.drain_at = Some(Instant::now());
        let Some(id) = self.capture_id.clone() else {
            self.state.audio_state.state = "stopped".into();
            return;
        };
        let Some(handle) = self.capture.take() else {
            return;
        };
        let cancel = self.stop_requested == Some(true);
        let tx = self.tx.clone();
        tauri::async_runtime::spawn(async move {
            let result = if cancel {
                handle.cancel().await
            } else {
                handle.stop().await
            };
            let _ = tx.send(Message::CaptureStopped(id, result)).await;
        });
    }
    /// K3 and the subtitle cancel button share this immediate operation path.
    /// Cancellation never waits for editor flush or discards the editor's draft.
    fn cancel_current(&mut self) {
        self.consume_holds();
        if let Some(editor) = &mut self.editor {
            editor.pending = None;
            editor.requested_at = None;
        }
        if self.state.document.phase != Phase::Idle
            || self.capture_id.is_some()
            || self.pending_start.is_some()
        {
            self.abort(false);
        } else {
            self.state.cancel_generation = self.state.cancel_generation.wrapping_add(1);
        }
    }
    fn abort(&mut self, close: bool) {
        if self.microphone_check.is_some() {
            self.stop_microphone_check("cancelled", None);
            return;
        }
        self.state.cancel_generation = self.state.cancel_generation.wrapping_add(1);
        if close {
            self.editor = None;
        } else if let Some(editor) = &mut self.editor {
            editor.pending = None;
            editor.requested_at = None;
        }
        self.pending_origin = None;
        self.close_pending |= close;
        self.pending_start = None;
        let effects = self.state.document.cancel(close);
        self.effects(effects);
        self.cancel_asr();
        if let Some(cancel) = self.polish.take() {
            cancel.cancel();
        }
        self.recording_at = None;
        self.drain_at = None;
        if self.capture_id.is_some() {
            if close {
                self.state.document.visible = true;
            }
            self.request_stop(true);
        } else {
            self.state.audio_state = AudioState {
                state: "stopped".into(),
                detail: self.state.audio_state.detail.clone(),
            };
            if close {
                self.finish_close();
            }
        }
    }
    fn device(&mut self, event: Event) {
        if !self.saved.settings.external_keyboard_enabled { return; }
        match event {
            Event::Rearmed { generation } => {
                // This shares the device FIFO with key frames. An old Status
                // saying `applied` cannot bypass a new release/receipt fence.
                if generation == self.activation_generation && !self.suspended {
                    self.hardware_armed = true;
                    self.state.service.ready = true;
                }
            }
            Event::Failure(error) => {
                // A port can fail while the same live serial owner continues
                // discovery. Keep that instance alive for reconnect; only a
                // terminal bridge/start failure retires its instance epoch.
                self.hardware_lost(Some(error));
                self.state.external_keyboard.state = "connecting".into();
            }
            Event::Status(status) => {
                if !self.state.device.display_available && status.display_available {
                    self.display_sent = None;
                }
                if !status.connected {
                    self.hardware_lost(status.detail.clone());
                }
                self.state.external_keyboard.state = if status.connected { "connected" } else { "connecting" }.into();
                self.state.external_keyboard.detail = status.detail.clone();
                self.state.device = status;
                self.apply_policy();
                if self.close_pending {
                    self.finish_close();
                }
            }
            Event::Frame(value) => {
                let event = value["event"].as_str().unwrap_or("");
                if !event.starts_with("button_") {
                    return;
                }
                let Ok(logical) =
                    nomifun_keyboard::protocol::key(value["button"].as_str().unwrap_or(""))
                else {
                    return;
                };
                if self.suspended || self.stop_flag.load(Ordering::Acquire) || !self.hardware_armed
                {
                    return;
                }
                if logical == 3 {
                    match event {
                        "button_down" => {
                            if self.key_hold.press() {
                                self.begin_capture(InputSource::Hardware);
                            }
                        }
                        "button_up" => {
                            let consumed = self.key_hold.consumed();
                            self.key_hold.release();
                            if let Some(editor) = &mut self.editor {
                                if matches!(
                                    editor.pending,
                                    Some(EditorOperation::Capture { source: InputSource::Hardware, .. })
                                ) {
                                    editor.pending = None;
                                    editor.requested_at = None;
                                }
                            }
                            if self
                                .pending_start
                                .as_ref()
                                .is_some_and(|pending| pending.source.hardware())
                            {
                                self.cancel_asr();
                            }
                            if consumed
                                && self.operation_source == InputSource::Hardware
                                && self.state.document.phase == Phase::Recording
                            {
                                if self.capture_starting {
                                    self.abort(false);
                                } else {
                                    let effects = self.state.document.stop();
                                    self.effects(effects);
                                }
                            }
                        }
                        _ => {}
                    }
                } else if event == "button_click" {
                    match logical {
                        0 => {
                            // K1 controls the workbench only. Subtitle state,
                            // text and the active voice operation stay intact.
                            if let Err(error) = platform::toggle_main(&self.app) {
                                self.state.external_keyboard.detail = Some(format!("无法切换 NomiFun 主窗口：{error}"));
                            }
                        }
                        1 if self.saved.settings.k2_enabled => {
                            self.request_intent(Intent::Beautify, InputSource::Hardware);
                        }
                        2 if self.saved.settings.k3_enabled => {
                            self.cancel_current();
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    fn request_intent(&mut self, intent: Intent, source: InputSource) {
        self.consume_holds();
        if self.suspended || self.stop_flag.load(Ordering::Acquire) {
            self.state.document.notice =
                Some("当前系统会话不可执行字幕操作，请解锁或重新启动服务".into());
            return;
        }
        if self.editor.is_some() {
            self.defer_editor(EditorOperation::Intent(intent, source));
            return;
        }
        if self.pending_start.is_some() {
            self.cancel_asr();
        }
        if self
            .capture_id
            .as_ref()
            .is_some_and(|id| !self.state.document.matches(id))
        {
            self.state.document.notice = Some("等待系统麦克风停止确认".into());
            return;
        }
        match self.state.document.phase {
            Phase::Idle => {
                self.operation_source = source;
                self.pending_origin = None;
            }
            Phase::Recording | Phase::Finalizing => self.pending_origin = Some(source),
            _ => {}
        }
        let effects = self
            .state
            .document
            .request_with_capture(self.capture_id.as_ref(), intent);
        self.effects(effects);
    }
    fn effects(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::StopCapture => self.request_stop(false),
                Effect::CancelCapture => self.request_stop(true),
                Effect::CancelModel => {
                    self.cancel_asr();
                    if let Some(cancel) = self.polish.take() {
                        cancel.cancel();
                    }
                }
                Effect::Show => {
                    if let Err(error) = self.show() {
                        self.state.document.error = Some(error);
                    }
                }
                Effect::Hide => self.hide(),
                Effect::Beautify(id, text) => {
                    if let Some(origin) = self.pending_origin.take() {
                        self.operation_source = origin;
                    }
                    if !self.state.document.visible {
                        if let Err(error) = self.show() {
                            self.state.document.fail(&id, error);
                            continue;
                        }
                    }
                    let Some(model) = self.saved.settings.text_model.clone() else {
                        self.state.document.fail(&id, "请先选择文本模型".into());
                        continue;
                    };
                    let models = self.models.clone();
                    let instruction = self.saved.settings.beautify_instruction.clone();
                    let cancel = CancellationToken::new();
                    self.polish = Some(cancel.clone());
                    let tx = self.tx.clone();
                    tauri::async_runtime::spawn(async move {
                        let delta_tx = tx.clone();
                        let delta_id = id.clone();
                        let mut candidate = String::new();
                        let result = models
                            .polish(
                                &selection(&model),
                                &text,
                                &instruction,
                                cancel,
                                move |delta| {
                                    candidate.push_str(delta);
                                    let _ = delta_tx.try_send(Message::Candidate(
                                        delta_id.clone(),
                                        candidate.clone(),
                                    ));
                                },
                            )
                            .await;
                        let _ = tx.send(Message::Polished(id, result)).await;
                    });
                }

            }
        }
    }
    fn revoke_editor_hardware(&mut self) {
        if let Some(editor) = &mut self.editor {
            if editor
                .pending
                .as_ref()
                .is_some_and(EditorOperation::hardware)
            {
                editor.pending = None;
                editor.requested_at = None;
            }
        }
    }
    fn defer_editor(&mut self, operation: EditorOperation) {
        let Some(editor) = &mut self.editor else {
            return;
        };
        editor.pending = Some(operation);
        editor.requested_at.get_or_insert_with(Instant::now);
        if let Err(error) = self.app.emit(
            "keyboard://edit-flush",
            serde_json::json!({
                "documentId":editor.document,"editId":editor.id,
            }),
        ) {
            editor.pending = None;
            editor.requested_at = None;
            self.consume_holds();
            self.state.document.error = Some(format!("请求保存编辑失败，操作已撤销：{error}"));
        }
    }
    fn finish_editor(&mut self, id: &str, selection: Option<TextSelection>) -> Result<(), String> {
        if self.suspended || self.stop_flag.load(Ordering::Acquire) {
            self.editor = None;
            return Err("当前系统会话不可执行字幕操作".into());
        }
        let editor = self.editor.as_ref().ok_or("编辑已结束，请重新进入编辑")?;
        if editor.id != id || editor.document != self.state.document.document_id {
            return Err("编辑已失效，请操作当前文稿".into());
        }
        if let Some(selection) = selection {
            if let Err(error) = selection.validate(&self.state.document.text) {
                self.consume_holds();
                if let Some(editor) = &mut self.editor {
                    editor.pending = None;
                    editor.requested_at = None;
                }
                return Err(error);
            }
            let document = self
                .state
                .document
                .document_id
                .clone()
                .ok_or("缺少当前文稿")?;
            self.text_cursor = Some(TextCursor {
                document,
                revision: self.state.document.revision,
                selection,
            });
        }
        let editor = self.editor.take().unwrap();
        if editor
            .requested_at
            .is_some_and(|at| at.elapsed() >= Duration::from_secs(15))
        {
            self.consume_holds();
            self.state.document.error = Some("等待编辑保存超时，按键操作已撤销".into());
            return Ok(());
        }
        match editor.pending {
            Some(EditorOperation::Intent(intent, source)) => {
                let enabled = match source {
                    InputSource::Ui => true,
                    InputSource::Shortcut => self.saved.settings.enabled,
                    InputSource::Hardware => self.hardware_armed && self.saved.settings.external_keyboard_enabled && self.saved.settings.k2_enabled,
                };
                if enabled {
                    self.request_intent(intent, source);
                }
            }
            Some(EditorOperation::Capture {
                source,
                generation,
            }) => {
                if self.source_authorized(source, generation) {
                    self.begin_capture(source);
                }
            }
            None => {}
        }
        Ok(())
    }
    fn action(
        &mut self,
        action: &str,
        document: Option<String>,
        revision: Option<u64>,
        cancel_generation: Option<u64>,
        text: Option<String>,
        edit_id: Option<String>,
        selection: Option<TextSelection>,
        test_id: Option<String>,
        audio_device_id: Option<String>,
    ) -> Result<(), String> {
        if matches!(action, "cancel" | "close") {
            if document != self.state.document.document_id {
                return Err("文稿已更新，请操作当前文稿".into());
            }
            if action == "cancel" {
                let generation = cancel_generation.ok_or("缺少有效取消标识")?;
                if generation != self.state.cancel_generation {
                    // A duplicate/late cancel belongs to an already revoked
                    // operation. It cannot cancel the user's later gesture.
                    return Ok(());
                }
            }
        }
        if matches!(action, "beautify" | "restore_original")
            && cancel_generation != Some(self.state.cancel_generation)
        {
            return Err("操作已取消，请重新操作当前文稿".into());
        }
        if matches!(
            action,
            "stop"
                | "beautify"
                | "restore_original"
                | "edit"
                | "edit_begin"
                | "edit_end"
                | "edit_cancel"
        ) && (document != self.state.document.document_id
            || revision != Some(self.state.document.revision))
        {
            return Err("文稿已更新，请操作当前文稿".into());
        }
        match action {
            "microphone_test_start" => self.start_microphone_check(test_id, audio_device_id)?,
            "microphone_test_stop" => {
                if self.microphone_check.as_ref().is_some_and(|check| Some(check.test_id.as_str()) == test_id.as_deref()) {
                    self.stop_microphone_check("cancelled", None);
                }
            }
            "open_settings" => platform::open_keyboard_settings(&self.app).map_err(|error| error.to_string())?,
            "edit_begin" => {
                if self.state.document.phase != Phase::Idle
                    || self.capture_id.is_some()
                    || self.pending_start.is_some()
                    || self.state.audio_state.state != "stopped"
                    || self.close_pending
                    || self.suspended
                    || self.stop_flag.load(Ordering::Acquire)
                {
                    return Err("请先结束当前录音或处理再编辑".into());
                }
                let id = edit_id
                    .filter(|id| !id.is_empty() && id.len() <= 128)
                    .ok_or("缺少有效编辑标识")?;
                if let Some(editor) = &self.editor {
                    if editor.id != id {
                        return Err("当前文稿已有编辑，请先撤销旧编辑状态".into());
                    }
                } else {
                    self.state.document.open_for_edit()?;
                    self.editor = Some(EditorLease {
                        id,
                        document: self.state.document.document_id.clone(),
                        pending: None,
                        requested_at: None,
                    });
                }
            }
            "edit" => {
                if self.capture_id.is_some()
                    || self.pending_start.is_some()
                    || self.state.audio_state.state != "stopped"
                    || self.close_pending
                    || self.suspended
                    || self.stop_flag.load(Ordering::Acquire)
                {
                    return Err("请先结束当前录音或处理再编辑".into());
                }
                match (&self.editor, edit_id.as_deref()) {
                    (Some(editor), Some(id))
                        if editor.id == id
                            && editor.document == self.state.document.document_id => {}
                    (None, None) => {}
                    _ => return Err("编辑已失效，请操作当前文稿".into()),
                }
                self.state.document.edit(text.ok_or("缺少编辑正文")?)?;
            }
            "edit_end" => {
                self.finish_editor(edit_id.as_deref().ok_or("缺少编辑标识")?, selection)?
            }
            "edit_cancel" => {
                let editor = self.editor.as_ref().ok_or("编辑已结束")?;
                if Some(editor.id.as_str()) != edit_id.as_deref()
                    || editor.document != self.state.document.document_id
                {
                    return Err("编辑已失效，请操作当前文稿".into());
                }
                self.editor = None;
            }
            "stop" | "test_stop" => {
                self.consume_holds();
                if let Some(editor) = &mut self.editor {
                    editor.pending = None;
                    editor.requested_at = None;
                }
                if self.pending_start.is_some() {
                    self.cancel_asr();
                    self.state.audio_state = AudioState {
                        state: "stopped".into(),
                        detail: None,
                    };
                }
                let effects = self.state.document.stop();
                self.effects(effects);
            }
            "cancel" => self.cancel_current(),
            "close" => {
                self.consume_holds();
                self.abort(true);
            }
            "beautify" => {
                self.request_intent(Intent::Beautify, InputSource::Ui);
            }
            "restore_original" => {
                self.consume_holds();
                if self.editor.is_some() {
                    return Err("请先保存编辑再恢复原文".into());
                }
                if self.pending_start.is_some() {
                    self.cancel_asr();
                }
                self.state.document.restore()?;
            }
            "toggle_pin" => {
                self.saved.settings.always_on_top = !self.saved.settings.always_on_top;
                self.state.settings = self.saved.settings.clone();
                if let Some(window) = self.app.get_webview_window("keyboard-subtitle") {
                    platform::set_subtitle_topmost(&window, self.saved.settings.always_on_top)
                        .map_err(|e| e.to_string())?;
                }
                self.persist()?;
            }
            "open" => {
                self.close_pending = false;
                self.state.document.open();
                self.show()?;
                self.prepare_asr();
            }
            "reset_position" => {
                let path = platform::position_file(&self.root);
                if path.exists() {
                    std::fs::remove_file(path).map_err(|e| e.to_string())?;
                }
                if let Some(window) = self.app.get_webview_window("keyboard-subtitle") {
                    platform::position_subtitle(&window, false).map_err(|e| e.to_string())?;
                }
            }
            "test_start" => {
                self.capture_admitted()?;
                if self.suspended || self.stop_flag.load(Ordering::Acquire) {
                    return Err("当前系统会话不可录音，请解锁或重新启动服务".into());
                }
                if self.saved.settings.asr_model.is_none() {
                    return Err("请先选择当前模式的 ASR 模型".into());
                }
                if self.state.document.phase != Phase::Idle
                    || self.capture_id.is_some()
                    || self.pending_start.is_some()
                {
                    return Err("请先停止或取消当前操作".into());
                }
                self.close_pending = false;
                self.consume_holds();
                self.retry_at = Instant::now();
                if self.saved.settings.asr_mode == AsrMode::Platform {
                    self.begin_capture(InputSource::Ui);
                } else if self.asr.is_none() {
                    self.queue_capture_start(InputSource::Ui);
                } else {
                    self.begin_capture(InputSource::Ui);
                }
            }
            _ => return Err("未知字幕板操作".into()),
        }
        Ok(())
    }
    fn settings(&mut self, mut settings: Settings) -> Result<(), String> {
        settings.dictionary = settings
            .dictionary
            .into_iter()
            .map(|term| term.trim().to_owned())
            .filter(|term| !term.is_empty())
            .collect();
        settings.validate()?;
        shortcuts::validate(&settings)?;
        let old = self.saved.settings.clone();
        let shortcut_changed = old.enabled != settings.enabled || old.shortcuts != settings.shortcuts;
        let external_changed = old.external_keyboard_enabled != settings.external_keyboard_enabled;
        let retry_external = old == settings && settings.external_keyboard_enabled && self.state.external_keyboard.state == "error";
        let register_shortcuts = shortcut_changed || old == settings;
        let shortcut_checkpoint = self.shortcuts.as_ref().map(shortcuts::Controller::checkpoint);
        if register_shortcuts {
            if let Some(controller) = &mut self.shortcuts {
                match controller.reconfigure(&settings) {
                    Ok(count) => { self.state.shortcut_state.registered = count; self.state.shortcut_state.detail = None; }
                    Err(error) => { self.state.shortcut_state.registered = controller.registered(); self.state.shortcut_state.detail = Some(error.clone()); return Err(error); }
                }
            } else if self.production && settings.enabled {
                return Err(self.state.shortcut_state.detail.clone().unwrap_or("此系统会话无法注册全局快捷键，可使用字幕按钮".into()));
            }
        }
        // Persist before replacing the Actor configuration or cancelling an
        // operation. Failed disk writes restore both saved intent and actual OS
        // registrations; an in-memory echo is not a successful settings save.
        self.saved.settings = settings.clone();
        if let Err(error) = self.persist() {
            self.saved.settings = old.clone();
            if register_shortcuts {
                if let Some(controller) = &mut self.shortcuts {
                    match controller.restore(shortcut_checkpoint.expect("controller checkpoint")) {
                        Ok(count) => self.state.shortcut_state.registered = count,
                        Err(restore) => { self.state.shortcut_state.registered = controller.registered(); self.state.shortcut_state.detail = Some(restore.clone()); return Err(format!("{error}；快捷键恢复失败：{restore}")); }
                    }
                }
            }
            return Err(error);
        }
        if shortcut_changed {
            self.revoke_shortcut_hold();
            if self.operation_source == InputSource::Shortcut && self.capture_id.is_some() { self.abort(false); }
        }
        let audio_changed = old.asr_mode != settings.asr_mode
            || old.asr_model != settings.asr_model
            || old.audio_device_id != settings.audio_device_id
            || old.language != settings.language
            || old.dictionary != settings.dictionary;
        if audio_changed { self.revoke_shortcut_hold(); }
        let hardware_changed = external_changed
            || old.k4_enabled != settings.k4_enabled
            || audio_changed;
        if audio_changed && self.microphone_check.is_some() {
            self.stop_microphone_check("cancelled", None);
        }
        let disabling_hardware = (old.external_keyboard_enabled && !settings.external_keyboard_enabled)
            || (old.k4_enabled && !settings.k4_enabled);
        if disabling_hardware { self.hardware_lost(None); }
        if !settings.enabled && self.pending_origin == Some(InputSource::Shortcut)
        {
            self.state.document.pending_action = None;
            self.pending_origin = None;
        }
        let shortcut_disabled = old.enabled && !settings.enabled
            && self.operation_source == InputSource::Shortcut
            && (self.capture_id.is_some() || self.state.document.phase != Phase::Idle);
        if (audio_changed && (self.capture_id.is_some() || matches!(self.state.document.phase, Phase::Recording | Phase::Finalizing))) || shortcut_disabled {
            self.abort(false);
        } else if audio_changed {
            self.cancel_asr();
        }
        self.saved.settings = settings.clone();
        self.state.settings = settings;
        if external_changed || retry_external { self.request_external(self.saved.settings.external_keyboard_enabled); }
        if audio_changed && self.state.document.phase == Phase::Idle {
            self.state.document.error = None;
            self.state.document.notice = None;
        }
        if hardware_changed {
            self.rearm_hardware();
            self.apply_policy();
        }
        if let Some(window) = self.app.get_webview_window("keyboard-subtitle") {
            platform::set_subtitle_topmost(&window, self.saved.settings.always_on_top)
                .map_err(|e| e.to_string())?;
        }
        self.prepare_asr();
        Ok(())
    }
    fn persist(&self) -> Result<(), String> {
        use std::io::Write;
        let bytes = serde_json::to_vec_pretty(&self.saved).map_err(|e| e.to_string())?;
        let mut temp = tempfile::NamedTempFile::new_in(&self.root).map_err(|e| e.to_string())?;
        let path = self.root.join("keyboard-settings.json");
        temp.write_all(&bytes).map_err(|e| e.to_string())?;
        temp.as_file().sync_all().map_err(|e| e.to_string())?;
        temp.persist(path).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        std::fs::File::open(&self.root)
            .and_then(|dir| dir.sync_all())
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    fn subtitle_window(&self) -> Result<tauri::WebviewWindow, String> {
        let window = if let Some(window) = self.app.get_webview_window("keyboard-subtitle") {
            window
        } else {
            let builder = tauri::WebviewWindowBuilder::new(
                &self.app,
                "keyboard-subtitle",
                tauri::WebviewUrl::App("index.html#/keyboard-subtitle".into()),
            )
            .title("NomiFun 字幕板")
            .inner_size(platform::SUBTITLE_WIDTH, platform::SUBTITLE_HEIGHT)
            .min_inner_size(360.0, 80.0)
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .maximizable(false)
            .minimizable(false)
            .visible(false)
            .focused(false)
            .skip_taskbar(true)
            .always_on_top(self.saved.settings.always_on_top);
            #[cfg(target_os = "macos")]
            let builder = builder.accept_first_mouse(true);
            #[cfg(target_os = "linux")]
            let builder = builder.focusable(false);
            let window = builder.build().map_err(|e| e.to_string())?;
            let mut restored = false;
            if let Ok(bytes) = std::fs::read(platform::position_file(&self.root)) {
                if let Ok((x, y)) = serde_json::from_slice::<(i32, i32)>(&bytes) {
                    if let Some(monitor) = window
                        .available_monitors()
                        .map_err(|e| e.to_string())?
                        .iter()
                        .find(|m| {
                            let p = m.position();
                            let s = m.size();
                            x >= p.x - 320
                                && x < p.x + s.width as i32
                                && y >= p.y
                                && y < p.y + s.height as i32 - 40
                        })
                    {
                        let area = monitor.work_area();
                        let size = tauri::LogicalSize::new(
                            platform::SUBTITLE_WIDTH,
                            platform::SUBTITLE_HEIGHT,
                        )
                        .to_physical::<u32>(monitor.scale_factor());
                        let x = x.clamp(
                            area.position.x,
                            area.position.x + area.size.width.saturating_sub(size.width) as i32,
                        );
                        let y = y.clamp(
                            area.position.y,
                            area.position.y + area.size.height.saturating_sub(size.height) as i32,
                        );
                        window
                            .set_position(tauri::PhysicalPosition::new(x, y))
                            .map_err(|e| e.to_string())?;
                        restored = true;
                    }
                }
            }
            if !restored {
                platform::position_subtitle(&window, true).map_err(|e| e.to_string())?;
            }
            window
        };
        Ok(window)
    }
    fn show(&mut self) -> Result<(), String> {
        let window = self.subtitle_window()?;
        platform::show_without_activation(&window).map_err(|e| e.to_string())?;
        self.state.document.visible = true;
        Ok(())
    }
    fn finish_close(&mut self) {
        self.consume_holds();
        if self.capture_id.is_some() || self.state.audio_state.state == "error" {
            return;
        }
        if self.operation_source == InputSource::Hardware && self.state.device.connected
            && (self.state.device.policy_state != "applied"
                || self.state.device.capture_state != "stopped")
        {
            self.state.document.error = Some("键盘 NONE 策略尚未确认，保持字幕窗口".into());
            return;
        }
        self.close_pending = false;
        self.state.document.cancelled_capture();
        self.hide();
    }
    fn hide(&mut self) {
        self.consume_holds();
        if self.capture_id.is_some() || self.close_pending {
            return;
        }
        if let Some(window) = self.app.get_webview_window("keyboard-subtitle") {
            match platform::hide_subtitle(&window) {
                Ok(()) => {
                    self.editor = None;
                    self.state.document.discard_closed();
                }
                Err(error) => {
                    self.state.document.visible = true;
                    self.state.document.error =
                        Some(format!("操作已完成，隐藏字幕窗失败：{error}"));
                }
            }
        } else {
            self.editor = None;
            self.state.document.discard_closed();
        }
    }
}
#[tauri::command]
pub(crate) fn keyboard_get_state(app: tauri::AppHandle) -> Result<KeyboardState, String> {
    let runtime = app.try_state::<KeyboardRuntime>().ok_or("语音键盘正在初始化，请稍候")?;
    let snapshot = runtime.snapshot.lock().map_err(|_| "语音状态暂不可用")?.clone();
    Ok(snapshot)
}
#[tauri::command]
pub(crate) async fn keyboard_update_settings(
    state: tauri::State<'_, KeyboardRuntime>,
    settings: Settings,
) -> Result<KeyboardState, String> {
    let (tx, rx) = oneshot::channel();
    state
        .tx
        .send(Message::Settings(settings, tx))
        .await
        .map_err(|_| "字幕服务已停止")?;
    rx.await.map_err(|_| "字幕服务已停止")?
}
#[tauri::command]
pub(crate) async fn keyboard_model_catalog(
    state: tauri::State<'_, KeyboardRuntime>,
) -> Result<models::KeyboardModelCatalog, String> {
    state.models.catalog().await
}
#[tauri::command]
pub(crate) async fn keyboard_audio_devices() -> Result<Vec<audio::AudioDevice>, String> {
    tauri::async_runtime::spawn_blocking(audio::input_devices)
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
pub(crate) async fn keyboard_action(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, KeyboardRuntime>,
    action: String,
    document_id: Option<String>,
    revision: Option<u64>,
    cancel_generation: Option<u64>,
    text: Option<String>,
    edit_id: Option<String>,
    selection: Option<TextSelection>,
    test_id: Option<String>,
    audio_device_id: Option<String>,
) -> Result<KeyboardState, String> {
    if window.label() == "keyboard-subtitle"
        && !matches!(
            action.as_str(),
            "stop"
                | "cancel"
                | "beautify"
                | "restore_original"
                | "close"
                | "toggle_pin"
                | "edit_begin"
                | "edit"
                | "edit_end"
                | "edit_cancel"
                | "open_settings"
                | "microphone_test_stop"
        )
    {
        return Err("字幕窗口无权执行该操作".into());
    }
    let (tx, rx) = oneshot::channel();
    state
        .tx
        .send(Message::Action {
            action,
            document: document_id,
            revision,
            cancel_generation,
            text,
            edit_id,
            selection,
            test_id,
            audio_device_id,
            reply: tx,
        })
        .await
        .map_err(|_| "字幕服务已停止")?;
    rx.await.map_err(|_| "字幕服务已停止")?
}
pub(crate) fn window_event(window: &tauri::Window, event: &tauri::WindowEvent) -> bool {
    if window.label() != "keyboard-subtitle" {
        return false;
    }
    if let Some(runtime) = window.app_handle().try_state::<KeyboardRuntime>() {
        match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let state = runtime.snapshot.lock().unwrap().clone();
                let (tx, _) = oneshot::channel();
                let _ = runtime.urgent.send(Message::Action {
                    action: "close".into(),
                    document: state.document.document_id,
                    revision: Some(state.document.revision),
                    cancel_generation: None,
                    text: None,
                    edit_id: None,
                    selection: None,
                    test_id: None,
                    audio_device_id: None,
                    reply: tx,
                });
            }
            tauri::WindowEvent::Moved(position) => {
                let _ = std::fs::write(
                    platform::position_file(&runtime.root),
                    serde_json::to_vec(&(position.x, position.y)).unwrap_or_default(),
                );
            }
            _ => {}
        }
    }
    true
}
pub(crate) fn begin_shutdown(app: &tauri::AppHandle) {
    if let Some(runtime) = app.try_state::<KeyboardRuntime>() {
        runtime.capture_safety.fence_shutdown();
        runtime.stop_flag.store(true, Ordering::Release);
        runtime.external.fence();
    }
}
/// Final file-replacement gate. The host must already own its shared stop flag
/// before calling this, including when voice initialization is still queued.
/// Optional device/plugin/UI state never substitutes for producer stop proof.
pub(crate) fn require_capture_stopped(app: &tauri::AppHandle) -> Result<(), String> {
    let runtime = app.try_state::<KeyboardRuntime>();
    update_pause::require_stopped(runtime.as_ref().map(|runtime| runtime.capture_safety.as_ref()))
}
pub(crate) async fn shutdown_main(app: &tauri::AppHandle) -> Result<(), String> {
    begin_shutdown(app);
    if let Some(runtime) = app.try_state::<KeyboardRuntime>() {
        let (tx, rx) = oneshot::channel();
        let _ = runtime.urgent.send(Message::Shutdown(tx));
        update_pause::await_stopped(&runtime.capture_safety, rx, Duration::from_secs(10)).await?;
        // Once the same native producer proves stopped, optional USB cleanup
        // cannot hold the application or installer hostage. Its owned task is
        // still fenced and joins off-Actor; process exit releases OS resources.
        if let Err(error) = runtime.external.shutdown_optional(Duration::from_millis(500)).await {
            tracing::warn!(%error, "optional external keyboard cleanup did not finish; native capture is stopped");
            let _ = runtime.urgent.send(Message::ExternalSettled(runtime.external.epoch(), Err(error)));
        }
    }
    Ok(())
}
async fn pause_for_maintenance(app: &tauri::AppHandle) -> Result<Option<UpdatePause>, String> {
    let Some(runtime) = app.try_state::<KeyboardRuntime>() else {
        return Ok(None);
    };
    let (pause, stopped) = match UpdatePause::begin(runtime.urgent.clone(), runtime.update_admission.clone(), runtime.capture_safety.clone()) {
        Ok(prepared) => prepared,
        Err(_) if runtime.capture_safety.stopped() => return Ok(None),
        Err(error) => return Err(format!("麦克风停止仍未确认：{error}")),
    };
    // This guard exists before the first await, so a cancelled command future
    // cannot leave the actor permanently suspended.
    let result = update_pause::await_stopped(&runtime.capture_safety, stopped, Duration::from_secs(10)).await;
    match result {
        Ok(()) => Ok(Some(pause)),
        Err(error) => {
            let recovery = pause.cancel().await;
            if runtime.capture_safety.stopped() { return Ok(None); }
            if let Err(recovery) = recovery {
                return Err(format!("{error}；语音输入恢复未确认：{recovery}"));
            }
            Err(error)
        }
    }
}
/// Authenticated external maintenance must be reversible while native stop is
/// unconfirmed. On success desktop_control invokes process exit after replying;
/// it must not recursively stop its own external endpoint or the device owner.
pub(crate) async fn prepare_external_maintenance(app: &tauri::AppHandle) -> Result<(), String> {
    if let Some(pause) = pause_for_maintenance(app).await? {
        // Preflight already proved native stop. shutdown_main repeats that
        // proof before it closes and joins the GUI-owned serial transport.
        shutdown_main(app).await?;
        pause.commit();
    }
    Ok(())
}
pub(crate) async fn prepare_update(app: &tauri::AppHandle) -> Result<Option<UpdatePause>, String> {
    // Until package handoff commits, this is a reversible actor pause. The
    // same transport remains owned so cancellation can send a fresh Rearm.
    pause_for_maintenance(app).await
}
#[cfg(feature = "native-keyboard-validation")]
pub(crate) async fn validation_edit(
    app: &tauri::AppHandle,
    text: String,
    document: Option<String>,
    revision: u64,
    edit_id: Option<String>,
) -> Result<serde_json::Value, String> {
    validation_edit_action(app, "edit", document, revision, Some(text), edit_id, None).await
}
#[cfg(feature = "native-keyboard-validation")]
pub(crate) async fn validation_edit_selection(
    app: &tauri::AppHandle,
    action: &str,
    edit_id: &str,
    document: Option<String>,
    revision: u64,
    selection: Option<TextSelection>,
) -> Result<serde_json::Value, String> {
    validation_edit_action(
        app,
        action,
        document,
        revision,
        None,
        Some(edit_id.into()),
        selection,
    )
    .await
}
#[cfg(feature = "native-keyboard-validation")]
async fn validation_edit_action(
    app: &tauri::AppHandle,
    action: &str,
    document: Option<String>,
    revision: u64,
    text: Option<String>,
    edit_id: Option<String>,
    selection: Option<TextSelection>,
) -> Result<serde_json::Value, String> {
    let runtime = app.state::<KeyboardRuntime>();
    let (reply, result) = oneshot::channel();
    runtime
        .tx
        .send(Message::Action {
            action: action.into(),
            document,
            revision: Some(revision),
            cancel_generation: None,
            text,
            edit_id,
            selection,
            test_id: None,
            audio_device_id: None,
            reply,
        })
        .await
        .map_err(|_| "actor stopped")?;
    serde_json::to_value(result.await.map_err(|_| "actor stopped")??).map_err(|e| e.to_string())
}
/// Submit an explicitly captured UI document request in the isolated fixture.
#[cfg(feature = "native-keyboard-validation")]
pub(crate) async fn validation_document_operation(
    app: &tauri::AppHandle,
    action: &str,
    document: Option<String>,
    revision: u64,
    cancel_generation: Option<u64>,
) -> Result<serde_json::Value, String> {
    if !matches!(action, "beautify" | "restore_original" | "cancel" | "close") {
        return Err("only captured document operations are permitted".into());
    }
    let runtime = app.state::<KeyboardRuntime>();
    let (reply, result) = oneshot::channel();
    runtime.tx.send(Message::Action {
        action: action.into(),
        document,
        revision: Some(revision),
        cancel_generation,
        text: None,
        edit_id: None,
        selection: None,
        test_id: None,
        audio_device_id: None,
        reply,
    }).await.map_err(|_| "actor stopped")?;
    serde_json::to_value(result.await.map_err(|_| "actor stopped")??).map_err(|e| e.to_string())
}

/// Call during single-threaded GUI startup, after all headless entry points.
#[cfg(target_os = "linux")]
pub(crate) unsafe fn configure_gui_backend() {
    unsafe {
        platform::configure_gui_backend();
    }
}
