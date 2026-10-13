//! Public LaunchServices admission for the development shell and fixed audit.
//! The private descriptor is never an ordinary production startup interface.
use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::{self, Read, Write},
    os::unix::{fs::{MetadataExt, OpenOptionsExt}, net::{UnixListener, UnixStream}},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

use block2::RcBlock;
use objc2::{rc::Retained, runtime::NSObjectProtocol};
use objc2_app_kit::{NSRunningApplication, NSWorkspace, NSWorkspaceOpenConfiguration};
use objc2_foundation::{NSArray, NSDate, NSDictionary, NSError, NSRunLoop, NSString, NSURL};
use serde::{Deserialize, Serialize};

pub const DESCRIPTOR_ARGUMENT: &str = "--nomifun-development-launch-descriptor";
const MAX_FRAME_BYTES: usize = 1024 * 1024;
const ADMISSION_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationScope { DesktopDevelopment, BrowserMediaAudit }

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Descriptor {
    protocol_version: u32,
    nonce: String,
    application_scope: ApplicationScope,
    app_path: PathBuf,
    program: PathBuf,
    cwd: PathBuf,
    args: Vec<String>,
    environment: BTreeMap<String, String>,
    socket_path: PathBuf,
    stdout_path: PathBuf,
    stderr_path: PathBuf,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Hello {
    event: String,
    nonce: String,
    application_scope: ApplicationScope,
    program: PathBuf,
    cwd: PathBuf,
    data_dir: PathBuf,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ShutdownReceipt {
    event: String,
    nonce: String,
    cleanup_verified: bool,
    exit_code: i32,
    restart_requested: bool,
}

fn invalid(message: &'static str) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, message) }

fn private_file(path: &Path) -> io::Result<std::fs::File> {
    if !path.is_absolute() { return Err(invalid("development descriptor path must be absolute")); }
    let directory = path.parent().ok_or_else(|| invalid("development descriptor has no directory"))?;
    let parent = std::fs::symlink_metadata(directory)?;
    if !parent.is_dir() || parent.uid() != unsafe { libc::geteuid() } || parent.mode() & 0o777 != 0o700 {
        return Err(invalid("development descriptor directory must be private and owned"));
    }
    // A FIFO must not block admission before its non-regular type is rejected.
    let file = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o777 != 0o600 || metadata.len() > MAX_FRAME_BYTES as u64 {
        return Err(invalid("development descriptor must be a private owned bounded file"));
    }
    Ok(file)
}

fn descriptor(path: &Path) -> io::Result<Descriptor> {
    let request: Descriptor = serde_json::from_reader(private_file(path)?)
        .map_err(|_| invalid("development descriptor format is invalid"))?;
    let directory = path.parent().unwrap();
    if request.protocol_version != 1 || request.nonce.len() != 64
        || !request.nonce.bytes().all(|byte| byte.is_ascii_hexdigit())
        || !request.app_path.is_absolute() || !request.program.is_absolute() || !request.cwd.is_absolute()
        || request.socket_path.parent() != Some(directory)
        || request.stdout_path.parent() != Some(directory) || request.stderr_path.parent() != Some(directory)
        || request.args.iter().any(|argument| argument.contains('\0') || argument == DESCRIPTOR_ARGUMENT)
        || request.environment.iter().any(|(key, value)| key.is_empty() || key.contains(['\0', '=']) || value.contains('\0')) {
        return Err(invalid("development descriptor fields are invalid"));
    }
    if request.program.canonicalize()? != request.app_path.join("Contents/MacOS/nomifun-desktop").canonicalize()? {
        return Err(invalid("development program is outside its exact application"));
    }
    let data = request.environment.get("NOMIFUN_DATA_DIR")
        .filter(|value| !value.is_empty() && Path::new(value).is_absolute())
        .ok_or_else(|| invalid("development launch requires an explicit absolute data root"))?;
    if data.contains('\0') { return Err(invalid("development data root is invalid")); }
    if request.application_scope == ApplicationScope::BrowserMediaAudit
        && !request.args.iter().any(|argument| argument == "--media-permissions") {
        return Err(invalid("browser media admission requires its fixed audit entry"));
    }
    Ok(request)
}

fn write_frame<T: Serialize>(stream: &mut UnixStream, value: &T) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(value).map_err(|_| invalid("development control frame serialization failed"))?;
    if bytes.len() >= MAX_FRAME_BYTES { return Err(invalid("development control frame exceeds its bound")); }
    bytes.push(b'\n');
    stream.write_all(&bytes)
}

#[derive(Clone, Copy)]
enum FrameReadMode { Blocking, Nonblocking }

fn read_frame(stream: &mut UnixStream, buffered: &mut Vec<u8>, deadline: Instant, mode: FrameReadMode) -> io::Result<Option<serde_json::Value>> {
    loop {
        let remaining = deadline.checked_duration_since(Instant::now()).ok_or_else(||
            io::Error::new(io::ErrorKind::TimedOut, "development control receive deadline exceeded"))?;
        if let Some(end) = buffered.iter().position(|byte| *byte == b'\n') {
            let frame = buffered.drain(..=end).collect::<Vec<_>>();
            return serde_json::from_slice(&frame).map(Some).map_err(|_| invalid("development control frame is invalid"));
        }
        if buffered.len() >= MAX_FRAME_BYTES { return Err(invalid("development control frame exceeds its bound")); }
        // Darwin rejects SO_RCVTIMEO after an AF_UNIX peer closes, even while
        // its final receipt is queued. A nonblocking controller must consume
        // those bytes without changing socket options. Blocking App admission
        // still uses the remaining absolute deadline for each receive.
        if matches!(mode, FrameReadMode::Blocking) {
            stream.set_read_timeout(Some(remaining.min(Duration::from_millis(50))))?;
        }
        let mut bytes = [0u8; 4096];
        match stream.read(&mut bytes) {
            Ok(0) => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "development control peer closed")),
            Ok(count) => {
                buffered.extend_from_slice(&bytes[..count]);
                if buffered.len() > MAX_FRAME_BYTES { return Err(invalid("development control frame exceeds its bound")); }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => return Ok(None),
            Err(error) => return Err(error),
        }
    }
}

fn redirect_output(path: &Path, descriptor: libc::c_int) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    let file = OpenOptions::new().write(true).append(true).custom_flags(libc::O_NOFOLLOW).open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o777 != 0o600 {
        return Err(invalid("development output file must be private and owned"));
    }
    if unsafe { libc::dup2(file.as_raw_fd(), descriptor) } < 0 { return Err(io::Error::last_os_error()); }
    Ok(())
}

/// Called before PATH discovery, storage or native/runtime worker creation.
pub fn prepare_application(scope: ApplicationScope) -> io::Result<Option<ApplicationBootstrap>> {
    let arguments = std::env::args_os().collect::<Vec<_>>();
    let positions = arguments.iter().enumerate().filter(|(_, argument)| *argument == DESCRIPTOR_ARGUMENT).map(|(index, _)| index).collect::<Vec<_>>();
    if positions.is_empty() { return Ok(None); }
    if positions.len() != 1 { return Err(invalid("duplicate development descriptor argument")); }
    let path = arguments.get(positions[0] + 1).map(PathBuf::from)
        .ok_or_else(|| invalid("development descriptor argument is missing"))?;
    let request = descriptor(&path)?;
    if request.application_scope != scope || std::env::current_exe()?.canonicalize()? != request.program.canonicalize()? {
        return Err(invalid("development application scope or executable mismatch"));
    }
    if std::env::var_os("NOMIFUN_DATA_DIR") != request.environment.get("NOMIFUN_DATA_DIR").map(std::ffi::OsString::from) {
        return Err(invalid("LaunchServices did not preserve the explicit development data root"));
    }
    std::env::set_current_dir(&request.cwd)?;
    let cwd = std::env::current_dir()?.canonicalize()?;
    if cwd != request.cwd.canonicalize()? { return Err(invalid("development working directory mismatch")); }
    redirect_output(&request.stdout_path, libc::STDOUT_FILENO)?;
    redirect_output(&request.stderr_path, libc::STDERR_FILENO)?;
    eprintln!("MACOS_DEV_APP_PHASE validated_environment_and_cwd");
    let mut stream = UnixStream::connect(&request.socket_path)?;
    stream.set_read_timeout(Some(ADMISSION_TIMEOUT))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    write_frame(&mut stream, &Hello { event: "hello".into(), nonce: request.nonce.clone(), application_scope: scope,
        program: request.program.canonicalize()?, cwd, data_dir: PathBuf::from(&request.environment["NOMIFUN_DATA_DIR"]) })?;
    eprintln!("MACOS_DEV_APP_PHASE hello_sent");
    let mut bytes = Vec::new();
    let deadline = Instant::now() + ADMISSION_TIMEOUT;
    let ack = loop {
        if let Some(frame) = read_frame(&mut stream, &mut bytes, deadline, FrameReadMode::Blocking)? { break frame; }
        if Instant::now() >= deadline { return Err(invalid("development admission acknowledgement timed out")); }
    };
    if ack["event"] != "admitted" || ack["nonce"] != request.nonce { return Err(invalid("development admission acknowledgement mismatch")); }
    eprintln!("MACOS_DEV_APP_PHASE admission_acknowledged");
    Ok(Some(ApplicationBootstrap { stream, nonce: request.nonce, reported: false, scope }))
}

pub struct ApplicationBootstrap { stream: UnixStream, nonce: String, reported: bool, scope: ApplicationScope }
impl ApplicationBootstrap {
    /// This is a cleanup fact, not exit proof; the controller also needs the
    /// exact registered kernel NOTE_EXITSTATUS before publishing success.
    pub fn report_exit(&mut self, exit_code: i32, cleanup_verified: bool) -> io::Result<()> {
        self.report_exit_with_restart(exit_code, cleanup_verified, false)
    }
    pub fn report_exit_with_restart(&mut self, exit_code: i32, cleanup_verified: bool, restart_requested: bool) -> io::Result<()> {
        if self.reported { return Err(invalid("development exit receipt was already sent")); }
        self.reported = true;
        if restart_requested && (self.scope != ApplicationScope::DesktopDevelopment || exit_code != 0 || !cleanup_verified) {
            return Err(invalid("development restart requires its verified desktop exit"));
        }
        write_frame(&mut self.stream, &ShutdownReceipt { event: "shutdown_verified".into(), nonce: self.nonce.clone(), cleanup_verified, exit_code, restart_requested })
    }
}

/// One application-owned receipt writer shared by AppKit termination and
/// `run_return`. Re-observing the exact delivered result sends no second
/// frame. A failed delivery or a contradictory result never acquires another
/// send attempt. The control socket stays owned until actual process exit.
#[derive(Clone)]
pub struct ApplicationExitReporter {
    state: Arc<Mutex<ApplicationExitReportState>>,
}
struct ApplicationExitReportState {
    bootstrap: Option<ApplicationBootstrap>,
    delivered: Option<(i32, bool, bool)>,
    delivery_failed: bool,
}
impl ApplicationExitReporter {
    pub fn new(bootstrap: Option<ApplicationBootstrap>) -> Self {
        Self { state: Arc::new(Mutex::new(ApplicationExitReportState {
            bootstrap, delivered: None, delivery_failed: false,
        })) }
    }

    pub fn report_exit_once(&self, exit_code: i32, cleanup_verified: bool) -> io::Result<()> {
        self.report_exit_once_with_restart(exit_code, cleanup_verified, false)
    }
    pub fn report_exit_once_with_restart(&self, exit_code: i32, cleanup_verified: bool, restart_requested: bool) -> io::Result<()> {
        let mut state = self.state.lock()
            .map_err(|_| invalid("development exit receipt owner is poisoned"))?;
        if state.bootstrap.is_none() { return Ok(()); }
        if state.delivery_failed {
            return Err(invalid("development exit receipt delivery already failed"));
        }
        if let Some(delivered) = state.delivered {
            return if delivered == (exit_code, cleanup_verified, restart_requested) { Ok(()) }
                else { Err(invalid("development exit receipt result cannot change")) };
        }
        // Commit the one send attempt before writing. Unwind or transport
        // failure cannot be repaired by sending a different successful fact.
        state.delivery_failed = true;
        eprintln!("MACOS_DEV_APP_EXIT_RECEIPT preparing code={exit_code} cleanup_verified={cleanup_verified}");
        let result = state.bootstrap.as_mut().ok_or_else(|| invalid("development receipt owner is missing"))?
            .report_exit_with_restart(exit_code, cleanup_verified, restart_requested);
        if let Err(error) = result {
            eprintln!("MACOS_DEV_APP_EXIT_RECEIPT send_failed error_kind={:?}", error.kind());
            return Err(error);
        }
        state.delivered = Some((exit_code, cleanup_verified, restart_requested));
        state.delivery_failed = false;
        eprintln!("MACOS_DEV_APP_EXIT_RECEIPT sent code={exit_code} cleanup_verified={cleanup_verified}");
        Ok(())
    }

    pub fn terminal_attempt_started(&self) -> bool {
        self.state.lock().map(|state| state.delivery_failed || state.delivered.is_some()).unwrap_or(true)
    }
    pub fn has_launch_owner(&self) -> bool {
        self.state.lock().map(|state| state.bootstrap.is_some()).unwrap_or(false)
    }
}

fn validate_receipt_scope(receipt: &ShutdownReceipt, scope: ApplicationScope) -> io::Result<()> {
    if receipt.restart_requested && (scope != ApplicationScope::DesktopDevelopment || !receipt.cleanup_verified || receipt.exit_code != 0) {
        return Err(invalid("restart receipt is not a verified desktop-development handoff"));
    }
    Ok(())
}

/// Tauri dev may use a release optimization profile. A packaged debug fast
/// build instead embeds custom-protocol assets but retains the explicit Dev
/// bundle identity. An ordinary release bundle is never eligible.
pub fn development_application_is_eligible() -> bool {
    let tauri_development = tauri::is_dev();
    let debug_build = cfg!(debug_assertions);
    if tauri_development || !debug_build { return development_identity_is_eligible(tauri_development, debug_build, None); }
    let Ok(program) = std::env::current_exe() else { return false; };
    let Some(contents) = program.parent().and_then(Path::parent) else { return false; };
    let identifier = plist::Value::from_file(contents.join("Info.plist")).ok()
        .and_then(|value| value.as_dictionary().and_then(|dictionary| dictionary.get("CFBundleIdentifier")).and_then(plist::Value::as_string).map(str::to_owned));
    development_identity_is_eligible(tauri_development, debug_build, identifier.as_deref())
}

fn development_identity_is_eligible(tauri_development: bool, debug_build: bool, bundle_identifier: Option<&str>) -> bool {
    tauri_development || debug_build && bundle_identifier == Some("com.nomifun.desktop.dev")
}

pub fn controller_main(path: &Path) -> io::Result<i32> {
    use std::os::unix::fs::PermissionsExt;
    let request = descriptor(path)?;
    let listener = UnixListener::bind(&request.socket_path)?;
    std::fs::set_permissions(&request.socket_path, std::fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    let (sender, receiver) = mpsc::channel::<io::Result<Retained<NSRunningApplication>>>();
    let completion = RcBlock::new(move |application: *mut NSRunningApplication, error: *mut NSError| {
        let result = if !error.is_null() || application.is_null() { Err(invalid("LaunchServices did not create the requested application")) }
        else {
            // Keep AppKit's exact application object after the completion
            // block. NSRunningApplication is publicly Send + Sync.
            unsafe { Retained::retain(application) }
                .ok_or_else(|| invalid("LaunchServices application identity is unavailable"))
        };
        let _ = sender.send(result);
    });
    let configuration = NSWorkspaceOpenConfiguration::configuration();
    configuration.setCreatesNewApplicationInstance(true);
    configuration.setAllowsRunningApplicationSubstitution(false);
    configuration.setAddsToRecentItems(false);
    configuration.setPromptsUserIfNeeded(false);
    configuration.setActivates(true);
    let arguments = request.args.iter().cloned().chain([DESCRIPTOR_ARGUMENT.into(), path.to_string_lossy().into_owned()]).map(|value| NSString::from_str(&value)).collect::<Vec<_>>();
    configuration.setArguments(&NSArray::from_retained_slice(&arguments));
    let keys = request.environment.keys().map(|key| NSString::from_str(key)).collect::<Vec<_>>();
    let values = request.environment.values().map(|value| NSString::from_str(value)).collect::<Vec<_>>();
    configuration.setEnvironment(&NSDictionary::from_slices(&keys.iter().map(|key| &**key).collect::<Vec<_>>(), &values.iter().map(|value| &**value).collect::<Vec<_>>()));
    let url = NSURL::fileURLWithPath(&NSString::from_str(&request.app_path.to_string_lossy()));
    NSWorkspace::sharedWorkspace().openApplicationAtURL_configuration_completionHandler(&url, &configuration, Some(&completion));
    eprintln!("MACOS_DEV_CONTROLLER_PHASE launch_submitted");
    let deadline = Instant::now() + ADMISSION_TIMEOUT;
    let mut launched = None;
    let mut peer = None;
    let mut buffered = Vec::new();
    let mut hello = None;
    while hello.is_none() {
        NSRunLoop::currentRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.02));
        if launched.is_none() {
            match receiver.try_recv() { Ok(result) => { eprintln!("MACOS_DEV_CONTROLLER_PHASE callback_received"); launched = Some(result?); }, Err(mpsc::TryRecvError::Empty) => {}, Err(_) => return Err(invalid("LaunchServices completion channel closed")) }
        }
        if peer.is_none() {
            match listener.accept() {
                Ok((stream, _)) => { stream.set_nonblocking(true)?; peer = Some(stream); eprintln!("MACOS_DEV_CONTROLLER_PHASE peer_connected"); },
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {},
                Err(error) => return Err(error),
            }
        }
        if hello.is_none() && let Some(stream) = peer.as_mut() && let Some(frame) = read_frame(stream, &mut buffered, deadline, FrameReadMode::Nonblocking)? {
            hello = Some(serde_json::from_value::<Hello>(frame).map_err(|_| invalid("development hello format is invalid"))?);
            eprintln!("MACOS_DEV_CONTROLLER_PHASE hello_received");
        }
        if Instant::now() >= deadline { eprintln!("MACOS_DEV_CONTROLLER_ADMISSION_TIMEOUT callback_received={} peer_connected={} hello_received={}", launched.is_some(), peer.is_some(), hello.is_some()); return Err(invalid("LaunchServices application admission timed out")); }
    }
    let hello = hello.unwrap();
    if hello.event != "hello" || hello.nonce != request.nonce || hello.application_scope != request.application_scope
        || hello.program != request.program.canonicalize()? || hello.cwd != request.cwd.canonicalize()?
        || hello.data_dir != PathBuf::from(&request.environment["NOMIFUN_DATA_DIR"]) {
        return Err(invalid("LaunchServices application admission facts did not match"));
    }
    let mut peer = peer.unwrap();
    let pid = nomi_process_runtime::launched_application_peer_pid(&peer)?;
    // Completion may wait for finishLaunching; startup is deliberately held
    // before runtime workers until its kernel exit knote is registered. Use
    // only the authenticated peer's exact public record to break that cycle.
    let provisional = loop {
        if let Some(application) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid as i32) {
            validate_running_application(&application, pid, &request)?;
            break application;
        }
        if Instant::now() >= deadline { return Err(invalid("LaunchServices exact application lookup timed out")); }
        NSRunLoop::currentRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.02));
    };
    let mut observer = nomi_process_runtime::own_launched_application(&peer, pid, &request.program)?;
    eprintln!("MACOS_DEV_CONTROLLER_PHASE exact_kernel_observer_registered");
    peer.set_nonblocking(false)?;
    peer.set_write_timeout(Some(Duration::from_secs(5)))?;
    write_frame(&mut peer, &serde_json::json!({ "event":"admitted", "nonce":request.nonce }))?;
    eprintln!("MACOS_DEV_CONTROLLER_PHASE admission_ack_sent");
    peer.set_nonblocking(true)?;
    // The provisional ACK grants startup, not successful launch. Require the
    // later callback to name the very same object and kernel incarnation.
    while launched.is_none() {
        NSRunLoop::currentRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.02));
        match receiver.try_recv() {
            Ok(result) => { eprintln!("MACOS_DEV_CONTROLLER_PHASE callback_received"); launched = Some(result?); },
            Err(mpsc::TryRecvError::Empty) => {},
            Err(_) => return Err(invalid("LaunchServices completion channel closed")),
        }
        if observer.poll_exit(Duration::ZERO)?.is_some() { return Err(invalid("development application exited before LaunchServices confirmation")); }
        if Instant::now() >= deadline { return Err(invalid("LaunchServices completion confirmation timed out")); }
    }
    let launched = launched.unwrap();
    validate_running_application(&launched, pid, &request)?;
    if !provisional.isEqual(Some(&launched)) { return Err(invalid("LaunchServices callback substituted the admitted application")); }
    observer.confirm_running_generation()?;
    eprintln!("MACOS_DEV_CONTROLLER_PHASE completed_launch_identity_confirmed");
    println!("{}", serde_json::json!({"event":"ready","pid":pid,"cwd":hello.cwd,"data_dir":hello.data_dir}));
    let mut receipt = None;
    let mut peer_closed = false;
    loop {
        if receipt.is_none() && !peer_closed {
            match read_frame(&mut peer, &mut buffered, Instant::now() + Duration::from_millis(50), FrameReadMode::Nonblocking) {
                Ok(Some(frame)) => {
                    let received = serde_json::from_value::<ShutdownReceipt>(frame).map_err(|_| invalid("development shutdown receipt is invalid"))?;
                    validate_receipt_scope(&received, request.application_scope)?;
                    eprintln!("MACOS_DEV_CONTROLLER_EXIT receipt_received code={} cleanup_verified={}", received.exit_code, received.cleanup_verified);
                    receipt = Some(received);
                },
                Ok(None) => {},
                Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => { eprintln!("MACOS_DEV_CONTROLLER_EXIT peer_closed receipt_received=false"); peer_closed = true; },
                Err(error) => { eprintln!("MACOS_DEV_CONTROLLER_EXIT receipt_read_failed error_kind={:?}", error.kind()); return Err(error); },
            }
        }
        if let Some(status) = observer.poll_exit(Duration::from_millis(50)).map_err(|error| {
            eprintln!("MACOS_DEV_CONTROLLER_EXIT observer_failed reason={} error_kind={:?} native_errno={:?} receipt_received={} peer_closed={}", observer_error_category(&error), error.kind(), error.raw_os_error(), receipt.is_some(), peer_closed);
            error
        })? {
            eprintln!("MACOS_DEV_CONTROLLER_EXIT actual_status code={:?} signal={:?} receipt_received={} peer_closed={}", status.code, status.signal, receipt.is_some(), peer_closed);
            // Drain the bytes queued before the exact kernel exit; EOF alone
            // never supplies a successful cleanup or application exit status.
            if receipt.is_none() && !peer_closed && let Some(frame) = read_frame(&mut peer, &mut buffered, Instant::now() + Duration::from_secs(5), FrameReadMode::Nonblocking).map_err(|error| {
                eprintln!("MACOS_DEV_CONTROLLER_EXIT terminal_receipt_read_failed error_kind={:?}", error.kind()); error
            })? {
                let received = serde_json::from_value::<ShutdownReceipt>(frame).map_err(|_| invalid("development shutdown receipt is invalid"))?;
                validate_receipt_scope(&received, request.application_scope)?;
                eprintln!("MACOS_DEV_CONTROLLER_EXIT terminal_receipt_received code={} cleanup_verified={}", received.exit_code, received.cleanup_verified);
                receipt = Some(received);
            }
            let verified = receipt.as_ref().is_some_and(|receipt| receipt.event == "shutdown_verified" && receipt.nonce == request.nonce
                && receipt.cleanup_verified && receipt.exit_code == 0 && status.code == Some(0) && status.signal.is_none());
            eprintln!("MACOS_DEV_CONTROLLER_EXIT final_verified={verified}");
            let restart_requested = verified && receipt.as_ref().is_some_and(|receipt| receipt.restart_requested);
            println!("{}", serde_json::json!({"event":"exit","pid":pid,"code":status.code,"signal":status.signal,"cleanup_verified":verified,"restart_requested":restart_requested}));
            return Ok(if verified { 0 } else { 1 });
        }
    }
}

fn validate_running_application(application: &NSRunningApplication, pid: u32, request: &Descriptor) -> io::Result<()> {
    let paths = application.bundleURL().and_then(|url| url.path())
        .zip(application.executableURL().and_then(|url| url.path()))
        .ok_or_else(|| invalid("LaunchServices application public paths are unavailable"))?;
    if application.processIdentifier() != pid as i32
        || PathBuf::from(paths.0.to_string()).canonicalize()? != request.app_path.canonicalize()?
        || PathBuf::from(paths.1.to_string()).canonicalize()? != request.program.canonicalize()? {
        return Err(invalid("LaunchServices public application record did not match admission"));
    }
    Ok(())
}

fn observer_error_category(error: &io::Error) -> &'static str {
    // Output only known static categories, never native messages or paths.
    match error.to_string().as_str() {
        "admitted application exec changed; exit/cleanup is unknown" => "exec_changed",
        "application exit observation is missing the kernel exit status" => "exit_status_missing",
        "application exit observation has an invalid identity or kernel error" => "event_identity_or_error",
        "application exit status is outside the native wait-status range" => "wait_status_outside_range",
        "application observation is not a terminal wait status" => "wait_status_nonterminal",
        "admitted application generation changed; exit/cleanup is unknown" => "generation_changed",
        "admitted application executable changed; exit/cleanup is unknown" => "executable_changed",
        _ if error.raw_os_error().is_some() => "native_error",
        _ => "observation_uncertain",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protocol_rejects_missing_cleanup_as_exit_proof() {
        let receipt: Result<ShutdownReceipt, _> = serde_json::from_value(serde_json::json!({"event":"shutdown_verified","nonce":"private","exit_code":0,"restart_requested":false}));
        assert!(receipt.is_err());
    }
    #[test]
    fn nonblocking_queued_receipt_survives_the_real_unix_peer_exit() {
        let (mut reader, mut writer) = UnixStream::pair().unwrap();
        reader.set_nonblocking(true).unwrap();
        write_frame(&mut writer, &ShutdownReceipt { event: "shutdown_verified".into(), nonce: "fixture".into(), cleanup_verified: true, exit_code: 0, restart_requested: false }).unwrap();
        drop(writer);
        let received = read_frame(&mut reader, &mut Vec::new(), Instant::now() + Duration::from_secs(1), FrameReadMode::Nonblocking).unwrap().unwrap();
        let receipt: ShutdownReceipt = serde_json::from_value(received).unwrap();
        assert!(receipt.cleanup_verified);
        assert_eq!(receipt.exit_code, 0);
        assert_eq!(receipt.event, "shutdown_verified");
    }
    #[test]
    fn private_descriptor_rejects_permissions_symlinks_fifos_and_oversize() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        use std::os::unix::ffi::OsStrExt;
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = root.path().join("descriptor.json");
        let file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path).unwrap();
        assert!(private_file(&path).is_ok());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(private_file(&path).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(private_file(&path).is_err());
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let link = root.path().join("descriptor-link.json");
        symlink(&path, &link).unwrap();
        assert!(private_file(&link).is_err());
        let directory_link = root.path().join("directory-link");
        symlink(root.path(), &directory_link).unwrap();
        assert!(private_file(&directory_link.join("descriptor.json")).is_err());
        let fifo = root.path().join("descriptor.fifo");
        let fifo_c = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo_c.as_ptr(), 0o600) }, 0);
        assert!(private_file(&fifo).is_err());
        file.set_len(MAX_FRAME_BYTES as u64 + 1).unwrap();
        assert!(private_file(&path).is_err());
        assert!(private_file(Path::new("relative-descriptor.json")).is_err());
    }
    #[test]
    fn shutdown_receipt_is_one_shot_even_after_failed_delivery() {
        let (mut reader, writer) = UnixStream::pair().unwrap();
        let mut bootstrap = ApplicationBootstrap { stream: writer, nonce: "fixture".into(), reported: false, scope: ApplicationScope::DesktopDevelopment };
        bootstrap.report_exit(0, false).unwrap();
        assert!(bootstrap.report_exit(0, true).is_err());
        let frame = read_frame(&mut reader, &mut Vec::new(), Instant::now() + Duration::from_secs(1), FrameReadMode::Blocking).unwrap().unwrap();
        assert_eq!(frame["cleanup_verified"], false);
        let (reader, writer) = UnixStream::pair().unwrap();
        drop(reader);
        let mut failed = ApplicationBootstrap { stream: writer, nonce: "fixture".into(), reported: false, scope: ApplicationScope::DesktopDevelopment };
        assert!(failed.report_exit(0, true).is_err());
        assert!(failed.report_exit(0, true).is_err());
    }
    #[test]
    fn shared_exit_reporter_delivers_one_actual_frame_across_concurrent_routes() {
        let (mut reader, writer) = UnixStream::pair().unwrap();
        let reporter = ApplicationExitReporter::new(Some(ApplicationBootstrap {
            stream: writer, nonce: "fixture".into(), reported: false, scope: ApplicationScope::DesktopDevelopment,
        }));
        let other_route = reporter.clone();
        let gate = Arc::new(std::sync::Barrier::new(2));
        let other_gate = gate.clone();
        let thread = std::thread::spawn(move || {
            other_gate.wait();
            other_route.report_exit_once(0, true)
        });
        gate.wait();
        reporter.report_exit_once(0, true).unwrap();
        thread.join().unwrap().unwrap();
        let mut buffered = Vec::new();
        let frame = read_frame(&mut reader, &mut buffered, Instant::now() + Duration::from_secs(1), FrameReadMode::Blocking).unwrap().unwrap();
        assert_eq!(frame["exit_code"], 0);
        assert_eq!(frame["cleanup_verified"], true);
        reader.set_nonblocking(true).unwrap();
        assert!(read_frame(&mut reader, &mut buffered, Instant::now() + Duration::from_secs(1), FrameReadMode::Nonblocking).unwrap().is_none(), "no duplicate frame; peer stays owned until process exit");
        assert!(reporter.report_exit_once(1, true).is_err());
        assert!(reporter.report_exit_once(0, false).is_err());
    }
    #[test]
    fn shared_exit_reporter_cannot_upgrade_failure_or_retry_broken_transport() {
        let (mut reader, writer) = UnixStream::pair().unwrap();
        let reporter = ApplicationExitReporter::new(Some(ApplicationBootstrap {
            stream: writer, nonce: "fixture".into(), reported: false, scope: ApplicationScope::DesktopDevelopment,
        }));
        reporter.report_exit_once(1, false).unwrap();
        assert!(reporter.clone().report_exit_once(0, true).is_err());
        let mut buffered = Vec::new();
        let frame = read_frame(&mut reader, &mut buffered, Instant::now() + Duration::from_secs(1), FrameReadMode::Blocking).unwrap().unwrap();
        assert_eq!(frame["exit_code"], 1);
        assert_eq!(frame["cleanup_verified"], false);
        reader.set_nonblocking(true).unwrap();
        assert!(read_frame(&mut reader, &mut buffered, Instant::now() + Duration::from_secs(1), FrameReadMode::Nonblocking).unwrap().is_none(), "failure cannot publish a second successful receipt");

        let (reader, writer) = UnixStream::pair().unwrap();
        drop(reader);
        let failed = ApplicationExitReporter::new(Some(ApplicationBootstrap {
            stream: writer, nonce: "fixture".into(), reported: false, scope: ApplicationScope::DesktopDevelopment,
        }));
        assert!(failed.report_exit_once(0, true).is_err());
        assert!(failed.clone().report_exit_once(0, true).is_err());
        assert!(failed.report_exit_once(1, false).is_err());
        let state = failed.state.lock().unwrap();
        assert!(state.delivery_failed);
        assert!(state.delivered.is_none());
        assert!(state.bootstrap.as_ref().unwrap().reported);
    }
    #[test]
    fn ordinary_application_has_no_development_receipt_authority() {
        let absent = ApplicationExitReporter::new(None);
        absent.report_exit_once(0, true).unwrap();
        assert!(absent.state.lock().unwrap().delivered.is_none());
    }
    #[test]
    fn restart_intent_is_a_required_boolean_and_media_never_has_restart_authority() {
        let base = serde_json::json!({"event":"shutdown_verified","nonce":"fixture","exit_code":0,"cleanup_verified":true});
        assert!(serde_json::from_value::<ShutdownReceipt>(base.clone()).is_err());
        for invalid_flag in [serde_json::Value::Null, serde_json::json!(0), serde_json::json!("true")] {
            let mut value = base.clone();
            value["restart_requested"] = invalid_flag;
            assert!(serde_json::from_value::<ShutdownReceipt>(value).is_err());
        }
        let mut value = base;
        value["restart_requested"] = serde_json::json!(true);
        let mut receipt: ShutdownReceipt = serde_json::from_value(value).unwrap();
        validate_receipt_scope(&receipt, ApplicationScope::DesktopDevelopment).unwrap();
        assert!(validate_receipt_scope(&receipt, ApplicationScope::BrowserMediaAudit).is_err());
        receipt.cleanup_verified = false;
        assert!(validate_receipt_scope(&receipt, ApplicationScope::DesktopDevelopment).is_err());
        receipt.cleanup_verified = true;
        receipt.exit_code = 1;
        assert!(validate_receipt_scope(&receipt, ApplicationScope::DesktopDevelopment).is_err());
    }
    #[test]
    fn shared_restart_intent_is_immutable_in_the_single_actual_receipt() {
        let (mut reader, writer) = UnixStream::pair().unwrap();
        let reporter = ApplicationExitReporter::new(Some(ApplicationBootstrap {
            stream: writer, nonce: "fixture".into(), reported: false, scope: ApplicationScope::DesktopDevelopment,
        }));
        assert!(!reporter.terminal_attempt_started());
        reporter.report_exit_once_with_restart(0, true, true).unwrap();
        assert!(reporter.terminal_attempt_started());
        reporter.clone().report_exit_once_with_restart(0, true, true).unwrap();
        assert!(reporter.report_exit_once(0, true).is_err());
        let mut buffered = Vec::new();
        let frame = read_frame(&mut reader, &mut buffered, Instant::now() + Duration::from_secs(1), FrameReadMode::Blocking).unwrap().unwrap();
        assert_eq!(frame["restart_requested"], true);
        reader.set_nonblocking(true).unwrap();
        assert!(read_frame(&mut reader, &mut buffered, Instant::now() + Duration::from_secs(1), FrameReadMode::Nonblocking).unwrap().is_none());

        let (mut reader, writer) = UnixStream::pair().unwrap();
        let normal = ApplicationExitReporter::new(Some(ApplicationBootstrap {
            stream: writer, nonce: "fixture".into(), reported: false, scope: ApplicationScope::DesktopDevelopment,
        }));
        normal.report_exit_once(0, true).unwrap();
        assert!(normal.report_exit_once_with_restart(0, true, true).is_err());
        let mut buffered = Vec::new();
        let frame = read_frame(&mut reader, &mut buffered, Instant::now() + Duration::from_secs(1), FrameReadMode::Blocking).unwrap().unwrap();
        assert_eq!(frame["restart_requested"], false);
        reader.set_nonblocking(true).unwrap();
        assert!(read_frame(&mut reader, &mut buffered, Instant::now() + Duration::from_secs(1), FrameReadMode::Nonblocking).unwrap().is_none());
    }
    #[test]
    fn rejected_media_restart_attempt_cannot_be_replaced_with_a_successful_frame() {
        let (mut reader, writer) = UnixStream::pair().unwrap();
        let reporter = ApplicationExitReporter::new(Some(ApplicationBootstrap {
            stream: writer, nonce: "fixture".into(), reported: false, scope: ApplicationScope::BrowserMediaAudit,
        }));
        assert!(reporter.report_exit_once_with_restart(0, true, true).is_err());
        assert!(reporter.report_exit_once(0, true).is_err());
        assert!(reporter.terminal_attempt_started());
        reader.set_nonblocking(true).unwrap();
        assert!(read_frame(&mut reader, &mut Vec::new(), Instant::now() + Duration::from_secs(1), FrameReadMode::Nonblocking).unwrap().is_none());
    }
    #[test]
    fn one_eligibility_policy_serves_early_admission_and_lifetime_observers() {
        assert!(development_identity_is_eligible(true, false, None)); // release-optimized Tauri dev
        assert!(development_identity_is_eligible(false, true, Some("com.nomifun.desktop.dev"))); // packaged debug-fast
        assert!(!development_identity_is_eligible(false, false, Some("com.nomifun.desktop.dev")));
        assert!(!development_identity_is_eligible(false, true, Some("com.nomifun.desktop")));
        assert!(!development_identity_is_eligible(false, true, None));
        assert!(!development_identity_is_eligible(false, false, None));
    }
    #[test]
    fn slow_partial_frame_cannot_restart_the_absolute_admission_deadline() {
        let (mut reader, mut writer) = UnixStream::pair().unwrap();
        let sending = std::thread::spawn(move || {
            for byte in b"{\"event\":\"admitted\"}\n" {
                if writer.write_all(&[*byte]).is_err() { break; }
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        let start = Instant::now();
        let deadline = start + Duration::from_millis(30);
        let mut bytes = Vec::new();
        let error = loop {
            match read_frame(&mut reader, &mut bytes, deadline, FrameReadMode::Blocking) {
                Err(error) => break error,
                Ok(None) => continue,
                Ok(Some(_)) => panic!("slow incomplete frame must not be admitted"),
            }
        };
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(start.elapsed() < Duration::from_millis(250));
        drop(reader);
        sending.join().unwrap();
    }
}
