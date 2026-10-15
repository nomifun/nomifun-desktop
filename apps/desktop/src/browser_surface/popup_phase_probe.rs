//! Temporary, explicitly opted-in observations of one existing WebView2 page.
//! No domains are enabled, no page code is executed, and no browser setting is
//! changed. Samples are never assigned to a native navigation or load attempt.
//! A response sample proves headers only; body completion remains unknown until
//! an exactly routed loadingFinished sample. Byte counters never include data.

use nomifun_browser_platform::url_projection::project_metadata_url;
use serde::Deserialize;
use std::{cell::{Cell, RefCell}, collections::BTreeMap, rc::Rc, sync::{Arc, atomic::{AtomicBool, Ordering}}, time::{Duration, Instant}};
use webview2_com::{
    CallDevToolsProtocolMethodCompletedHandler, DevToolsProtocolEventReceivedEventHandler,
    Microsoft::Web::WebView2::Win32::{
        ICoreWebView2, ICoreWebView2DevToolsProtocolEventReceivedEventArgs2,
        ICoreWebView2DevToolsProtocolEventReceiver,
    },
};
use windows::core::{HSTRING, Interface, PCWSTR, PWSTR};

const MAX_SAMPLES: u64 = 256;
const MAX_PAYLOAD: usize = 64 * 1024;
const MAX_DOCUMENTS: usize = 16;

pub(super) struct Samples {
    view: String,
    started: Instant,
    count: Cell<u64>,
    binding_phase: Cell<&'static str>,
    documents: RefCell<DocumentRequests>,
}
impl Samples {
    fn next(&self) -> Option<(u64, u64)> {
        let next = self.count.get().saturating_add(1);
        self.count.set(next);
        if next == MAX_SAMPLES + 1 {
            tracing::info!(target: "browser_popup_phase_probe", view = %self.view,
                "debug native phase probe sample limit reached");
        }
        (next <= MAX_SAMPLES).then(|| (next, self.started.elapsed().as_millis() as u64))
    }
}

pub(super) struct Probe {
    samples: Rc<Samples>,
    receivers: Vec<(ICoreWebView2DevToolsProtocolEventReceiver, i64)>,
}
impl Drop for Probe {
    fn drop(&mut self) {
        self.samples.documents.borrow_mut().0.clear();
        for (receiver, token) in &self.receivers {
            let _ = unsafe { receiver.remove_DevToolsProtocolEventReceived(*token) };
        }
    }
}

// Deserialize only the allowed fields. Headers, postData, errorText and other
// unselected protocol data are discarded rather than retained or logged.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DocumentEvent {
    #[serde(rename = "type")]
    resource_type: Option<String>,
    request_id: Option<String>,
    frame_id: Option<String>,
    loader_id: Option<String>,
    request: Option<Request>,
    response: Option<Response>,
    canceled: Option<bool>,
    data_length: Option<f64>,
    encoded_data_length: Option<f64>,
}
#[derive(Deserialize)]
struct Request { url: String }
#[derive(Deserialize)]
struct Response { url: String, status: f64 }

fn identifier(value: Option<&str>) -> &str {
    value.filter(|value| !value.is_empty() && value.len() <= 256
        && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"-_.:".contains(&byte)))
        .unwrap_or("unavailable")
}
fn safe_url(value: Option<&str>) -> String {
    match value {
        Some("about:blank") => "about:blank".into(),
        Some(value) => project_metadata_url(value),
        None => "unavailable".into(),
    }
}
fn document_event(text: &str) -> Result<DocumentEvent, serde_json::Error> {
    serde_json::from_str(text)
}
fn http_status(value: Option<f64>) -> Option<u16> {
    value.filter(|status| status.is_finite() && status.fract() == 0.0
        && (100.0..600.0).contains(status)).map(|status| status as u16)
}
fn byte_count(value: Option<f64>) -> Option<u64> {
    // A finite, integral counter only. This probe never asks for response data.
    value.filter(|value| value.is_finite() && value.fract() == 0.0
        && (0.0..=(1_u64 << 40) as f64).contains(value)).map(|value| value as u64)
}

struct DocumentRequest { started: Instant, first_data: bool }
#[derive(Default)]
struct DocumentRequests(BTreeMap<String, DocumentRequest>);
struct DocumentSample { phase: &'static str, duration_ms: Option<u64>, tracking_unavailable: bool }
impl DocumentRequests {
    fn observe(&mut self, method: &str, event: &DocumentEvent, now: Instant) -> Option<DocumentSample> {
        let id = event.request_id.as_deref().filter(|id| identifier(Some(id)) != "unavailable");
        let declared_document = event.resource_type.as_deref() == Some("Document");
        let mut unavailable = false;
        let (phase, started) = match method {
            "Network.requestWillBeSent" if declared_document => {
                if let Some(id) = id {
                    if self.0.len() < MAX_DOCUMENTS || self.0.contains_key(id) {
                        self.0.entry(id.into()).or_insert(DocumentRequest { started: now, first_data: false });
                    } else { unavailable = true; }
                } else { unavailable = true; }
                ("request", None)
            },
            "Network.responseReceived" if declared_document => ("response_headers", None),
            "Network.dataReceived" => {
                let request = self.0.get_mut(id?)?;
                if request.first_data { return None; }
                request.first_data = true;
                ("first_body_counter", Some(request.started))
            },
            "Network.loadingFinished" => ("body_transfer_complete", Some(self.0.remove(id?)?.started)),
            "Network.loadingFailed" => {
                let request = id.and_then(|id| self.0.remove(id));
                if !declared_document && request.is_none() { return None; }
                ("failed", request.map(|request| request.started))
            },
            _ => return None,
        };
        Some(DocumentSample { phase, duration_ms: started.map(|started| now.saturating_duration_since(started).as_millis() as u64), tracking_unavailable: unavailable })
    }
}

impl Probe {
    pub(super) fn install(core: &ICoreWebView2, label: &str) -> Option<Self> {
        let enabled = std::env::var_os("NOMIFUN_BROWSER_POPUP_PHASE_PROBE")
            .map_or(option_env!("NOMIFUN_BROWSER_POPUP_PHASE_PROBE") == Some("1"), |value| value == "1");
        if !enabled { return None; }
        let samples = Rc::new(Samples { view: label.into(), started: Instant::now(), count: Cell::new(0),
            binding_phase: Cell::new(if label.starts_with("browser-popup-") { "before_binding" } else { "ordinary_page" }),
            documents: RefCell::new(DocumentRequests::default()) });
        let mut probe = Self { samples, receivers: Vec::new() };
        for method in ["Network.requestWillBeSent", "Network.responseReceived", "Network.loadingFailed", "Network.dataReceived", "Network.loadingFinished"] {
            let samples = probe.samples.clone();
            let handler = DevToolsProtocolEventReceivedEventHandler::create(Box::new(move |_, args| {
                let event = (|| -> Result<Option<DocumentEvent>, ()> {
                    let args = args.ok_or(())?;
                    let args2 = args.cast::<ICoreWebView2DevToolsProtocolEventReceivedEventArgs2>().map_err(|_| ())?;
                    let mut raw = PWSTR::null();
                    let result = unsafe { args2.SessionId(&mut raw) };
                    let session = super::super::event_string(raw, 256);
                    result.map_err(|_| ())?;
                    if !session.ok_or(())?.is_empty() { return Ok(None); }
                    let mut raw = PWSTR::null();
                    let result = unsafe { args.ParameterObjectAsJson(&mut raw) };
                    let text = super::super::event_string(raw, MAX_PAYLOAD);
                    result.map_err(|_| ())?;
                    document_event(&text.ok_or(())?).map(Some).map_err(|_| ())
                })();
                if let Ok(Some(event)) = event {
                    let observed = samples.documents.borrow_mut().observe(method, &event, Instant::now());
                    let Some(observed) = observed else { return Ok(()) };
                    if let Some((sample, elapsed_ms)) = samples.next() {
                        let url = safe_url(event.response.as_ref().map(|response| response.url.as_str())
                            .or_else(|| event.request.as_ref().map(|request| request.url.as_str())));
                        let status = http_status(event.response.as_ref().map(|response| response.status));
                        tracing::info!(target: "browser_popup_phase_probe", view = %samples.view,
                            sample, elapsed_ms, method, binding_phase = samples.binding_phase.get(), resource_type = "Document", url = %url,
                            phase = observed.phase, duration_ms = observed.duration_ms,
                            tracking_unavailable = observed.tracking_unavailable,
                            data_length = byte_count(event.data_length), encoded_data_length = byte_count(event.encoded_data_length),
                            request_id = identifier(event.request_id.as_deref()),
                            frame_id = identifier(event.frame_id.as_deref()),
                            loader_id = identifier(event.loader_id.as_deref()),
                            status, canceled = event.canceled,
                            "debug native document network sample; headers do not prove body completion; navigation association unknown");
                    }
                } else if event.is_err() {
                    if let Some((sample, elapsed_ms)) = samples.next() {
                        tracing::info!(target: "browser_popup_phase_probe", view = %samples.view,
                            sample, elapsed_ms, method, binding_phase = samples.binding_phase.get(),
                            "debug native document event unavailable; no absence inference");
                    }
                }
                Ok(())
            }));
            let name = HSTRING::from(method);
            let installed = (|| -> windows::core::Result<_> {
                let receiver = unsafe { core.GetDevToolsProtocolEventReceiver(PCWSTR(name.as_ptr()))? };
                let mut token = 0;
                unsafe { receiver.add_DevToolsProtocolEventReceived(&handler, &mut token)?; }
                Ok((receiver, token))
            })();
            match installed {
                Ok(receiver) => probe.receivers.push(receiver),
                Err(_) => {
                    tracing::info!(target: "browser_popup_phase_probe", view = label,
                        method, "debug native phase probe receiver unavailable");
                    return None;
                }
            }
        }
        tracing::info!(target: "browser_popup_phase_probe", view = label,
            "debug native phase probe installed; existing domains only");
        Some(probe)
    }

    pub(super) fn samples(&self) -> Rc<Samples> { self.samples.clone() }
}

pub(super) fn snapshot(samples: &Rc<Samples>, core: &ICoreWebView2, issued_phase: &'static str) {
        samples.binding_phase.set(if issued_phase == "before_binding" { "binding_in_progress" } else { "after_binding" });
        // Submit without awaiting: observation must not hold the popup deferral.
        // A reply may cross binding, so phase identifies submission only.
        for method in ["Target.getTargetInfo", "Page.getFrameTree"] {
            let reply_samples = samples.clone();
            let settled = Arc::new(AtomicBool::new(false));
            let reply_settled = settled.clone();
            let callback = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |status, response| {
                if reply_settled.swap(true, Ordering::AcqRel) { return Ok(()); }
                let Some((sample, elapsed_ms)) = reply_samples.next() else { return Ok(()) };
                let value = (status.is_ok() && response.len() <= MAX_PAYLOAD)
                    .then(|| serde_json::from_str::<RootSnapshot>(&response).ok()).flatten();
                let target = value.as_ref().and_then(|value| value.target_info.as_ref());
                let frame = value.as_ref().and_then(|value| value.frame_tree.as_ref()).map(|tree| &tree.frame);
                let target_type = match target.and_then(|target| target.target_type.as_deref()) {
                    Some("page") => "page", Some("iframe") => "iframe", Some("worker") => "worker", _ => "unavailable",
                };
                tracing::info!(target: "browser_popup_phase_probe", view = %reply_samples.view,
                    sample, elapsed_ms, method, issued_phase, reply_may_cross_binding = true,
                    available = target.is_some() || frame.is_some(), target_type,
                    target_id = identifier(target.and_then(|target| target.target_id.as_deref())),
                    frame_id = identifier(frame.and_then(|frame| frame.id.as_deref())),
                    parent_frame_id = identifier(frame.and_then(|frame| frame.parent_id.as_deref())),
                    loader_id = identifier(frame.and_then(|frame| frame.loader_id.as_deref())),
                    url = %safe_url(target.and_then(|target| target.url.as_deref()).or_else(|| frame.and_then(|frame| frame.url.as_deref()))),
                    "debug native root query sample; navigation association unknown");
                Ok(())
            }));
            let name = HSTRING::from(method);
            let params = HSTRING::from("{}");
            if unsafe { core.CallDevToolsProtocolMethod(PCWSTR(name.as_ptr()), PCWSTR(params.as_ptr()), &callback) }.is_err() {
                settled.store(true, Ordering::Release);
                tracing::info!(target: "browser_popup_phase_probe", view = %samples.view,
                    method, issued_phase, "debug native root query unavailable");
            } else {
                let view = samples.view.clone();
                // Expire the diagnostic reply only; never cancel a browser/CDP
                // command or delay the native WindowProxy transaction.
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    if !settled.swap(true, Ordering::AcqRel) {
                        tracing::info!(target: "browser_popup_phase_probe", view = %view,
                            method, issued_phase, "debug native root query unavailable after reply deadline");
                    }
                });
            }
        }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RootSnapshot { target_info: Option<TargetInfo>, frame_tree: Option<FrameTree> }
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TargetInfo {
    target_id: Option<String>,
    #[serde(rename = "type")]
    target_type: Option<String>,
    url: Option<String>,
}
#[derive(Deserialize)]
struct FrameTree { frame: RootFrame }
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RootFrame { id: Option<String>, parent_id: Option<String>, loader_id: Option<String>, url: Option<String> }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_probe_urls_never_retain_credentials_query_or_fragment() {
        assert_eq!(safe_url(Some("https://user:password@search.bilibili.com/all?keyword=private#secret")),
            "https://search.bilibili.com/all");
        assert_eq!(safe_url(Some("invalid?cookie=secret")), "[REDACTED_URL]");
        assert_eq!(safe_url(Some("data:text/plain,secret")), "[REDACTED_URL]");
        assert_eq!(safe_url(Some("about:blank")), "about:blank");
    }

    #[test]
    fn native_probe_accepts_document_samples_without_retaining_unselected_fields() {
        let event = document_event(r#"{"type":"Document","requestId":"1.2","request":{"url":"https://example.test/path?secret=1","headers":{"Cookie":"secret"},"postData":"secret"},"errorText":"untrusted secret"}"#).unwrap();
        assert_eq!(identifier(event.request_id.as_deref()), "1.2");
        assert_eq!(safe_url(event.request.as_ref().map(|request| request.url.as_str())), "https://example.test/path");
        let mut documents = DocumentRequests::default();
        assert!(documents.observe("Network.requestWillBeSent", &document_event(r#"{"type":"XHR","requestId":"1.2"}"#).unwrap(), Instant::now()).is_none());
        assert!(documents.observe("Network.loadingFailed", &document_event(r#"{"errorText":"untrusted secret"}"#).unwrap(), Instant::now()).is_none());
        assert!(document_event("invalid").is_err());
    }

    #[test]
    fn native_probe_ids_and_http_status_are_bounded_native_values() {
        assert_eq!(identifier(Some("ABCD-12_3.4:5")), "ABCD-12_3.4:5");
        assert_eq!(identifier(Some("https://example.test/?secret=1")), "unavailable");
        assert_eq!(identifier(Some(&"a".repeat(257))), "unavailable");
        assert_eq!(http_status(Some(200.0)), Some(200));
        for status in [f64::NAN, f64::INFINITY, 99.0, 600.0, 200.5] { assert_eq!(http_status(Some(status)), None); }
    }

    #[test]
    fn native_probe_root_snapshot_discards_title_children_and_errors() {
        let value: RootSnapshot = serde_json::from_str(r#"{"targetInfo":{"targetId":"A1","type":"page","url":"https://example.test/?secret=1","title":"secret"},"frameTree":{"frame":{"id":"F1","loaderId":"L1","url":"about:blank"},"childFrames":[{"secret":"discard"}]},"error":{"message":"untrusted secret"}}"#).unwrap();
        assert_eq!(identifier(value.target_info.as_ref().and_then(|target| target.target_id.as_deref())), "A1");
        assert_eq!(safe_url(value.target_info.as_ref().and_then(|target| target.url.as_deref())), "https://example.test/");
        assert_eq!(identifier(value.frame_tree.as_ref().and_then(|tree| tree.frame.id.as_deref())), "F1");
    }

    #[test]
    fn native_probe_body_counters_route_exact_document_ids_once_and_retire() {
        let mut documents = DocumentRequests::default();
        let now = Instant::now();
        let request = document_event(r#"{"type":"Document","requestId":"doc"}"#).unwrap();
        documents.observe("Network.requestWillBeSent", &request, now).unwrap();
        let data = document_event(r#"{"requestId":"doc","dataLength":12,"encodedDataLength":8}"#).unwrap();
        let first = documents.observe("Network.dataReceived", &data, now + Duration::from_millis(10)).unwrap();
        assert_eq!(first.phase, "first_body_counter");
        assert_eq!(first.duration_ms, Some(10));
        assert!(documents.observe("Network.dataReceived", &data, now).is_none());
        let finished = documents.observe("Network.loadingFinished", &data, now + Duration::from_millis(20)).unwrap();
        assert_eq!(finished.phase, "body_transfer_complete");
        assert_eq!(finished.duration_ms, Some(20));
        assert!(documents.0.is_empty());
        assert!(documents.observe("Network.loadingFinished", &data, now).is_none());
        documents.observe("Network.requestWillBeSent", &request, now).unwrap();
        assert_eq!(documents.observe("Network.loadingFailed", &data, now).unwrap().phase, "failed");
        assert!(documents.0.is_empty());
    }

    #[test]
    fn native_probe_document_inventory_and_byte_counters_are_bounded() {
        let mut documents = DocumentRequests::default();
        for index in 0..MAX_DOCUMENTS + 1 {
            let event = document_event(&format!(r#"{{"type":"Document","requestId":"doc{index}"}}"#)).unwrap();
            let observed = documents.observe("Network.requestWillBeSent", &event, Instant::now()).unwrap();
            assert_eq!(observed.tracking_unavailable, index == MAX_DOCUMENTS);
        }
        assert_eq!(documents.0.len(), MAX_DOCUMENTS);
        assert_eq!(byte_count(Some(0.0)), Some(0));
        assert_eq!(byte_count(Some(512.0)), Some(512));
        for value in [f64::NAN, f64::INFINITY, -1.0, 0.5, (1_u64 << 41) as f64] { assert_eq!(byte_count(Some(value)), None); }
    }
}
