//! Read-only diagnostics from the owned WebView2 and its attached iframe sessions.
//! No Runtime.evaluate, remote object lookup, request bodies, or global targets.
use nomifun_browser_platform::{
    runtime::{BrowserDiagnostic, BrowserTabSnapshot},
    url_projection::project_metadata_url,
};
use serde_json::Value;
use std::{
    cell::RefCell,
    collections::HashMap,
    sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}},
};
use tokio::sync::mpsc;
use tauri::Manager;
use tokio_util::sync::CancellationToken;
use webview2_com::{DevToolsProtocolEventReceivedEventHandler, Microsoft::Web::WebView2::Win32::*};
use windows::core::{HSTRING, Interface, PCWSTR, PWSTR};

const MAX_ENTRIES: usize = 32;
const MAX_TEXT: usize = 512;
const MAX_REQUESTS: usize = 128;
#[path = "diagnostic_scopes.rs"]
mod scopes;

fn lifecycle(method: &str) -> bool {
    method == "Nomi.frameTree" || method.starts_with("Target.") || method.starts_with("Page.frame") || method.starts_with("Runtime.executionContext")
}
type DiagnosticSender = mpsc::Sender<(u64, String, &'static str, String)>;
struct Registration {
    receivers: Vec<(ICoreWebView2DevToolsProtocolEventReceiver, i64)>,
    stop: CancellationToken,
    sender: DiagnosticSender,
    metadata: Arc<Mutex<BrowserTabSnapshot>>,
    routing_lost: Arc<AtomicBool>,
    refresh: mpsc::Sender<()>,
}
impl Drop for Registration {
    fn drop(&mut self) {
        self.stop.cancel();
        for (receiver, token) in &self.receivers {
            let _ = unsafe { receiver.remove_DevToolsProtocolEventReceived(*token) };
        }
    }
}
thread_local! { static REGISTRATIONS: RefCell<HashMap<String,Registration>> = RefCell::default(); }
pub(super) fn close_view(label: &str) {
    drop(REGISTRATIONS.with(|entries| entries.borrow_mut().remove(label)));
}
pub(crate) fn document_loaded(view: &tauri::Webview) {
    let label=view.label().to_owned();
    let _=view.app_handle().run_on_main_thread(move || {
        REGISTRATIONS.with(|entries| {
            if let Some(entry)=entries.borrow().get(&label) { let _=entry.refresh.try_send(()); }
        });
    });
}
pub(super) fn mark_unavailable(view: &tauri::Webview) {
    let label=view.label().to_owned();
    let _=view.app_handle().run_on_main_thread(move || {
        REGISTRATIONS.with(|entries| {
            if let Some(entry)=entries.borrow().get(&label) {
                entry.routing_lost.store(true,Ordering::Release);
                let mut tab=entry.metadata.lock().unwrap_or_else(|error|error.into_inner());
                tab.diagnostics.dropped=tab.diagnostics.dropped.saturating_add(1);
                tab.diagnostics.unavailable=true;
            }
        });
    });
}

fn enqueue(sender: &DiagnosticSender, metadata: &Mutex<BrowserTabSnapshot>, lost: &AtomicBool, method: &'static str, event: Option<(String,String)>) {
    let generation=metadata.lock().unwrap_or_else(|error|error.into_inner()).target.document_generation;
    if !event.is_some_and(|(session,text)|sender.try_send((generation,session,method,text)).is_ok()) {
        if lifecycle(method) { lost.store(true,Ordering::Release); }
        let mut tab=metadata.lock().unwrap_or_else(|error|error.into_inner());
        tab.diagnostics.dropped=tab.diagnostics.dropped.saturating_add(1);
        if lost.load(Ordering::Acquire) { tab.diagnostics.unavailable=true; }
    }
}

fn clipped(value: &str) -> String {
    value.chars().take(MAX_TEXT).collect()
}
fn location(value: &str) -> String {
    clipped(&project_metadata_url(value))
}

#[derive(Default)]
struct Projection {
    generation: u64,
    next_id: u64,
    requests: HashMap<(String, String), String>,
}
impl Projection {
    #[cfg(test)]
    fn apply(
        &mut self,
        tab: &mut BrowserTabSnapshot,
        generation: u64,
        method: &str,
        data: Value,
    ) -> bool {
        self.apply_scoped(tab, generation, "", method, data)
    }
    fn apply_scoped(&mut self, tab: &mut BrowserTabSnapshot, generation: u64, session: &str, method: &str, data: Value) -> bool {
        if generation != tab.target.document_generation {
            return false;
        }
        if self.generation != generation {
            self.generation = generation;
            self.requests.clear();
        }
        let source_url;
        let (kind, level, message) = match method {
            "Runtime.consoleAPICalled" => {
                let level = match data["type"].as_str().unwrap_or("") {
                    "error" | "assert" => "error",
                    "warning" => "warn",
                    "debug" => "debug",
                    _ => "info",
                };
                let args = data["args"]
                    .as_array()
                    .map(|args| {
                        args.iter()
                            .take(8)
                            .map(|arg| {
                                // Render a bounded preview. Never follow an objectId or invoke a getter.
                                if let Some(value) = arg
                                    .get("value")
                                    .filter(|value| !value.is_object() && !value.is_array())
                                {
                                    return clipped(
                                        value
                                            .as_str()
                                            .map(str::to_owned)
                                            .unwrap_or_else(|| value.to_string())
                                            .as_str(),
                                    );
                                }
                                clipped(
                                    arg["unserializableValue"]
                                        .as_str()
                                        .or_else(|| arg["description"].as_str())
                                        .unwrap_or("[object]"),
                                )
                            })
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default();
                source_url = location(
                    data["stackTrace"]["callFrames"][0]["url"]
                        .as_str()
                        .unwrap_or(""),
                );
                ("console", level, clipped(&args))
            }
            "Runtime.exceptionThrown" => {
                let detail = &data["exceptionDetails"];
                source_url = location(detail["url"].as_str().unwrap_or(""));
                (
                    "page_error",
                    "error",
                    clipped(
                        detail["exception"]["description"]
                            .as_str()
                            .or_else(|| detail["text"].as_str())
                            .unwrap_or("Page exception"),
                    ),
                )
            }
            "Network.requestWillBeSent" => {
                if let Some(id) = data["requestId"].as_str().filter(|id| id.len() <= 256) {
                    let key = (session.to_owned(), id.to_owned());
                    if self.requests.len() >= MAX_REQUESTS && !self.requests.contains_key(&key) {
                        tab.diagnostics.dropped = tab.diagnostics.dropped.saturating_add(1);
                        return true;
                    }
                    self.requests.insert(
                        key,
                        location(data["request"]["url"].as_str().unwrap_or("")),
                    );
                }
                return false;
            }
            "Network.loadingFinished" => {
                if let Some(id) = data["requestId"].as_str() {
                    self.requests.remove(&(session.to_owned(), id.to_owned()));
                }
                return false;
            }
            "Network.loadingFailed" => {
                let Some(url) = data["requestId"]
                    .as_str()
                    .and_then(|id| self.requests.remove(&(session.to_owned(), id.to_owned())))
                else {
                    // A failure from an earlier navigation or a dropped request
                    // cannot be attributed to this document's request inventory.
                    tab.diagnostics.dropped = tab.diagnostics.dropped.saturating_add(1);
                    return true;
                };
                source_url = url;
                (
                    "network",
                    "error",
                    clipped(
                        data["errorText"]
                            .as_str()
                            .unwrap_or("Network request failed"),
                    ),
                )
            }
            "Network.responseReceived" => {
                let response = &data["response"];
                let status = response["status"].as_u64().unwrap_or(0);
                if status < 400 {
                    return false;
                }
                if !data["requestId"]
                    .as_str()
                    .is_some_and(|id| self.requests.contains_key(&(session.to_owned(), id.to_owned())))
                {
                    tab.diagnostics.dropped = tab.diagnostics.dropped.saturating_add(1);
                    return true;
                }
                source_url = location(response["url"].as_str().unwrap_or(""));
                ("network", "error", format!("HTTP {status}"))
            }
            _ => return false,
        };
        self.next_id = self.next_id.saturating_add(1);
        if tab.diagnostics.entries.len() >= MAX_ENTRIES {
            tab.diagnostics.entries.remove(0);
            tab.diagnostics.dropped = tab.diagnostics.dropped.saturating_add(1);
        }
        tab.diagnostics.entries.push(BrowserDiagnostic {
            id: self.next_id,
            kind: kind.into(),
            level: level.into(),
            message,
            source_url,
        });
        true
    }
}

pub(crate) async fn install(
    view: &tauri::Webview,
    metadata: Arc<Mutex<BrowserTabSnapshot>>,
) {
    if install_inner(view, metadata.clone()).await.is_err() {
        let mut tab=metadata.lock().unwrap_or_else(|error|error.into_inner());
        tab.diagnostics.unavailable=true;
        drop(tab);
        mark_unavailable(view);
        tracing::warn!("Native browser diagnostics are unavailable; the page remains usable");
    }
}

async fn install_inner(view: &tauri::Webview, metadata: Arc<Mutex<BrowserTabSnapshot>>) -> Result<(), String> {
    let label = view.label().to_owned();
    let (sender, mut receiver) = mpsc::channel::<(u64, String, &'static str, String)>(64);
    let routing_lost = Arc::new(AtomicBool::new(false));
    let worker_routing_lost = routing_lost.clone();
    let stop = CancellationToken::new();
    let (refresh,mut refresh_requests)=mpsc::channel(1);
    let refresh_stop=stop.clone();
    let refresh_view=view.clone();
    // One coalescing metadata worker per native view, not one detached task
    // per navigation. In-flight protocol callbacks retain native settlement.
    tokio::spawn(async move {
        loop {
            tokio::select! {biased;
                _=refresh_stop.cancelled()=>break,
                request=refresh_requests.recv()=>if request.is_none(){break},
            }
            if seed_session(&refresh_view,None).await.is_err() && !refresh_stop.is_cancelled() {
                mark_unavailable(&refresh_view);
            }
        }
    });
    let worker_stop = stop.clone();
    let worker_metadata = metadata.clone();
    tokio::spawn(async move {
        let mut projection = scopes::ScopedProjection::default();
        loop {
            let event = tokio::select! {
                biased;
                _ = worker_stop.cancelled() => break,
                event = receiver.recv() => event,
            };
            let Some(event) = event else { break };
            let mut batch = vec![event];
            while batch.len() < 32 {
                match receiver.try_recv() {
                    Ok(event) => batch.push(event),
                    Err(_) => break,
                }
            }
            {
                let mut tab = worker_metadata
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                for (generation, session, method, parameters) in batch {
                    // Missing lifecycle data is not a license to guess which
                    // document owns subsequent errors. A new native install
                    // gets a fresh scope; this stream remains fail-closed.
                    if worker_routing_lost.load(Ordering::Acquire) { tab.diagnostics.unavailable=true; continue; }
                    if let Ok(data) = serde_json::from_str(&parameters) {
                        projection.apply(&mut tab, generation, &session, method, data);
                    } else if lifecycle(method) {
                        worker_routing_lost.store(true, Ordering::Release);
                        tab.diagnostics.dropped = tab.diagnostics.dropped.saturating_add(1);
                        tab.diagnostics.unavailable=true;
                    }
                }
            }
            tokio::select! {
                _ = worker_stop.cancelled() => break,
                _ = tokio::time::sleep(std::time::Duration::from_millis(16)) => {},
            }
        }
    });
    let (tx, rx) = tokio::sync::oneshot::channel();
    view.with_webview(move |platform| {
        let result = (|| -> windows::core::Result<()> {
            if REGISTRATIONS.with(|entries| entries.borrow().contains_key(&label)) {
                stop.cancel();
                return Ok(());
            }
            let core = unsafe { platform.controller().CoreWebView2()? };
            let mut registration = Registration {
                receivers: vec![],
                stop,
                sender: sender.clone(),
                metadata: metadata.clone(),
                routing_lost: routing_lost.clone(),
                refresh,
            };
            for method in [
                "Target.attachedToTarget",
                "Target.detachedFromTarget",
                "Page.frameNavigated",
                "Page.frameDetached",
                "Runtime.executionContextCreated",
                "Runtime.executionContextDestroyed",
                "Runtime.executionContextsCleared",
                "Runtime.consoleAPICalled",
                "Runtime.exceptionThrown",
                "Network.requestWillBeSent",
                "Network.loadingFinished",
                "Network.loadingFailed",
                "Network.responseReceived",
            ] {
                let sender = sender.clone();
                let metadata = metadata.clone();
                let routing_lost = routing_lost.clone();
                let handler =
                    DevToolsProtocolEventReceivedEventHandler::create(Box::new(move |_, args| {
                        let event=(||->windows::core::Result<Option<(String,String)>> {
                        let Some(args) = args else { return Ok(None) };
                        let args2 =
                            args.cast::<ICoreWebView2DevToolsProtocolEventReceivedEventArgs2>()?;
                        let mut raw = PWSTR::null();
                        let result = unsafe { args2.SessionId(&mut raw) };
                        let session = super::event_string(raw, 256);
                        result?;
                        let mut raw = PWSTR::null();
                        let result = unsafe { args.ParameterObjectAsJson(&mut raw) };
                        let text = super::event_string(raw, 32 * 1024);
                        result?;
                        Ok(session.zip(text))
                        })();
                        // COM read/cast failure and null/oversized payloads all
                        // take the same lifecycle-loss path as a full queue.
                        enqueue(&sender,&metadata,&routing_lost,method,event.ok().flatten());
                        Ok(())
                    }));
                let name = HSTRING::from(method);
                let receiver =
                    unsafe { core.GetDevToolsProtocolEventReceiver(PCWSTR(name.as_ptr()))? };
                let mut token = 0;
                unsafe {
                    receiver.add_DevToolsProtocolEventReceived(&handler, &mut token)?;
                }
                registration.receivers.push((receiver, token));
            }
            REGISTRATIONS.with(|entries| entries.borrow_mut().insert(label, registration));
            Ok(())
        })();
        let _ = tx.send(
            result.map_err(|_| "Native browser diagnostics could not be installed.".to_owned()),
        );
    })
    .map_err(|error| error.to_string())?;
    rx.await.map_err(|error| error.to_string())??;
    enable_session(view, None).await
}

/// Called only by the root installer or the existing owned iframe attach
/// worker. Snapshot metadata seeds already-loaded frames; no target discovery.
pub(super) async fn enable_session(view: &tauri::Webview, session: Option<&str>) -> Result<(), String> {
    super::protocol_call_session(view, session, "Page.enable", serde_json::json!({})).await?;
    seed_session(view,session).await?;
    super::protocol_call_session(view, session, "Runtime.enable", serde_json::json!({})).await?;
    super::protocol_call_session(view, session, "Network.enable", serde_json::json!({"maxTotalBufferSize":0,"maxResourceBufferSize":0,"maxPostDataSize":0})).await?;
    Ok(())
}

async fn seed_session(view: &tauri::Webview, session: Option<&str>) -> Result<(), String> {
    let label = view.label().to_owned();
    let (tx, rx) = tokio::sync::oneshot::channel();
    view.app_handle().run_on_main_thread(move || {
        let sink = REGISTRATIONS.with(|entries| entries.borrow().get(&label).map(|entry| {
            let generation = entry.metadata.lock().unwrap_or_else(|error| error.into_inner()).target.document_generation;
            (entry.sender.clone(), generation, entry.metadata.clone(), entry.routing_lost.clone())
        }));
        let _ = tx.send(sink);
    }).map_err(|_| "Diagnostic seed dispatch failed.".to_owned())?;
    let sink = rx.await.map_err(|_| "Diagnostic seed owner unavailable.".to_owned())?;
    if let Some((sender, generation, metadata, lost)) = sink {
        let tree = super::protocol_call_session(view, session, "Page.getFrameTree", serde_json::json!({})).await?;
        let text = tree.to_string();
        if text.len() > 64 * 1024 || sender.try_send((generation, session.unwrap_or("").into(), "Nomi.frameTree", text)).is_err() {
            lost.store(true, Ordering::Release);
            let mut tab = metadata.lock().unwrap_or_else(|error| error.into_inner());
            tab.diagnostics.dropped = tab.diagnostics.dropped.saturating_add(1);
            tab.diagnostics.unavailable=true;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "diagnostics_tests.rs"]
mod tests;
