//! Joinable serial ownership inside the running desktop process.
//! Commands and events are bounded in-memory queues. This module never launches
//! another process, publishes an endpoint, or captures audio.
use crate::protocol::{self, Parser};
use anyhow::{Context, Result, bail, ensure};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::OpenOptions,
    io::{Read, Write},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

const WAITING_RELEASE_DETAIL: &str = "键盘采音已关闭；等待旧按键释放";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceStatus {
    pub connected: bool,
    pub id: Option<String>,
    pub port: Option<String>,
    pub firmware: Option<String>,
    pub policy_state: String,
    pub capture_state: String,
    pub detail: Option<String>,
    #[serde(default)]
    pub native_protocol: bool,
    #[serde(default)]
    pub display_available: bool,
    #[serde(default)]
    pub display_state: Option<String>,
    #[serde(default)]
    pub display_detail: Option<String>,
}
impl Default for DeviceStatus {
    fn default() -> Self {
        Self {
            connected: false,
            id: None,
            port: None,
            firmware: None,
            policy_state: "pending".into(),
            capture_state: "unknown".into(),
            detail: Some("等待设备".into()),
            native_protocol: false,
            display_available: false,
            display_state: None,
            display_detail: None,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Event {
    Status(DeviceStatus),
    Frame(Value),
    Rearmed { generation: u64 },
    Failure(String),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Control {
    Policy { voice_key: String },
    Rearm { generation: u64 },
    Display { state: String },
}

/// A bounded command handle; it cannot outlive the serial worker's ownership.
#[derive(Clone)]
pub struct Commands {
    tx: SyncSender<Control>,
    stop: Arc<AtomicBool>,
}
impl Commands {
    pub fn control(&self, control: Control) -> Result<()> {
        match &control {
            Control::Policy { voice_key } => ensure!(voice_key == "none", "keyboard policy only permits NONE"),
            Control::Rearm { generation } => ensure!(*generation != 0, "invalid rearm generation"),
            Control::Display { .. } => {}
        }
        ensure!(!self.stop.load(Ordering::Acquire), "keyboard transport stopped");
        if let Err(error) = self.tx.try_send(control) {
            // Losing an admission/release command must revoke this transport,
            // never leave an earlier generation authorized indefinitely.
            self.stop.store(true, Ordering::Release);
            bail!("keyboard command queue unavailable: {error}");
        }
        Ok(())
    }
}

/// Owns the thread and exclusive serial lock until teardown has been joined.
/// The GUI calls shutdown only after native capture has reported stopped.
pub struct Transport {
    commands: Commands,
    worker: Option<JoinHandle<Result<()>>>,
}
impl Transport {
    pub fn start() -> Result<(Self, Receiver<Event>)> {
        Self::start_at(crate::ipc::runtime_dir()?, true)
    }
    /// Explicit disposable fixture: no USB enumeration, audio, or endpoint.
    #[cfg(feature = "maintenance-validation")]
    pub fn start_validation(dir: PathBuf) -> Result<(Self, Receiver<Event>)> {
        ensure!(dir.is_dir(), "isolated transport directory must exist");
        Self::start_at(dir, false)
    }
    fn start_at(dir: PathBuf, scan_enabled: bool) -> Result<(Self, Receiver<Event>)> {
        Self::start_with_device(dir, scan_enabled, None)
    }
    fn start_with_device(dir: PathBuf, scan_enabled: bool, device: Option<Device>) -> Result<(Self, Receiver<Event>)> {
        let path = dir.join("owner.lock");
        ensure!(!path.is_symlink(), "owner lock is a symlink");
        let lock = OpenOptions::new().create(true).truncate(false).read(true).write(true).open(path)?;
        lock.try_lock_exclusive().context("keyboard serial owner is already running")?;
        let (tx, rx) = mpsc::sync_channel(64);
        let (events, receiver) = mpsc::sync_channel(40);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let worker = std::thread::Builder::new().name("nomifun-keyboard-serial".into()).spawn(move || {
            #[cfg(feature = "maintenance-validation")]
            let _validation = validation::Worker::new();
            let result = owner_loop(rx, events, worker_stop.clone(), scan_enabled, device);
            worker_stop.store(true, Ordering::Release);
            // File is deliberately retained until after Device and serial I/O
            // have been dropped by owner_loop, even on a failed NONE receipt.
            drop(lock);
            result
        })?;
        Ok((Self { commands: Commands { tx, stop }, worker: Some(worker) }, receiver))
    }
    #[cfg(feature = "maintenance-validation")]
    pub fn validation_owner(&self) -> Option<String> {
        self.worker.as_ref().filter(|worker| !worker.is_finished())
            .map(|worker| format!("{}:{:?}", std::process::id(), worker.thread().id()))
    }
    #[cfg(feature = "maintenance-validation")]
    pub fn validation_last_control(&self) -> Option<Control> {
        self.worker.as_ref().and_then(|worker| validation::last(worker.thread().id()))
    }
    #[cfg(feature = "maintenance-validation")]
    pub fn validation_lock_held(dir: &std::path::Path) -> Result<bool> {
        let lock = OpenOptions::new().read(true).write(true).open(dir.join("owner.lock"))?;
        match lock.try_lock_exclusive() {
            Ok(()) => Ok(false),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(true),
            Err(error) => Err(error.into()),
        }
    }
    pub fn commands(&self) -> Commands { self.commands.clone() }
    pub fn shutdown(&mut self) -> Result<()> {
        self.commands.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.join().map_err(|_| anyhow::anyhow!("keyboard serial worker panicked"))??;
        }
        Ok(())
    }
}
impl Drop for Transport {
    fn drop(&mut self) {
        // Drop is a final backstop, not an asynchronous or detached teardown.
        let _ = self.shutdown();
    }
}
trait Wire: Read + Write + Send {}
impl<T: Read + Write + Send> Wire for T {}
fn validate_device_id(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty()
            && id.len() <= 128
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.:".contains(c)),
        "invalid stable device id"
    );
    Ok(())
}

struct Device {
    port: Box<dyn Wire>,
    parser: Parser,
    epoch: u32,
    identity: bool,
    capabilities: bool,
    heartbeat: Instant,
    opened: Instant,
    request: u32,
    pending: Option<(u32, Instant, Option<String>)>,
    policy_again: bool,
    rearm_latest: Option<u64>,
    rearm_request: Option<(u32, u64)>,
    rearmed_ready: Option<u64>,
    display_next: Option<String>,
    pressed: [bool; 4],
    click_armed: [bool; 4],
    status: DeviceStatus,
}
impl Device {
    fn send(&mut self, value: Value) -> Result<()> {
        self.port.write_all(&protocol::command(value)?)?;
        Ok(())
    }
    fn policy(&mut self) -> Result<()> {
        if !self.capabilities {
            self.status.policy_state = "error".into();
            self.status.capture_state = "unknown".into();
            self.status.detail = Some("设备缺少原生四键能力确认".into());
            return Ok(());
        }
        self.status.policy_state = "pending".into();
        self.status.capture_state = "stopping".into();
        if self.pending.is_some() {
            self.policy_again = true;
            return Ok(());
        }
        self.request = self
            .request
            .checked_add(1)
            .context("device request ids exhausted; reconnect required")?;
        self.send(json!({"event":"nk_policy","epoch":self.epoch,"request_id":self.request,"voice_key":"none"}))?;
        self.pending = Some((self.request, Instant::now(), None));
        self.rearm_request = self
            .rearm_latest
            .map(|generation| (self.request, generation));
        self.pressed = [false; 4];
        self.click_armed = [false; 4];
        self.status.policy_state = "pending".into();
        self.status.capture_state = "stopping".into();
        Ok(())
    }
    fn reset_rearm(&mut self) {
        self.rearm_latest = None;
        self.rearm_request = None;
        self.rearmed_ready = None;
    }
    fn rearm(&mut self, generation: u64) -> Result<()> {
        ensure!(generation != 0, "invalid rearm generation");
        if self.rearm_latest.is_some_and(|latest| generation <= latest) {
            // An exact retry may repeat only a completed proof, never a new
            // policy transaction or a proof for an older queued request.
            if self.rearm_latest == Some(generation)
                && self
                    .rearm_request
                    .is_some_and(|(_, current)| current == generation)
                && !self.none_pending()
                && self.status.policy_state == "applied"
            {
                self.rearmed_ready = Some(generation);
            }
            return Ok(());
        }
        self.rearm_latest = Some(generation);
        self.rearmed_ready = None;
        self.policy()
    }
    fn apply(&mut self, value: &Value) -> Result<bool> {
        ensure!(value["epoch"] == self.epoch, "stale policy receipt epoch");
        let Some((request, _, kind)) = &self.pending else {
            return Ok(false);
        };
        if value["request_id"] != *request {
            return Ok(false);
        }
        ensure!(
            kind.is_none(),
            "NONE receipt conflicts with display request"
        );
        ensure!(
            value["voice_key"] == "none"
                && value["capture_stopped"] == true
                && value["waiting_release"].is_boolean(),
            "invalid NONE policy receipt"
        );
        ensure!(
            value["session_id"] == 0,
            "NONE must not own a device audio session"
        );
        let applied_rearm = self
            .rearm_request
            .filter(|(id, generation)| id == request && Some(*generation) == self.rearm_latest)
            .map(|(_, generation)| generation);
        self.pending = None;
        self.status.policy_state = "applied".into();
        self.status.capture_state = "stopped".into();
        self.status.detail = if value["waiting_release"] == true {
            Some(WAITING_RELEASE_DETAIL.into())
        } else {
            None
        };
        self.next_control()?;
        if !self.none_pending() {
            self.rearmed_ready = applied_rearm;
        }
        Ok(true)
    }
    fn none_pending(&self) -> bool {
        self.policy_again
            || self
                .pending
                .as_ref()
                .is_some_and(|(_, _, kind)| kind.is_none())
    }
    fn next_control(&mut self) -> Result<()> {
        if std::mem::take(&mut self.policy_again) {
            self.policy()?;
        } else if let Some(state) = self.display_next.take() {
            self.display(&state)?;
        }
        Ok(())
    }
    fn display(&mut self, state: &str) -> Result<()> {
        if ![
            "connecting",
            "sleeping",
            "ready",
            "starting",
            "listening",
            "recognizing",
            "polishing",
            "error",
        ]
        .contains(&state)
        {
            self.status.display_detail = Some("无效的状态屏幕请求".into());
            return Ok(());
        }
        if !self.status.display_available {
            self.status.display_detail = Some("固件尚未提供可用状态屏幕".into());
            return Ok(());
        }
        if self.pending.is_some() {
            self.display_next = Some(state.into());
            return Ok(());
        }
        self.request = self
            .request
            .checked_add(1)
            .context("device request ids exhausted")?;
        self.send(json!({"event":"nk_display","epoch":self.epoch,"request_id":self.request,"state":state}))?;
        self.pending = Some((self.request, Instant::now(), Some(state.into())));
        Ok(())
    }
    fn display_failed(&mut self, message: &str) -> Result<()> {
        self.status.display_available = false;
        self.status.display_detail = Some(message.into());
        self.pending = None;
        self.display_next = None;
        self.next_control()
    }
    fn apply_display(&mut self, value: &Value) -> Result<bool> {
        if value["epoch"] != self.epoch {
            return Ok(false);
        }
        let Some((id, _, Some(state))) = &self.pending else {
            return Ok(false);
        };
        if value["request_id"] != *id {
            return Ok(false);
        }
        if value["state"].as_str() != Some(state.as_str()) || value["applied"] != true {
            self.display_failed("屏幕回执无效")?;
            return Ok(true);
        }
        self.status.display_state = Some(state.clone());
        self.status.display_detail = Some("已提交面板；外观尚待目视确认".into());
        self.pending = None;
        self.next_control()?;
        Ok(true)
    }
    fn button(&mut self, value: &Value, ready: bool) -> Result<bool> {
        ensure!(self.identity, "button before device identity");
        let event = value["event"].as_str().context("missing button event")?;
        let name = value["button"].as_str().context("missing button")?;
        let index = protocol::key(name)?;
        ensure!(self.capabilities, "button before native capabilities");
        ensure!(value["epoch"] == self.epoch, "stale button epoch");
        ensure!(
            value.get("session_id").is_none(),
            "device voice session while key-only policy required"
        );
        if name != "k1" && (!ready || self.none_pending() || self.status.policy_state != "applied")
        {
            return Ok(false);
        }
        match event {
            "button_down" => {
                if self.pressed[index] {
                    return Ok(false);
                }
                self.pressed[index] = true;
                self.click_armed[index] = false;
            }
            "button_up" => {
                if !self.pressed[index] {
                    return Ok(false);
                }
                self.pressed[index] = false;
                self.click_armed[index] = true;
            }
            "button_click" => {
                if !std::mem::take(&mut self.click_armed[index]) {
                    return Ok(false);
                }
            }
            _ => bail!("unknown button event"),
        }
        // A current, ordered native gesture can only leave the firmware
        // after its all-keys-released barrier. Preserve unrelated failures.
        if self.status.detail.as_deref() == Some(WAITING_RELEASE_DETAIL) {
            self.status.detail = None;
        }
        Ok(true)
    }
}
fn emit(subscriber: &mut Option<SyncSender<Event>>, event: Event) -> Result<()> {
    if let Some(tx) = subscriber {
        if tx.try_send(event).is_err() {
            *subscriber = None;
            bail!("desktop key/event queue disconnected or overflowed");
        }
    }
    Ok(())
}
fn discover() -> Result<Option<Device>> {
    for info in serialport::available_ports()? {
        if !matches!(&info.port_type,serialport::SerialPortType::UsbPort(usb) if usb.vid==0x303a&&usb.pid==0x1001)
        {
            continue;
        }
        let port = match serialport::new(&info.port_name, 115200)
            .timeout(Duration::from_millis(20))
            .dtr_on_open(false)
            .open()
        {
            Ok(port) => port,
            Err(_) => continue,
        };
        // A previous connection may have ended inside a frame. Discard its
        // received bytes before the new transport handshake and parser exist.
        port.clear(serialport::ClearBuffer::Input)
            .context("keyboard serial input reset failed")?;
        let mut device = Device {
            port: Box::new(port),
            parser: Parser::default(),
            epoch: 0,
            identity: false,
            capabilities: false,
            heartbeat: Instant::now(),
            opened: Instant::now(),
            request: 0,
            pending: None,
            policy_again: false,
            rearm_latest: None,
            rearm_request: None,
            rearmed_ready: None,
            display_next: None,
            pressed: [false; 4],
            click_armed: [false; 4],
            status: DeviceStatus {
                port: Some(info.port_name),
                ..Default::default()
            },
        };
        device.send(json!({"event":"transport","kind":"usb"}))?;
        device.send(json!({"event":"get_device_info"}))?;
        return Ok(Some(device));
    }
    Ok(None)
}

/// Every terminal failure retains the serial device until bounded NONE cleanup
/// has completed or failed explicitly. Queue overflow follows the same teardown
/// as shutdown; an event delivery failure cannot bypass the device receipt.
fn owner_loop(
    rx: Receiver<Control>,
    events: SyncSender<Event>,
    stop: Arc<AtomicBool>,
    scan_enabled: bool,
    mut device: Option<Device>,
) -> Result<()> {
    let result = owner_active_loop(rx, events, stop, scan_enabled, &mut device);
    if let Err(error) = result {
        if let Some(d) = device.as_mut() {
            if let Err(cleanup) = stop_device(d) {
                return Err(error.context(format!("keyboard NONE cleanup failed: {cleanup:#}")));
            }
        }
        return Err(error);
    }
    Ok(())
}
fn stop_device(d: &mut Device) -> Result<()> {
    d.reset_rearm();
    d.policy()?;
    d.display("sleeping")?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while d.none_pending() || d.status.capture_state != "stopped" || d.status.policy_state != "applied" {
        ensure!(Instant::now() < deadline, "keyboard NONE cleanup deadline exceeded");
        poll_device(d, &mut None, false)?;
    }
    Ok(())
}
fn owner_active_loop(
    rx: Receiver<Control>,
    events: SyncSender<Event>,
    stop: Arc<AtomicBool>,
    scan_enabled: bool,
    device: &mut Option<Device>,
) -> Result<()> {
    let mut subscriber = Some(events);
    let mut ready = false;
    let mut rearm_generation: Option<u64> = None;
    let mut scan = Instant::now() - Duration::from_secs(2);
    let mut shutdown: Option<Instant> = None;
    emit(&mut subscriber, Event::Status(device.as_ref().map(|d| d.status.clone()).unwrap_or_default()))?;
    loop {
        // A separate stop signal cannot be blocked by a full command queue.
        if stop.load(Ordering::Acquire) && shutdown.is_none() {
            shutdown = Some(Instant::now());
            ready = false;
            subscriber = None;
            if let Some(d) = device.as_mut() {
                d.reset_rearm();
                d.policy()?;
                d.display("sleeping")?;
            }
        }
        if shutdown.is_none() {
            for control in rx.try_iter().take(64) {
                #[cfg(feature = "maintenance-validation")]
                validation::record(control.clone());
                match control {
                    Control::Display { state } => {
                        if let Some(d) = device.as_mut() { d.display(&state)?; }
                    }
                    Control::Policy { voice_key } => {
                        ensure!(voice_key == "none", "keyboard policy only permits NONE");
                        ready = false;
                        rearm_generation = None;
                        if let Some(d) = device.as_mut() {
                            d.reset_rearm();
                            d.policy()?;
                        }
                    }
                    Control::Rearm { generation } => {
                        ensure!(generation != 0, "invalid rearm generation");
                        rearm_generation = Some(rearm_generation.map_or(generation, |latest| latest.max(generation)));
                        ready = true;
                        if let Some(d) = device.as_mut() {
                            d.rearm(generation)?;
                            emit(&mut subscriber, Event::Status(d.status.clone()))?;
                            if let Some(generation) = d.rearmed_ready.take() {
                                emit(&mut subscriber, Event::Rearmed { generation })?;
                            }
                        }
                    }
                }
            }
        }
        if shutdown.is_none() && scan_enabled && device.is_none() && scan.elapsed() > Duration::from_secs(2) {
            scan = Instant::now();
            match discover() {
                Ok(mut found) => {
                    if let Some(d) = found.as_mut() { d.rearm_latest = rearm_generation; }
                    *device = found;
                }
                Err(error) => emit(&mut subscriber, Event::Failure(error.to_string()))?,
            }
        }
        if let Some(at) = shutdown {
            ensure!(at.elapsed() < Duration::from_secs(10), "keyboard NONE shutdown deadline exceeded");
            if device.as_ref().is_none_or(|d| !d.none_pending() && d.status.capture_state == "stopped" && d.status.policy_state == "applied") {
                return Ok(());
            }
        }
        let result = if let Some(d) = device.as_mut() {
            poll_device(d, &mut subscriber, ready)
        } else {
            std::thread::sleep(Duration::from_millis(25));
            Ok(())
        };
        if let Err(error) = result {
            ready = false;
            if shutdown.is_some() || subscriber.is_none() { return Err(error); }
            // Revoke desktop capture promptly. If either event queue send
            // fails, owner_loop still owns Device for its final NONE cleanup.
            emit(&mut subscriber, Event::Failure(error.to_string()))?;
            emit(&mut subscriber, Event::Status(DeviceStatus { detail: Some(error.to_string()), ..Default::default() }))?;
            if let Some(mut failed) = device.take() { let _ = stop_device(&mut failed); }
        }
    }
}
fn native_capabilities(value: &Value, epoch: u32) -> Result<()> {
    ensure!(
        epoch != 0
            && value["epoch"] == epoch
            && value["protocol"] == 1
            && value["policy_ack"] == true
            && value["wait_release"] == true,
        "invalid native key capabilities"
    );
    ensure!(
        value["audio_source"] == "system",
        "unsupported keyboard audio source"
    );
    Ok(())
}
fn poll_device(
    d: &mut Device,
    subscriber: &mut Option<SyncSender<Event>>,
    ready: bool,
) -> Result<()> {
    ensure!(
        d.identity || d.opened.elapsed() < Duration::from_secs(4),
        "device identity handshake timed out"
    );
    ensure!(
        d.capabilities || d.opened.elapsed() < Duration::from_secs(4),
        "native capabilities handshake timed out"
    );
    ensure!(
        !d.identity || d.opened.elapsed() < Duration::from_secs(6),
        "device identity refresh timed out after reset/disconnect"
    );
    if let Some((_, at, kind)) = &d.pending {
        if at.elapsed() >= Duration::from_secs(4) {
            if kind.is_none() {
                bail!("device NONE receipt timed out; capture state unknown");
            }
            d.display_failed("屏幕状态回执超时")?;
            emit(subscriber, Event::Status(d.status.clone()))?;
        }
    }
    if d.heartbeat.elapsed() >= Duration::from_secs(1) {
        d.send(json!({"event":"ping"}))?;
        d.send(json!({"event":"get_device_info"}))?;
        d.heartbeat = Instant::now();
        emit(subscriber, Event::Status(d.status.clone()))?;
    }
    let mut bytes = [0u8; 4096];
    let n = match d.port.read(&mut bytes) {
        Ok(0) => bail!("device disconnected"),
        Ok(n) => n,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            ) =>
        {
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };
    for frame in d.parser.append(&bytes[..n])? {
        let value = &frame;
        let event = value["event"].as_str().context("device event missing")?;
        if d.identity && !d.capabilities {
            ensure!(
                event == "nk_capabilities",
                "capabilities must immediately follow native identity"
            );
        }
        let mut forward = false;
        match event {
            "device_info" => {
                ensure!(
                    value["hardware"] == "vibe_keyboard",
                    "unsupported hardware identity"
                );
                let id = value["device_id"]
                    .as_str()
                    .context("stable device id missing")?;
                validate_device_id(id)?;
                if d.identity {
                    ensure!(
                        d.status.id.as_deref() == Some(id),
                        "device identity changed within connection"
                    );
                    ensure!(
                        value["epoch"] == d.epoch,
                        "device reset changed epoch; reconnect required"
                    );
                }
                d.opened = Instant::now();
                d.identity = true;
                d.epoch = value["epoch"].as_u64().unwrap_or(0).try_into()?;
                ensure!(
                    value["native_keyboard_protocol"] == 1 && d.epoch != 0,
                    "unsupported native protocol"
                );
                d.status.connected = true;
                d.status.id = Some(id.into());
                d.status.firmware = value["firmware_version"].as_str().map(str::to_owned);
                emit(subscriber, Event::Status(d.status.clone()))?;
            }
            "nk_capabilities" => {
                ensure!(d.identity, "capabilities before identity");
                native_capabilities(value, d.epoch)?;
                let first = !d.capabilities;
                let was_available = d.status.display_available;
                d.capabilities = true;
                d.status.native_protocol = true;
                let display = &value["display"];
                d.status.display_available = display["available"] == true
                    && display["version"] == 1
                    && display["width"] == 428
                    && display["height"] == 142
                    && display["privacy"] == "status_only";
                d.status.display_detail = if d.status.display_available {
                    None
                } else {
                    Some("当前固件没有可用状态屏幕；键盘功能继续".into())
                };
                if first {
                    d.policy()?;
                }
                if d.status.display_available && (first || !was_available) && !ready {
                    d.display(if subscriber.is_some() {
                        "connecting"
                    } else {
                        "sleeping"
                    })?;
                }
                emit(subscriber, Event::Status(d.status.clone()))?;
            }
            "nk_policy_applied" => {
                if !d.apply(value)? {
                    continue;
                }
                emit(subscriber, Event::Status(d.status.clone()))?;
                if let Some(generation) = d.rearmed_ready.take() {
                    emit(subscriber, Event::Rearmed { generation })?;
                }
            }
            "nk_display_applied" => {
                if d.apply_display(value)? {
                    emit(subscriber, Event::Status(d.status.clone()))?;
                }
            }
            "nk_error" => {
                if value["epoch"] != d.epoch {
                    continue;
                }
                let Some((id, _, kind)) = &d.pending else {
                    continue;
                };
                if value["request_id"] != *id {
                    continue;
                }
                if kind.is_some() {
                    d.display_failed("设备无法提交状态屏幕；四键继续可用")?;
                    emit(subscriber, Event::Status(d.status.clone()))?;
                    continue;
                }
                bail!(
                    "device rejected NONE: {}",
                    value["code"].as_str().unwrap_or("unknown")
                );
            }
            "button_down" | "button_up" | "button_click" => {
                forward = d.button(value, ready)?;
            }
            _ => {}
        }
        if forward {
            emit(subscriber, Event::Frame(frame))?;
        }
    }
    Ok(())
}
#[cfg(feature = "maintenance-validation")]
mod validation {
    use super::Control;
    use std::{collections::HashMap, sync::{Mutex, OnceLock}, thread::ThreadId};
    fn controls() -> &'static Mutex<HashMap<ThreadId, Control>> {
        static CONTROLS: OnceLock<Mutex<HashMap<ThreadId, Control>>> = OnceLock::new();
        CONTROLS.get_or_init(Default::default)
    }
    pub(super) struct Worker(ThreadId);
    impl Worker {
        pub(super) fn new() -> Self { Self(std::thread::current().id()) }
    }
    impl Drop for Worker {
        fn drop(&mut self) { controls().lock().unwrap().remove(&self.0); }
    }
    pub(super) fn record(control: Control) {
        controls().lock().unwrap().insert(std::thread::current().id(), control);
    }
    pub(super) fn last(id: ThreadId) -> Option<Control> {
        controls().lock().unwrap().get(&id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transport_stop_joins_owner_and_releases_lock_without_a_daemon_descriptor() {
        let root = tempfile::tempdir().unwrap();
        let (mut transport, events) = Transport::start_at(root.path().into(), false).unwrap();
        assert!(matches!(events.recv_timeout(Duration::from_secs(1)).unwrap(), Event::Status(_)));
        assert!(!root.path().join("owner.json").exists());
        assert!(Transport::start_at(root.path().into(), false).is_err());
        let commands = transport.commands();
        transport.shutdown().unwrap();
        assert!(commands.control(Control::Rearm { generation: 1 }).is_err());
        let (next, _) = Transport::start_at(root.path().into(), false).unwrap();
        drop(next);
    }
    #[test]
    fn transport_drop_joins_even_when_other_command_handles_survive() {
        let root = tempfile::tempdir().unwrap();
        let (transport, _) = Transport::start_at(root.path().into(), false).unwrap();
        let commands = transport.commands();
        drop(transport);
        assert!(commands.control(Control::Rearm { generation: 1 }).is_err());
        let (next, _) = Transport::start_at(root.path().into(), false).unwrap();
        drop(next);
        assert!(!root.path().join("owner.json").exists());
    }
    #[test]
    fn command_overflow_revokes_admission_without_blocking_shutdown() {
        let (tx, _rx) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let commands = Commands { tx, stop: stop.clone() };
        commands.control(Control::Rearm { generation: 1 }).unwrap();
        assert!(commands.control(Control::Policy { voice_key: "none".into() }).is_err());
        assert!(stop.load(Ordering::Acquire));
        assert!(commands.control(Control::Rearm { generation: 2 }).is_err());
    }
    struct ControlledWire {
        input: Receiver<Vec<u8>>,
        output: Arc<std::sync::Mutex<Vec<u8>>>,
        dropped: Arc<AtomicBool>,
    }
    impl Read for ControlledWire {
        fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
            match self.input.recv_timeout(Duration::from_millis(20)) {
                Ok(frame) => { bytes[..frame.len()].copy_from_slice(&frame); Ok(frame.len()) }
                Err(_) => Err(std::io::ErrorKind::TimedOut.into()),
            }
        }
    }
    impl Write for ControlledWire {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.output.lock().unwrap().extend_from_slice(bytes); Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
    impl Drop for ControlledWire {
        fn drop(&mut self) { self.dropped.store(true, Ordering::Release); }
    }
    #[test]
    fn transport_stop_waits_for_current_none_receipt_and_drops_serial_before_unlock() {
        let root = tempfile::tempdir().unwrap();
        let (input, frames) = mpsc::sync_channel(8);
        let output = Arc::new(std::sync::Mutex::new(Vec::new()));
        let dropped = Arc::new(AtomicBool::new(false));
        let mut serial = device(vec![]);
        serial.status.capture_state = "stopped".into();
        serial.port = Box::new(ControlledWire { input: frames, output: output.clone(), dropped: dropped.clone() });
        let (mut transport, events) = Transport::start_with_device(root.path().into(), false, Some(serial)).unwrap();
        events.recv_timeout(Duration::from_secs(1)).unwrap();
        let stopping = std::thread::spawn(move || transport.shutdown());
        let deadline = Instant::now() + Duration::from_secs(1);
        while output.lock().unwrap().is_empty() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(!stopping.is_finished() && !dropped.load(Ordering::Acquire));
        assert!(Transport::start_at(root.path().into(), false).is_err());
        let commands = Parser::default().append(&output.lock().unwrap()).unwrap();
        assert_eq!(commands[0]["event"], "nk_policy");
        assert_eq!(commands[0]["voice_key"], "none");
        // A different request in this epoch cannot prove the pending NONE.
        // A different epoch is a reset/protocol failure and closes the wire.
        input.send(protocol::command(none_receipt(2)).unwrap()).unwrap();
        std::thread::sleep(Duration::from_millis(30));
        assert!(!stopping.is_finished());
        input.send(protocol::command(none_receipt(1)).unwrap()).unwrap();
        stopping.join().unwrap().unwrap();
        assert!(dropped.load(Ordering::Acquire));
        let (next, _) = Transport::start_at(root.path().into(), false).unwrap();
        drop(next);
    }
    #[test]
    fn event_overflow_retains_serial_lock_until_none_cleanup_is_confirmed() {
        let root = tempfile::tempdir().unwrap();
        let (input, frames) = mpsc::sync_channel(64);
        let output = Arc::new(std::sync::Mutex::new(Vec::new()));
        let dropped = Arc::new(AtomicBool::new(false));
        let mut serial = device(vec![]);
        serial.status.capture_state = "stopped".into();
        serial.port = Box::new(ControlledWire { input: frames, output: output.clone(), dropped: dropped.clone() });
        let (mut transport, _undrained_events) = Transport::start_with_device(root.path().into(), false, Some(serial)).unwrap();
        // Initial status plus 40 identity refreshes overflow the bounded FIFO.
        let identity = protocol::command(json!({"event":"device_info","hardware":"vibe_keyboard",
            "device_id":"NK-TEST","native_keyboard_protocol":1,"epoch":7})).unwrap();
        for _ in 0..40 { input.send(identity.clone()).unwrap(); }
        let deadline = Instant::now() + Duration::from_secs(1);
        while output.lock().unwrap().is_empty() {
            assert!(Instant::now() < deadline, "overflow did not request NONE cleanup");
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(!dropped.load(Ordering::Acquire));
        assert!(Transport::start_at(root.path().into(), false).is_err());
        let commands = Parser::default().append(&output.lock().unwrap()).unwrap();
        assert_eq!(commands[0]["event"], "nk_policy");
        assert_eq!(commands[0]["voice_key"], "none");
        input.send(protocol::command(none_receipt(1)).unwrap()).unwrap();
        let failure = transport.shutdown().unwrap_err().to_string();
        assert!(failure.contains("queue"), "original overflow error was lost: {failure}");
        assert!(dropped.load(Ordering::Acquire));
        let (next, _) = Transport::start_at(root.path().into(), false).unwrap();
        drop(next);
    }
    struct FakeWire {
        input: std::io::Cursor<Vec<u8>>,
        output: Arc<std::sync::Mutex<Vec<u8>>>,
    }
    impl Read for FakeWire {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            self.input.read(buffer)
        }
    }
    impl Write for FakeWire {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.output.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    fn device(input: Vec<u8>) -> Device {
        Device {
            port: Box::new(FakeWire {
                input: std::io::Cursor::new(input),
                output: Default::default(),
            }),
            parser: Parser::default(),
            epoch: 7,
            identity: true,
            capabilities: true,
            heartbeat: Instant::now(),
            opened: Instant::now(),
            request: 0,
            pending: None,
            policy_again: false,
            rearm_latest: None,
            rearm_request: None,
            rearmed_ready: None,
            display_next: None,
            pressed: [false; 4],
            click_armed: [false; 4],
            status: DeviceStatus {
                id: Some("NK-TEST".into()),
                policy_state: "applied".into(),
                ..Default::default()
            },
        }
    }
    fn poll(d: &mut Device) -> Result<()> {
        poll_device(d, &mut None, true)
    }
    #[test]
    fn controls_reject_retired_or_extra_fields() {
        for value in [
            json!({"kind":"stop","session_id":1}),
            json!({"kind":"cancel","session_id":1}),
            json!({"kind":"k1_mapping","device_id":"NK-TEST","key":"k3"}),
            json!({"kind":"policy","voice_key":"none","raw":"x"}),
            json!({"kind":"raw"}),
        ] {
            assert!(serde_json::from_value::<Control>(value).is_err());
        }
    }
    #[test]
    fn native_capabilities_require_system_audio_and_the_current_epoch() {
        native_capabilities(&json!({"epoch":7,"protocol":1,"policy_ack":true,"wait_release":true,"audio_source":"system"}),7).unwrap();
        for rejected in [
            json!({"epoch":7,"protocol":1,"policy_ack":true,"wait_release":true}),
            json!({"epoch":7,"protocol":1,"policy_ack":true,"wait_release":true,"audio_source":"device"}),
            json!({"epoch":8,"protocol":1,"policy_ack":true,"wait_release":true,"audio_source":"system"}),
        ] {
            assert!(native_capabilities(&rejected, 7).is_err());
        }
    }
    #[test]
    fn identity_rejects_retired_firmware_before_buttons_can_run() {
        for identity in [
            json!({"event":"device_info","hardware":"vibe_keyboard","device_id":"NK-TEST","firmware_version":"0.3.8"}),
            json!({"event":"device_info","hardware":"vibe_keyboard","device_id":"NK-TEST","native_keyboard_protocol":1,"epoch":0}),
        ] {
            let mut d = device(protocol::command(identity).unwrap());
            d.identity = false;
            d.capabilities = false;
            assert!(
                poll(&mut d)
                    .unwrap_err()
                    .to_string()
                    .contains("unsupported native protocol")
            );
        }
    }
    #[test]
    fn matching_none_receipt_is_required() {
        let input=protocol::command(json!({"event":"nk_policy_applied","epoch":7,"request_id":1,"session_id":0,"voice_key":"none","capture_stopped":true,"waiting_release":true})).unwrap();
        let mut d = device(input);
        d.policy().unwrap();
        poll(&mut d).unwrap();
        assert!(d.pending.is_none());
        assert_eq!(d.status.capture_state, "stopped");
    }
    #[test]
    fn wrong_none_receipt_is_rejected() {
        let mut d=device(protocol::command(json!({"event":"nk_policy_applied","epoch":7,"request_id":1,"session_id":0,"voice_key":"k4","capture_stopped":true,"waiting_release":true})).unwrap());
        d.policy().unwrap();
        assert!(poll(&mut d).is_err());
        let mut d = device(vec![]);
        d.policy().unwrap();
        let mut receipt = none_receipt(1);
        receipt["session_id"] = json!(1);
        assert!(d.apply(&receipt).is_err());
    }
    #[test]
    fn repeated_none_requests_are_serialized() {
        let mut d=device(protocol::command(json!({"event":"nk_policy_applied","epoch":7,"request_id":1,"session_id":0,"voice_key":"none","capture_stopped":true,"waiting_release":true})).unwrap());
        d.policy().unwrap();
        d.policy().unwrap();
        assert_eq!(d.request, 1);
        poll(&mut d).unwrap();
        assert_eq!(d.request, 2);
        assert!(d.pending.is_some());
    }
    fn none_receipt(request: u32) -> Value {
        json!({"event":"nk_policy_applied","epoch":7,"request_id":request,"session_id":0,"voice_key":"none","capture_stopped":true,"waiting_release":true})
    }
    #[test]
    fn rearm_proof_follows_real_none_ack_and_rejects_old_key_frames() {
        let mut bytes = Vec::new();
        for event in ["button_down", "button_up", "button_click"] {
            bytes
                .extend(protocol::command(json!({"event":event,"button":"k4","epoch":7})).unwrap());
        }
        bytes.extend(protocol::command(none_receipt(1)).unwrap());
        for event in ["button_down", "button_up", "button_click"] {
            bytes
                .extend(protocol::command(json!({"event":event,"button":"k4","epoch":7})).unwrap());
        }
        let mut d = device(bytes);
        d.pressed[3] = true;
        d.click_armed[3] = true;
        d.rearm(9).unwrap();
        assert!(!d.pressed[3] && !d.click_armed[3]);
        assert!(d.rearmed_ready.is_none());
        let (tx, rx) = mpsc::sync_channel(16);
        let mut subscriber = Some(tx);
        poll_device(&mut d, &mut subscriber, true).unwrap();
        let events: Vec<_> = rx.try_iter().collect();
        assert!(matches!(events[0], Event::Status(_)));
        assert!(matches!(events[1], Event::Rearmed { generation: 9 }));
        assert_eq!(events.len(), 5); // only the three post-proof gestures survive
        assert!(
            events[2..]
                .iter()
                .all(|event| matches!(event, Event::Frame(_)))
        );
    }
    #[test]
    fn queued_new_rearm_and_duplicate_requests_cannot_prove_an_old_round() {
        let mut d = device(vec![]);
        d.rearm(2).unwrap();
        d.rearm(3).unwrap();
        d.rearm(3).unwrap();
        d.rearm(1).unwrap();
        assert_eq!(d.request, 1);
        assert!(!d.apply(&none_receipt(99)).unwrap());
        assert!(d.rearmed_ready.is_none());
        assert!(d.apply(&none_receipt(1)).unwrap());
        assert_eq!(d.request, 2);
        assert!(d.rearmed_ready.is_none());
        assert!(d.apply(&none_receipt(2)).unwrap());
        assert_eq!(d.rearmed_ready.take(), Some(3));
        d.rearm(2).unwrap();
        assert!(d.rearmed_ready.is_none());
        d.rearm(3).unwrap();
        assert_eq!(d.rearmed_ready.take(), Some(3));
        assert_eq!(d.request, 2);
        d.reset_rearm();
        d.rearm(1).unwrap();
        assert_eq!(d.request, 3);
        assert!(d.rearmed_ready.is_none());
        assert!(d.rearm(0).is_err());
    }
    #[test]
    fn display_receipt_never_substitutes_for_rearm_none_proof() {
        let mut d = device(vec![]);
        d.status.display_available = true;
        d.display("ready").unwrap();
        d.rearm(4).unwrap();
        assert!(d.rearmed_ready.is_none());
        d.apply_display(&json!({"epoch":7,"request_id":1,"state":"ready","applied":true}))
            .unwrap();
        assert_eq!(d.request, 2);
        assert!(d.rearmed_ready.is_none());
        d.apply(&none_receipt(2)).unwrap();
        assert_eq!(d.rearmed_ready, Some(4));
    }
    #[test]
    fn reconnected_device_cannot_reuse_completed_rearm_proof() {
        let mut old = device(vec![]);
        old.rearm(6).unwrap();
        old.apply(&none_receipt(1)).unwrap();
        assert_eq!(old.rearmed_ready.take(), Some(6));
        // owner_loop preserves the GUI generation, never the removed Device's
        // request/completion state. A duplicate before new capabilities/ack
        // therefore cannot make the host arm the replacement connection.
        let mut current = device(vec![]);
        current.capabilities = false;
        current.rearm_latest = Some(6);
        current.rearm(6).unwrap();
        assert!(current.rearmed_ready.is_none());
        assert_eq!(current.request, 0);
        current.capabilities = true;
        current.policy().unwrap();
        current.rearm(6).unwrap();
        assert!(current.rearmed_ready.is_none());
        current.apply(&none_receipt(1)).unwrap();
        assert_eq!(current.rearmed_ready, Some(6));
    }
    #[test]
    fn rearm_without_native_capabilities_cannot_accept_any_button() {
        let mut d = device(vec![]);
        d.capabilities = false;
        d.rearm(1).unwrap();
        assert!(d.rearmed_ready.is_none());
        assert!(
            d.button(
                &json!({"event":"button_click","button":"k1","epoch":7}),
                true
            )
            .is_err()
        );
        assert!(
            d.button(
                &json!({"event":"button_down","button":"k4","epoch":7}),
                true
            )
            .is_err()
        );
        assert_eq!(d.status.policy_state, "error");
    }
    #[test]
    fn unexpected_usb_audio_fails_explicitly() {
        let mut bytes = vec![1, 1, 16, 0];
        bytes.extend(1u32.to_le_bytes());
        bytes.extend(0u32.to_le_bytes());
        bytes.extend([1, 0, 1, 0, 55]);
        let mut d = device(bytes);
        assert!(
            poll(&mut d)
                .unwrap_err()
                .to_string()
                .contains("unexpected USB audio")
        );
    }
    #[test]
    fn native_click_is_consumed_once_and_voice_sessions_rejected() {
        let mut d = device(vec![]);
        for event in ["button_down", "button_up", "button_click"] {
            assert!(
                d.button(&json!({"event":event,"epoch":7,"button":"k4"}), true)
                    .unwrap()
            );
        }
        assert!(
            !d.button(
                &json!({"event":"button_click","epoch":7,"button":"k4"}),
                true
            )
            .unwrap()
        );
        assert!(
            d.button(
                &json!({"event":"button_down","epoch":7,"button":"k4","session_id":1}),
                true
            )
            .is_err()
        );
    }
    #[test]
    fn release_wait_detail_requires_current_ordered_native_buttons() {
        let mut d = device(vec![]);
        d.policy().unwrap();
        d.apply(&none_receipt(1)).unwrap();
        assert_eq!(d.status.detail.as_deref(), Some(WAITING_RELEASE_DETAIL));
        assert!(
            d.button(
                &json!({"event":"button_down","epoch":6,"button":"k4"}),
                true
            )
            .is_err()
        );
        for event in ["button_up", "button_click"] {
            assert!(
                !d.button(&json!({"event":event,"epoch":7,"button":"k4"}), true)
                    .unwrap()
            );
            assert_eq!(d.status.detail.as_deref(), Some(WAITING_RELEASE_DETAIL));
        }
        assert!(
            !d.button(
                &json!({"event":"button_down","epoch":7,"button":"k4"}),
                false
            )
            .unwrap()
        );
        assert_eq!(d.status.detail.as_deref(), Some(WAITING_RELEASE_DETAIL));
        for event in ["button_down", "button_up", "button_click"] {
            d.status.detail = Some(WAITING_RELEASE_DETAIL.into());
            let frame = json!({"event":event,"epoch":7,"button":"k4"});
            assert!(d.button(&frame, true).unwrap());
            assert!(d.status.detail.is_none());
            d.status.detail = Some(WAITING_RELEASE_DETAIL.into());
            assert!(!d.button(&frame, true).unwrap());
            assert_eq!(d.status.detail.as_deref(), Some(WAITING_RELEASE_DETAIL));
        }
    }
    #[test]
    fn native_buttons_preserve_unrelated_failures() {
        let mut d = device(vec![]);
        d.status.detail = Some("屏幕状态回执超时".into());
        let original = d.status.detail.clone();
        for event in ["button_down", "button_up", "button_click"] {
            assert!(
                d.button(&json!({"event":event,"epoch":7,"button":"k1"}), true)
                    .unwrap()
            );
            assert_eq!(d.status.detail, original);
        }
    }
    #[test]
    fn only_fixed_k1_is_available_before_desktop_readiness() {
        for button in ["k1", "k2", "k3", "k4"] {
            let mut d = device(vec![]);
            for event in ["button_down", "button_up", "button_click"] {
                assert_eq!(
                    d.button(&json!({"event":event,"button":button,"epoch":7}), false)
                        .unwrap(),
                    button == "k1"
                );
            }
        }
    }
    #[test]
    fn fixed_four_keys_forward_once_after_readiness_without_mapping() {
        let mut input = vec![];
        for button in ["k1", "k2", "k3", "k4"] {
            for event in ["button_down", "button_up", "button_click"] {
                let frame =
                    protocol::command(json!({"event":event,"button":button,"epoch":7})).unwrap();
                input.extend(&frame);
                input.extend(frame); // duplicate edges must not replay an action
            }
        }
        let mut d = device(input);
        let (tx, rx) = mpsc::sync_channel(16);
        poll_device(&mut d, &mut Some(tx), true).unwrap();
        let frames: Vec<_> = rx
            .try_iter()
            .map(|event| match event {
                Event::Frame(frame) => frame,
                other => panic!("unexpected event: {other:?}"),
            })
            .collect();
        assert_eq!(frames.len(), 12);
        for (index, frame) in frames.iter().enumerate() {
            assert_eq!(frame["button"], format!("k{}", index / 3 + 1));
            assert_eq!(
                frame["event"],
                ["button_down", "button_up", "button_click"][index % 3]
            );
        }
    }
    #[test]
    fn display_queue_coalesces_and_none_has_priority_without_dropping_key_up() {
        let mut d = device(vec![]);
        d.status.display_available = true;
        d.policy().unwrap();
        d.display("starting").unwrap();
        d.display("listening").unwrap();
        d.apply(&json!({"event":"nk_policy_applied","epoch":7,"request_id":1,"session_id":0,"voice_key":"none","capture_stopped":true,"waiting_release":true})).unwrap();
        assert_eq!(d.pending.as_ref().unwrap().2.as_deref(), Some("listening"));
        assert!(
            d.button(
                &json!({"event":"button_down","epoch":7,"button":"k4"}),
                true
            )
            .unwrap()
        );
        assert!(
            d.button(&json!({"event":"button_up","epoch":7,"button":"k4"}), true)
                .unwrap()
        );
        d.display("ready").unwrap();
        d.policy().unwrap();
        d.apply_display(&json!({"event":"nk_display_applied","epoch":7,"request_id":2,"state":"listening","applied":true})).unwrap();
        assert_eq!(d.request, 3);
        assert!(d.pending.as_ref().unwrap().2.is_none());
        d.apply(&json!({"event":"nk_policy_applied","epoch":7,"request_id":3,"session_id":0,"voice_key":"none","capture_stopped":true,"waiting_release":true})).unwrap();
        assert_eq!(d.pending.as_ref().unwrap().2.as_deref(), Some("ready"));
    }
    #[test]
    fn panel_failure_only_reports_status_and_keeps_key_policy() {
        let input = protocol::command(
            json!({"event":"nk_error","epoch":7,"request_id":1,"code":"display_unavailable"}),
        )
        .unwrap();
        let mut d = device(input);
        d.status.display_available = true;
        d.display("ready").unwrap();
        let (tx, rx) = mpsc::sync_channel(8);
        poll_device(&mut d, &mut Some(tx), true).unwrap();
        assert!(matches!(rx.recv().unwrap(),Event::Status(status)if !status.display_available));
        assert!(
            rx.try_iter()
                .all(|event| !matches!(event, Event::Failure(_)))
        );
        assert_eq!(d.status.policy_state, "applied");
    }
    #[test]
    fn panel_timeout_does_not_disconnect_keyboard_or_report_capture_failure() {
        let mut d = device(protocol::command(json!({"event":"unused_status"})).unwrap());
        d.status.display_available = true;
        d.pending = Some((
            1,
            Instant::now() - Duration::from_secs(5),
            Some("ready".into()),
        ));
        poll(&mut d).unwrap();
        assert!(!d.status.display_available);
        assert_eq!(d.status.policy_state, "applied");
    }
    #[test]
    fn capabilities_refresh_preserves_active_key_and_none_policy() {
        let mut bytes=protocol::command(json!({"event":"device_info","hardware":"vibe_keyboard","device_id":"NK-TEST","firmware_version":"0.3-screen.1","native_keyboard_protocol":1,"epoch":7})).unwrap();
        bytes.extend(protocol::command(json!({"event":"nk_capabilities","epoch":7,"protocol":1,"policy_ack":true,"wait_release":true,"audio_source":"system","display":{"version":1,"available":true,"width":428,"height":142,"privacy":"status_only"}})).unwrap());
        let mut d = device(bytes);
        d.pressed[3] = true;
        poll(&mut d).unwrap();
        assert!(d.pressed[3] && d.status.display_available);
        assert_eq!(d.request, 0);
        assert_eq!(d.status.policy_state, "applied");
    }
    #[test]
    fn silent_device_reset_is_detected_by_identity_refresh_deadline() {
        let mut d = device(vec![]);
        d.opened = Instant::now() - Duration::from_secs(7);
        assert!(
            poll(&mut d)
                .unwrap_err()
                .to_string()
                .contains("identity refresh")
        );
    }
}
