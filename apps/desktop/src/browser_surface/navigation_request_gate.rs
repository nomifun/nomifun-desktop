//! Request-stage hold for the owned root document. NavigationStarting alone
//! cannot prevent WebView2 from starting a GET before its cancellation callback.
//! Fetch IDs and the native root FrameId are private, transient routing proof.
use serde::Deserialize;
use serde_json::{Value, json};
use std::{cell::RefCell, collections::{HashMap, HashSet, VecDeque}, rc::Rc};
use tokio::sync::{oneshot, watch};
use webview2_com::{
    CallDevToolsProtocolMethodCompletedHandler, DevToolsProtocolEventReceivedEventHandler,
    Microsoft::Web::WebView2::Win32::*,
};
use windows::core::{HSTRING, Interface, PCWSTR, PWSTR};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

const MAX_PENDING: usize = 128;
const MAX_EVENT_TEXT: usize = 4 * 1024 * 1024;
const UNAVAILABLE: &str = "Browser request-stage navigation control is unavailable.";
type Completion = Result<Value, String>;
type Waiter = watch::Receiver<Option<Completion>>;

#[derive(Clone, Copy)]
pub(crate) struct GateLease { epoch: u64 }

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase { Disabled, Enabling, Enabled, Disabling, Unavailable, Closed }
struct PendingCall {
    completion: watch::Sender<Option<Completion>>,
    request: Option<String>,
}
struct State {
    phase: Phase,
    root: Option<String>,
    retired_roots: HashSet<String>,
    fetch_may_be_enabled: bool,
    fetch_enabled: bool,
    allow_after: Option<f64>,
    network: HashMap<String, (String, Option<f64>)>,
    network_order: VecDeque<String>,
    terminal_network: HashMap<String, String>,
    terminal_order: VecDeque<String>,
    awaiting_network: HashMap<String, (String, String)>,
    epoch: u64,
    next_call: u64,
    calls: HashMap<u64, PendingCall>,
    // Failed resolutions retain ownership until native controller destruction.
    requests: HashMap<String, Option<u64>>,
    request_routes: HashMap<String, (String, Option<String>)>,
}
impl State {
    fn new() -> Self {
        Self { phase: Phase::Disabled, root: None, retired_roots: HashSet::new(),
            fetch_may_be_enabled: false, fetch_enabled: false, allow_after: None,
            network: HashMap::new(), network_order: VecDeque::new(),
            terminal_network: HashMap::new(), terminal_order: VecDeque::new(), awaiting_network: HashMap::new(),
            epoch: 0, next_call: 0, calls: HashMap::new(), requests: HashMap::new(), request_routes: HashMap::new() }
    }
    fn fail(&mut self) {
        if self.phase != Phase::Closed {
            self.phase = if self.fetch_may_be_enabled { Phase::Unavailable } else { Phase::Disabled };
        }
    }
    fn set_root(&mut self, root: String) -> Result<(), String> {
        if self.root.as_ref().is_some_and(|previous| previous != &root) {
            if self.retired_roots.len() >= MAX_PENDING { self.root = None; self.fail(); return Err(UNAVAILABLE.into()); }
            self.retired_roots.insert(self.root.take().unwrap());
        }
        self.root = Some(root);
        // Parentless native lifecycle proof can re-establish a root under
        // MPArch/prerender activation. It must not remain classified as retired.
        self.retired_roots.remove(self.root.as_ref().unwrap());
        Ok(())
    }
    fn disposition(&self, event: &Paused) -> Result<Option<bool>, String> {
        if self.retired_roots.contains(&event.frame_id) { return Ok(Some(true)); }
        let root = self.root.as_deref().ok_or(UNAVAILABLE)?;
        if root != event.frame_id { return Ok(Some(false)); }
        if self.phase != Phase::Disabled { return Ok(Some(true)); }
        let network = event.network_id.as_deref().ok_or(UNAVAILABLE)?;
        if let Some(frame) = self.terminal_network.get(network) {
            if frame != &event.frame_id { return Err(UNAVAILABLE.into()); }
            return Ok(Some(true));
        }
        let Some((frame, at)) = self.network.get(network) else { return Ok(None); };
        if frame != &event.frame_id { return Err(UNAVAILABLE.into()); }
        let Some(at) = at else { return Ok(Some(true)); };
        let cutoff = self.allow_after.ok_or(UNAVAILABLE)?;
        Ok(Some(*at <= cutoff))
    }
    fn terminal(&self, frame: &str, network: Option<&str>) -> bool {
        network.is_some_and(|network| self.terminal_network.get(network).is_some_and(|known| known == frame))
    }
}
struct Registration {
    core: ICoreWebView2,
    receivers: Vec<(ICoreWebView2DevToolsProtocolEventReceiver, i64)>,
    state: Rc<RefCell<State>>,
}
impl Drop for Registration {
    fn drop(&mut self) {
        // This owner is removed only after positive native controller closure.
        // Fetch.disable on a live Page would resume unresolved intercepted jobs.
        for (receiver, token) in &self.receivers {
            let _ = unsafe { receiver.remove_DevToolsProtocolEventReceived(*token) };
        }
        let mut state = self.state.borrow_mut();
        state.phase = Phase::Closed;
        state.requests.clear();
        state.request_routes.clear();
        state.awaiting_network.clear();
        state.network.clear();
        for (_, call) in state.calls.drain() {
            call.completion.send_replace(Some(Err(UNAVAILABLE.into())));
        }
    }
}
thread_local! { static REGISTRATIONS: RefCell<HashMap<String, Registration>> = RefCell::default(); }

fn owner(label: &str) -> Result<(ICoreWebView2, Rc<RefCell<State>>), String> {
    REGISTRATIONS.with(|entries| entries.borrow().get(label)
        .map(|entry| (entry.core.clone(), entry.state.clone())).ok_or_else(|| UNAVAILABLE.into()))
}
fn identifier(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Paused {
    request_id: String,
    frame_id: String,
    resource_type: String,
    network_id: Option<String>,
    response_error_reason: Option<serde::de::IgnoredAny>,
    response_status_code: Option<serde::de::IgnoredAny>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NetworkStart {
    request_id: String,
    frame_id: Option<String>,
    #[serde(rename = "type")]
    resource_type: Option<String>,
    timestamp: f64,
    redirect_response: Option<serde::de::IgnoredAny>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NetworkEnd { request_id: String }
#[derive(Deserialize)]
struct FrameNavigation { frame: FrameIdentity }
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FrameIdentity { id: String, parent_id: Option<String> }
fn paused(text: &str) -> Result<Paused, String> {
    // Unknown fields, including request headers, cookies and POST data, are
    // consumed by IgnoredAny; none are copied into a retained JSON object.
    let event: Paused = serde_json::from_str(text).map_err(|_| UNAVAILABLE.to_owned())?;
    if !identifier(&event.request_id) || !identifier(&event.frame_id)
        || event.network_id.as_deref().is_some_and(|id| !identifier(id))
        || event.resource_type != "Document" || event.response_error_reason.is_some()
        || event.response_status_code.is_some() { return Err(UNAVAILABLE.into()); }
    Ok(event)
}

/// Chromium's Windows TimeTicks always chooses QPC on CPUs reporting invariant
/// TSC (base/cpu.cc leaf 0x80000007 EDX bit 8; base/time/time_win.cc). Other clock
/// choices, including per-process rollover tracking, do not establish origin
/// equality. Return unavailable before installing Fetch on those machines.
fn monotonic_seconds() -> Result<f64, String> {
    #[cfg(target_arch = "x86_64")]
    let invariant = {
        std::arch::x86_64::__cpuid(0x80000000).eax >= 0x80000007
            && std::arch::x86_64::__cpuid(0x80000007).edx & (1 << 8) != 0
    };
    #[cfg(target_arch = "x86")]
    let invariant = {
        std::arch::x86::__cpuid(0x80000000).eax >= 0x80000007
            && std::arch::x86::__cpuid(0x80000007).edx & (1 << 8) != 0
    };
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    let invariant = false;
    if !invariant { return Err(UNAVAILABLE.into()); }
    let (mut ticks, mut frequency) = (0_i64, 0_i64);
    unsafe { QueryPerformanceFrequency(&mut frequency) }.map_err(|_| UNAVAILABLE)?;
    unsafe { QueryPerformanceCounter(&mut ticks) }.map_err(|_| UNAVAILABLE)?;
    if ticks <= 0 || frequency <= 0 { return Err(UNAVAILABLE.into()); }
    // Match QPCValueToTimeDelta's integer microsecond truncation and
    // TimeDelta::InSecondsF, without the intermediate i64 multiplication overflow.
    // Windows documents ambiguous cross-thread ordering within one QPC tick;
    // include that tick in the blocked side of the Resume boundary.
    Ok((((i128::from(ticks) + 1) * 1_000_000) / i128::from(frequency)) as f64 / 1_000_000.0)
}

pub(crate) fn supported() -> bool {
    monotonic_seconds().is_ok()
}

/// UI-thread evidence that the acknowledged browser interceptor is holding the
/// verified root. A first activation still pending does not obtain this proof.
pub(super) fn blocking(label: &str) -> bool {
    owner(label).is_ok_and(|(_, state)| {
        let current = state.borrow();
        current.fetch_enabled && current.root.is_some()
            && matches!(current.phase, Phase::Enabling | Phase::Enabled | Phase::Disabling)
    })
}

fn reply(method: &str, response: &str) -> Completion {
    if response.len() > MAX_EVENT_TEXT { return Err(UNAVAILABLE.into()); }
    let value: Value = serde_json::from_str(response).map_err(|_| UNAVAILABLE.to_owned())?;
    let object = value.as_object().ok_or(UNAVAILABLE)?;
    if object.contains_key("error") || object.contains_key("errorText") { return Err(UNAVAILABLE.into()); }
    if method == "Page.getFrameTree" {
        root_frame(&value)?;
    } else if !object.is_empty() { return Err(UNAVAILABLE.into()); }
    Ok(value)
}
fn root_frame(tree: &Value) -> Result<String, String> {
    let frame = &tree["frameTree"]["frame"];
    let root = frame["id"].as_str().filter(|id| identifier(id)).ok_or(UNAVAILABLE)?;
    if frame.get("parentId").is_some() { return Err(UNAVAILABLE.into()); }
    Ok(root.to_owned())
}

/// Browser-side gate commands have their own bounded ownership. They must not
/// depend on renderer readiness or the ordinary DOM/CDP command queue's quota.
fn submit(core: &ICoreWebView2, state: Rc<RefCell<State>>, method: &str,
    params: Value, request: Option<String>) -> Result<Waiter, String> {
    let (completion, waiter) = watch::channel(None);
    let id = {
        let mut current = state.borrow_mut();
        if current.phase == Phase::Closed || current.calls.len() >= MAX_PENDING {
            current.fail();
            return Err(UNAVAILABLE.into());
        }
        current.next_call = current.next_call.saturating_add(1);
        let id = current.next_call;
        if let Some(request) = &request {
            current.awaiting_network.remove(request);
            current.requests.insert(request.clone(), Some(id));
        }
        // Set only after admission, immediately before actual native dispatch.
        // A synchronous/unknown activation result cannot authorize rollback.
        if method == "Fetch.enable" { current.fetch_may_be_enabled = true; }
        current.calls.insert(id, PendingCall { completion, request });
        id
    };
    let callback_state = state.clone();
    let callback_core = core.clone();
    let response_method = method.to_owned();
    let callback = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |status, response| {
        let result = status.map_err(|_| UNAVAILABLE.to_owned()).and_then(|()| reply(&response_method, &response));
        let failed = result.is_err();
        let live_failure = {
            let mut current = callback_state.borrow_mut();
            if !failed && response_method == "Fetch.enable" && current.phase != Phase::Closed {
                current.fetch_enabled = true;
            }
            if let Some(call) = current.calls.remove(&id) {
                let retired = call.request.as_ref().and_then(|request| current.request_routes.get(request))
                    .is_some_and(|(frame, network)| current.terminal(frame, network.as_deref()));
                if let Some(request) = &call.request {
                    if current.requests.get(request) == Some(&Some(id)) {
                        if failed && !retired { current.requests.insert(request.clone(), None); }
                        else { current.requests.remove(request); current.request_routes.remove(request); }
                    }
                }
                if failed && !retired { current.fail(); }
                // Native terminal proof retires routing, but the caller still
                // receives the actual failed protocol result, never a fake ACK.
                call.completion.send_replace(Some(result));
                failed && !retired
            } else { false }
        };
        if live_failure { let _ = unsafe { callback_core.Stop() }; }
        Ok(())
    }));
    let method = HSTRING::from(method);
    let params = HSTRING::from(params.to_string());
    // No RefCell or registry borrow is held across a possibly reentrant COM call.
    let result = unsafe { core.CallDevToolsProtocolMethod(PCWSTR(method.as_ptr()), PCWSTR(params.as_ptr()), &callback) };
    if result.is_err() {
        let mut current = state.borrow_mut();
        if let Some(call) = current.calls.remove(&id) {
            if let Some(request) = &call.request { current.requests.insert(request.clone(), None); }
            current.fail();
            call.completion.send_replace(Some(Err(UNAVAILABLE.into())));
        }
        drop(current);
        let _ = unsafe { core.Stop() };
    }
    Ok(waiter)
}
async fn completed(mut waiter: Waiter) -> Completion {
    loop {
        if let Some(result) = waiter.borrow_and_update().clone() { return result; }
        waiter.changed().await.map_err(|_| UNAVAILABLE.to_owned())?;
    }
}
async fn call(view: &tauri::Webview, method: &'static str, params: Value) -> Completion {
    let label = view.label().to_owned();
    let (tx, rx) = oneshot::channel();
    view.with_webview(move |_| {
        let result = owner(&label).and_then(|(core, state)| submit(&core, state, method, params, None));
        let _ = tx.send(result);
    }).map_err(|_| UNAVAILABLE.to_owned())?;
    completed(rx.await.map_err(|_| UNAVAILABLE.to_owned())??).await
}

fn resolve(core: &ICoreWebView2, state: Rc<RefCell<State>>, request_id: String, block: bool) -> Result<(), String> {
    let (method, params) = if block {
        ("Fetch.failRequest", json!({"requestId":request_id,"errorReason":"Aborted"}))
    } else { ("Fetch.continueRequest", json!({"requestId":request_id})) };
    let _ = submit(core, state, method, params, Some(request_id))?;
    Ok(())
}
fn resolve_waiting(core: &ICoreWebView2, state: Rc<RefCell<State>>) -> Result<(), String> {
    let events: Vec<_> = state.borrow().awaiting_network.iter().map(|(request, (network, frame))| Paused {
        request_id: request.clone(), frame_id: frame.clone(), resource_type: "Document".into(),
        network_id: Some(network.clone()), response_error_reason: None, response_status_code: None,
    }).collect();
    for event in events {
        if state.borrow().terminal(&event.frame_id, event.network_id.as_deref()) {
            let mut current = state.borrow_mut();
            current.awaiting_network.remove(&event.request_id);
            current.requests.remove(&event.request_id);
            current.request_routes.remove(&event.request_id);
            continue;
        }
        let disposition = state.borrow().disposition(&event)?;
        if let Some(block) = disposition { resolve(core, state.clone(), event.request_id, block)?; }
    }
    Ok(())
}
fn handle_event(core: &ICoreWebView2, state: Rc<RefCell<State>>, method: &str, text: &str) -> Result<(), String> {
    if state.borrow().phase == Phase::Closed { return Ok(()); }
    match method {
        "Fetch.requestPaused" => {
            let event = paused(text)?;
            let disposition = {
                let mut current = state.borrow_mut();
                if !current.fetch_may_be_enabled { return Ok(()); }
                if current.requests.len() >= MAX_PENDING || current.requests.contains_key(&event.request_id) {
                    current.fail(); return Err(UNAVAILABLE.into());
                }
                if current.terminal(&event.frame_id, event.network_id.as_deref()) {
                    // A previously proved native Document/frame terminal is
                    // separate settlement evidence. No continue or replay, and
                    // no Aborted protocol acknowledgement is invented.
                    return Ok(());
                }
                let disposition = current.disposition(&event)?;
                current.request_routes.insert(event.request_id.clone(), (event.frame_id.clone(), event.network_id.clone()));
                if disposition.is_none() {
                    let network = event.network_id.clone().ok_or(UNAVAILABLE)?;
                    current.awaiting_network.insert(event.request_id.clone(), (network, event.frame_id.clone()));
                    current.requests.insert(event.request_id.clone(), None);
                }
                disposition
            };
            if let Some(block) = disposition { resolve(core, state, event.request_id, block)?; }
        }
        "Network.requestWillBeSent" => {
            let event: NetworkStart = serde_json::from_str(text).map_err(|_| UNAVAILABLE.to_owned())?;
            if event.resource_type.as_deref() != Some("Document") { return Ok(()); }
            let frame = event.frame_id.filter(|frame| identifier(frame)).ok_or(UNAVAILABLE)?;
            if !identifier(&event.request_id) || !event.timestamp.is_finite() || event.timestamp <= 0.0 { return Err(UNAVAILABLE.into()); }
            {
                let mut current = state.borrow_mut();
                if let Some((previous_frame, at)) = current.network.get_mut(&event.request_id) {
                    if previous_frame != &frame { return Err(UNAVAILABLE.into()); }
                    // Redirect hops reuse Network IDs; retain the creation time
                    // so a late hop cannot turn a held request into a fresh one.
                    if let Some(at) = at { *at = at.min(event.timestamp); }
                } else {
                    current.network_order.push_back(event.request_id.clone());
                    // A redirect without its earliest creation fact is not a
                    // fresh request, even if an old terminal tombstone evicted.
                    let at = if event.redirect_response.is_some() || current.terminal_network.contains_key(&event.request_id) { None } else { Some(event.timestamp) };
                    current.network.insert(event.request_id, (frame, at));
                }
                while current.network_order.len() > MAX_PENDING {
                    if let Some(old) = current.network_order.pop_front() {
                        let root_fact = current.network.get(&old).is_some_and(|(frame, _)|
                            current.root.as_ref() == Some(frame) || current.retired_roots.contains(frame));
                        if current.fetch_may_be_enabled && root_fact {
                            // Without the earliest fact, a subsequent redirect
                            // under this ID could look newer than Resume.
                            current.fail(); return Err(UNAVAILABLE.into());
                        }
                        current.network.remove(&old);
                    }
                }
            }
            resolve_waiting(core, state)?;
        }
        "Network.loadingFinished" | "Network.loadingFailed" => {
            let event: NetworkEnd = serde_json::from_str(text).map_err(|_| UNAVAILABLE.to_owned())?;
            if !identifier(&event.request_id) { return Err(UNAVAILABLE.into()); }
            let mut current = state.borrow_mut();
            // A native terminal event ends this ID's redirect lineage. A late
            // Fetch event with that ID remains unknown and can never be allowed
            // by borrowing another request's time.
            if let Some((frame, _)) = current.network.remove(&event.request_id) {
                if current.root.as_ref() == Some(&frame) || current.retired_roots.contains(&frame) {
                    if !current.terminal_network.contains_key(&event.request_id) {
                        current.terminal_order.push_back(event.request_id.clone());
                    }
                    current.terminal_network.insert(event.request_id.clone(), frame);
                    while current.terminal_order.len() > MAX_PENDING {
                        if let Some(old) = current.terminal_order.pop_front() { current.terminal_network.remove(&old); }
                    }
                }
            }
            current.network_order.retain(|id| id != &event.request_id);
            drop(current);
            resolve_waiting(core, state)?;
        }
        "Page.frameNavigated" => {
            let event: FrameNavigation = serde_json::from_str(text).map_err(|_| UNAVAILABLE.to_owned())?;
            if event.frame.parent_id.is_none() {
                if !identifier(&event.frame.id) { return Err(UNAVAILABLE.into()); }
                state.borrow_mut().set_root(event.frame.id)?;
                resolve_waiting(core, state)?;
            }
        }
        _ => return Err(UNAVAILABLE.into()),
    }
    Ok(())
}

pub(crate) async fn install(view: &tauri::Webview) -> Result<(), String> {
    let label = view.label().to_owned();
    let (tx, rx) = oneshot::channel();
    view.with_webview(move |platform| {
        let result = (|| -> windows::core::Result<()> {
            if REGISTRATIONS.with(|entries| entries.borrow().contains_key(&label)) { return Ok(()); }
            let core = unsafe { platform.controller().CoreWebView2()? };
            // The event session identity is required to exclude attached iframes.
            let _ = core.cast::<ICoreWebView2_11>()?;
            let state = Rc::new(RefCell::new(State::new()));
            let mut registration = Registration { core: core.clone(), receivers: Vec::new(), state: state.clone() };
            for method in ["Fetch.requestPaused", "Network.requestWillBeSent", "Network.loadingFinished", "Network.loadingFailed", "Page.frameNavigated"] {
                let event_state = state.clone();
                let event_core = core.clone();
                let handler = DevToolsProtocolEventReceivedEventHandler::create(Box::new(move |_, args| {
                    let result = (|| -> Result<(), String> {
                        let args = args.ok_or(UNAVAILABLE)?;
                        let args2 = args.cast::<ICoreWebView2DevToolsProtocolEventReceivedEventArgs2>().map_err(|_| UNAVAILABLE)?;
                        let mut raw = PWSTR::null();
                        unsafe { args2.SessionId(&mut raw) }.map_err(|_| UNAVAILABLE)?;
                        let session = super::event_string(raw, 256).ok_or(UNAVAILABLE)?;
                        if !session.is_empty() { return Ok(()); }
                        let mut raw = PWSTR::null();
                        unsafe { args.ParameterObjectAsJson(&mut raw) }.map_err(|_| UNAVAILABLE)?;
                        let text = super::event_string(raw, MAX_EVENT_TEXT).ok_or(UNAVAILABLE)?;
                        handle_event(&event_core, event_state.clone(), method, &text)
                    })();
                    let live_failure = result.is_err() && {
                        let current = event_state.borrow();
                        current.fetch_may_be_enabled && current.phase != Phase::Closed
                    };
                    if live_failure {
                        {
                            let mut current = event_state.borrow_mut();
                            if method == "Page.frameNavigated" { current.root = None; }
                            current.fail();
                        }
                        let _ = unsafe { event_core.Stop() };
                    }
                    Ok(())
                }));
                let name = HSTRING::from(method);
                let receiver = unsafe { core.GetDevToolsProtocolEventReceiver(PCWSTR(name.as_ptr()))? };
                let mut token = 0;
                unsafe { receiver.add_DevToolsProtocolEventReceived(&handler, &mut token)?; }
                registration.receivers.push((receiver, token));
            }
            REGISTRATIONS.with(|entries| entries.borrow_mut().insert(label, registration));
            Ok(())
        })().map_err(|_| UNAVAILABLE.to_owned());
        let _ = tx.send(result);
    }).map_err(|_| UNAVAILABLE.to_owned())?;
    rx.await.map_err(|_| UNAVAILABLE.to_owned())?
}

pub(crate) async fn enable(view: &tauri::Webview) -> Result<GateLease, String> {
    // Establish the supported shared clock before any Fetch activation. A
    // definite failure here permits the native pending-hold flag to roll back.
    monotonic_seconds()?;
    let label = view.label().to_owned();
    let (tx, rx) = oneshot::channel::<Result<(u64, bool), String>>();
    view.with_webview(move |_| {
        let result = owner(&label).and_then(|(core, state)| {
            let (epoch, activated) = {
                let mut current = state.borrow_mut();
                if current.phase != Phase::Disabled { return Err(UNAVAILABLE.into()); }
                current.epoch = current.epoch.saturating_add(1);
                current.phase = Phase::Enabling;
                (current.epoch, current.fetch_enabled)
            };
            resolve_waiting(&core, state)?;
            Ok((epoch, activated))
        });
        let _ = tx.send(result);
    }).map_err(|_| UNAVAILABLE.to_owned())?;
    let (epoch, activated) = rx.await.map_err(|_| UNAVAILABLE.to_owned())??;
    let tree = call(view, "Page.getFrameTree", json!({})).await?;
    let root = root_frame(&tree)?;
    drop(tree);
    let label = view.label().to_owned();
    let (tx, rx) = oneshot::channel();
    view.with_webview(move |_| {
        let result = owner(&label).and_then(|(_, state)| {
            let mut current = state.borrow_mut();
            if current.epoch != epoch || current.phase != Phase::Enabling { return Err(UNAVAILABLE.into()); }
            current.set_root(root)?;
            Ok(())
        });
        let _ = tx.send(result);
    }).map_err(|_| UNAVAILABLE.to_owned())?;
    rx.await.map_err(|_| UNAVAILABLE.to_owned())??;
    call(view, "Network.enable", json!({"maxTotalBufferSize":0,"maxResourceBufferSize":0,"maxPostDataSize":0})).await?;
    if !activated {
        call(view, "Fetch.enable", json!({"patterns":[{"urlPattern":"*","resourceType":"Document","requestStage":"Request"}],"handleAuthRequests":false})).await?;
    }
    let label = view.label().to_owned();
    let (tx, rx) = oneshot::channel();
    view.with_webview(move |_| {
        let result = owner(&label).and_then(|(_, state)| {
            let mut current = state.borrow_mut();
            if current.epoch != epoch || current.phase != Phase::Enabling || !current.fetch_enabled { return Err(UNAVAILABLE.into()); }
            current.phase = Phase::Enabled;
            Ok(())
        });
        let _ = tx.send(result);
    }).map_err(|_| UNAVAILABLE.to_owned())?;
    rx.await.map_err(|_| UNAVAILABLE.to_owned())??;
    Ok(GateLease { epoch })
}

pub(crate) async fn disable_guarded(view: &tauri::Webview, expected: Option<super::navigation::Receipt>) -> Result<(), String> {
    release(view, expected, None).await
}

pub(crate) async fn disable_owned(view: &tauri::Webview, lease: GateLease) -> Result<(), String> {
    release(view, None, Some(lease)).await
}

async fn release(view: &tauri::Webview, expected: Option<super::navigation::Receipt>, lease: Option<GateLease>) -> Result<(), String> {
    let label = view.label().to_owned();
    let (tx, rx) = oneshot::channel::<Result<Option<u64>, String>>();
    view.with_webview(move |_| {
        if let Some(expected) = expected {
            if let Err(error) = super::navigation::check(&label, expected) { let _ = tx.send(Err(error)); return; }
        }
        let result = match owner(&label) {
            // A failed installation never enabled this private Fetch domain.
            Err(_) => Ok(None),
            Ok((core, state)) => (|| {
            let mut current = state.borrow_mut();
            if lease.is_some_and(|lease| lease.epoch != current.epoch) { return Err(UNAVAILABLE.into()); }
            if !current.fetch_may_be_enabled { current.phase = Phase::Disabled; return Ok(None); }
            if current.phase == Phase::Disabled { return Ok(None); }
            if !current.fetch_enabled { return Err(UNAVAILABLE.into()); }
            if !matches!(current.phase, Phase::Enabled | Phase::Enabling | Phase::Disabling) { return Err(UNAVAILABLE.into()); }
            if lease.is_none() { current.epoch = current.epoch.saturating_add(1); }
            current.phase = Phase::Disabling;
            let epoch = current.epoch;
            drop(current);
            resolve_waiting(&core, state)?;
            Ok(Some(epoch))
            })(),
        };
        let _ = tx.send(result);
    }).map_err(|_| UNAVAILABLE.to_owned())?;
    let Some(epoch) = rx.await.map_err(|_| UNAVAILABLE.to_owned())?? else { return Ok(()); };
    loop {
        let label = view.label().to_owned();
        let (tx, rx) = oneshot::channel::<Result<Vec<(Waiter, Option<(String, String)>)>, String>>();
        view.with_webview(move |_| {
            let result = owner(&label).and_then(|(_, state)| {
                let current = state.borrow();
                if current.epoch != epoch || current.phase != Phase::Disabling { return Err(UNAVAILABLE.into()); }
                if current.requests.values().any(Option::is_none) { return Err(UNAVAILABLE.into()); }
                Ok(current.calls.values().filter(|call| call.request.is_some()).map(|call| {
                    let route = call.request.as_ref().and_then(|request| current.request_routes.get(request))
                        .and_then(|(frame, network)| network.as_ref().map(|network| (frame.clone(), network.clone())));
                    (call.completion.subscribe(), route)
                }).collect::<Vec<_>>())
            });
            let _ = tx.send(result);
        }).map_err(|_| UNAVAILABLE.to_owned())?;
        let waiters = rx.await.map_err(|_| UNAVAILABLE.to_owned())??;
        if waiters.is_empty() { break; }
        for (waiter, route) in waiters {
            if completed(waiter).await.is_err() {
                let label = view.label().to_owned();
                let (tx, rx) = oneshot::channel();
                view.with_webview(move |_| {
                    let retired = owner(&label).is_ok_and(|(_, state)| route.as_ref().is_some_and(|(frame, network)| state.borrow().terminal(frame, Some(network))));
                    let _ = tx.send(retired);
                }).map_err(|_| UNAVAILABLE.to_owned())?;
                if !rx.await.map_err(|_| UNAVAILABLE.to_owned())? { return Err(UNAVAILABLE.into()); }
            }
        }
    }
    // Fetch remains attached until controller destruction: disabling its native
    // interceptor would release jobs whose events have not arrived yet. The
    // atomic native-clock cutoff makes those late requests independently old.
    let label = view.label().to_owned();
    let (tx, rx) = oneshot::channel::<Result<(), String>>();
    view.with_webview(move |_| {
        let result = owner(&label).and_then(|(_, state)| {
            let mut current = state.borrow_mut();
            if current.epoch != epoch || current.phase != Phase::Disabling || !current.requests.is_empty() { return Err(UNAVAILABLE.into()); }
            if lease.is_some_and(|lease| lease.epoch != current.epoch) { return Err(UNAVAILABLE.into()); }
            if let Some(expected) = expected {
                if let Err(error) = super::navigation::check(&label, expected) {
                    // This release still owns its epoch. Preserve the hold;
                    // never release a successor using an old control receipt.
                    current.phase = Phase::Enabled;
                    return Err(error);
                }
            }
            current.allow_after = Some(monotonic_seconds()?);
            current.phase = Phase::Disabled;
            Ok(())
        });
        let _ = tx.send(result);
    }).map_err(|_| UNAVAILABLE.to_owned())?;
    rx.await.map_err(|_| UNAVAILABLE.to_owned())?
}

/// True only if this helper has definitely never submitted Fetch activation.
/// An unsupported/pre-activation failure can release a native pending hold;
/// unknown activation or intercepted ownership cannot obtain that authority.
pub(crate) async fn safeinactive(view: &tauri::Webview) -> Result<bool, String> {
    let label = view.label().to_owned();
    let (tx, rx) = oneshot::channel();
    view.with_webview(move |_| {
        let inactive = owner(&label).map_or(true, |(_, state)| !state.borrow().fetch_may_be_enabled);
        let _ = tx.send(inactive);
    }).map_err(|_| UNAVAILABLE.to_owned())?;
    rx.await.map_err(|_| UNAVAILABLE.to_owned())
}

pub(super) fn close_view(label: &str) {
    drop(REGISTRATIONS.with(|entries| entries.borrow_mut().remove(label)));
}
pub(super) fn document_exited(label: &str) {
    // Renderer exit does not destroy the browser-process Fetch interceptor.
    // Retain its receiver and callbacks: recovery drains/disables this same
    // native Page through browser-side CDP. Only close_view releases authority.
    // An exited renderer must not strand a frame-tree/control waiter and the
    // operation coordinator. Exact intercepted requests and their browser-side
    // resolution callbacks remain owned until acknowledged or native close.
    if let Ok((_, state)) = owner(label) {
        let mut current = state.borrow_mut();
        let controls: Vec<_> = current.calls.iter().filter_map(|(id, call)| call.request.is_none().then_some(*id)).collect();
        for id in controls {
            if let Some(call) = current.calls.remove(&id) {
                call.completion.send_replace(Some(Err(UNAVAILABLE.into())));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn request_stage_document_identity_excludes_response_and_unidentified_events() {
        assert!(paused(r#"{"requestId":"r1","frameId":"root","resourceType":"Document"}"#).is_ok());
        for text in [
            r#"{"requestId":"","frameId":"root","resourceType":"Document"}"#,
            r#"{"requestId":"r1","frameId":"","resourceType":"Document"}"#,
            r#"{"requestId":"r1","frameId":"root","resourceType":"XHR"}"#,
            r#"{"requestId":"r1","frameId":"root","resourceType":"Document","responseStatusCode":200}"#,
        ] { assert!(paused(text).is_err()); }
    }
    #[test]
    fn only_native_frame_tree_root_proves_main_document() {
        assert_eq!(root_frame(&json!({"frameTree":{"frame":{"id":"root"},"childFrames":[{"frame":{"id":"iframe","parentId":"root"}}]}})).unwrap(), "root");
        assert!(root_frame(&json!({"frameTree":{"frame":{"id":"iframe","parentId":"root"}}})).is_err());
        assert!(root_frame(&json!({"frameTree":{"frame":{"id":""}}})).is_err());
    }
    #[test]
    fn private_request_fields_are_not_retained_by_routing_parser() {
        let event = paused(r#"{"requestId":"r1","frameId":"iframe","resourceType":"Document","request":{"url":"https://secret.test/?token=secret","headers":{"Cookie":"session=secret"},"postData":"secret"}}"#).unwrap();
        assert_eq!(event.request_id, "r1");
        assert_eq!(event.frame_id, "iframe");
        assert_eq!(event.resource_type, "Document");
    }
    #[test]
    fn late_held_request_is_aborted_after_resume_and_fresh_exact_join_continues() {
        let mut state = State::new();
        state.set_root("root".into()).unwrap();
        state.fetch_may_be_enabled = true;
        state.fetch_enabled = true;
        state.phase = Phase::Disabled;
        state.allow_after = Some(20.0);
        state.network.insert("old-network".into(), ("root".into(), Some(19.0)));
        state.network.insert("new-network".into(), ("root".into(), Some(21.0)));
        let old = paused(r#"{"requestId":"old-fetch","networkId":"old-network","frameId":"root","resourceType":"Document"}"#).unwrap();
        let fresh = paused(r#"{"requestId":"new-fetch","networkId":"new-network","frameId":"root","resourceType":"Document"}"#).unwrap();
        assert_eq!(state.disposition(&old).unwrap(), Some(true));
        assert_eq!(state.disposition(&fresh).unwrap(), Some(false));
        state.phase = Phase::Enabled;
        assert_eq!(state.disposition(&fresh).unwrap(), Some(true));
    }
    #[test]
    fn unassociated_or_wrong_frame_root_request_cannot_use_resume_authority() {
        let mut state = State::new();
        state.set_root("root".into()).unwrap();
        state.allow_after = Some(20.0);
        let event = paused(r#"{"requestId":"fetch","networkId":"missing","frameId":"root","resourceType":"Document"}"#).unwrap();
        assert_eq!(state.disposition(&event).unwrap(), None);
        state.network.insert("missing".into(), ("iframe".into(), Some(21.0)));
        assert!(state.disposition(&event).is_err());
        let unknown = paused(r#"{"requestId":"fetch","frameId":"root","resourceType":"Document"}"#).unwrap();
        assert!(state.disposition(&unknown).is_err());
    }
    #[test]
    fn frame_scope_retirement_does_not_turn_old_root_into_an_allowed_iframe() {
        let mut state = State::new();
        state.set_root("old-root".into()).unwrap();
        state.set_root("new-root".into()).unwrap();
        let old = paused(r#"{"requestId":"old","frameId":"old-root","resourceType":"Document"}"#).unwrap();
        let iframe = paused(r#"{"requestId":"child","frameId":"iframe","resourceType":"Document"}"#).unwrap();
        assert_eq!(state.disposition(&old).unwrap(), Some(true));
        assert_eq!(state.disposition(&iframe).unwrap(), Some(false));
    }
    #[test]
    fn protocol_error_objects_never_acknowledge_fetch_activation_or_resolution() {
        assert!(reply("Fetch.enable", "{}").is_ok());
        for text in [r#"{"error":{"code":-32601,"message":"unknown"}}"#, "null", "[]", r#"{"result":{}}"#] {
            assert!(reply("Fetch.enable", text).is_err());
            assert!(reply("Fetch.failRequest", text).is_err());
        }
        assert!(reply("Page.getFrameTree", r#"{"frameTree":{"frame":{"id":"root"}}}"#).is_ok());
        assert!(reply("Page.getFrameTree", "{}").is_err());
    }
    #[test]
    fn definite_failure_before_activation_and_unknown_activation_have_distinct_rollback_authority() {
        let mut state = State::new();
        state.phase = Phase::Enabling;
        state.fail();
        assert!(matches!(state.phase, Phase::Disabled));
        assert!(!state.fetch_may_be_enabled);
        state.phase = Phase::Enabling;
        state.fetch_may_be_enabled = true;
        state.fail();
        assert!(matches!(state.phase, Phase::Unavailable));
        assert!(state.fetch_may_be_enabled);
    }
    #[test]
    fn terminal_then_late_fetch_never_resurrects_timestamp_or_blocks_fresh_identity() {
        let mut state = State::new();
        state.set_root("root".into()).unwrap();
        state.allow_after = Some(20.0);
        state.terminal_network.insert("closed".into(), "root".into());
        let late = paused(r#"{"requestId":"late","networkId":"closed","frameId":"root","resourceType":"Document"}"#).unwrap();
        assert!(state.terminal(&late.frame_id, late.network_id.as_deref()));
        state.network.insert("closed".into(), ("root".into(), Some(21.0)));
        assert_eq!(state.disposition(&late).unwrap(), Some(true));
        // A redirect whose earliest fact/tombstone is absent remains unknown
        // creation, independently of its apparently newer hop timestamp.
        state.terminal_network.remove("closed");
        state.network.insert("closed".into(), ("root".into(), None));
        assert_eq!(state.disposition(&late).unwrap(), Some(true));
        state.network.insert("fresh".into(), ("root".into(), Some(22.0)));
        let fresh = paused(r#"{"requestId":"fresh","networkId":"fresh","frameId":"root","resourceType":"Document"}"#).unwrap();
        assert_eq!(state.disposition(&fresh).unwrap(), Some(false));
    }
    #[test]
    fn authoritative_native_root_reestablishment_revokes_its_retired_classification() {
        let mut state = State::new();
        state.set_root("a".into()).unwrap();
        state.set_root("b".into()).unwrap();
        state.set_root("a".into()).unwrap();
        assert!(!state.retired_roots.contains("a"));
        assert!(state.retired_roots.contains("b"));
        state.allow_after = Some(20.0);
        state.network.insert("fresh".into(), ("a".into(), Some(21.0)));
        let fresh = paused(r#"{"requestId":"fresh","networkId":"fresh","frameId":"a","resourceType":"Document"}"#).unwrap();
        assert_eq!(state.disposition(&fresh).unwrap(), Some(false));
    }
}
