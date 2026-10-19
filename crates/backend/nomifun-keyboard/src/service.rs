//! One serial owner, entered before desktop single-instance or GUI startup.
//! IPC is loopback-only, authenticated with a fresh per-owner 256-bit secret in
//! the user's private runtime directory. Keyboard transport never captures audio.
use crate::protocol::{self, Parser};
use anyhow::{Context, Result, bail, ensure};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    time::{Duration, Instant},
};

const IPC_LIMIT: u64 = 32768;
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
    DesktopStopped,
    Display { state: String },
    Heartbeat,
    Shutdown,
}
#[derive(Serialize, Deserialize)]
struct Descriptor {
    port: u16,
    token: String,
    pid: u32,
}
enum Message {
    Connect(SyncSender<Event>),
    Disconnect,
    Control(Control),
    Shutdown(SyncSender<std::result::Result<(), String>>),
    ShutdownAfterDesktopExit(SyncSender<std::result::Result<(), String>>, String),
}

pub fn runtime_dir() -> Result<PathBuf> {
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .context("LOCALAPPDATA unavailable")?;
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME unavailable")?
        .join("Library/Application Support");
    #[cfg(target_os = "linux")]
    let base = if let Some(root) = std::env::var_os("XDG_RUNTIME_DIR") {
        PathBuf::from(root)
    } else {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .context("HOME unavailable")?
            .join(".local/state")
    };
    let dir = base.join("NomiFun/keyboard-service");
    fs::create_dir_all(&dir)?;
    ensure!(
        !fs::symlink_metadata(&dir)?.file_type().is_symlink(),
        "runtime directory is a symlink"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        ensure!(
            fs::metadata(&dir)?.uid() == unsafe { libc_getuid() },
            "runtime directory owner mismatch"
        );
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(dir)
}
#[cfg(unix)]
unsafe extern "C" {
    #[link_name = "getuid"]
    fn libc_getuid() -> u32;
    #[link_name = "setsid"]
    fn libc_setsid() -> i32;
}
pub(crate) fn private_create(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).read(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}
pub(crate) fn send_line<T: Serialize>(stream: &mut TcpStream, value: &T) -> Result<()> {
    let mut bytes = serde_json::to_vec(value)?;
    ensure!(bytes.len() < IPC_LIMIT as usize, "IPC message too large");
    bytes.push(b'\n');
    stream.write_all(&bytes)?;
    Ok(())
}
pub(crate) fn read_line<T: for<'de> Deserialize<'de>>(
    reader: &mut BufReader<TcpStream>,
) -> Result<T> {
    let mut line = String::new();
    let n = reader.take(IPC_LIMIT + 1).read_line(&mut line)?;
    ensure!(
        n > 0 && n as u64 <= IPC_LIMIT && line.ends_with('\n'),
        "IPC closed or message limit exceeded"
    );
    Ok(serde_json::from_str(&line)?)
}
pub(crate) fn token_equal(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes().zip(b.bytes()).fold(0u8, |v, (a, b)| v | (a ^ b)) == 0
}

/// The file lock stays held for the process lifetime, including serial teardown.
pub fn run() -> Result<()> {
    run_at(runtime_dir()?, true)
}
fn run_at(dir: PathBuf, scan_enabled: bool) -> Result<()> {
    let lock_path = dir.join("owner.lock");
    ensure!(!lock_path.is_symlink(), "owner lock is a symlink");
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)?;
    if lock.try_lock_exclusive().is_err() {
        return Ok(());
    }
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
    listener.set_nonblocking(true)?;
    let mut random = [0u8; 32];
    getrandom::getrandom(&mut random).map_err(|_| anyhow::anyhow!("secure random unavailable"))?;
    let descriptor = Descriptor {
        port: listener.local_addr()?.port(),
        token: random.iter().map(|b| format!("{b:02x}")).collect(),
        pid: std::process::id(),
    };
    let path = dir.join("owner.json");
    if path.exists() {
        fs::remove_file(&path)?;
    }
    let mut file = private_create(&path)?;
    serde_json::to_writer(&mut file, &descriptor)?;
    file.sync_all()?;
    drop(file);
    let (tx, rx) = mpsc::sync_channel(64);
    let alive = Arc::new(AtomicBool::new(true));
    let accepting = alive.clone();
    let token = descriptor.token;
    let accept = std::thread::spawn(move || {
        let busy = Arc::new(AtomicBool::new(false));
        let mut connections: Vec<std::thread::JoinHandle<()>> = Vec::new();
        while accepting.load(Ordering::Acquire) {
            connections.retain(|connection| !connection.is_finished());
            match listener.accept() {
                Ok((mut stream, addr)) => {
                    if !addr.ip().is_loopback() || connections.len() >= 16 {
                        continue;
                    }
                    let busy = busy.clone();
                    let token = token.clone();
                    let tx = tx.clone();
                    connections.push(std::thread::spawn(move || {
                        let mut desktop = false;
                        let mut owner_guard = None;
                        let result = (|| -> Result<()> {
                            // Winsock inherits listener nonblocking mode on accept.
                            stream.set_nonblocking(false)?;
                            stream.set_read_timeout(Some(Duration::from_secs(3)))?;
                            stream.set_write_timeout(Some(Duration::from_secs(1)))?;
                            let mut reader = BufReader::new(stream.try_clone()?);
                            let auth: Value = read_line(&mut reader)?;
                            ensure!(
                                auth["protocol"] == 1
                                    && auth["token"]
                                        .as_str()
                                        .is_some_and(|value| token_equal(value, &token)),
                                "IPC authentication failed"
                            );
                            if auth["mode"] == "control" {
                                send_line(&mut stream, &json!({"accepted":true}))?;
                                let control: Control = read_line(&mut reader)?;
                                ensure!(
                                    matches!(control, Control::Shutdown),
                                    "management connection only permits shutdown"
                                );
                                let (reply, result) = mpsc::sync_channel(1);
                                if let Some(proof) = auth["desktop_exit_proof"].as_str() {
                                    tx.send(Message::ShutdownAfterDesktopExit(
                                        reply,
                                        proof.into(),
                                    ))?;
                                } else {
                                    tx.send(Message::Shutdown(reply))?;
                                }
                                let result =
                                    result.recv_timeout(Duration::from_secs(12)).unwrap_or_else(
                                        |_| Err("owner shutdown proof timed out".into()),
                                    );
                                match result {
                                    Ok(()) => send_line(&mut stream, &json!({"completed":true}))?,
                                    Err(error) => send_line(
                                        &mut stream,
                                        &json!({"completed":false,"error":error}),
                                    )?,
                                }
                                return Ok(());
                            }
                            ensure!(
                                !busy.swap(true, Ordering::AcqRel),
                                "desktop IPC owner already connected"
                            );
                            desktop = true;
                            owner_guard = Some(ConnectionGuard(busy));
                            send_line(&mut stream, &json!({"accepted":true}))?;
                            let (events_tx, events_rx) = mpsc::sync_channel(40);
                            tx.send(Message::Connect(events_tx))?;
                            let read_tx = tx.clone();
                            let mut read_stream = reader;
                            let read_thread = std::thread::spawn(move || -> Result<()> {
                                let result = (|| -> Result<()> {
                                    loop {
                                        let cmd: Control = read_line(&mut read_stream)?;
                                        read_tx.send(Message::Control(cmd))?;
                                    }
                                })();
                                let _ = read_tx.send(Message::Disconnect);
                                result
                            });
                            let writing = (|| -> Result<()> {
                                while let Ok(event) = events_rx.recv() {
                                    send_line(&mut stream, &event)?;
                                }
                                Ok(())
                            })();
                            let _ = stream.shutdown(std::net::Shutdown::Both);
                            let _ = read_thread.join();
                            writing
                        })();
                        #[cfg(test)]
                        if let Err(e) = &result {
                            eprintln!("IPC test connection: {e:#}");
                        }
                        let _ = result;
                        if desktop {
                            let _ = tx.send(Message::Disconnect);
                        }
                        // A new connection cannot overtake either old disconnect.
                        drop(owner_guard);
                    }));
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(40))
                }
                Err(_) => break,
            }
        }
        // Final management proof must reach the socket before service exit.
        for connection in connections {
            let _ = connection.join();
        }
    });
    let result = owner_loop(rx, scan_enabled, dir);
    alive.store(false, Ordering::Release);
    let _ = accept.join();
    let _ = fs::remove_file(path);
    drop(lock);
    result
}
struct ConnectionGuard(Arc<AtomicBool>);
impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

pub struct Client {
    writer: TcpStream,
    reader: BufReader<TcpStream>,
}
impl Client {
    pub fn connect() -> Result<Self> {
        Self::connect_at(runtime_dir()?.join("owner.json"))
    }
    fn connect_at(path: PathBuf) -> Result<Self> {
        ensure!(!path.is_symlink(), "IPC descriptor is a symlink");
        let d: Descriptor = serde_json::from_reader(File::open(path)?)?;
        ensure!(d.token.len() == 64, "invalid IPC descriptor");
        let mut writer =
            TcpStream::connect_timeout(&([127, 0, 0, 1], d.port).into(), Duration::from_secs(1))?;
        writer.set_write_timeout(Some(Duration::from_secs(1)))?;
        writer.set_read_timeout(Some(Duration::from_secs(4)))?;
        send_line(
            &mut writer,
            &json!({"protocol":1,"token":d.token,"mode":"desktop"}),
        )?;
        let mut reader = BufReader::new(writer.try_clone()?);
        let accepted: Value = read_line(&mut reader)?;
        ensure!(
            accepted["accepted"] == true,
            "IPC rejected desktop connection"
        );
        Ok(Self { writer, reader })
    }
    pub fn control(&mut self, control: Control) -> Result<()> {
        send_line(&mut self.writer, &control)
    }
    pub fn next(&mut self) -> Result<Event> {
        read_line(&mut self.reader)
    }
    pub fn writer(&self) -> Result<TcpStream> {
        Ok(self.writer.try_clone()?)
    }
}
pub fn write_control(writer: &mut TcpStream, control: Control) -> Result<()> {
    send_line(writer, &control)
}
pub fn request_shutdown() -> Result<()> {
    request_shutdown_at(runtime_dir()?.join("owner.json"))
}
/// External maintenance only: the typed guard proves the GUI process has
/// exited and prevents a replacement GUI until this management call finishes.
pub fn request_shutdown_after_desktop_exit(
    proof: &crate::desktop_control::DesktopQuiescence,
) -> Result<()> {
    let dir = runtime_dir()?;
    request_shutdown_with_proof_at(dir.join("owner.json"), Some(proof.proof_for(&dir)?))
}
fn request_shutdown_at(path: PathBuf) -> Result<()> {
    request_shutdown_with_proof_at(path, None)
}
fn request_shutdown_with_proof_at(path: PathBuf, desktop_exit_proof: Option<&str>) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    ensure!(!path.is_symlink(), "IPC descriptor is a symlink");
    let descriptor: Descriptor = serde_json::from_reader(File::open(path)?)?;
    let mut stream = TcpStream::connect_timeout(
        &([127, 0, 0, 1], descriptor.port).into(),
        Duration::from_secs(1),
    )?;
    stream.set_read_timeout(Some(Duration::from_secs(13)))?;
    stream.set_write_timeout(Some(Duration::from_secs(1)))?;
    send_line(
        &mut stream,
        &json!({"protocol":1,"token":descriptor.token,"mode":"control","desktop_exit_proof":desktop_exit_proof}),
    )?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let accepted: Value = read_line(&mut reader).context("management authentication reply")?;
    ensure!(
        accepted["accepted"] == true,
        "management IPC authentication rejected"
    );
    send_line(&mut stream, &Control::Shutdown).context("management shutdown send")?;
    let accepted: Value = read_line(&mut reader).context("management final shutdown proof")?;
    ensure!(
        accepted["completed"] == true,
        "shutdown not completed: {}",
        accepted["error"]
            .as_str()
            .unwrap_or("missing capture stop proof")
    );
    Ok(())
}
fn transient_mount(path: &Path) -> bool {
    path.components().any(|component| {
        component
            .as_os_str()
            .to_string_lossy()
            .starts_with(".mount_")
    })
}
fn launch_file(path: &Path) -> bool {
    if !path.is_absolute() || !path.is_file() || transient_mount(path) {
        return false;
    }
    let Ok(resolved) = fs::canonicalize(path) else {
        return false;
    };
    if transient_mount(&resolved) {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if !fs::metadata(&resolved).is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
        {
            return false;
        }
    }
    true
}
/// Select the durable outer AppImage, never the temporary FUSE executable.
/// Invalid ambient APPIMAGE values do not override an ordinary installed exe.
fn launch_path(app_image: Option<&Path>, current_exe: &Path) -> Result<PathBuf> {
    if let Some(image) = app_image.filter(|image| launch_file(image)) {
        return Ok(image.to_path_buf());
    }
    ensure!(
        launch_file(current_exe),
        "durable NomiFun executable unavailable; APPIMAGE must be an existing absolute executable outside .mount_ directories"
    );
    Ok(current_exe.to_path_buf())
}
fn configure_detached(command: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Only async-signal-safe setsid runs after fork. Preserve the inherited
        // DISPLAY/Wayland/DBus environment while releasing the parent's TTY.
        unsafe {
            command.pre_exec(|| {
                if libc_setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
}
fn launch_command(mode: &str) -> Result<std::process::Command> {
    let current_exe = std::env::current_exe()?;
    #[cfg(target_os = "linux")]
    let image = std::env::var_os("APPIMAGE").map(PathBuf::from);
    #[cfg(not(target_os = "linux"))]
    let image: Option<PathBuf> = None;
    let mut command = std::process::Command::new(launch_path(image.as_deref(), &current_exe)?);
    command.arg(mode);
    configure_detached(&mut command);
    Ok(command)
}
pub fn spawn() -> Result<()> {
    launch_command("--keyboard-service")?.spawn()?;
    Ok(())
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

#[derive(Default)]
struct LaunchGate {
    last: Option<Instant>,
}
impl LaunchGate {
    fn claim(&mut self, now: Instant) -> bool {
        if self
            .last
            .is_some_and(|last| now.duration_since(last) < Duration::from_secs(3))
        {
            return false;
        }
        self.last = Some(now);
        true
    }
    fn ready(&mut self) {
        self.last = None;
    }
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
#[derive(Default)]
struct ShutdownReplies(Vec<SyncSender<std::result::Result<(), String>>>);
impl ShutdownReplies {
    fn success(&mut self) {
        for reply in self.0.drain(..) {
            let _ = reply.send(Ok(()));
        }
    }
}
impl Drop for ShutdownReplies {
    fn drop(&mut self) {
        for reply in self.0.drain(..) {
            let _ = reply.send(Err(
                "NONE/desktop capture stop proof failed; service did not complete a clean shutdown"
                    .into(),
            ));
        }
    }
}
struct Shutdown {
    started: Instant,
    needs_desktop: bool,
}
fn owner_loop(rx: Receiver<Message>, scan_enabled: bool, dir: PathBuf) -> Result<()> {
    let mut device: Option<Device> = None;
    let mut subscriber = None;
    let mut ready = false;
    let mut rearm_generation: Option<u64> = None;
    let mut desktop_unproven = false;
    let mut desktop_stopped = false;
    let mut lease = Instant::now();
    let mut scan = Instant::now() - Duration::from_secs(2);
    let mut shutdown: Option<Shutdown> = None;
    let mut replies = ShutdownReplies::default();
    let mut launch = LaunchGate::default();
    loop {
        while let Ok(message) = rx.try_recv() {
            let mut request_shutdown = false;
            match message {
                Message::Connect(tx) => {
                    ensure!(shutdown.is_none(), "desktop connection during shutdown");
                    subscriber = Some(tx);
                    ready = false;
                    rearm_generation = None;
                    desktop_unproven = true;
                    desktop_stopped = false;
                    lease = Instant::now();
                    if let Some(d) = device.as_mut() {
                        d.reset_rearm();
                        d.policy()?;
                        d.display("connecting")?;
                    }
                    let _ = emit(
                        &mut subscriber,
                        Event::Status(
                            device
                                .as_ref()
                                .map(|d| d.status.clone())
                                .unwrap_or_default(),
                        ),
                    );
                }
                Message::Disconnect => {
                    subscriber = None;
                    ready = false;
                    rearm_generation = None;
                    if let Some(d) = device.as_mut() {
                        d.reset_rearm();
                        let _ = d.policy();
                        let _ = d.display("sleeping");
                    }
                    // A broken socket is not evidence that the OS microphone stopped.
                    if shutdown.as_ref().is_some_and(|s| s.needs_desktop) && !desktop_stopped {
                        bail!("desktop disconnected without native OS capture stop proof");
                    }
                }
                Message::Shutdown(reply) => {
                    replies.0.push(reply);
                    request_shutdown = true;
                }
                Message::ShutdownAfterDesktopExit(reply, proof) => {
                    if let Err(error) = crate::desktop_control::verify_quiescence(&dir, &proof) {
                        let _ = reply.send(Err(format!("desktop exit proof rejected: {error}")));
                        continue;
                    }
                    desktop_stopped = true;
                    desktop_unproven = false;
                    ready = false;
                    replies.0.push(reply);
                    request_shutdown = true;
                }
                Message::Control(control) => {
                    lease = Instant::now();
                    match control {
                        Control::Heartbeat => {}
                        Control::DesktopStopped => {
                            rearm_generation = None;
                            desktop_stopped = true;
                            desktop_unproven = false;
                            ready = false;
                            if let Some(d) = device.as_mut() {
                                d.reset_rearm();
                                d.display("sleeping")?;
                            }
                        }
                        Control::Display { state } => {
                            if let Some(d) = device.as_mut() {
                                d.display(&state)?;
                                let _ = emit(&mut subscriber, Event::Status(d.status.clone()));
                            }
                        }
                        Control::Policy { voice_key } => {
                            if voice_key != "none" {
                                let _ = emit(
                                    &mut subscriber,
                                    Event::Failure(
                                        "键盘策略只允许 NONE；语音来自系统麦克风".into(),
                                    ),
                                );
                                continue;
                            }
                            if shutdown.is_some() {
                                continue;
                            }
                            ready = true;
                            desktop_stopped = false;
                            desktop_unproven = true;
                            launch.ready();
                            if let Some(d) = device.as_mut() {
                                d.policy()?;
                                let _ = emit(&mut subscriber, Event::Status(d.status.clone()));
                            }
                        }
                        Control::Rearm { generation } => {
                            if shutdown.is_some() {
                                continue;
                            }
                            ensure!(generation != 0, "invalid rearm generation");
                            rearm_generation = Some(
                                rearm_generation
                                    .map_or(generation, |latest| latest.max(generation)),
                            );
                            ready = true;
                            desktop_stopped = false;
                            desktop_unproven = true;
                            launch.ready();
                            if let Some(d) = device.as_mut() {
                                d.rearm(generation)?;
                                let _ = emit(&mut subscriber, Event::Status(d.status.clone()));
                                if let Some(generation) = d.rearmed_ready.take() {
                                    emit(&mut subscriber, Event::Rearmed { generation })?;
                                }
                            }
                        }
                        Control::Shutdown => {
                            request_shutdown = true;
                        }
                    }
                }
            }
            if request_shutdown {
                if shutdown.is_none() {
                    shutdown = Some(Shutdown {
                        started: Instant::now(),
                        needs_desktop: desktop_unproven,
                    });
                    ready = false;
                    let _ = emit(
                        &mut subscriber,
                        Event::Failure("device_service_stopping".into()),
                    );
                    if let Some(d) = device.as_mut() {
                        d.policy()?;
                        d.display("sleeping")?;
                    }
                }
                ensure!(replies.0.len() <= 8, "too many shutdown waiters");
            }
        }
        if ready && lease.elapsed() > Duration::from_secs(3) {
            ready = false;
            if let Some(d) = device.as_mut() {
                let _ = d.policy();
            }
            let _ = emit(
                &mut subscriber,
                Event::Failure("主程序心跳中断；设备保持 NONE".into()),
            );
        }
        if shutdown.is_none()
            && scan_enabled
            && device.is_none()
            && scan.elapsed() > Duration::from_secs(2)
        {
            scan = Instant::now();
            match discover() {
                Ok(mut found) => {
                    if let Some(d) = found.as_mut() {
                        d.rearm_latest = rearm_generation;
                    }
                    device = found;
                }
                Err(error) => {
                    let _ = emit(&mut subscriber, Event::Failure(error.to_string()));
                }
            }
        }
        let result = if let Some(d) = device.as_mut() {
            poll_device(d, &mut subscriber, ready, &mut launch)
        } else {
            std::thread::sleep(Duration::from_millis(25));
            Ok(())
        };
        if let Err(error) = result {
            device = None;
            ready = false;
            let _ = emit(&mut subscriber, Event::Failure(error.to_string()));
            let _ = emit(
                &mut subscriber,
                Event::Status(DeviceStatus {
                    detail: Some(error.to_string()),
                    ..Default::default()
                }),
            );
            if shutdown.is_some() {
                return Err(error);
            }
        }
        if let Some(state) = &shutdown {
            ensure!(
                state.started.elapsed() < Duration::from_secs(10),
                "shutdown deadline exceeded without native OS capture stop proof"
            );
            let keyboard_stopped = device.as_ref().is_none_or(|d| {
                !d.none_pending()
                    && d.status.capture_state == "stopped"
                    && d.status.policy_state == "applied"
            });
            if keyboard_stopped && (!state.needs_desktop || desktop_stopped) {
                replies.success();
                return Ok(());
            }
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
    launch: &mut LaunchGate,
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
                if forward
                    && value["button"] == "k1"
                    && event == "button_click"
                    && subscriber.is_none()
                    && launch.claim(Instant::now())
                {
                    spawn_main()?;
                }
            }
            _ => {}
        }
        if forward {
            emit(subscriber, Event::Frame(frame))?;
        }
    }
    Ok(())
}
fn spawn_main() -> Result<()> {
    launch_command("--keyboard-activate")?.spawn()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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
        poll_device(d, &mut None, true, &mut LaunchGate::default())
    }
    #[test]
    fn authentication_is_exact() {
        assert!(token_equal("abcdef", "abcdef"));
        assert!(!token_equal("abcdef", "abcdeg"));
        assert!(!token_equal("abc", "abcdef"));
    }
    #[test]
    fn ipc_rejects_retired_or_extra_control() {
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
    fn rearm_proof_follows_real_none_ack_and_old_socket_keys() {
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
        poll_device(&mut d, &mut subscriber, true, &mut LaunchGate::default()).unwrap();
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
    fn cold_launch_clicks_are_throttled() {
        let mut gate = LaunchGate::default();
        let now = Instant::now();
        assert!(gate.claim(now));
        assert!(!gate.claim(now + Duration::from_millis(200)));
        assert!(gate.claim(now + Duration::from_secs(3)));
        gate.ready();
        assert!(gate.claim(now + Duration::from_secs(3)));
    }
    #[test]
    fn management_shutdown_works_while_desktop_is_connected() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let path = root.join("owner.json");
        let owner = std::thread::spawn(move || run_at(root, false));
        let started = Instant::now();
        while !path.exists() {
            assert!(started.elapsed() < Duration::from_secs(3));
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut desktop = Client::connect_at(path.clone()).unwrap();
        assert!(matches!(desktop.next().unwrap(), Event::Status(_)));
        desktop.control(Control::DesktopStopped).unwrap();
        request_shutdown_at(path.clone()).unwrap();
        owner.join().unwrap().unwrap();
        assert!(!path.exists());
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open(dir.path().join("owner.lock"))
            .unwrap();
        lock.try_lock_exclusive().unwrap();
    }
    #[test]
    fn orphan_daemon_accepts_only_live_gui_exit_lock_proof() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let path = root.join("owner.json");
        let owner = std::thread::spawn(move || run_at(root, false));
        let start = Instant::now();
        while !path.exists() {
            assert!(start.elapsed() < Duration::from_secs(3));
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut desktop = Client::connect_at(path.clone()).unwrap();
        desktop.next().unwrap();
        let lifetime = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(dir.path().join("desktop.lock"))
            .unwrap();
        FileExt::try_lock_exclusive(&lifetime).unwrap();
        assert!(
            crate::desktop_control::request_at(dir.path().into(), Duration::from_millis(100))
                .is_err()
        );
        desktop
            .control(Control::Policy {
                voice_key: "none".into(),
            })
            .unwrap();
        // Abruptly gone, with no DesktopStopped: existing disconnect logic
        // intentionally retains desktop_unproven rather than fabricating proof.
        drop(desktop);
        drop(lifetime); // the OS releases this handle when a GUI crashes
        let proof =
            crate::desktop_control::request_at(dir.path().into(), Duration::from_secs(1)).unwrap();
        assert!(request_shutdown_with_proof_at(path.clone(), Some("forged")).is_err());
        assert!(path.exists() && !owner.is_finished());
        let token = proof.proof_for(dir.path()).unwrap();
        request_shutdown_with_proof_at(path.clone(), Some(token)).unwrap();
        owner.join().unwrap().unwrap();
        assert!(!path.exists());
        drop(proof);
        assert!(!dir.path().join("desktop-exited.json").exists());
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
        poll_device(&mut d, &mut Some(tx), true, &mut LaunchGate::default()).unwrap();
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
    fn disconnected_desktop_can_reconnect_before_policy() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let path = root.join("owner.json");
        let owner = std::thread::spawn(move || run_at(root, false));
        let start = Instant::now();
        while !path.exists() {
            assert!(start.elapsed() < Duration::from_secs(3));
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut first = Client::connect_at(path.clone()).unwrap();
        first.next().unwrap();
        drop(first);
        let mut second = loop {
            if let Ok(client) = Client::connect_at(path.clone()) {
                break client;
            }
            assert!(start.elapsed() < Duration::from_secs(4));
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(matches!(second.next().unwrap(), Event::Status(_)));
        second.control(Control::DesktopStopped).unwrap();
        request_shutdown_at(path).unwrap();
        owner.join().unwrap().unwrap();
    }
    #[test]
    fn disconnect_does_not_prove_desktop_capture_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let (tx, rx) = mpsc::sync_channel(8);
        let owner = std::thread::spawn(move || owner_loop(rx, false, root));
        let (events, reader) = mpsc::sync_channel(8);
        tx.send(Message::Connect(events)).unwrap();
        reader.recv().unwrap();
        let (reply, result) = mpsc::sync_channel(1);
        tx.send(Message::Shutdown(reply)).unwrap();
        tx.send(Message::Disconnect).unwrap();
        assert!(owner.join().unwrap().is_err());
        assert!(result.recv().unwrap().is_err());
    }
    #[test]
    fn management_waits_for_late_native_capture_stop_proof() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let path = root.join("owner.json");
        let owner = std::thread::spawn(move || run_at(root, false));
        let start = Instant::now();
        while !path.exists() {
            assert!(start.elapsed() < Duration::from_secs(3));
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut desktop = Client::connect_at(path.clone()).unwrap();
        desktop.next().unwrap();
        let request_path = path.clone();
        let shutdown = std::thread::spawn(move || request_shutdown_at(request_path));
        assert!(
            matches!(desktop.next().unwrap(),Event::Failure(value)if value=="device_service_stopping")
        );
        assert!(!shutdown.is_finished());
        desktop.control(Control::DesktopStopped).unwrap();
        shutdown.join().unwrap().unwrap();
        owner.join().unwrap().unwrap();
        assert!(!path.exists());
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
        poll_device(&mut d, &mut Some(tx), true, &mut LaunchGate::default()).unwrap();
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
    fn executable_fixture(path: &Path) {
        fs::write(path, b"fixture").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
    }
    #[test]
    fn appimage_launch_uses_durable_outer_image_instead_of_mount() {
        let dir = tempfile::tempdir().unwrap();
        let image = dir.path().join("NomiFun.AppImage");
        executable_fixture(&image);
        let mount = dir.path().join(".mount_NomiFun");
        fs::create_dir(&mount).unwrap();
        let inner = mount.join("nomifun-desktop");
        executable_fixture(&inner);
        assert_eq!(launch_path(Some(&image), &inner).unwrap(), image);
        assert!(launch_path(None, &inner).is_err());
        assert!(launch_path(Some(Path::new("relative.AppImage")), &inner).is_err());
        assert!(launch_path(Some(&inner), &inner).is_err());
    }
    #[test]
    fn invalid_appimage_does_not_override_valid_normal_executable() {
        let dir = tempfile::tempdir().unwrap();
        let current = dir.path().join("nomifun-desktop");
        executable_fixture(&current);
        for candidate in [
            Path::new("relative.AppImage"),
            dir.path(),
            &dir.path().join("missing.AppImage"),
        ] {
            assert_eq!(launch_path(Some(candidate), &current).unwrap(), current);
        }
        assert_eq!(launch_path(None, &current).unwrap(), current);
        assert!(launch_path(None, Path::new("relative-executable")).is_err());
        assert!(launch_path(None, &dir.path().join("missing")).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn unix_child_gets_own_session_and_keeps_graphical_environment() {
        unsafe extern "C" {
            fn getpgid(pid: i32) -> i32;
            fn getsid(pid: i32) -> i32;
        }
        let mut command = std::process::Command::new("/bin/sh");
        configure_detached(&mut command);
        command.arg("-c").arg("printf '%s|%s|%s' \"$DISPLAY\" \"$WAYLAND_DISPLAY\" \"$DBUS_SESSION_BUS_ADDRESS\"; read NOMIFUN_TEST_DONE")
            .env("DISPLAY",":nomifun-test").env("WAYLAND_DISPLAY","wayland-nomifun-test").env("DBUS_SESSION_BUS_ADDRESS","unix:path=/nomifun-test-bus")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped());
        let mut child = command.spawn().unwrap();
        let id = child.id() as i32;
        assert_eq!(unsafe { getpgid(id) }, id);
        assert_eq!(unsafe { getsid(id) }, id);
        child.stdin.take().unwrap().write_all(b"\n").unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            ":nomifun-test|wayland-nomifun-test|unix:path=/nomifun-test-bus"
        );
    }
    #[test]
    fn stopped_gui_disconnect_leaves_owner_ready_for_next_main() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let path = root.join("owner.json");
        let owner = std::thread::spawn(move || run_at(root, false));
        let start = Instant::now();
        while !path.exists() {
            assert!(start.elapsed() < Duration::from_secs(3));
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut first = Client::connect_at(path.clone()).unwrap();
        first.next().unwrap();
        first
            .control(Control::Policy {
                voice_key: "none".into(),
            })
            .unwrap();
        first.control(Control::DesktopStopped).unwrap();
        drop(first);
        let mut next = loop {
            if let Ok(client) = Client::connect_at(path.clone()) {
                break client;
            }
            assert!(start.elapsed() < Duration::from_secs(4));
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(!owner.is_finished() && path.exists());
        assert!(matches!(next.next().unwrap(), Event::Status(_)));
        // A fresh GUI connection is permitted after the previous GUI's proof;
        // K1 cold launch uses the existing no-subscriber path and launch gate.
        next.control(Control::DesktopStopped).unwrap();
        request_shutdown_at(path).unwrap();
        owner.join().unwrap().unwrap();
    }
}
