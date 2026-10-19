//! A high-integrity Windows desktop transport under the canonical Computer
//! owner. Ordinary operations remain in the login GUI. A transport change
//! retires the old observation before any input; no uncertain input is retried.
use crate::router::agent_role_host::ComputerToolPort;
use nomi_computer::ComputerTool;
use nomi_system_execution::{
    SystemExecutionClient, SystemExecutionError, WindowsExecutionExtension,
};
use nomi_types::tool::{ToolImage, ToolResult};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    mem, ptr,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, HWND, POINT},
    Security::{
        GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TOKEN_ELEVATION,
        TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TokenElevation, TokenIntegrityLevel,
    },
    System::{
        StationsAndDesktops::{
            CloseDesktop, DESKTOP_READOBJECTS, GetThreadDesktop, GetUserObjectInformationW,
            OpenInputDesktop, UOI_NAME,
        },
        Threading::{
            GetCurrentProcess, GetCurrentThreadId, OpenProcess, OpenProcessToken,
            PROCESS_QUERY_LIMITED_INFORMATION,
        },
    },
    UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId, WindowFromPoint},
};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Request {
    version: u32,
    action_id: String,
    input: Value,
    clear_before: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    content: String,
    is_error: bool,
    images: Vec<ToolImage>,
    input_desktop: String,
    native_pid: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    control_delivery_error: Option<String>,
}

fn rejected(message: impl Into<String>) -> SystemExecutionError {
    SystemExecutionError {
        code: "COMPUTER_NATIVE_NOT_ADMITTED".into(),
        message: message.into(),
        details: json!({"user_code_not_started":true,"cleanup_proven":true}),
    }
}
fn unknown(message: impl Into<String>) -> SystemExecutionError {
    SystemExecutionError {
        code: "COMPUTER_NATIVE_RESULT_UNKNOWN".into(),
        message: message.into(),
        details: json!({"user_code_not_started":false,"cleanup_proven":false}),
    }
}
fn io_error(context: &str) -> String {
    format!("{context}: {}", std::io::Error::last_os_error())
}

struct OwnedHandle(HANDLE);
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
fn process_token(process: HANDLE) -> Result<OwnedHandle, String> {
    let mut token = ptr::null_mut();
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
        return Err(io_error("Cannot inspect target token"));
    }
    Ok(OwnedHandle(token))
}
fn elevated() -> Result<bool, String> {
    let token = process_token(unsafe { GetCurrentProcess() })?;
    let mut elevation: TOKEN_ELEVATION = unsafe { mem::zeroed() };
    let mut length = 0;
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenElevation,
            (&mut elevation as *mut TOKEN_ELEVATION).cast(),
            mem::size_of_val(&elevation) as u32,
            &mut length,
        )
    } == 0
    {
        return Err(io_error("Cannot inspect native runner elevation"));
    }
    Ok(elevation.TokenIsElevated != 0)
}
fn desktop_name(desktop: HANDLE) -> Result<String, String> {
    let mut length = 0;
    unsafe {
        GetUserObjectInformationW(desktop, UOI_NAME, ptr::null_mut(), 0, &mut length);
    }
    if length == 0 {
        return Err(io_error("Cannot inspect input desktop"));
    }
    let mut name = vec![0u16; (length as usize).div_ceil(2)];
    if unsafe {
        GetUserObjectInformationW(
            desktop,
            UOI_NAME,
            name.as_mut_ptr().cast(),
            length,
            &mut length,
        )
    } == 0
    {
        return Err(io_error("Cannot inspect input desktop"));
    }
    let end = name
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(name.len());
    String::from_utf16(&name[..end]).map_err(|_| "Input desktop name is not valid Unicode".into())
}
/// Never switch desktops or attempt input on UAC/Winlogon/locked desktops.
pub fn verify_login_input_desktop() -> Result<(), String> {
    let input = unsafe { OpenInputDesktop(0, 0, DESKTOP_READOBJECTS) };
    if input.is_null() {
        return Err(io_error(
            "The login input desktop is unavailable; unlock Windows or complete the native secure-desktop prompt",
        ));
    }
    let input_name = desktop_name(input);
    unsafe {
        CloseDesktop(input);
    }
    let input_name = input_name?;
    let thread_desktop = unsafe { GetThreadDesktop(GetCurrentThreadId()) };
    if thread_desktop.is_null() {
        return Err(io_error("The calling thread has no login desktop"));
    }
    if !input_name.eq_ignore_ascii_case("Default")
        || !desktop_name(thread_desktop)?.eq_ignore_ascii_case(&input_name)
    {
        return Err("The active desktop is not the caller's Default login desktop; complete the native prompt or unlock Windows before observing again".into());
    }
    Ok(())
}
fn high_integrity_window(window: HWND) -> Result<bool, String> {
    if window.is_null() {
        return Ok(false);
    }
    let mut pid = 0;
    if unsafe { GetWindowThreadProcessId(window, &mut pid) } == 0 || pid == 0 {
        return Err(io_error("Cannot identify target window owner"));
    }
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return Err(io_error("Cannot inspect target process integrity"));
    }
    let process = OwnedHandle(process);
    let token = process_token(process.0)?;
    let mut length = 0;
    unsafe {
        GetTokenInformation(
            token.0,
            TokenIntegrityLevel,
            ptr::null_mut(),
            0,
            &mut length,
        );
    }
    if length == 0 {
        return Err(io_error("Cannot inspect target integrity label"));
    }
    let mut storage = vec![0usize; (length as usize).div_ceil(mem::size_of::<usize>())];
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenIntegrityLevel,
            storage.as_mut_ptr().cast(),
            length,
            &mut length,
        )
    } == 0
    {
        return Err(io_error("Cannot inspect target integrity label"));
    }
    let label = unsafe { &*storage.as_ptr().cast::<TOKEN_MANDATORY_LABEL>() };
    let count = unsafe { *GetSidSubAuthorityCount(label.Label.Sid) };
    if count == 0 {
        return Err("Target integrity label has no authority".into());
    }
    Ok(unsafe { *GetSidSubAuthority(label.Label.Sid, u32::from(count - 1)) } >= 0x3000)
}
fn needs_elevation(tool: &ComputerTool, input: &Value) -> Result<bool, String> {
    if input["action"] == "focus_window" {
        if let Some(window) = input["window_id"]
            .as_u64()
            .and_then(|id| usize::try_from(id).ok())
        {
            return high_integrity_window(window as HWND);
        }
    }
    for (x, y) in tool.windows_input_target_points(input) {
        if high_integrity_window(unsafe { WindowFromPoint(POINT { x, y }) })? {
            return Ok(true);
        }
    }
    high_integrity_window(unsafe { GetForegroundWindow() })
}
fn stale(message: &str) -> ToolResult {
    ToolResult::error(format!(
        "COMPUTER_NATIVE_OBSERVATION_REQUIRED_NO_EFFECT: {message}. No native input was dispatched; acquire a fresh observation before input."
    ))
}
pub(crate) fn is_proven_observation_rejection(result: &ToolResult) -> bool {
    result.is_error && result.images.is_empty()
        && result.content.strip_prefix("COMPUTER_NATIVE_OBSERVATION_REQUIRED_NO_EFFECT: ")
            .is_some_and(|reason| !reason.is_empty() && reason.ends_with(". No native input was dispatched; acquire a fresh observation before input."))
}
fn no_native_effect(error: &SystemExecutionError) -> bool {
    error.details["user_code_not_started"] == true && error.details["cleanup_proven"] == true
}

trait DesktopRoutingProbe: Send + Sync {
    fn verify(&self) -> Result<(), String>;
    fn requires_high(&self, tool: &ComputerTool, input: &Value) -> Result<bool, String>;
}
struct NativeRoutingProbe;
impl DesktopRoutingProbe for NativeRoutingProbe {
    fn verify(&self) -> Result<(), String> {
        verify_login_input_desktop()
    }
    fn requires_high(&self, tool: &ComputerTool, input: &Value) -> Result<bool, String> {
        needs_elevation(tool, input)
    }
}
#[async_trait::async_trait]
trait ComputerBridgeTransport: Send + Sync {
    async fn allow_foreground_from_current_app(&self);
    async fn call(&self, request: Value) -> Result<Value, SystemExecutionError>;
    async fn call_controlled(&self, request: Value, _cancellation: tokio_util::sync::CancellationToken) -> Result<Value, SystemExecutionError> {
        self.call(request).await
    }
}
#[async_trait::async_trait]
impl ComputerBridgeTransport for SystemExecutionClient {
    async fn allow_foreground_from_current_app(&self) {
        let _ = self.allow_windows_foreground_from_current_app().await;
    }
    async fn call(&self, request: Value) -> Result<Value, SystemExecutionError> {
        SystemExecutionClient::call(self, request).await
    }
    async fn call_controlled(&self, request: Value, cancellation: tokio_util::sync::CancellationToken) -> Result<Value, SystemExecutionError> {
        self.call_windows_controlled(request, &cancellation).await
    }
}

#[derive(Default)]
struct Route {
    client: Option<Arc<dyn ComputerBridgeTransport>>,
    remote_observation: bool,
    force_remote_observe: bool,
    epoch: u64,
}
pub(crate) struct WindowsComputerPort {
    local: Arc<ComputerTool>,
    route: tokio::sync::Mutex<Route>,
    epoch: AtomicU64,
    probe: Arc<dyn DesktopRoutingProbe>,
}
impl WindowsComputerPort {
    pub(crate) fn new(local: Arc<ComputerTool>) -> Self {
        Self {
            local,
            route: Default::default(),
            epoch: AtomicU64::new(1),
            probe: Arc::new(NativeRoutingProbe),
        }
    }
}
#[async_trait::async_trait]
impl ComputerToolPort for WindowsComputerPort {
    fn clear_cached_observation(&self) {
        self.local.clear_cached_observation();
        self.epoch.fetch_add(1, Ordering::AcqRel);
    }
    fn permission_status(&self) -> nomi_computer::permissions::PermissionStatus {
        nomi_computer::permissions::permission_status()
    }
    async fn execute_authorized(&self, action_id: &str, input: Value) -> ToolResult {
        self.execute_authorized_controlled(action_id, input, Default::default()).await
    }

    async fn execute_authorized_controlled(&self, action_id: &str, input: Value,
        control: nomifun_agent_kernel::InvocationControl) -> ToolResult {
        if control.is_cancelled() { return nomi_computer::tool::cancelled_before_dispatch_result(); }
        if action_id == nomi_computer::capability::COMPUTER_LAUNCH_ACTION_ID {
            return self.local.execute_authorized_controlled(action_id, input, &control.cancellation()).await;
        }
        if action_id == nomi_computer::capability::COMPUTER_OBSERVE_ACTION_ID
            && input["action"] == "wait"
        {
            let mut route = tokio::select! {
                biased;
                _ = control.cancelled() => return nomi_computer::tool::cancelled_before_dispatch_result(),
                route = self.route.lock() => route,
            };
            route.remote_observation = false;
            drop(route);
            return self.local.execute_authorized_controlled(action_id, input, &control.cancellation()).await;
        }
        let is_input = action_id == nomi_computer::capability::COMPUTER_INPUT_ACTION_ID;
        if let Err(error) = self.probe.verify() {
            return if is_input {
                stale(&error)
            } else {
                ToolResult::error(error)
            };
        }
        let mut route = tokio::select! {
            biased;
            _ = control.cancelled() => return nomi_computer::tool::cancelled_before_dispatch_result(),
            route = self.route.lock() => route,
        };
        if control.is_cancelled() { return nomi_computer::tool::cancelled_before_dispatch_result(); }
        let inspect_target = (is_input && !route.remote_observation)
            || (action_id == nomi_computer::capability::COMPUTER_A11Y_OBSERVE_ACTION_ID
                && input["action"] == "observe");
        let high = if !inspect_target {
            // The bridge owns that observation's refs and screenshot scale.
            // Never resolve remote image coordinates through the local cache.
            // Ordinary screen/window metadata needs no process token query.
            false
        } else {
            match self.probe.requires_high(&self.local, &input) {
                Ok(high) => high,
                Err(error) => {
                    return if is_input {
                        stale(&error)
                    } else {
                        ToolResult::error(error)
                    };
                }
            }
        };
        if is_input && !route.remote_observation && high {
            route.force_remote_observe = true;
            self.local.clear_cached_observation();
            return stale(
                "The target requires the authenticated high-integrity desktop bridge; acquire a fresh observation through that bridge before input",
            );
        }
        if is_input && route.remote_observation && route.epoch != self.epoch.load(Ordering::Acquire)
        {
            return stale(
                "The native bridge observation was invalidated; observe again before input",
            );
        }
        let observation = matches!(
            input["action"].as_str(),
            Some("observe" | "screenshot" | "list_windows")
        );
        let high_tree =
            high && action_id == nomi_computer::capability::COMPUTER_A11Y_OBSERVE_ACTION_ID;
        // Screenshots and window metadata work at ordinary integrity. They
        // move only after an input has explicitly required a fresh bridge
        // observation; UIA trees of high-integrity targets need it directly.
        let remote = if is_input {
            route.remote_observation
        } else {
            observation && (high_tree || route.force_remote_observe)
        };
        if !remote {
            route.remote_observation = false;
            return self.local.execute_authorized_controlled(action_id, input, &control.cancellation()).await;
        }
        if route.client.is_none() {
            match tokio::task::spawn_blocking(SystemExecutionClient::connect).await {
                Ok(Ok(client)) => route.client = Some(Arc::new(client)),
                Ok(Err(error)) => {
                    return if is_input && no_native_effect(&error) {
                        stale(&error.to_string())
                    } else {
                        ToolResult::error(error.to_string())
                    };
                }
                Err(error) => {
                    return ToolResult::error(format!(
                        "COMPUTER_BRIDGE_RESULT_UNKNOWN: native connection worker ended: {error}"
                    ));
                }
            }
        }
        if control.is_cancelled() { return nomi_computer::tool::cancelled_before_dispatch_result(); }
        if is_input && input["action"] == "focus_window" {
            route.client.as_ref().expect("bridge initialized")
                .allow_foreground_from_current_app().await;
        }
        // Connection/authentication remains with its original worker. If Stop
        // arrived while it was pending, no desktop request may follow it.
        if control.is_cancelled() { return nomi_computer::tool::cancelled_before_dispatch_result(); }
        let epoch = self.epoch.load(Ordering::Acquire);
        let request = Request {
            version: 1,
            action_id: action_id.into(),
            input,
            clear_before: route.epoch != epoch || !is_input,
        };
        let response = route
            .client
            .as_ref()
            .expect("bridge initialized")
            .call_controlled(json!({"op":"extension","request":request}), control.cancellation())
            .await;
        let reply = match response {
            Ok(reply) => match serde_json::from_value::<Reply>(reply) {
                Ok(reply) if reply.input_desktop == "Default" && reply.native_pid != 0 => reply,
                _ => {
                    return ToolResult::error(
                        "COMPUTER_BRIDGE_RESULT_UNKNOWN: native Computer receipt was invalid; do not replay input",
                    );
                }
            },
            Err(error) => {
                if error.code == "SYSTEM_EXECUTION_CANCELLED_BEFORE_DISPATCH" && no_native_effect(&error) {
                    return nomi_computer::tool::cancelled_before_dispatch_result();
                }
                return if is_input && no_native_effect(&error) {
                    stale(&error.to_string())
                } else {
                    ToolResult::error(format!(
                        "COMPUTER_BRIDGE_RESULT_UNKNOWN: {error}; do not replay input"
                    ))
                };
            }
        };
        route.epoch = epoch;
        if !is_input && !reply.is_error {
            route.remote_observation = true;
            route.force_remote_observe = false;
        }
        let mut content = reply.content;
        if let Some(error) = reply.control_delivery_error {
            tracing::warn!(%error, "Computer cancellation delivery failed; original native result retained");
            if !reply.is_error { content.push_str(&format!("\nUser Stop signal delivery failed: {error}. The original native operation completed with the result above.")); }
        }
        ToolResult {
            content,
            is_error: reply.is_error,
            images: reply.images,
        }
    }
}

struct NativeComputerHandler {
    tool: ComputerTool,
    runtime: OnceLock<Result<tokio::runtime::Runtime, String>>,
    operation: Mutex<()>,
}
impl WindowsExecutionExtension for NativeComputerHandler {
    fn call(&self, request: Value) -> Result<Value, SystemExecutionError> {
        self.call_controlled(request, Default::default())
    }

    fn call_controlled(&self, request: Value, cancellation: nomi_system_execution::WindowsCancellation) -> Result<Value, SystemExecutionError> {
        cancellation.require_open()?;
        let request: Request = serde_json::from_value(request)
            .map_err(|_| rejected("Invalid typed native Computer request"))?;
        if request.version != 1 {
            return Err(rejected("Unsupported native Computer transport version"));
        }
        let _operation = loop {
            cancellation.require_open()?;
            match self.operation.try_lock() {
                Ok(operation) => break operation,
                Err(std::sync::TryLockError::Poisoned(_)) => return Err(unknown("Native Computer operation ownership was lost")),
                Err(std::sync::TryLockError::WouldBlock) => std::thread::sleep(std::time::Duration::from_millis(20)),
            }
        };
        cancellation.require_open()?;
        if !elevated().map_err(rejected)? {
            return Err(rejected(
                "Native desktop bridge requires the authenticated elevated runner",
            ));
        }
        verify_login_input_desktop().map_err(rejected)?;
        let runtime = self
            .runtime
            .get_or_init(|| {
                tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(|error| rejected(error.clone()))?;
        if request.clear_before {
            self.tool.clear_cached_observation();
        }
        let result = runtime.block_on(async {
            let token = tokio_util::sync::CancellationToken::new();
            if cancellation.is_cancelled() { token.cancel(); }
            let operation = self.tool.execute_authorized_controlled(&request.action_id, request.input, &token);
            tokio::pin!(operation);
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => { token.cancel(); operation.await }
                result = &mut operation => result,
            }
        });
        Ok(json!(Reply {
            content: result.content,
            is_error: result.is_error,
            images: result.images,
            input_desktop: "Default".into(),
            native_pid: std::process::id(),
            control_delivery_error: None,
        }))
    }
    fn shutdown(&self) -> Result<(), SystemExecutionError> {
        let _operation = self
            .operation
            .lock()
            .map_err(|_| unknown("Native Computer operation ownership was lost"))?;
        self.tool.clear_cached_observation();
        Ok(())
    }
}
fn factory() -> Arc<dyn WindowsExecutionExtension> {
    Arc::new(NativeComputerHandler {
        tool: ComputerTool::new(&Default::default()),
        runtime: OnceLock::new(),
        operation: Mutex::new(()),
    })
}
pub fn install_execution_handler() -> Result<(), SystemExecutionError> {
    nomi_system_execution::install_windows_extension_handler(factory)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bridge_transport_switch_is_an_existing_canonical_no_effect_rejection() {
        assert!(is_proven_observation_rejection(&stale(
            "transport changed; observe again"
        )));
        assert!(!is_proven_observation_rejection(&ToolResult::error(
            "COMPUTER_BRIDGE_RESULT_UNKNOWN"
        )));
    }
    #[test]
    fn native_bridge_rejects_unknown_wire_fields_before_native_execution() {
        assert!(serde_json::from_value::<Request>(json!({"version":1,"action_id":"computer/input","input":{"action":"key","key":"enter"},"clear_before":true,"execution_identity":"administrator"})).is_err());
        assert!(
            serde_json::from_value::<Request>(
                json!({"version":1,"action_id":"computer/input","input":{},"clear_before":true})
            )
            .is_ok()
        );
    }
    #[test]
    fn native_desktop_probe_reports_real_state_without_input_or_desktop_switch() {
        let result = verify_login_input_desktop();
        eprintln!(
            "WINDOWS_INPUT_DESKTOP_EVIDENCE pid={} elevated={:?} default_desktop={result:?}",
            std::process::id(),
            elevated()
        );
        if let Err(error) = result {
            assert!(!error.is_empty());
        }
    }
    struct Probe {
        high: std::sync::atomic::AtomicBool,
        secure: std::sync::atomic::AtomicBool,
        target_queries: AtomicU64,
    }
    impl DesktopRoutingProbe for Probe {
        fn verify(&self) -> Result<(), String> {
            if self.secure.load(Ordering::Acquire) {
                Err("Winlogon secure desktop".into())
            } else {
                Ok(())
            }
        }
        fn requires_high(&self, _: &ComputerTool, _: &Value) -> Result<bool, String> {
            self.target_queries.fetch_add(1, Ordering::AcqRel);
            Ok(self.high.load(Ordering::Acquire))
        }
    }
    #[derive(Default)]
    struct Bridge {
        calls: Mutex<Vec<Value>>,
        lose_reply: std::sync::atomic::AtomicBool,
        foreground_delegations: std::sync::atomic::AtomicUsize,
        block: std::sync::atomic::AtomicBool,
        entered: tokio::sync::Notify,
        release: tokio::sync::Notify,
    }
    #[async_trait::async_trait]
    impl ComputerBridgeTransport for Bridge {
        async fn allow_foreground_from_current_app(&self) {
            self.foreground_delegations.fetch_add(1, Ordering::AcqRel);
        }
        async fn call(&self, request: Value) -> Result<Value, SystemExecutionError> {
            self.calls.lock().unwrap().push(request);
            if self.block.load(Ordering::Acquire) {
                self.entered.notify_one();
                self.release.notified().await;
            }
            if self.lose_reply.load(Ordering::Acquire) {
                return Err(SystemExecutionError {
                    code: "SYSTEM_EXECUTION_RESULT_UNKNOWN".into(),
                    message: "reply lost after admission".into(),
                    details: json!({"user_code_not_started":false,"cleanup_proven":false}),
                });
            }
            Ok(json!(Reply {
                content: "fixture native result".into(),
                is_error: false,
                images: vec![],
                input_desktop: "Default".into(),
                native_pid: 123,
                control_delivery_error: None,
            }))
        }
    }
    async fn fixture() -> (WindowsComputerPort, Arc<Probe>, Arc<Bridge>) {
        let probe = Arc::new(Probe {
            high: Default::default(),
            secure: Default::default(),
            target_queries: Default::default(),
        });
        let bridge = Arc::new(Bridge::default());
        let mut port = WindowsComputerPort::new(Arc::new(ComputerTool::new(&Default::default())));
        port.probe = probe.clone();
        port.route.get_mut().client = Some(bridge.clone());
        (port, probe, bridge)
    }
    #[tokio::test]
    async fn controlled_windows_route_queue_cancels_without_native_bridge_dispatch() {
        let (port, _, bridge) = fixture().await;
        let _original_route = port.route.lock().await;
        let stop = tokio_util::sync::CancellationToken::new();
        let waiting = port.execute_authorized_controlled("computer/a11y.observe", json!({"action":"observe"}),
            nomifun_agent_kernel::InvocationControl::new(stop.clone()));
        tokio::pin!(waiting);
        tokio::select! {
            biased;
            _ = &mut waiting => panic!("the native route is still held by its original owner"),
            _ = tokio::task::yield_now() => {},
        }
        stop.cancel();
        let result = tokio::time::timeout(std::time::Duration::from_millis(100), waiting).await.unwrap();
        assert!(nomi_computer::tool::is_proven_cancelled_before_dispatch(&result));
        assert!(bridge.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn controlled_windows_bridge_keeps_a_dispatched_call_until_its_real_reply() {
        let (port, probe, bridge) = fixture().await;
        probe.high.store(true, Ordering::Release);
        assert!(!port.execute_authorized("computer/a11y.observe", json!({"action":"observe"})).await.is_error);
        bridge.block.store(true, Ordering::Release);
        let stop = tokio_util::sync::CancellationToken::new();
        let port = Arc::new(port);
        let task = {
            let port = port.clone(); let stop = stop.clone();
            tokio::spawn(async move { port.execute_authorized_controlled("computer/input", json!({"action":"key","key":"enter"}),
                nomifun_agent_kernel::InvocationControl::new(stop)).await })
        };
        bridge.entered.notified().await;
        stop.cancel();
        assert!(!task.is_finished(), "a dispatched bridge call cannot be dropped as a cancellation receipt");
        bridge.release.notify_one();
        let result = task.await.unwrap();
        assert!(!result.is_error);
        assert_eq!(bridge.calls.lock().unwrap().len(), 2, "the original input is dispatched exactly once");
    }

    #[tokio::test]
    async fn controlled_windows_bridge_receives_original_token_and_only_a_proven_queued_cancel_is_no_effect() {
        struct QueuedBridge { queued:std::sync::atomic::AtomicBool, entered:tokio::sync::Notify, native:AtomicU64 }
        #[async_trait::async_trait]
        impl ComputerBridgeTransport for QueuedBridge {
            async fn allow_foreground_from_current_app(&self) {}
            async fn call(&self, _:Value)->Result<Value,SystemExecutionError> {
                Ok(json!(Reply {content:"original native result".into(),is_error:false,images:vec![],
                    input_desktop:"Default".into(),native_pid:123,control_delivery_error:None}))
            }
            async fn call_controlled(&self, request:Value, cancellation:tokio_util::sync::CancellationToken)->Result<Value,SystemExecutionError> {
                if request["request"]["action_id"] == "computer/input" {
                    if self.queued.load(Ordering::Acquire) {
                        self.entered.notify_one(); cancellation.cancelled().await;
                        return Err(SystemExecutionError {code:"SYSTEM_EXECUTION_CANCELLED_BEFORE_DISPATCH".into(),
                            message:"original queued command withdrawn".into(),details:json!({"user_code_not_started":true,"cleanup_proven":true})});
                    }
                    assert!(!cancellation.is_cancelled(), "old Stop cannot target a new request token");
                    self.native.fetch_add(1, Ordering::SeqCst);
                }
                self.call(request).await
            }
        }
        let (mut port, probe, _) = fixture().await;
        let bridge = Arc::new(QueuedBridge {queued:true.into(),entered:Default::default(),native:AtomicU64::new(0)});
        port.route.get_mut().client = Some(bridge.clone()); probe.high.store(true,Ordering::Release);
        assert!(!port.execute_authorized("computer/a11y.observe",json!({"action":"observe"})).await.is_error);
        let port = Arc::new(port); let stop = tokio_util::sync::CancellationToken::new();
        let original = tokio::spawn({let port=port.clone();let stop=stop.clone();async move {
            port.execute_authorized_controlled("computer/input",json!({"action":"key","key":"enter"}),
                nomifun_agent_kernel::InvocationControl::new(stop)).await
        }});
        bridge.entered.notified().await; stop.cancel();
        assert!(nomi_computer::tool::is_proven_cancelled_before_dispatch(&original.await.unwrap()));
        assert_eq!(bridge.native.load(Ordering::SeqCst),0);
        bridge.queued.store(false,Ordering::Release); stop.cancel();
        assert!(!port.execute_authorized("computer/a11y.observe",json!({"action":"observe"})).await.is_error);
        assert!(!port.execute_authorized("computer/input",json!({"action":"key","key":"enter"})).await.is_error);
        assert_eq!(bridge.native.load(Ordering::SeqCst),1);
    }

    #[tokio::test]
    async fn high_integrity_transition_requires_fresh_observation_before_exactly_one_input() {
        let (port, probe, bridge) = fixture().await;
        assert!(
            !port
                .execute_authorized("computer/observe", json!({"action":"wait","seconds":0}))
                .await
                .is_error
        );
        assert!(
            bridge.calls.lock().unwrap().is_empty(),
            "ordinary operation stays in login owner"
        );
        probe.high.store(true, Ordering::Release);
        let result = port
            .execute_authorized("computer/input", json!({"action":"key","key":"enter"}))
            .await;
        assert!(is_proven_observation_rejection(&result));
        assert!(
            bridge.calls.lock().unwrap().is_empty(),
            "no input dispatch on transport change"
        );
        port.clear_cached_observation();
        assert!(
            !port
                .execute_authorized("computer/a11y.observe", json!({"action":"observe"}))
                .await
                .is_error
        );
        assert!(
            !port
                .execute_authorized("computer/input", json!({"action":"key","key":"enter"}))
                .await
                .is_error
        );
        let calls = bridge.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0]["request"]["action_id"], "computer/a11y.observe");
        assert_eq!(calls[0]["request"]["clear_before"], true);
        assert_eq!(calls[1]["request"]["clear_before"], false);
    }
    #[tokio::test]
    async fn unknown_bridge_input_is_never_retried_or_settled_as_no_effect() {
        let (port, probe, bridge) = fixture().await;
        probe.high.store(true, Ordering::Release);
        assert!(
            !port
                .execute_authorized("computer/a11y.observe", json!({"action":"observe"}))
                .await
                .is_error
        );
        bridge.lose_reply.store(true, Ordering::Release);
        let result = port
            .execute_authorized("computer/input", json!({"action":"key","key":"enter"}))
            .await;
        assert!(result.is_error);
        assert!(!is_proven_observation_rejection(&result));
        assert!(result.content.contains("RESULT_UNKNOWN"));
        assert_eq!(
            bridge.calls.lock().unwrap().len(),
            2,
            "exactly one accepted input call, without retry"
        );
    }
    #[tokio::test]
    async fn secure_desktop_is_rejected_before_bridge_input() {
        let (port, probe, bridge) = fixture().await;
        probe.high.store(true, Ordering::Release);
        probe.secure.store(true, Ordering::Release);
        let result = port
            .execute_authorized("computer/input", json!({"action":"key","key":"enter"}))
            .await;
        assert!(is_proven_observation_rejection(&result));
        assert!(bridge.calls.lock().unwrap().is_empty());
    }
    #[tokio::test]
    async fn invalidated_remote_observation_cannot_dispatch_input() {
        let (port, probe, bridge) = fixture().await;
        probe.high.store(true, Ordering::Release);
        assert!(
            !port
                .execute_authorized("computer/a11y.observe", json!({"action":"observe"}))
                .await
                .is_error
        );
        port.clear_cached_observation();
        let result = port
            .execute_authorized("computer/input", json!({"action":"key","key":"enter"}))
            .await;
        assert!(is_proven_observation_rejection(&result));
        assert_eq!(bridge.calls.lock().unwrap().len(), 1);
    }
    #[tokio::test]
    async fn foreground_delegation_is_only_for_fresh_remote_focus_without_retry() {
        let (port, probe, bridge) = fixture().await;
        probe.high.store(true, Ordering::Release);
        assert!(!port.execute_authorized("computer/a11y.observe", json!({"action":"observe"})).await.is_error);
        assert_eq!(bridge.foreground_delegations.load(Ordering::Acquire),0);
        assert!(!port.execute_authorized("computer/input", json!({"action":"key","key":"enter"})).await.is_error);
        assert_eq!(bridge.foreground_delegations.load(Ordering::Acquire),0);
        bridge.lose_reply.store(true, Ordering::Release);
        let result=port.execute_authorized("computer/input", json!({"action":"focus_window","window_id":42})).await;
        assert!(result.is_error && result.content.contains("RESULT_UNKNOWN"));
        assert_eq!(bridge.foreground_delegations.load(Ordering::Acquire),1);
        assert_eq!(bridge.calls.lock().unwrap().len(),3,"observe, key, one focus; no replay");
        port.clear_cached_observation();
        assert!(is_proven_observation_rejection(&port.execute_authorized("computer/input", json!({"action":"focus_window","window_id":42})).await));
        assert_eq!(bridge.foreground_delegations.load(Ordering::Acquire),1,"stale observation cannot delegate or dispatch");
    }

    #[tokio::test]
    async fn waiting_for_unlock_never_opens_a_uac_connection() {
        let (port, probe, bridge) = fixture().await;
        probe.high.store(true, Ordering::Release);
        probe.secure.store(true, Ordering::Release);
        assert!(
            !port
                .execute_authorized("computer/observe", json!({"action":"wait","seconds":0}))
                .await
                .is_error
        );
        assert!(bridge.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn cursor_metadata_never_inspects_the_foreground_token_or_opens_uac() {
        let (port, probe, bridge) = fixture().await;
        probe.high.store(true, Ordering::Release);
        let result = port
            .execute_authorized("computer/observe", json!({"action":"cursor_position"}))
            .await;
        assert!(!result.is_error, "{}", result.content);
        assert_eq!(probe.target_queries.load(Ordering::Acquire), 0);
        assert!(bridge.calls.lock().unwrap().is_empty());
    }
}
