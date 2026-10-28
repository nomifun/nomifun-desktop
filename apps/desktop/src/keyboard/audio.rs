//! Native OS microphone input. The CPAL stream stays on one owner thread;
//! callers receive a Send control handle and an explicit stop/join result.
#[path = "audio_dsp.rs"]
mod dsp;
#[path = "capture_input.rs"]
mod capture_input;
pub use capture_input::{AudioStats, BufferedCapture, CaptureInput, MeterCapture};
#[cfg(windows)]
#[path = "windows_com.rs"]
mod windows_com;
use cpal::{
    FromSample, SizedSample,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
        mpsc,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDevice {
    pub id: String,
    pub label: String,
    pub is_default: bool,
}

fn device_id(host: &str, name: &str) -> String {
    format!("{host}:{}", hex::encode(name.as_bytes()))
}
pub fn input_devices() -> Result<Vec<AudioDevice>, String> {
    #[cfg(windows)]
    let _com = windows_com::thread_guard()?;
    let host = cpal::default_host();
    let host_name = host.id().name();
    let default = host
        .default_input_device()
        .and_then(|device| device.name().ok());
    let mut names = BTreeMap::<String, usize>::new();
    for device in host.input_devices().map_err(|e| e.to_string())? {
        *names
            .entry(device.name().map_err(|e| e.to_string())?)
            .or_default() += 1;
    }
    Ok(names
        .into_iter()
        .map(|(name, count)| AudioDevice {
            id: device_id(host_name, &name),
            label: if count > 1 {
                format!("{name}（同名设备 {count} 个，请在系统中改名后指定）")
            } else {
                name.clone()
            },
            is_default: default.as_ref() == Some(&name),
        })
        .collect())
}

enum Control {
    Stop { flush: bool },
    Wake,
}
const OPENING: u8 = 0;
const STARTING: u8 = 1;
const PLAYING: u8 = 2;
const PHASE_MASK: u8 = 3;
const STOP_REQUESTED: u8 = 4;
const OWNER_FINISHED: u8 = 8;
const FIRST_INPUT_TIMEOUT: Duration = Duration::from_secs(5);

/// Native phase timings contain no device identity, samples or recognized text.
/// One attempt spans startup, the live owner and the confirmed stop/drain.
struct CaptureTiming {
    attempt: u64,
    started: Instant,
    previous: Instant,
}
impl CaptureTiming {
    fn new() -> Self {
        static NEXT_ATTEMPT: AtomicU64 = AtomicU64::new(1);
        let now = Instant::now();
        Self {
            attempt: NEXT_ATTEMPT.fetch_add(1, Ordering::Relaxed),
            started: now,
            previous: now,
        }
    }
    fn mark(&mut self, phase: &'static str) {
        let now = Instant::now();
        let elapsed_ms = u64::try_from(now.duration_since(self.started).as_millis())
            .unwrap_or(u64::MAX);
        let phase_ms = u64::try_from(now.duration_since(self.previous).as_millis())
            .unwrap_or(u64::MAX);
        tracing::info!(target: "nomifun_keyboard", stage = "native_audio_timing",
            capture_attempt = self.attempt, phase, elapsed_ms, phase_ms,
            "native microphone timing");
        self.previous = now;
    }
}

/// Seed CPAL's existing process-wide COM enumerator before the first hold.
/// This never creates or plays an audio stream. A failed preparation remains
/// an ordinary startup error when the user later requests the microphone.
#[cfg(windows)]
pub fn prepare_runtime() {
    let spawned = std::thread::Builder::new()
        .name("nomifun-audio-runtime-prepare".into())
        .spawn(|| {
            let started = Instant::now();
            let success = windows_com::prepare_lifetime().is_ok();
            let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            tracing::info!(target: "nomifun_keyboard", stage = "native_audio_prepare",
                success, elapsed_ms, "native audio runtime preparation");
        });
    if spawned.is_err() {
        tracing::warn!(target: "nomifun_keyboard", stage = "native_audio_prepare",
            success = false, "native audio runtime preparation was not scheduled");
    }
}

#[derive(Clone, Default)]
struct FirstInput {
    observed: Arc<AtomicBool>,
}
impl FirstInput {
    fn observe(&self, sample_count: usize) -> bool {
        sample_count != 0 && !self.observed.swap(true, Ordering::AcqRel)
    }
}
#[derive(Debug, PartialEq, Eq)]
enum InputStart {
    Ready,
    Stopped { flush: bool },
    Failed(String),
}
fn wait_for_first_input(
    gate: &CaptureStartGate,
    first: &FirstInput,
    fault: &Mutex<Option<String>>,
    control: &mpsc::Receiver<Control>,
    timeout: Duration,
) -> InputStart {
    let deadline = Instant::now() + timeout;
    loop {
        match fault.lock() {
            Ok(error) => {
                if let Some(error) = &*error {
                    return InputStart::Failed(error.clone());
                }
            }
            Err(_) => return InputStart::Failed("麦克风错误状态损坏".into()),
        }
        if gate.stopped() {
            return InputStart::Stopped {
                flush: !gate.cancelled.load(Ordering::Acquire),
            };
        }
        if first.observed.load(Ordering::Acquire)
            && gate.phase.load(Ordering::Acquire) & PHASE_MASK == PLAYING
            && gate.live.load(Ordering::Acquire)
        {
            gate.input_ready.store(true, Ordering::Release);
            if !gate.stopped() {
                return InputStart::Ready;
            }
            continue;
        }
        if Instant::now() >= deadline {
            return InputStart::Failed("系统麦克风启动后未收到音频回调，尚未进入聆听".into());
        }
        match control.recv_timeout(Duration::from_millis(20).min(deadline.saturating_duration_since(Instant::now()))) {
            Ok(Control::Stop { flush }) => return InputStart::Stopped { flush },
            Ok(Control::Wake) | Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return InputStart::Failed("麦克风启动控制通道已关闭".into());
            }
        }
    }
}

/// Created before a blocking driver open, so an early key release/disable can
/// veto a later play without waiting for the driver or for CaptureReady.
#[derive(Clone)]
pub struct CaptureStartGate {
    phase: Arc<AtomicU8>,
    tx: Arc<Mutex<Option<mpsc::SyncSender<Control>>>>,
    live: Arc<AtomicBool>,
    cancelled: Arc<AtomicBool>,
    input_ready: Arc<AtomicBool>,
}
impl CaptureStartGate {
    /// Latched native input-ready proof survives a later stop request.
    pub fn callback_observed(&self) -> bool {
        self.input_ready.load(Ordering::Acquire)
    }
    pub fn new() -> Self {
        Self {
            phase: Arc::new(AtomicU8::new(OPENING)),
            tx: Arc::new(Mutex::new(None)),
            live: Arc::new(AtomicBool::new(true)),
            cancelled: Arc::new(AtomicBool::new(false)),
            input_ready: Arc::new(AtomicBool::new(false)),
        }
    }
    pub fn stop(&self, cancel: bool) {
        if cancel {
            self.cancelled.store(true, Ordering::Release);
        }
        let before = self.phase.fetch_or(STOP_REQUESTED, Ordering::AcqRel);
        // Normal active capture drains its accepted tail after native Drop.
        // During an uncompleted play call, late callbacks are never admitted.
        if cancel || before & PHASE_MASK != PLAYING || !self.input_ready.load(Ordering::Acquire) {
            self.live.store(false, Ordering::Release);
        }
        if let Ok(tx) = self.tx.lock() {
            if let Some(tx) = &*tx {
                let _ = tx.try_send(Control::Stop { flush: !cancel });
            }
        }
    }
    fn bind(&self, control: mpsc::SyncSender<Control>) -> Result<(), String> {
        let mut tx = self.tx.lock().map_err(|_| "麦克风启动控制状态损坏")?;
        if tx.is_some() || self.phase.load(Ordering::Acquire) & OWNER_FINISHED != 0 {
            return Err("麦克风启动 gate 不能重复使用".into());
        }
        *tx = Some(control);
        Ok(())
    }
    fn stopped(&self) -> bool {
        self.phase.load(Ordering::Acquire) & STOP_REQUESTED != 0
    }
    fn admit_play(&self) -> bool {
        // This CAS linearizes start admission against stop's fetch_or. If
        // stop wins while the driver opens, play is never called. If start
        // wins first, stop keeps ownership and closes admission until Drop.
        self.phase
            .compare_exchange(OPENING, STARTING, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
    fn played(&self) {
        let _ = self.phase.compare_exchange(
            STARTING,
            PLAYING,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
}
impl Default for CaptureStartGate {
    fn default() -> Self {
        Self::new()
    }
}
pub struct NativeCapture;
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StopReceipt {
    pub capture_stopped: bool,
    pub cancelled: bool,
    pub error: Option<String>,
}
impl StopReceipt {
    fn unknown(cancelled: bool, message: &str) -> Self {
        Self {
            capture_stopped: false,
            cancelled,
            error: Some(message.into()),
        }
    }
}
pub struct CaptureStartFailure {
    pub message: String,
    pub capture_stopped: bool,
    /// Retain this owner while a blocked OS driver finishes opening. Its
    /// canceller is already set; a late driver can never start live capture.
    pub pending: Option<CaptureHandle>,
}
impl std::fmt::Debug for CaptureStartFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CaptureStartFailure")
            .field("message", &self.message)
            .field("capture_stopped", &self.capture_stopped)
            .field("pending", &self.pending.is_some())
            .finish()
    }
}
impl std::fmt::Display for CaptureStartFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
#[derive(Clone)]
pub struct CaptureCanceller {
    gate: CaptureStartGate,
}
impl CaptureCanceller {
    pub fn cancel(&self) {
        self.gate.stop(true);
    }
}
pub struct CaptureHandle {
    worker: Option<JoinHandle<StopReceipt>>,
    gate: CaptureStartGate,
}
impl CaptureHandle {
    pub fn callback_observed(&self) -> bool {
        self.gate.callback_observed()
    }
    /// Early cancellation can still return an owner handle for its stop proof.
    /// Such a handle is never permission to show the listening state.
    pub fn input_ready(&self) -> bool {
        self.gate.input_ready.load(Ordering::Acquire)
            && self.gate.live.load(Ordering::Acquire)
            && !self.gate.stopped()
    }
    pub fn canceller(&self) -> CaptureCanceller {
        CaptureCanceller {
            gate: self.gate.clone(),
        }
    }
    pub async fn stop(self) -> StopReceipt {
        self.finish(true).await
    }
    pub async fn cancel(self) -> StopReceipt {
        self.finish(false).await
    }
    async fn finish(mut self, flush: bool) -> StopReceipt {
        self.gate.stop(!flush);
        let Some(worker) = self.worker.take() else {
            return StopReceipt::unknown(!flush, "麦克风 owner 不可用");
        };
        let cancelled = self.gate.cancelled.clone();
        let joined = tokio::task::spawn_blocking(move || {
            worker.join().unwrap_or_else(|_| {
                StopReceipt::unknown(
                    cancelled.load(Ordering::Acquire),
                    "麦克风线程异常退出，停止未确认",
                )
            })
        });
        match tokio::time::timeout(Duration::from_secs(3), joined).await {
            Ok(Ok(receipt)) => receipt,
            Ok(Err(_)) => StopReceipt::unknown(
                self.gate.cancelled.load(Ordering::Acquire),
                "麦克风停止任务异常，停止未确认",
            ),
            Err(_) => StopReceipt::unknown(
                self.gate.cancelled.load(Ordering::Acquire),
                "系统麦克风停止未确认；继续等待原生 owner 终态回执",
            ),
        }
    }
}
impl Drop for CaptureHandle {
    fn drop(&mut self) {
        if self.worker.is_some() {
            self.canceller().cancel();
        }
    }
}

impl NativeCapture {
    /// Dispatch the existing blocking start without sharing Tokio's blocking
    /// pool with database/provider startup. This relay never owns a stream.
    pub fn spawn_start(
        device: Option<String>,
        gate: CaptureStartGate,
        input: CaptureInput,
        on_error: impl Fn(String) + Send + Sync + 'static,
        on_stopped: impl Fn(StopReceipt) + Send + Sync + 'static,
        on_ready: impl FnOnce(Result<CaptureHandle, CaptureStartFailure>) + Send + 'static,
    ) -> Result<(), String> {
        Self::spawn_start_with(
            move || Self::start(device, gate, input, on_error, on_stopped),
            on_ready,
        )
    }
    fn spawn_start_with(
        start: impl FnOnce() -> Result<CaptureHandle, CaptureStartFailure> + Send + 'static,
        on_ready: impl FnOnce(Result<CaptureHandle, CaptureStartFailure>) + Send + 'static,
    ) -> Result<(), String> {
        std::thread::Builder::new()
            .name("nomifun-microphone-start-relay".into())
            .spawn(move || on_ready(start()))
            .map(|_| ())
            .map_err(|error| format!("无法创建麦克风启动线程：{error}"))
    }
    /// Blocking entry for the dedicated start relay: driver/permission calls
    /// can block. No stream is created before an explicit K4/test action.
    pub fn start(
        device: Option<String>,
        gate: CaptureStartGate,
        input: CaptureInput,
        on_error: impl Fn(String) + Send + Sync + 'static,
        on_stopped: impl Fn(StopReceipt) + Send + Sync + 'static,
    ) -> Result<CaptureHandle, CaptureStartFailure> {
        #[cfg(windows)]
        let initialize = windows_com::thread_guard;
        #[cfg(not(windows))]
        let initialize = || Ok::<(), String>(());
        Self::start_with_initializer(device, gate, move || input, on_error, on_stopped, initialize)
    }
    fn start_with_initializer<G: 'static>(
        device: Option<String>,
        gate: CaptureStartGate,
        input: impl FnOnce() -> CaptureInput + Send + 'static,
        on_error: impl Fn(String) + Send + Sync + 'static,
        on_stopped: impl Fn(StopReceipt) + Send + Sync + 'static,
        initialize: impl FnOnce() -> Result<G, String> + Send + 'static,
    ) -> Result<CaptureHandle, CaptureStartFailure> {
        let mut timing = CaptureTiming::new();
        let (tx, rx) = mpsc::sync_channel(4);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        gate.bind(tx.clone()).map_err(|message| CaptureStartFailure {
            message,
            capture_stopped: true,
            pending: None,
        })?;
        let active = gate.live.clone();
        let cancellation = gate.cancelled.clone();
        let owner_gate = gate.clone();
        let wake = tx.clone();
        let worker = std::thread::Builder::new()
            .name("nomifun-os-microphone".into())
            .spawn(move || {
                timing.mark("owner_started");
                let fault = Arc::new(Mutex::new(None::<String>));
                let first = FirstInput::default();
                let mut capture_stopped = false;
                let result = (|| -> Result<(), String> {
                    if owner_gate.stopped() {
                        active.store(false, Ordering::Release);
                        capture_stopped = true;
                        let _ = ready_tx.send(Ok(()));
                        return Ok(());
                    }
                    // Initialization cannot own a microphone yet. Report both
                    // startup failure and positive stop proof on this path;
                    // otherwise the actor would retain an already-dead owner.
                    // Keep a successful COM guard until after native Drop.
                    let _native_runtime = match initialize() {
                        Ok(guard) => guard,
                        Err(error) => {
                            active.store(false, Ordering::Release);
                            capture_stopped = true;
                            let _ = ready_tx.send(Err(error.clone()));
                            return Err(error);
                        }
                    };
                    timing.mark("runtime_ready");
                    if owner_gate.stopped() {
                        active.store(false, Ordering::Release);
                        capture_stopped = true;
                        let _ = ready_tx.send(Ok(()));
                        return Ok(());
                    }
                    let input = input();
                    let opened = (|| -> Result<_, String> {
                        let host = cpal::default_host();
                        let device = if let Some(selected) = device {
                            let mut matches = Vec::new();
                            for device in host.input_devices().map_err(|e| e.to_string())? {
                                let name = device.name().map_err(|e| e.to_string())?;
                                if device_id(host.id().name(), &name) == selected {
                                    matches.push(device);
                                }
                            }
                            if matches.len() != 1 {
                                return Err(if matches.is_empty() {
                                    "指定麦克风不可用，请重新选择"
                                } else {
                                    "存在同名麦克风，无法唯一选择；请在系统中改名"
                                }
                                .into());
                            }
                            matches.pop().unwrap()
                        } else {
                            host.default_input_device().ok_or("系统没有默认麦克风")?
                        };
                        timing.mark("device_selected");
                        let supported = device
                            .default_input_config()
                            .map_err(|e| format!("无法打开麦克风配置：{e}"))?;
                        let config: cpal::StreamConfig = supported.clone().into();
                        timing.mark("format_ready");
                        let converter = Arc::new(Mutex::new(dsp::PcmConverter::new(
                            config.sample_rate.0,
                            config.channels as usize,
                        )?));
                        if owner_gate.stopped() {
                            return Ok(None);
                        }
                        let stream = match supported.sample_format() {
                            cpal::SampleFormat::I8 => build::<i8>(
                                &device, &config, &converter, &input, &fault, &wake, &active, &first,
                            ),
                            cpal::SampleFormat::I16 => build::<i16>(
                                &device, &config, &converter, &input, &fault, &wake, &active, &first,
                            ),
                            cpal::SampleFormat::I24 => build::<cpal::I24>(
                                &device, &config, &converter, &input, &fault, &wake, &active, &first,
                            ),
                            cpal::SampleFormat::I32 => build::<i32>(
                                &device, &config, &converter, &input, &fault, &wake, &active, &first,
                            ),
                            cpal::SampleFormat::I64 => build::<i64>(
                                &device, &config, &converter, &input, &fault, &wake, &active, &first,
                            ),
                            cpal::SampleFormat::U8 => build::<u8>(
                                &device, &config, &converter, &input, &fault, &wake, &active, &first,
                            ),
                            cpal::SampleFormat::U16 => build::<u16>(
                                &device, &config, &converter, &input, &fault, &wake, &active, &first,
                            ),
                            cpal::SampleFormat::U32 => build::<u32>(
                                &device, &config, &converter, &input, &fault, &wake, &active, &first,
                            ),
                            cpal::SampleFormat::U64 => build::<u64>(
                                &device, &config, &converter, &input, &fault, &wake, &active, &first,
                            ),
                            cpal::SampleFormat::F32 => build::<f32>(
                                &device, &config, &converter, &input, &fault, &wake, &active, &first,
                            ),
                            cpal::SampleFormat::F64 => build::<f64>(
                                &device, &config, &converter, &input, &fault, &wake, &active, &first,
                            ),
                            other => return Err(format!("麦克风采样格式不受支持：{other}")),
                        }?;
                        timing.mark("stream_built");
                        if !owner_gate.admit_play() {
                            drop(stream);
                            return Ok(None);
                        }
                        stream
                            .play()
                            .map_err(|e| format!("麦克风未能启动，请检查系统隐私权限：{e}"))?;
                        owner_gate.played();
                        timing.mark("play_dispatched");
                        Ok(Some((stream, converter)))
                    })();
                    let (stream, converter) = match opened {
                        Ok(Some(value)) => value,
                        Ok(None) => {
                            active.store(false, Ordering::Release);
                            capture_stopped = true;
                            let _ = ready_tx.send(Ok(()));
                            return Ok(());
                        }
                        Err(error) => {
                            active.store(false, Ordering::Release);
                            capture_stopped = true;
                            let _ = ready_tx.send(Err(error.clone()));
                            return Err(error);
                        }
                    };
                    let mut startup_error = None;
                    let mut stopped_before_ready = false;
                    let flush = match wait_for_first_input(
                        &owner_gate, &first, &fault, &rx, FIRST_INPUT_TIMEOUT,
                    ) {
                        InputStart::Ready => {
                            timing.mark("input_ready");
                            if ready_tx.send(Ok(())).is_err() {
                                active.store(false, Ordering::Release);
                                drop(stream);
                                capture_stopped = true;
                                return Ok(());
                            }
                            loop {
                                if fault.lock().map_or(true, |state| state.is_some()) {
                                    break false;
                                }
                                if owner_gate.stopped() {
                                    break !cancellation.load(Ordering::Acquire);
                                }
                                match rx.recv_timeout(Duration::from_millis(20)) {
                                    Ok(Control::Stop { flush }) => break flush,
                                    Ok(Control::Wake) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                                    Err(mpsc::RecvTimeoutError::Disconnected) => break false,
                                }
                            }
                        }
                        InputStart::Stopped { flush } => {
                            stopped_before_ready = true;
                            flush
                        }
                        InputStart::Failed(error) => {
                            startup_error = Some(error);
                            false
                        }
                    };
                    // Drop closes the native stream and joins its callback machinery.
                    // Only after that boundary may the retained callback PCM be flushed.
                    timing.mark("native_stopping");
                    drop(stream);
                    timing.mark("stream_dropped");
                    capture_stopped = true;
                    active.store(false, Ordering::Release);
                    if let Some(error) = startup_error {
                        // Return a startup failure only after native Drop. If
                        // Drop blocks, the existing outer timeout retains the
                        // pending owner and cannot claim capture has stopped.
                        let _ = ready_tx.send(Err(error.clone()));
                        return Err(error);
                    }
                    if stopped_before_ready {
                        // Ownership acknowledgement, not listening readiness.
                        // CaptureHandle::input_ready remains false on this path.
                        let _ = ready_tx.send(Ok(()));
                    }
                    if let Some(error) = fault.lock().map_err(|_| "麦克风错误状态损坏")?.take()
                    {
                        on_error(error.clone());
                        return Err(error);
                    }
                    let chunks = converter
                        .lock()
                        .map_err(|_| "麦克风转换状态损坏")?
                        .finish(flush && !cancellation.load(Ordering::Acquire))?;
                    for chunk in chunks {
                        if cancellation.load(Ordering::Acquire) {
                            break;
                        }
                        input.append_pcm16_16k(&chunk)?;
                    }
                    timing.mark("tail_drained");
                    Ok(())
                })();
                let receipt = StopReceipt {
                    capture_stopped,
                    cancelled: cancellation.load(Ordering::Acquire),
                    error: result.err(),
                };
                timing.mark("owner_finished");
                if capture_stopped {
                    owner_gate.phase.fetch_or(OWNER_FINISHED, Ordering::AcqRel);
                }
                on_stopped(receipt.clone());
                receipt
            })
            .map_err(|e| CaptureStartFailure {
                message: e.to_string(),
                capture_stopped: true,
                pending: None,
            })?;
        let mut handle = CaptureHandle {
            worker: Some(worker),
            gate,
        };
        match ready_rx.recv_timeout(Duration::from_secs(15)) {
            Ok(Ok(())) => Ok(handle),
            Ok(Err(error)) => {
                // ready Err may precede COM teardown or on_stopped delivery.
                // Joining here unconditionally would escape the 15s deadline.
                let finished = handle.worker.as_ref().is_some_and(|worker| worker.is_finished());
                let stopped = finished && handle
                    .worker
                    .take()
                    .and_then(|worker| worker.join().ok())
                    .is_some_and(|receipt| receipt.capture_stopped);
                Err(CaptureStartFailure {
                    message: error,
                    capture_stopped: stopped,
                    pending: (!finished).then_some(handle),
                })
            }
            Err(_) => {
                handle.canceller().cancel();
                Err(CaptureStartFailure {
                    message: "系统麦克风打开超时；正在等待原生驱动退出，尚未确认停止".into(),
                    capture_stopped: false,
                    pending: Some(handle),
                })
            }
        }
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    converter: &Arc<Mutex<dsp::PcmConverter>>,
    input: &CaptureInput,
    fault: &Arc<Mutex<Option<String>>>,
    wake: &mpsc::SyncSender<Control>,
    live: &Arc<AtomicBool>,
    first: &FirstInput,
) -> Result<cpal::Stream, String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let data_converter = converter.clone();
    let input = input.clone();
    let data_fault = fault.clone();
    let data_wake = wake.clone();
    let data_live = live.clone();
    let data_first = first.clone();
    let error_fault = fault.clone();
    let error_wake = wake.clone();
    let error_live = live.clone();
    device
        .build_input_stream(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                match deliver_input(data, &data_converter, &input, &data_live, &data_first) {
                    Ok(true) => { let _ = data_wake.try_send(Control::Wake); }
                    Ok(false) => {}
                    Err(error) => {
                        data_live.store(false, Ordering::Release);
                        if let Ok(mut state) = data_fault.lock() {
                            *state = Some(error);
                        }
                        let _ = data_wake.try_send(Control::Wake);
                    }
                }
            },
            move |error| {
                error_live.store(false, Ordering::Release);
                if let Ok(mut state) = error_fault.lock() {
                    *state = Some(format!("系统麦克风采集中断：{error}"));
                }
                let _ = error_wake.try_send(Control::Wake);
            },
            Some(Duration::from_secs(2)),
        )
        .map_err(|e| format!("系统麦克风不可用或未授权：{e}"))
}

fn deliver_input<T>(
    data: &[T],
    converter: &Mutex<dsp::PcmConverter>,
    input: &CaptureInput,
    live: &AtomicBool,
    first: &FirstInput,
) -> Result<bool, String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    if data.is_empty() || !live.load(Ordering::Acquire) {
        return Ok(false);
    }
    let samples: Vec<f32> = data.iter().map(|sample| sample.to_sample::<f32>()).collect();
    let chunks = converter.lock().map_err(|_| "麦克风转换状态损坏")?.push(&samples)?;
    for chunk in chunks {
        if !live.load(Ordering::Acquire) {
            break;
        }
        input.append_pcm16_16k(&chunk)?;
    }
    Ok(live.load(Ordering::Acquire) && first.observe(data.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_meter_distinguishes_native_zero_callback_from_resampled_pcm() {
        let meter = MeterCapture::new();
        let input = meter.input();
        let gate = CaptureStartGate::new();
        let first = FirstInput::default();
        let (tx, rx) = mpsc::sync_channel(4);
        gate.bind(tx).unwrap();
        assert!(gate.admit_play());
        gate.played();
        assert!(!gate.callback_observed());
        let converter = Mutex::new(dsp::PcmConverter::new(16_000, 1).unwrap());
        assert!(deliver_input(&[0_i16; 32], &converter, &input, &gate.live, &first).unwrap());
        assert_eq!(wait_for_first_input(&gate, &first, &Mutex::new(None), &rx, Duration::from_secs(1)), InputStart::Ready);
        assert!(gate.callback_observed());
        assert_eq!(meter.stats().samples, 0, "a real callback precedes a complete PCM chunk");
        gate.stop(false);
        assert!(gate.callback_observed(), "native callback proof remains after stop");
        for tail in converter.lock().unwrap().finish(true).unwrap() {
            input.append_pcm16_16k(&tail).unwrap();
        }
        assert_eq!(meter.stats().samples, 32);
        assert_eq!(meter.stats().nonzero, 0);
        assert_eq!(meter.levels(), (0.0, 0.0));
        meter.close();
        assert!(input.append_pcm16_16k(&[7]).is_err());
    }
    #[test]
    fn first_input_wait_requires_play_and_nonempty_native_data() {
        let gate = CaptureStartGate::new();
        let first = FirstInput::default();
        let fault = Mutex::new(None);
        let (tx, rx) = mpsc::sync_channel(4);
        gate.bind(tx).unwrap();
        assert!(!first.observe(0));
        assert!(gate.admit_play());
        assert!(first.observe(1));
        assert!(matches!(wait_for_first_input(&gate, &first, &fault, &rx, Duration::from_millis(20)), InputStart::Failed(_)));
        assert!(!gate.input_ready.load(Ordering::Acquire));
        gate.played();
        assert_eq!(wait_for_first_input(&gate, &first, &fault, &rx, Duration::from_millis(20)), InputStart::Ready);
        assert!(gate.input_ready.load(Ordering::Acquire));
    }
    #[test]
    fn first_input_wait_keeps_preparing_until_a_zero_level_callback_arrives() {
        let gate = CaptureStartGate::new();
        let first = FirstInput::default();
        let fault = Arc::new(Mutex::new(None));
        let (tx, rx) = mpsc::sync_channel(4);
        gate.bind(tx.clone()).unwrap();
        assert!(gate.admit_play());
        gate.played();
        let owner_gate = gate.clone();
        let observed = first.clone();
        let owner_fault = fault.clone();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let owner = std::thread::spawn(move || {
            ready_tx.send(wait_for_first_input(&owner_gate, &observed, &owner_fault, &rx, Duration::from_secs(1))).unwrap();
        });
        assert!(ready_rx.recv_timeout(Duration::from_millis(25)).is_err());
        let zeros = [0_i16; 32];
        let buffer = BufferedCapture::new();
        let converter = Mutex::new(dsp::PcmConverter::new(16_000, 1).unwrap());
        assert!(deliver_input(&zeros, &converter, &buffer.input(), &gate.live, &first).unwrap());
        tx.send(Control::Wake).unwrap();
        assert_eq!(ready_rx.recv_timeout(Duration::from_secs(1)).unwrap(), InputStart::Ready);
        owner.join().unwrap();
        assert!(gate.input_ready.load(Ordering::Acquire));
    }
    #[test]
    fn first_input_wait_stops_even_when_the_control_queue_is_full() {
        for cancel in [false, true] {
            let gate = CaptureStartGate::new();
            let first = FirstInput::default();
            let (tx, rx) = mpsc::sync_channel(4);
            gate.bind(tx.clone()).unwrap();
            assert!(gate.admit_play());
            gate.played();
            for _ in 0..4 { tx.try_send(Control::Wake).unwrap(); }
            gate.stop(cancel);
            assert!(!gate.live.load(Ordering::Acquire));
            assert_eq!(wait_for_first_input(&gate, &first, &Mutex::new(None), &rx, Duration::from_secs(1)), InputStart::Stopped { flush: !cancel });
            first.observe(32);
            assert_eq!(wait_for_first_input(&gate, &first, &Mutex::new(None), &rx, Duration::from_secs(1)), InputStart::Stopped { flush: !cancel });
            assert!(!gate.input_ready.load(Ordering::Acquire));
        }
    }
    #[test]
    fn first_input_wait_fault_and_timeout_never_claim_listening() {
        let gate = CaptureStartGate::new();
        let first = FirstInput::default();
        let (tx, rx) = mpsc::sync_channel(4);
        gate.bind(tx).unwrap();
        assert!(gate.admit_play());
        gate.played();
        assert!(matches!(wait_for_first_input(&gate, &first, &Mutex::new(None), &rx, Duration::from_millis(20)), InputStart::Failed(message) if message.contains("未收到音频回调")));
        first.observe(32);
        assert_eq!(wait_for_first_input(&gate, &first, &Mutex::new(Some("injected native error".into())), &rx, Duration::from_secs(1)), InputStart::Failed("injected native error".into()));
        assert!(!gate.input_ready.load(Ordering::Acquire));
    }
    #[test]
    fn first_input_preserves_the_first_frame_and_subchunk_tail_exactly_once() {
        let buffer = BufferedCapture::new();
        let input = buffer.input();
        let converter = Mutex::new(dsp::PcmConverter::new(16_000, 1).unwrap());
        let first = FirstInput::default();
        let live = AtomicBool::new(true);
        assert!(!deliver_input::<i16>(&[], &converter, &input, &live, &first).unwrap());
        assert!(!first.observed.load(Ordering::Acquire));
        assert!(deliver_input(&[0_i16; 32], &converter, &input, &live, &first).unwrap());
        assert_eq!(input.stats().samples, 0); // readiness never needs a 60 ms upload chunk
        assert!(!deliver_input(&[7_i16; 960], &converter, &input, &live, &first).unwrap());
        for chunk in converter.lock().unwrap().finish(true).unwrap() {
            input.append_pcm16_16k(&chunk).unwrap();
        }
        let samples = buffer.finish().unwrap();
        assert_eq!(samples.len(), 992);
        assert_eq!(&samples[..32], &[0_i16; 32]);
        assert_eq!(&samples[32..], &[7_i16; 960]);
        assert_eq!(input.stats().samples, 992);
        assert_eq!(input.stats().nonzero, 960);
    }
    #[test]
    fn stopping_before_ready_preserves_only_the_accepted_normal_tail() {
        for cancel in [false, true] {
            let gate = CaptureStartGate::new();
            let first = FirstInput::default();
            let (tx, rx) = mpsc::sync_channel(4);
            gate.bind(tx).unwrap();
            assert!(gate.admit_play());
            gate.played();
            let converter = Mutex::new(dsp::PcmConverter::new(16_000, 1).unwrap());
            let buffer = BufferedCapture::new();
            let input = buffer.input();
            assert!(deliver_input(&[13_i16; 32], &converter, &input, &gate.live, &first).unwrap());
            gate.stop(cancel);
            let InputStart::Stopped { flush } = wait_for_first_input(&gate, &first, &Mutex::new(None), &rx, Duration::from_secs(1)) else { panic!() };
            assert!(!gate.input_ready.load(Ordering::Acquire));
            assert!(!deliver_input(&[17_i16; 32], &converter, &input, &gate.live, &first).unwrap());
            for chunk in converter.lock().unwrap().finish(flush && !gate.cancelled.load(Ordering::Acquire)).unwrap() {
                input.append_pcm16_16k(&chunk).unwrap();
            }
            let samples = buffer.finish().unwrap();
            if cancel { assert!(samples.is_empty()); }
            else { assert_eq!(samples, [13_i16; 32]); }
        }
    }
    #[test]
    fn first_input_rejects_invalid_failed_or_stopped_callbacks() {
        let buffer = BufferedCapture::new();
        let input = buffer.input();
        let converter = Mutex::new(dsp::PcmConverter::new(16_000, 1).unwrap());
        let first = FirstInput::default();
        let live = AtomicBool::new(true);
        assert!(deliver_input(&[f32::NAN], &converter, &input, &live, &first).is_err());
        assert!(!first.observed.load(Ordering::Acquire));
        buffer.cancel();
        assert!(deliver_input(&[1_i16; 960], &converter, &input, &live, &first).is_err());
        assert!(!first.observed.load(Ordering::Acquire));
        live.store(false, Ordering::Release);
        assert!(!deliver_input(&[1_i16; 32], &converter, &input, &live, &first).unwrap());
        assert!(!first.observed.load(Ordering::Acquire));
    }
    #[test]
    fn input_ready_handle_rejects_faulted_or_stopped_streams() {
        let gate = CaptureStartGate::new();
        let first = FirstInput::default();
        let (tx, rx) = mpsc::sync_channel(4);
        gate.bind(tx).unwrap();
        assert!(gate.admit_play());
        gate.played();
        first.observe(32);
        assert_eq!(wait_for_first_input(&gate, &first, &Mutex::new(None), &rx, Duration::from_secs(1)), InputStart::Ready);
        let handle = CaptureHandle { worker: None, gate: gate.clone() };
        assert!(handle.input_ready());
        gate.live.store(false, Ordering::Release);
        assert!(!handle.input_ready());
        gate.live.store(true, Ordering::Release);
        gate.stop(false);
        assert!(!handle.input_ready());
    }
    fn controlled_handle(
        tx: mpsc::SyncSender<Control>,
        worker: JoinHandle<StopReceipt>,
        live: Arc<AtomicBool>,
        cancelled: Arc<AtomicBool>,
    ) -> CaptureHandle {
        CaptureHandle {
            worker: Some(worker),
            gate: CaptureStartGate {
                phase: Arc::new(AtomicU8::new(PLAYING)),
                tx: Arc::new(Mutex::new(Some(tx))),
                live,
                cancelled,
                input_ready: Arc::new(AtomicBool::new(true)),
            },
        }
    }
    #[test]
    fn runtime_initialization_failure_proves_unopened_capture_stopped() {
        let (stopped_tx, stopped_rx) = mpsc::sync_channel(1);
        let result = NativeCapture::start_with_initializer(
            None,
            CaptureStartGate::new(),
            || panic!("failed initialization must not reach audio input or open a microphone"),
            |_| panic!("no active stream can report a capture error"),
            move |receipt| stopped_tx.send(receipt).unwrap(),
            || Err::<(), _>("injected COM initialization failure".to_string()),
        );
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("initialization failure must reject capture"),
        };
        assert_eq!(error.message, "injected COM initialization failure");
        if let Some(pending) = error.pending {
            assert!(!error.capture_stopped);
            let runtime = tokio::runtime::Runtime::new().unwrap();
            assert!(runtime.block_on(pending.cancel()).capture_stopped);
        } else {
            assert!(error.capture_stopped);
        }
        let terminal = stopped_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(terminal.capture_stopped);
        assert!(!terminal.cancelled);
        assert_eq!(terminal.error.as_deref(), Some("injected COM initialization failure"));
        assert!(stopped_rx.try_recv().is_err());
    }
    #[test]
    fn start_relay_does_not_queue_behind_a_saturated_tokio_blocking_pool() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .max_blocking_threads(1)
            .build()
            .unwrap();
        runtime.block_on(async {
            let (entered_tx, entered_rx) = mpsc::sync_channel(1);
            let (release_tx, release_rx) = mpsc::sync_channel(1);
            let occupied = tokio::task::spawn_blocking(move || {
                entered_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            });
            entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
            let queued_started = Arc::new(AtomicBool::new(false));
            let observed = queued_started.clone();
            let queued = tokio::task::spawn_blocking(move || {
                observed.store(true, Ordering::Release);
            });
            let (ready_tx, ready_rx) = mpsc::sync_channel(1);
            NativeCapture::spawn_start_with(
                || NativeCapture::start_with_initializer(
                    None,
                    CaptureStartGate::new(),
                    || panic!("injected initialization must not reach a real microphone"),
                    |_| panic!("no stream exists"),
                    |_| {},
                    || Err::<(), _>("injected relay initialization".into()),
                ),
                move |result| {
                    let name = std::thread::current().name().map(str::to_owned);
                    ready_tx.send((result, name)).unwrap();
                },
            ).unwrap();
            let ready = ready_rx.recv_timeout(Duration::from_millis(500));
            let still_queued = !queued_started.load(Ordering::Acquire);
            release_tx.send(()).unwrap();
            occupied.await.unwrap();
            queued.await.unwrap();
            let (result, name) = ready.expect("std start relay must not wait for Tokio's blocking pool");
            assert!(still_queued);
            assert_eq!(name.as_deref(), Some("nomifun-microphone-start-relay"));
            assert_eq!(result.err().expect("injected start must fail").message, "injected relay initialization");
        });
    }
    #[tokio::test]
    async fn startup_error_does_not_join_a_blocked_terminal_callback() {
        let (terminal_tx, terminal_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        let release_rx = Mutex::new(release_rx);
        let (returned_tx, returned_rx) = mpsc::sync_channel(1);
        let gate = CaptureStartGate::new();
        let owner_gate = gate.clone();
        let starting = std::thread::spawn(move || {
            let result = NativeCapture::start_with_initializer(
                None,
                owner_gate,
                || panic!("injected initialization must not reach a real microphone"),
                |_| panic!("no stream exists"),
                move |receipt| {
                    terminal_tx.send(receipt).unwrap();
                    release_rx.lock().unwrap().recv_timeout(Duration::from_secs(2)).unwrap();
                },
                || Err::<(), _>("injected startup failure".into()),
            );
            returned_tx.send(result).unwrap();
        });
        let terminal = terminal_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(terminal.capture_stopped);
        let returned = returned_rx.recv_timeout(Duration::from_millis(250));
        let timely = returned.is_ok();
        if let Ok(Err(failure)) = &returned {
            assert!(!failure.capture_stopped);
            failure.pending.as_ref().expect("unfinished owner must be retained").canceller().cancel();
            assert!(gate.cancelled.load(Ordering::Acquire));
        }
        // Release even on regression, so a failing test cannot strand threads.
        release_tx.send(()).unwrap();
        let result = returned.unwrap_or_else(|_| returned_rx.recv_timeout(Duration::from_secs(1)).unwrap());
        starting.join().unwrap();
        assert!(timely, "startup Err escaped the deadline through worker.join");
        let failure = result.err().expect("injected start must fail");
        assert_eq!(failure.message, "injected startup failure");
        let pending = failure.pending.unwrap();
        let stopped = pending.cancel().await;
        assert!(stopped.capture_stopped);
        assert_eq!(stopped.error.as_deref(), Some("injected startup failure"));
    }
    #[tokio::test]
    async fn stop_while_runtime_opens_cannot_start_late_capture() {
        for cancel in [false, true] {
            let gate = CaptureStartGate::new();
            let owner_gate = gate.clone();
            let (entered_tx, entered_rx) = mpsc::sync_channel(1);
            let (release_tx, release_rx) = mpsc::sync_channel(1);
            let (stopped_tx, stopped_rx) = mpsc::sync_channel(1);
            let starting = std::thread::spawn(move || {
                NativeCapture::start_with_initializer(
                    None,
                    owner_gate,
                    || panic!("a stopped pending start must not open audio or construct its sink"),
                    |_| panic!("no stream exists"),
                    move |receipt| stopped_tx.send(receipt).unwrap(),
                    move || {
                        entered_tx.send(()).unwrap();
                        release_rx.recv_timeout(Duration::from_secs(2)).unwrap();
                        Ok(())
                    },
                )
            });
            entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
            gate.stop(cancel);
            release_tx.send(()).unwrap();
            let handle = starting.join().unwrap().unwrap();
            let receipt = if cancel { handle.cancel().await } else { handle.stop().await };
            assert!(receipt.capture_stopped);
            assert_eq!(receipt.cancelled, cancel);
            assert!(receipt.error.is_none());
            let terminal = stopped_rx.recv_timeout(Duration::from_secs(1)).unwrap();
            assert!(terminal.capture_stopped && terminal.error.is_none());
            assert_eq!(terminal.cancelled, cancel);
            assert!(!gate.live.load(Ordering::Acquire));
        }
    }
    #[tokio::test]
    async fn repeated_early_release_and_startup_failure_release_each_native_owner() {
        // Exercise the real start relay/owner/receipt path without reaching
        // CPAL. A fresh key hold must not inherit a previous gate or queued
        // control message, including when stop cannot enqueue its wakeup.
        for round in 0..200 {
            let gate = CaptureStartGate::new();
            let starting_gate = gate.clone();
            let (entered_tx, entered_rx) = mpsc::sync_channel(1);
            let (release_tx, release_rx) = mpsc::sync_channel(1);
            let (terminal_tx, terminal_rx) = mpsc::sync_channel(2);
            let (ready_tx, ready_rx) = mpsc::sync_channel(1);
            let fail_initialization = round % 5 == 0;
            NativeCapture::spawn_start_with(
                move || NativeCapture::start_with_initializer(
                    None,
                    starting_gate,
                    || panic!("released startup must not construct a sink or open the microphone"),
                    |_| panic!("an unopened capture cannot produce an input fault"),
                    move |receipt| { terminal_tx.send(receipt).unwrap(); },
                    move || {
                        entered_tx.send(()).unwrap();
                        release_rx.recv_timeout(Duration::from_secs(2)).unwrap();
                        if fail_initialization { Err("injected initialization failure".into()) }
                        else { Ok(()) }
                    },
                ),
                move |result| { ready_tx.send(result).unwrap(); },
            ).unwrap();
            entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
            {
                let control = gate.tx.lock().unwrap();
                let control = control.as_ref().unwrap();
                for _ in 0..4 { control.try_send(Control::Wake).unwrap(); }
            }
            let cancel = round % 2 == 0;
            gate.stop(false);
            if cancel { gate.stop(true); }
            release_tx.send(()).unwrap();
            let result = ready_rx.recv_timeout(Duration::from_secs(1)).unwrap();
            match result {
                Ok(handle) => {
                    assert!(!fail_initialization);
                    assert!(!handle.input_ready());
                    let receipt = if cancel { handle.cancel().await } else { handle.stop().await };
                    assert!(receipt.capture_stopped);
                    assert_eq!(receipt.cancelled, cancel);
                    assert!(receipt.error.is_none());
                }
                Err(failure) => {
                    assert!(fail_initialization);
                    if let Some(handle) = failure.pending {
                        assert!(!failure.capture_stopped);
                        assert!(handle.stop().await.capture_stopped);
                    } else { assert!(failure.capture_stopped); }
                }
            }
            let terminal = terminal_rx.recv_timeout(Duration::from_secs(1)).unwrap();
            assert!(terminal.capture_stopped);
            assert_eq!(terminal.cancelled, cancel);
            assert_eq!(terminal.error.is_some(), fail_initialization);
            assert!(terminal_rx.try_recv().is_err(), "exactly one native terminal per round");
            assert!(gate.phase.load(Ordering::Acquire) & OWNER_FINISHED != 0);
            assert!(!gate.admit_play(), "a released owner must never play late");
            assert!(!gate.live.load(Ordering::Acquire));
        }
    }
    #[test]
    fn stop_during_blocked_driver_open_wins_before_play_admission() {
        let gate = CaptureStartGate::new();
        let owner_gate = gate.clone();
        let (tx, rx) = mpsc::sync_channel(4);
        gate.bind(tx).unwrap();
        let (entered_tx, entered_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        let played = Arc::new(AtomicBool::new(false));
        let observed = played.clone();
        let opening = std::thread::spawn(move || {
            // Simulated blocking driver operation; the production stream uses
            // this exact admission CAS only after build_input_stream returns.
            entered_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            if owner_gate.admit_play() {
                observed.store(true, Ordering::Release);
                owner_gate.played();
            }
        });
        entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        gate.stop(false);
        release_tx.send(()).unwrap();
        opening.join().unwrap();
        assert!(!played.load(Ordering::Acquire));
        assert!(!gate.live.load(Ordering::Acquire));
        assert!(matches!(rx.recv().unwrap(), Control::Stop { flush: true }));
    }
    #[test]
    fn stopping_an_admitted_play_never_reopens_pcm_admission() {
        for cancel in [false, true] {
            let gate = CaptureStartGate::new();
            assert!(gate.admit_play());
            gate.stop(cancel);
            gate.played();
            assert!(gate.stopped());
            assert!(!gate.live.load(Ordering::Acquire));
            assert_eq!(gate.cancelled.load(Ordering::Acquire), cancel);
        }
    }
    #[cfg(windows)]
    #[test]
    fn device_metadata_survives_short_lived_calling_threads() {
        // No microphone is opened. CPAL's process-global enumerator must
        // remain usable after each caller's COM apartment/thread exits.
        for _ in 0..3 {
            std::thread::spawn(input_devices)
                .join()
                .expect("metadata worker")
                .expect("live process COM enumerator");
        }
        input_devices().expect("enumerator remains usable on another thread");
    }
    #[tokio::test]
    async fn stop_waits_for_owner_release_and_requests_tail_flush() {
        let (tx, rx) = mpsc::sync_channel(4);
        let stopped = Arc::new(AtomicBool::new(false));
        let observed = stopped.clone();
        let worker = std::thread::spawn(move || {
            assert!(matches!(rx.recv().unwrap(), Control::Stop { flush: true }));
            observed.store(true, Ordering::Release);
            StopReceipt {
                capture_stopped: true,
                cancelled: false,
                error: None,
            }
        });
        controlled_handle(tx, worker, Arc::new(AtomicBool::new(true)), Arc::new(AtomicBool::new(false)))
        .stop()
        .await;
        assert!(stopped.load(Ordering::Acquire));
    }
    #[tokio::test]
    async fn cancel_stops_admission_and_joins_without_flushing() {
        let (tx, rx) = mpsc::sync_channel(4);
        let live = Arc::new(AtomicBool::new(true));
        let observed = live.clone();
        let worker = std::thread::spawn(move || {
            assert!(matches!(rx.recv().unwrap(), Control::Stop { flush: false }));
            assert!(!observed.load(Ordering::Acquire));
            StopReceipt {
                capture_stopped: true,
                cancelled: true,
                error: None,
            }
        });
        controlled_handle(tx, worker, live, Arc::new(AtomicBool::new(false)))
        .cancel()
        .await;
    }
    #[tokio::test]
    async fn a_processing_failure_does_not_erase_native_stop_proof() {
        let (tx, rx) = mpsc::sync_channel(4);
        let worker = std::thread::spawn(move || {
            let _ = rx.recv();
            StopReceipt {
                capture_stopped: true,
                cancelled: false,
                error: Some("tail upload failed".into()),
            }
        });
        let receipt = controlled_handle(tx, worker, Arc::new(AtomicBool::new(true)), Arc::new(AtomicBool::new(false)))
        .stop()
        .await;
        assert!(receipt.capture_stopped);
        assert!(receipt.error.is_some());
    }
    #[tokio::test]
    async fn cancel_can_upgrade_an_already_queued_stop() {
        let (tx, rx) = mpsc::sync_channel(4);
        let live = Arc::new(AtomicBool::new(true));
        let cancelled = Arc::new(AtomicBool::new(false));
        let observed = cancelled.clone();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            assert!(matches!(rx.recv().unwrap(), Control::Stop { flush: true }));
            let _ = started_tx.send(());
            release_rx.recv().unwrap();
            StopReceipt {
                capture_stopped: true,
                cancelled: observed.load(Ordering::Acquire),
                error: None,
            }
        });
        let handle = controlled_handle(tx, worker, live, cancelled);
        let canceller = handle.canceller();
        let stopping = tokio::spawn(handle.stop());
        started_rx.await.unwrap();
        canceller.cancel();
        release_tx.send(()).unwrap();
        let receipt = stopping.await.unwrap();
        assert!(receipt.capture_stopped && receipt.cancelled);
    }
    #[tokio::test]
    async fn a_join_timeout_never_claims_stopped_and_late_proof_is_preserved() {
        let (tx, rx) = mpsc::sync_channel(4);
        let (late_tx, late_rx) = tokio::sync::oneshot::channel();
        let worker = std::thread::spawn(move || {
            assert!(matches!(rx.recv().unwrap(), Control::Stop { flush: true }));
            std::thread::sleep(Duration::from_millis(3150));
            let receipt = StopReceipt {
                capture_stopped: true,
                cancelled: false,
                error: None,
            };
            let _ = late_tx.send(receipt.clone());
            receipt
        });
        let receipt = controlled_handle(tx, worker, Arc::new(AtomicBool::new(true)), Arc::new(AtomicBool::new(false)))
        .stop()
        .await;
        assert!(!receipt.capture_stopped);
        assert!(receipt.error.is_some());
        let late = tokio::time::timeout(Duration::from_secs(1), late_rx)
            .await
            .unwrap()
            .unwrap();
        assert!(late.capture_stopped);
    }
}
