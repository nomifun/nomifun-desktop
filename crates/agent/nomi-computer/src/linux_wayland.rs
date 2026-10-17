//! Native combined RemoteDesktop/ScreenCast session ownership.
//!
//! Only an actual operation calls `session`, which asks the desktop portal's
//! system UI for input and monitor sharing. Availability probes and input calls
//! never create a session. Lost/unknown input closes its native session; no
//! notification is replayed and no input operation silently reauthorizes.

use std::{
    collections::HashMap,
    fmt,
    future::Future,
    os::fd::OwnedFd,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use futures_util::StreamExt;
use tokio::sync::{Mutex, watch};
use zbus::{
    Connection, MatchRule, MessageStream,
    message::Type,
    zvariant::{OwnedObjectPath, OwnedValue},
};

type Values = HashMap<String, OwnedValue>;
const DESTINATION: &str = "org.freedesktop.portal.Desktop";
const DESKTOP_PATH: &str = "/org/freedesktop/portal/desktop";
const REMOTE: &str = "org.freedesktop.portal.RemoteDesktop";
const SCREENCAST: &str = "org.freedesktop.portal.ScreenCast";
const REQUEST: &str = "org.freedesktop.portal.Request";
const SESSION: &str = "org.freedesktop.portal.Session";
const METHOD_TIMEOUT: Duration = Duration::from_secs(5);
const UI_TIMEOUT: Duration = Duration::from_secs(120);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaylandError {
    code: &'static str,
    message: String,
}

impl WaylandError {
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    pub fn code(&self) -> &'static str {
        self.code
    }
}

impl fmt::Display for WaylandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for WaylandError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalStream {
    pub node_id: u32,
    /// ScreenCast v6's stable target identity prevents node-ID reuse after
    /// display reconfiguration from selecting an unrelated PipeWire stream.
    pub pipewire_serial: Option<u64>,
    /// Portal logical desktop coordinates; they are not raw frame pixels.
    pub position: Option<(i32, i32)>,
    pub size: Option<(u32, u32)>,
}

pub struct PortalSession {
    connection: Connection,
    handle: OwnedObjectPath,
    generation: u64,
    streams: Vec<PortalStream>,
    devices: u32,
    closed: AtomicBool,
    close_confirmed: AtomicBool,
    close_gate: Mutex<()>,
    monitor_stop: Arc<tokio::sync::Notify>,
    input_gate: Mutex<()>,
}

impl PortalSession {
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }
    pub fn streams(&self) -> &[PortalStream] {
        &self.streams
    }

    fn check_active(&self) -> Result<(), WaylandError> {
        if self.is_closed() {
            Err(WaylandError::new(
                "WAYLAND_SESSION_CLOSED",
                "The shared desktop session ended. Capture a fresh screenshot to request a new native session before sending input.",
            ))
        } else {
            Ok(())
        }
    }

    /// Obtain the compositor-provided PipeWire remote for this exact session.
    /// Keep this session alive until the frame consumer finishes with the fd.
    pub async fn open_pipewire_remote(self: &Arc<Self>) -> Result<OwnedFd, WaylandError> {
        let session = self.clone();
        dispatch(async move {
            session.check_active()?;
            let result = session
                .connection
                .call_method(
                    Some(DESTINATION),
                    DESKTOP_PATH,
                    Some(SCREENCAST),
                    "OpenPipeWireRemote",
                    &(session.handle.clone(), Values::new()),
                )
                .await;
            let fd: zbus::zvariant::OwnedFd = match result {
                Ok(message) => message.body().deserialize().map_err(|error| {
                    WaylandError::new(
                        "WAYLAND_SCREENCAST_FAILED",
                        format!("Invalid PipeWire remote: {error}"),
                    )
                })?,
                Err(error) => {
                    session.invalidate();
                    return Err(WaylandError::new(
                        "WAYLAND_SCREENCAST_FAILED",
                        error.to_string(),
                    ));
                }
            };
            session.check_active()?;
            Ok(fd.into())
        })
        .await
    }

    pub async fn notify_pointer_absolute(
        self: &Arc<Self>,
        node_id: u32,
        x: f64,
        y: f64,
    ) -> Result<(), WaylandError> {
        self.pointer_absolute_inner(None, node_id, x, y).await
    }

    pub(crate) async fn notify_pointer_absolute_cancellable(
        self: &Arc<Self>,
        cancel: Arc<AtomicBool>,
        node_id: u32,
        x: f64,
        y: f64,
    ) -> Result<(), WaylandError> {
        self.pointer_absolute_inner(Some(cancel), node_id, x, y)
            .await
    }

    async fn pointer_absolute_inner(
        self: &Arc<Self>,
        cancel: Option<Arc<AtomicBool>>,
        node_id: u32,
        x: f64,
        y: f64,
    ) -> Result<(), WaylandError> {
        if !x.is_finite() || !y.is_finite() || x < 0.0 || y < 0.0 {
            return Err(WaylandError::new(
                "WAYLAND_INVALID_COORDINATES",
                "Pointer coordinates must be finite and relative to a shared stream",
            ));
        }
        let stream = self
            .streams
            .iter()
            .find(|stream| stream.node_id == node_id)
            .ok_or_else(|| {
                WaylandError::new(
                    "WAYLAND_STALE_SCREENSHOT",
                    "The pointer target is not a stream in this desktop session",
                )
            })?;
        if stream
            .size
            .is_some_and(|(width, height)| x >= f64::from(width) || y >= f64::from(height))
        {
            return Err(WaylandError::new(
                "WAYLAND_INVALID_COORDINATES",
                "Pointer target is outside the shared stream's logical size",
            ));
        }
        let session = self.clone();
        dispatch(async move {
            let _guard = session.input_gate.lock().await;
            check_input_cancelled(cancel.as_deref())?;
            session.require_devices(2)?;
            session
                .notify(
                    "NotifyPointerMotionAbsolute",
                    &(session.handle.clone(), Values::new(), node_id, x, y),
                )
                .await
        })
        .await
    }

    pub async fn pointer_button(
        self: &Arc<Self>,
        evdev: i32,
        pressed: bool,
    ) -> Result<(), WaylandError> {
        self.pointer_button_inner(None, evdev, pressed).await
    }

    pub(crate) async fn pointer_button_cancellable(
        self: &Arc<Self>,
        cancel: Arc<AtomicBool>,
        evdev: i32,
        pressed: bool,
    ) -> Result<(), WaylandError> {
        self.pointer_button_inner(Some(cancel), evdev, pressed)
            .await
    }

    async fn pointer_button_inner(
        self: &Arc<Self>,
        cancel: Option<Arc<AtomicBool>>,
        evdev: i32,
        pressed: bool,
    ) -> Result<(), WaylandError> {
        if !matches!(evdev, 272..=279) {
            return Err(WaylandError::new(
                "WAYLAND_INVALID_BUTTON",
                "Unsupported Linux evdev pointer button",
            ));
        }
        let session = self.clone();
        dispatch(async move {
            let _guard = session.input_gate.lock().await;
            if pressed {
                check_input_cancelled(cancel.as_deref())?;
            }
            session.require_devices(2)?;
            session
                .notify(
                    "NotifyPointerButton",
                    &(
                        session.handle.clone(),
                        Values::new(),
                        evdev,
                        u32::from(pressed),
                    ),
                )
                .await
        })
        .await
    }

    pub async fn keyboard_keysym(
        self: &Arc<Self>,
        keysym: u32,
        pressed: bool,
    ) -> Result<(), WaylandError> {
        self.keyboard_keysym_inner(None, keysym, pressed).await
    }

    pub(crate) async fn keyboard_keysym_cancellable(
        self: &Arc<Self>,
        cancel: Arc<AtomicBool>,
        keysym: u32,
        pressed: bool,
    ) -> Result<(), WaylandError> {
        self.keyboard_keysym_inner(Some(cancel), keysym, pressed)
            .await
    }

    async fn keyboard_keysym_inner(
        self: &Arc<Self>,
        cancel: Option<Arc<AtomicBool>>,
        keysym: u32,
        pressed: bool,
    ) -> Result<(), WaylandError> {
        let keysym = i32::try_from(keysym).map_err(|_| {
            WaylandError::new(
                "WAYLAND_INVALID_KEY",
                "The keysym is outside the portal's signed integer range",
            )
        })?;
        let session = self.clone();
        dispatch(async move {
            let _guard = session.input_gate.lock().await;
            if pressed {
                check_input_cancelled(cancel.as_deref())?;
            }
            session.require_devices(1)?;
            session
                .notify(
                    "NotifyKeyboardKeysym",
                    &(
                        session.handle.clone(),
                        Values::new(),
                        keysym,
                        u32::from(pressed),
                    ),
                )
                .await
        })
        .await
    }

    pub async fn keyboard_keycode(
        self: &Arc<Self>,
        keycode: u32,
        pressed: bool,
    ) -> Result<(), WaylandError> {
        let keycode = i32::try_from(keycode).map_err(|_| {
            WaylandError::new(
                "WAYLAND_INVALID_KEY",
                "The keycode is outside the portal's signed integer range",
            )
        })?;
        let session = self.clone();
        dispatch(async move {
            let _guard = session.input_gate.lock().await;
            session.require_devices(1)?;
            session
                .notify(
                    "NotifyKeyboardKeycode",
                    &(
                        session.handle.clone(),
                        Values::new(),
                        keycode,
                        u32::from(pressed),
                    ),
                )
                .await
        })
        .await
    }

    pub async fn scroll(self: &Arc<Self>, axis: u32, steps: i32) -> Result<(), WaylandError> {
        self.scroll_inner(None, axis, steps).await
    }

    pub(crate) async fn scroll_cancellable(
        self: &Arc<Self>,
        cancel: Arc<AtomicBool>,
        axis: u32,
        steps: i32,
    ) -> Result<(), WaylandError> {
        self.scroll_inner(Some(cancel), axis, steps).await
    }

    async fn scroll_inner(
        self: &Arc<Self>,
        cancel: Option<Arc<AtomicBool>>,
        axis: u32,
        steps: i32,
    ) -> Result<(), WaylandError> {
        if axis > 1 {
            return Err(WaylandError::new(
                "WAYLAND_INVALID_SCROLL",
                "Portal scroll axis must be 0 (vertical) or 1 (horizontal)",
            ));
        }
        let session = self.clone();
        dispatch(async move {
            let _guard = session.input_gate.lock().await;
            check_input_cancelled(cancel.as_deref())?;
            session.require_devices(2)?;
            session
                .notify(
                    "NotifyPointerAxisDiscrete",
                    &(session.handle.clone(), Values::new(), axis, steps),
                )
                .await
        })
        .await
    }

    fn require_devices(&self, requested: u32) -> Result<(), WaylandError> {
        self.check_active()?;
        if self.devices & requested == requested {
            Ok(())
        } else {
            Err(WaylandError::new(
                "WAYLAND_DEVICE_PERMISSION_DENIED",
                "The desktop portal did not grant the requested input device",
            ))
        }
    }

    async fn notify<B>(self: &Arc<Self>, member: &str, body: &B) -> Result<(), WaylandError>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
    {
        self.check_active()?;
        let mut attempt = InputAttemptGuard(Some(self.clone()));
        let result = self
            .connection
            .call_method(Some(DESTINATION), DESKTOP_PATH, Some(REMOTE), member, body)
            .await;
        if let Err(error) = result {
            self.closed.store(true, Ordering::Release);
            let _ = self.close_inner().await;
            attempt.0 = None;
            return Err(WaylandError::new(
                "WAYLAND_INPUT_RESULT_UNKNOWN",
                format!(
                    "The input request failed after it may have reached the compositor; it was not retried. A fresh capture and session are required. {error}"
                ),
            ));
        }
        attempt.0 = None;
        self.check_active()
    }

    pub async fn close(self: &Arc<Self>) -> Result<(), WaylandError> {
        let session = self.clone();
        dispatch(async move {
            session.closed.store(true, Ordering::Release);
            session.close_inner().await
        })
        .await
    }

    async fn close_inner(&self) -> Result<(), WaylandError> {
        let _guard = self.close_gate.lock().await;
        if self.close_confirmed.load(Ordering::Acquire) {
            return Ok(());
        }
        self.connection.call_method(Some(DESTINATION), self.handle.as_str(), Some(SESSION), "Close", &()).await
            .map(|_| { self.close_confirmed.store(true, Ordering::Release); self.monitor_stop.notify_one(); })
            .map_err(|error| WaylandError::new("WAYLAND_SESSION_CLOSE_UNKNOWN", format!("The session is locally closed, but the portal close acknowledgement was lost: {error}")))
    }

    /// A frame consumer calls this when its stream is lost or cannot be mapped.
    /// It forbids every later input immediately and requests native revocation.
    pub fn invalidate(self: &Arc<Self>) {
        self.closed.store(true, Ordering::Release);
        let session = self.clone();
        if let Ok(runtime) = portal_runtime() {
            runtime.spawn(async move {
                let _ = session.close_inner().await;
            });
        }
    }
}

fn check_input_cancelled(cancel: Option<&AtomicBool>) -> Result<(), WaylandError> {
    if cancel.is_some_and(|cancel| cancel.load(Ordering::Acquire)) {
        Err(WaylandError::new(
            "WAYLAND_INPUT_CANCELLED",
            "The input operation was cancelled before sending another compositor event",
        ))
    } else {
        Ok(())
    }
}

/// A cancelled in-flight input may already have reached the compositor. Its
/// native session must end before another operation can reuse the grant.
struct InputAttemptGuard(Option<Arc<PortalSession>>);
impl Drop for InputAttemptGuard {
    fn drop(&mut self) {
        if let Some(session) = self.0.take() {
            session.invalidate();
        }
    }
}

impl Drop for PortalSession {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Release);
        self.monitor_stop.notify_one();
        if self.close_confirmed.load(Ordering::Acquire) {
            return;
        }
        let connection = self.connection.clone();
        let handle = self.handle.clone();
        if let Ok(runtime) = portal_runtime() {
            runtime.spawn(async move {
                close_native(&connection, &handle, SESSION).await;
            });
        }
    }
}

#[derive(Default)]
struct SessionOwner {
    current: Option<Arc<PortalSession>>,
    // RemoteDesktop v2 tokens are single-use. App-lifetime persistence requires
    // no new authorization store and intentionally requests a fresh grant after
    // App restart. Never keep the consumed token on a failed native request.
    restore_token: Option<String>,
}
static OWNER: OnceLock<Mutex<SessionOwner>> = OnceLock::new();
static RUNTIME: OnceLock<Result<tokio::runtime::Runtime, String>> = OnceLock::new();
#[derive(Clone)]
struct PendingCreation {
    id: u64,
    abort: tokio::task::AbortHandle,
    cleanup: watch::Receiver<Option<Result<(), WaylandError>>>,
}
static PENDING: OnceLock<std::sync::Mutex<Option<PendingCreation>>> = OnceLock::new();

/// Before any portal request is issued, cancellation is known to have no
/// native sharing effect. Transfer this sender to CreationGuard afterwards.
struct SetupCleanup(Option<watch::Sender<Option<Result<(), WaylandError>>>>);
impl Drop for SetupCleanup {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(Some(Ok(())));
        }
    }
}

fn take_pending() -> Option<PendingCreation> {
    PENDING
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take()
}

fn clear_pending(id: u64) {
    let mut slot = PENDING
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if slot.as_ref().is_some_and(|pending| pending.id == id) {
        slot.take();
    }
}

async fn wait_cancelled(cancel: &AtomicBool) {
    while !cancel.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn await_creation(
    mut task: tokio::task::JoinHandle<Result<(Arc<PortalSession>, Option<String>), WaylandError>>,
    pending: PendingCreation,
    cancel: Arc<AtomicBool>,
) -> Result<(Arc<PortalSession>, Option<String>), WaylandError> {
    let mut abort = AbortOnDrop(Some(task.abort_handle()));
    let result = tokio::select! {
        result = &mut task => result,
        _ = wait_cancelled(&cancel) => { task.abort(); task.await },
    };
    abort.0 = None;
    // The generation check prevents a late cancelled caller from clearing a
    // later creator's lifecycle handle. The cleanup receiver is this exact
    // native attempt, independent of the global owner slot.
    clear_pending(pending.id);
    match result {
        Err(error) => {
            settle_pending(pending).await?;
            Err(WaylandError::new(
                "WAYLAND_PERMISSION_CANCELLED",
                format!(
                    "The pending native desktop request was interrupted and its native cleanup was acknowledged: {error}"
                ),
            ))
        }
        Ok(Err(error)) => {
            settle_pending(pending).await?;
            Err(error)
        }
        Ok(Ok((session, token))) => {
            if cancel.load(Ordering::Acquire) {
                session.close().await?;
                return Err(WaylandError::new(
                    "WAYLAND_PERMISSION_CANCELLED",
                    "The capture was cancelled and its exact native desktop session was closed",
                ));
            }
            Ok((session, token))
        }
    }
}

async fn settle_pending(mut pending: PendingCreation) -> Result<(), WaylandError> {
    pending.abort.abort();
    tokio::time::timeout(METHOD_TIMEOUT * 3, async {
        loop {
            if let Some(result) = pending.cleanup.borrow().clone() {
                return result;
            }
            pending.cleanup.changed().await.map_err(|_| {
                WaylandError::new(
                    "WAYLAND_SESSION_CLOSE_UNKNOWN",
                    "The pending desktop request ended without a native cleanup receipt",
                )
            })?;
        }
    })
    .await
    .map_err(|_| {
        WaylandError::new(
            "WAYLAND_SESSION_CLOSE_UNKNOWN",
            "Closing the pending desktop sharing request timed out",
        )
    })?
}

fn portal_runtime() -> Result<&'static tokio::runtime::Runtime, WaylandError> {
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .thread_name("nomifun-wayland")
                .enable_all()
                .build()
                .map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(|error| WaylandError::new("WAYLAND_RUNTIME_UNAVAILABLE", error.clone()))
}

struct AbortOnDrop(Option<tokio::task::AbortHandle>);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            handle.abort();
        }
    }
}

async fn dispatch<F, T>(future: F) -> Result<T, WaylandError>
where
    F: Future<Output = Result<T, WaylandError>> + Send + 'static,
    T: Send + 'static,
{
    let task = portal_runtime()?.spawn(future);
    let mut guard = AbortOnDrop(Some(task.abort_handle()));
    let result = task.await.map_err(|error| {
        WaylandError::new(
            "WAYLAND_OPERATION_INTERRUPTED",
            format!("Desktop operation stopped: {error}"),
        )
    })?;
    guard.0 = None;
    result
}

/// Synchronous screenshot workers share the same long-lived native owner.
/// This never nests a Tokio `block_on` in the caller's runtime.
pub fn run_blocking<F, T>(future: F) -> Result<T, WaylandError>
where
    F: Future<Output = Result<T, WaylandError>> + Send + 'static,
    T: Send + 'static,
{
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let task = portal_runtime()?.spawn(async move {
        let _ = sender.send(future.await);
    });
    let mut guard = AbortOnDrop(Some(task.abort_handle()));
    let result = receiver.recv().map_err(|_| {
        WaylandError::new(
            "WAYLAND_OPERATION_INTERRUPTED",
            "The native desktop worker stopped",
        )
    })?;
    guard.0 = None;
    result
}

/// Request or reuse a native portal session. Call only for a real capture,
/// after the canonical tool operation has already passed its normal admission.
pub async fn session() -> Result<Arc<PortalSession>, WaylandError> {
    session_cancellable(Arc::new(AtomicBool::new(false))).await
}

/// A joined screenshot worker passes its own cancellation flag. Cancelling
/// this capture closes only its own pending native request/session and waits
/// for that attempt's real cleanup receipt before returning.
pub async fn session_cancellable(
    cancel: Arc<AtomicBool>,
) -> Result<Arc<PortalSession>, WaylandError> {
    dispatch(async move {
        let mut owner = tokio::select! {
            owner = OWNER.get_or_init(Mutex::default).lock() => owner,
            _ = wait_cancelled(&cancel) => return Err(WaylandError::new("WAYLAND_PERMISSION_CANCELLED", "The capture was cancelled before acquiring the native desktop owner")),
        };
        if cancel.load(Ordering::Acquire) { return Err(WaylandError::new("WAYLAND_PERMISSION_CANCELLED", "The capture was cancelled before requesting desktop sharing")); }
        if let Some(session) = owner
            .current
            .as_ref()
            .filter(|session| !session.is_closed())
        {
            return Ok(session.clone());
        }
        owner.current = None;
        if let Some(pending) = take_pending() {
            settle_pending(pending).await?;
        }
        let token = owner.restore_token.take();
        let (cleanup_sender, cleanup) = watch::channel(None);
        let mut setup = SetupCleanup(Some(cleanup_sender));
        let task = tokio::spawn(async move {
            let connection = zbus::connection::Builder::session()
                .map_err(unavailable)?
                .method_timeout(METHOD_TIMEOUT)
                .build()
                .await
                .map_err(unavailable)?;
            create_session(connection, token, setup.0.take()).await
        });
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let pending = PendingCreation { id, abort: task.abort_handle(), cleanup };
        *PENDING
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(pending.clone());
        let (session, next_token) = await_creation(task, pending, cancel).await?;
        owner.restore_token = next_token;
        owner.current = Some(session.clone());
        Ok(session)
    })
    .await
}

/// Get the existing grant without creating, restoring, or prompting. Input
/// must compare its captured screenshot's generation with this exact owner.
pub async fn active_session() -> Result<Arc<PortalSession>, WaylandError> {
    dispatch(async {
        let owner = OWNER.get_or_init(Mutex::default).try_lock().map_err(|_| WaylandError::new("WAYLAND_PERMISSION_PENDING", "A new native sharing request is pending. Input cannot wait for or reuse a new grant."))?;
        owner.current.as_ref().filter(|session| !session.is_closed()).cloned().ok_or_else(|| WaylandError::new("WAYLAND_SESSION_REQUIRED", "No active desktop sharing grant exists. Capture a new screenshot first; input never reauthorizes or reuses a stale screenshot."))
    }).await
}

/// Availability queries must not wait behind a system sharing dialog.
pub fn session_is_active() -> bool {
    OWNER
        .get()
        .and_then(|owner| owner.try_lock().ok())
        .is_some_and(|owner| {
            owner
                .current
                .as_ref()
                .is_some_and(|session| !session.is_closed())
        })
}

pub async fn close_session() -> Result<(), WaylandError> {
    dispatch(async {
        let pending_cleanup = if let Some(pending) = take_pending() {
            settle_pending(pending).await
        } else {
            Ok(())
        };
        let mut owner = OWNER.get_or_init(Mutex::default).lock().await;
        let session = owner.current.take();
        // Explicit release also discards the reusable native token.
        owner.restore_token = None;
        drop(owner);
        if let Some(session) = session {
            session.close().await?;
        }
        pending_cleanup
    })
    .await
}

fn unavailable(error: impl fmt::Display) -> WaylandError {
    WaylandError::new("WAYLAND_PORTAL_UNAVAILABLE", error.to_string())
}

struct CreationGuard {
    connection: Connection,
    session: Option<OwnedObjectPath>,
    request: Option<OwnedObjectPath>,
    armed: bool,
    cleanup: Option<watch::Sender<Option<Result<(), WaylandError>>>>,
}
impl Drop for CreationGuard {
    fn drop(&mut self) {
        if !self.armed {
            if let Some(cleanup) = self.cleanup.take() {
                let _ = cleanup.send(Some(Ok(())));
            }
            return;
        }
        let connection = self.connection.clone();
        let session = self.session.take();
        let request = self.request.take();
        let cleanup = self.cleanup.take();
        if let Ok(runtime) = portal_runtime() {
            runtime.spawn(async move {
                let mut result = Ok(());
                if let Some(request) = request {
                    if let Err(error) = close_native_result(&connection, &request, REQUEST).await {
                        result = Err(error);
                    }
                }
                if let Some(session) = session {
                    if let Err(error) = close_native_result(&connection, &session, SESSION).await {
                        result = Err(error);
                    }
                }
                if let Some(cleanup) = cleanup {
                    let _ = cleanup.send(Some(result));
                }
            });
        }
    }
}

async fn close_native(connection: &Connection, handle: &OwnedObjectPath, interface: &str) {
    let _ = close_native_result(connection, handle, interface).await;
}

async fn close_native_result(
    connection: &Connection,
    handle: &OwnedObjectPath,
    interface: &str,
) -> Result<(), WaylandError> {
    connection
        .call_method(
            Some(DESTINATION),
            handle.as_str(),
            Some(interface),
            "Close",
            &(),
        )
        .await
        .map(|_| ())
        .map_err(|error| {
            WaylandError::new(
                "WAYLAND_SESSION_CLOSE_UNKNOWN",
                format!("The native {interface} close acknowledgement was not received: {error}"),
            )
        })
}

async fn create_session(
    connection: Connection,
    restore_token: Option<String>,
    cleanup: Option<watch::Sender<Option<Result<(), WaylandError>>>>,
) -> Result<(Arc<PortalSession>, Option<String>), WaylandError> {
    let mut guard = CreationGuard {
        connection: connection.clone(),
        session: None,
        request: None,
        armed: true,
        cleanup,
    };
    let remote = zbus::Proxy::new(&connection, DESTINATION, DESKTOP_PATH, REMOTE)
        .await
        .map_err(unavailable)?;
    let remote_version: u32 = remote.get_property("version").await.map_err(unavailable)?;
    let devices: u32 = remote
        .get_property("AvailableDeviceTypes")
        .await
        .map_err(unavailable)?;
    if devices & 3 != 3 {
        return Err(WaylandError::new(
            "WAYLAND_DEVICE_UNAVAILABLE",
            "The compositor's RemoteDesktop portal does not provide both keyboard and pointer",
        ));
    }
    let capture = zbus::Proxy::new(&connection, DESTINATION, DESKTOP_PATH, SCREENCAST)
        .await
        .map_err(unavailable)?;
    let sources: u32 = capture
        .get_property("AvailableSourceTypes")
        .await
        .map_err(unavailable)?;
    if sources & 1 == 0 {
        return Err(WaylandError::new(
            "WAYLAND_SCREENCAST_UNAVAILABLE",
            "The compositor's ScreenCast portal does not provide monitor sharing",
        ));
    }

    let mut options = Values::new();
    options.insert(
        "session_handle_token".into(),
        string_value(format!(
            "nomifun_session_{}",
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ))?,
    );
    // Predict the session path too: cancellation can occur after creation but
    // before the response carrying the handle reaches us.
    let unique = connection
        .unique_name()
        .ok_or_else(|| unavailable("The desktop bus assigned no unique name"))?
        .as_str()
        .trim_start_matches(':')
        .replace('.', "_");
    let session_token = String::try_from(
        options["session_handle_token"]
            .try_clone()
            .map_err(unavailable)?,
    )
    .map_err(unavailable)?;
    guard.session = Some(
        OwnedObjectPath::try_from(format!("{DESKTOP_PATH}/session/{unique}/{session_token}"))
            .map_err(unavailable)?,
    );
    let mut response = request(
        &mut guard,
        REMOTE,
        "CreateSession",
        RequestBody::Create(options),
    )
    .await?;
    let handle_string = response
        .remove("session_handle")
        .and_then(|value| String::try_from(value).ok())
        .ok_or_else(|| unavailable("CreateSession returned no valid session handle"))?;
    let handle = OwnedObjectPath::try_from(handle_string).map_err(unavailable)?;
    guard.session = Some(handle.clone());

    // Subscribe before Start so immediate closure/revocation is never lost.
    let closed_rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender(DESTINATION)
        .map_err(unavailable)?
        .interface(SESSION)
        .map_err(unavailable)?
        .member("Closed")
        .map_err(unavailable)?
        .path(handle.clone())
        .map_err(unavailable)?
        .build();
    let closed_stream = MessageStream::for_match_rule(closed_rule, &connection, Some(8))
        .await
        .map_err(unavailable)?;
    let owner_rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender("org.freedesktop.DBus")
        .map_err(unavailable)?
        .interface("org.freedesktop.DBus")
        .map_err(unavailable)?
        .member("NameOwnerChanged")
        .map_err(unavailable)?
        .add_arg(DESTINATION)
        .map_err(unavailable)?
        .build();
    let owner_stream = MessageStream::for_match_rule(owner_rule, &connection, Some(8))
        .await
        .map_err(unavailable)?;

    let mut options = Values::new();
    options.insert("types".into(), OwnedValue::from(3_u32));
    if remote_version >= 2 {
        options.insert("persist_mode".into(), OwnedValue::from(1_u32));
        if let Some(token) = restore_token {
            options.insert("restore_token".into(), string_value(token)?);
        }
    }
    request(
        &mut guard,
        REMOTE,
        "SelectDevices",
        RequestBody::Session(handle.clone(), options),
    )
    .await?;
    let mut options = Values::new();
    options.insert("types".into(), OwnedValue::from(1_u32));
    options.insert("multiple".into(), OwnedValue::from(true));
    // Persistence belongs to RemoteDesktop exclusively for combined sessions.
    request(
        &mut guard,
        SCREENCAST,
        "SelectSources",
        RequestBody::Session(handle.clone(), options),
    )
    .await?;
    let mut started = request(
        &mut guard,
        REMOTE,
        "Start",
        RequestBody::Start(handle.clone(), Values::new()),
    )
    .await?;
    let granted = started
        .remove("devices")
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(0);
    if granted & 3 != 3 {
        return Err(WaylandError::new(
            "WAYLAND_DEVICE_PERMISSION_DENIED",
            "The native sharing dialog did not grant both keyboard and pointer",
        ));
    }
    let streams = decode_streams(
        started
            .remove("streams")
            .ok_or_else(|| unavailable("The portal returned no monitor streams"))?,
    )?;
    let next_token = started
        .remove("restore_token")
        .and_then(|value| String::try_from(value).ok())
        .filter(|token| !token.is_empty());
    let session = Arc::new(PortalSession {
        connection,
        handle,
        generation: NEXT_ID.fetch_add(1, Ordering::Relaxed),
        streams,
        devices: granted,
        closed: AtomicBool::new(false),
        close_confirmed: AtomicBool::new(false),
        close_gate: Mutex::new(()),
        monitor_stop: Arc::new(tokio::sync::Notify::new()),
        input_gate: Mutex::new(()),
    });
    let weak = Arc::downgrade(&session);
    let monitor_stop = session.monitor_stop.clone();
    tokio::spawn(async move {
        let mut closed_stream = closed_stream;
        let mut owner_stream = owner_stream;
        let closed_signal = tokio::select! {
            message = closed_stream.next() => message.is_some_and(|message| message.is_ok()),
            _ = owner_stream.next() => false,
            _ = monitor_stop.notified() => false,
        };
        if let Some(session) = weak.upgrade() {
            session.closed.store(true, Ordering::Release);
            if closed_signal {
                session.close_confirmed.store(true, Ordering::Release);
            }
        }
    });
    guard.armed = false;
    Ok((session, next_token))
}

enum RequestBody {
    Create(Values),
    Session(OwnedObjectPath, Values),
    Start(OwnedObjectPath, Values),
}

async fn request(
    guard: &mut CreationGuard,
    interface: &str,
    member: &str,
    mut body: RequestBody,
) -> Result<Values, WaylandError> {
    let token = format!(
        "nomifun_request_{}",
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    );
    let options = match &mut body {
        RequestBody::Create(options)
        | RequestBody::Session(_, options)
        | RequestBody::Start(_, options) => options,
    };
    options.insert("handle_token".into(), string_value(token.clone())?);
    let unique = guard
        .connection
        .unique_name()
        .ok_or_else(|| unavailable("The desktop bus assigned no unique name"))?
        .as_str()
        .trim_start_matches(':')
        .replace('.', "_");
    guard.request = Some(
        OwnedObjectPath::try_from(format!("{DESKTOP_PATH}/request/{unique}/{token}"))
            .map_err(unavailable)?,
    );
    // Subscribe before issuing the method. The portal is permitted to emit
    // Response before the method reply. Filter by the actual returned path.
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender(DESTINATION)
        .map_err(unavailable)?
        .interface(REQUEST)
        .map_err(unavailable)?
        .member("Response")
        .map_err(unavailable)?
        .build();
    let mut responses = MessageStream::for_match_rule(rule, &guard.connection, Some(16))
        .await
        .map_err(unavailable)?;
    let message = match body {
        RequestBody::Create(options) => {
            guard
                .connection
                .call_method(
                    Some(DESTINATION),
                    DESKTOP_PATH,
                    Some(interface),
                    member,
                    &(options,),
                )
                .await
        }
        RequestBody::Session(session, options) => {
            guard
                .connection
                .call_method(
                    Some(DESTINATION),
                    DESKTOP_PATH,
                    Some(interface),
                    member,
                    &(session, options),
                )
                .await
        }
        RequestBody::Start(session, options) => {
            guard
                .connection
                .call_method(
                    Some(DESTINATION),
                    DESKTOP_PATH,
                    Some(interface),
                    member,
                    &(session, "", options),
                )
                .await
        }
    }
    .map_err(unavailable)?;
    let handle: OwnedObjectPath = message.body().deserialize().map_err(unavailable)?;
    guard.request = Some(handle.clone());
    let response = tokio::time::timeout(UI_TIMEOUT, async {
        while let Some(message) = responses.next().await {
            let message = message.map_err(unavailable)?;
            if message
                .header()
                .path()
                .is_some_and(|path| path.as_str() == handle.as_str())
            {
                let (response, results): (u32, Values) =
                    message.body().deserialize().map_err(unavailable)?;
                // Every Response completes the request object, including
                // cancellation/denial. Only an unresolved request needs Close.
                guard.request = None;
                return match response {
                    0 => Ok(results),
                    1 => Err(WaylandError::new(
                        "WAYLAND_PERMISSION_CANCELLED",
                        "The native desktop sharing request was cancelled",
                    )),
                    2 => Err(WaylandError::new(
                        "WAYLAND_PERMISSION_FAILED",
                        "The portal reported that the native sharing request failed; response 2 does not distinguish denial from a backend error",
                    )),
                    _ => Err(unavailable(
                        "The desktop portal returned an unknown response code",
                    )),
                };
            }
        }
        Err(unavailable(
            "The desktop portal disconnected while awaiting native permission",
        ))
    })
    .await
    .map_err(|_| {
        WaylandError::new(
            "WAYLAND_PERMISSION_TIMEOUT",
            "The native desktop sharing request timed out and was closed",
        )
    })?;
    if response.is_ok() {
        guard.request = None;
    }
    response
}

fn string_value(value: String) -> Result<OwnedValue, WaylandError> {
    OwnedValue::try_from(zbus::zvariant::Value::from(value)).map_err(unavailable)
}

fn decode_streams(value: OwnedValue) -> Result<Vec<PortalStream>, WaylandError> {
    let native: Vec<(u32, Values)> = value.try_into().map_err(unavailable)?;
    let mut streams = Vec::new();
    for (node_id, mut metadata) in native {
        if node_id == 0
            || streams
                .iter()
                .any(|stream: &PortalStream| stream.node_id == node_id)
        {
            return Err(unavailable(
                "The portal returned an invalid or duplicate PipeWire node",
            ));
        }
        let position = metadata
            .remove("position")
            .and_then(|value| <(i32, i32)>::try_from(value).ok());
        let pipewire_serial = metadata
            .remove("pipewire-serial")
            .and_then(|value| u64::try_from(value).ok());
        let size = metadata
            .remove("logical_size")
            .or_else(|| metadata.remove("size"))
            .and_then(|value| <(i32, i32)>::try_from(value).ok())
            .and_then(|(width, height)| {
                Some((
                    u32::try_from(width).ok().filter(|value| *value > 0)?,
                    u32::try_from(height).ok().filter(|value| *value > 0)?,
                ))
            });
        streams.push(PortalStream {
            node_id,
            pipewire_serial,
            position,
            size,
        });
    }
    if streams.is_empty() {
        return Err(WaylandError::new(
            "WAYLAND_SCREENCAST_UNAVAILABLE",
            "No monitor stream was granted",
        ));
    }
    Ok(streams)
}

#[cfg(test)]
#[path = "linux_wayland_tests.rs"]
mod protocol_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_stream_coordinates_are_separate_from_pixel_frames() {
        let mut properties = Values::new();
        properties.insert(
            "position".into(),
            OwnedValue::try_from(zbus::zvariant::Value::from((-100_i32, 20_i32))).unwrap(),
        );
        properties.insert(
            "size".into(),
            OwnedValue::try_from(zbus::zvariant::Value::from((200_i32, 150_i32))).unwrap(),
        );
        let value =
            OwnedValue::try_from(zbus::zvariant::Value::from(vec![(42_u32, properties)])).unwrap();
        let streams = decode_streams(value).unwrap();
        assert_eq!(
            streams[0],
            PortalStream {
                node_id: 42,
                pipewire_serial: None,
                position: Some((-100, 20)),
                size: Some((200, 150))
            }
        );
    }

    #[test]
    fn an_empty_grant_is_not_a_desktop() {
        let value =
            OwnedValue::try_from(zbus::zvariant::Value::from(Vec::<(u32, Values)>::new())).unwrap();
        assert_eq!(
            decode_streams(value).unwrap_err().code(),
            "WAYLAND_SCREENCAST_UNAVAILABLE"
        );
    }

    #[tokio::test]
    async fn blocking_bridge_does_not_nest_the_callers_runtime() {
        let result = run_blocking(async { Ok(41_u32 + 1) });
        assert_eq!(result.unwrap(), 42);
    }
}
