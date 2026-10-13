//! Real website getUserMedia requests; every case denies before sensor access.
use super::fixture::Fixture;
use nomifun_browser_platform::runtime::*;
use serde_json::{Value, json};
use std::{sync::Arc, time::{Duration, Instant}};
use tokio_util::sync::CancellationToken;

pub(crate) async fn verify(app: &tauri::AppHandle, runtime: &Arc<dyn BrowserRuntime>, fixture: &Fixture, user_ready: bool) -> Result<Value, String> {
    eprintln!("MACOS_BROWSER_COMPATIBILITY_SMOKE_STAGE media_permission_ownership");
    let application = application_metadata();
    eprintln!("MACOS_BROWSER_COMPATIBILITY_SMOKE_MEDIA_APPLICATION {application}");
    let mut native_checks = Vec::new();
    let mut system_budget = SystemWaitBudget::new();
    for case in ["explicit_deny", "input_owner", "hide", "navigation", "close"] {
        let snapshot = runtime.execute(BrowserTabCommand::Create { url: fixture.url(&format!("/media-permission?case={case}&user_ready={user_ready}")) }, CancellationToken::new()).await.map_err(error)?;
        let id = snapshot.active_tab_id.ok_or("Permission fixture has no active tab")?;
        emit_native_diagnostic(app, fixture, case, "before_start", &mut native_checks).await?;
        if user_ready {
            eprintln!("MACOS_BROWSER_COMPATIBILITY_SMOKE_MEDIA_USER_READY case={case} Click Start media permission on the visible native page within 60 seconds");
            wait_user_ready(fixture, case).await?;
            emit_native_diagnostic(app, fixture, case, "after_start", &mut native_checks).await?;
        }
        let ready = wait_system_prerequisite(app, runtime, &id, fixture, case, &mut system_budget, &mut native_checks).await;
        let result = match ready {
            Ok(Some(tab)) => Ok(tab), // An actual typed website request is stronger than a cached OS query.
            Ok(None) => wait_permission(runtime, &id, fixture, case, user_ready).await,
            Err(failure) => Err(failure),
        };
        let tab = match result {
            Ok(tab) => tab,
            Err(failure) => {
                if let Err(diagnostic) = emit_native_diagnostic(app, fixture, case, "request_failure", &mut native_checks).await {
                    return Err(format!("{failure}; native diagnostic failed: {diagnostic}"));
                }
                return Err(failure);
            }
        };
        let request = tab.permission_requests.first().ok_or("Missing pending website request")?.clone();
        if request.kind != "camera" || request.origin != format!("http://{}", fixture.address) {
            return Err("Website media permission did not preserve its exact kind and origin".into());
        }
        match case {
            "explicit_deny" => {
                let mut stale = tab.target.clone(); stale.document_generation += 1;
                if !matches!(runtime.execute(BrowserTabCommand::Permission { target: stale, request_id: request.request_id.clone(), allow: true }, CancellationToken::new()).await, Err(WorkspaceError::StaleTarget)) {
                    return Err("Stale media target was admitted".into());
                }
                runtime.execute(BrowserTabCommand::Permission { target: tab.target.clone(), request_id: request.request_id.clone(), allow: false }, CancellationToken::new()).await.map_err(error)?;
                if runtime.execute(BrowserTabCommand::Permission { target: tab.target.clone(), request_id: request.request_id, allow: true }, CancellationToken::new()).await.is_ok() {
                    return Err("Completed permission could be granted a second time".into());
                }
            }
            "input_owner" => { runtime.lock_user_input().await.map_err(error)?; }
            "hide" => { runtime.surface().ok_or("Missing native surface")?.set_surface(bounds(), false, CancellationToken::new()).await.map_err(error)?; }
            "navigation" => { runtime.execute(BrowserTabCommand::Navigate { target: tab.target.clone(), url: fixture.url("/redirect-final") }, CancellationToken::new()).await.map_err(error)?; }
            _ => { runtime.execute(BrowserTabCommand::Close { target: tab.target.clone() }, CancellationToken::new()).await.map_err(error)?; }
        }
        if case != "close" {
            let current = runtime.snapshot().await.map_err(error)?.tabs.into_iter().find(|tab| tab.target.tab_id == id).ok_or("Permission tab disappeared")?;
            if !current.permission_requests.is_empty() { return Err(format!("{case} retained an unresolved native permission")); }
            if case == "navigation" && !current.blocked_permissions.is_empty() { return Err("Old media denial marked the successor document".into()); }
        }
        if ["explicit_deny", "input_owner", "hide"].contains(&case) { wait_denied(fixture, case).await?; }
        if case == "input_owner" { runtime.release_pressed_input().await.map_err(error)?; runtime.unlock_user_input().await.map_err(error)?; }
        if case == "hide" { runtime.surface().ok_or("Missing native surface")?.set_surface(bounds(), true, CancellationToken::new()).await.map_err(error)?; }
        if case != "close" {
            runtime.execute(BrowserTabCommand::Close { target: runtime.snapshot().await.map_err(error)?.tabs.into_iter().find(|tab| tab.target.tab_id == id).ok_or("Permission tab disappeared")?.target }, CancellationToken::new()).await.map_err(error)?;
        }
    }
    Ok(json!({"scope":"real_website_permission_delegate", "ephemeral_profile":true,
        "exact_origin_kind_and_document":true, "stale_reply_rejected":true, "completion_consumed_once":true,
        "explicit_denial_callback":true, "owner_switch_denial_callback":true, "hide_denial_callback":true,
        "navigation_and_close_revoke_request":true, "grant_and_system_tcc":"not_covered", "actual_sensor_capture":false,"application":application,
        "user_ready_mode":user_ready,"trusted_visible_focused_start_gate":user_ready,
        "get_user_media_trigger":if user_ready {"trusted_visible_focused_click"} else {"automatic_page_timer"},"native_diagnostics":native_checks,
        "system_not_determined_wait_ms":system_budget.used().as_millis(),"system_wait_budget_ms":SYSTEM_WAIT_BUDGET.as_millis()}))
}

const SYSTEM_WAIT_BUDGET: Duration = Duration::from_secs(120);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AvAuthorization { NotDetermined, Restricted, Denied, Authorized, Unavailable }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SystemStep { HandleWebsiteRequest, StartWebsiteDeadline, Wait, Reject(&'static str) }
struct SystemWaitBudget { remaining: Duration }
impl SystemWaitBudget {
    fn new() -> Self { Self { remaining: SYSTEM_WAIT_BUDGET } }
    fn consume(&mut self, elapsed: Duration) { self.remaining = self.remaining.saturating_sub(elapsed); }
    fn used(&self) -> Duration { SYSTEM_WAIT_BUDGET - self.remaining }
    fn step(&self, state: AvAuthorization, actual_website_request: bool) -> SystemStep {
        if actual_website_request { return SystemStep::HandleWebsiteRequest; }
        match state {
            AvAuthorization::Authorized => SystemStep::StartWebsiteDeadline,
            AvAuthorization::NotDetermined if !self.remaining.is_zero() => SystemStep::Wait,
            AvAuthorization::NotDetermined => SystemStep::Reject("OS_CAMERA_NOT_DETERMINED_BUDGET_EXHAUSTED"),
            AvAuthorization::Denied => SystemStep::Reject("OS_CAMERA_DENIED"),
            AvAuthorization::Restricted => SystemStep::Reject("OS_CAMERA_RESTRICTED"),
            AvAuthorization::Unavailable => SystemStep::Reject("OS_CAMERA_AUTHORIZATION_UNAVAILABLE"),
        }
    }
}
struct NativeDiagnostic { evidence: Value, camera: AvAuthorization }

fn has_camera_request(tab: &BrowserTabSnapshot, fixture: &Fixture) -> bool {
    let origin = format!("http://{}", fixture.address);
    tab.permission_requests.iter().any(|request|request.kind == "camera" && request.origin == origin)
}
async fn current_tab(runtime: &Arc<dyn BrowserRuntime>, id: &str) -> Result<BrowserTabSnapshot, String> {
    runtime.snapshot().await.map_err(error)?.tabs.into_iter().find(|tab|tab.target.tab_id == id).ok_or_else(|| "Permission tab disappeared".into())
}
async fn wait_system_prerequisite(app: &tauri::AppHandle, runtime: &Arc<dyn BrowserRuntime>, id: &str, fixture: &Fixture, case: &str,
    budget: &mut SystemWaitBudget, collected: &mut Vec<Value>) -> Result<Option<BrowserTabSnapshot>, String> {
    let mut last_state = None;
    loop {
        let current = current_tab(runtime, id).await?;
        if has_camera_request(&current, fixture) {
            // Do not let a stale/unavailable OS query prevent handling a real
            // request whose production completion still has its 30s lifetime.
            match native_diagnostic(app, fixture, case).await {
                Ok(mut native) => {
                    native.evidence["actual_website_request_present"] = json!(true);
                    native.evidence["os_query_matches_website_prerequisite"] = json!(native.camera == AvAuthorization::Authorized);
                    record_native_diagnostic(native.evidence, case, "website_request_already_present", collected);
                }
                Err(_) => record_native_diagnostic(json!({"camera_authorization_query_available":false,"actual_website_request_present":true}), case, "website_request_already_present", collected),
            }
            return Ok(Some(current));
        }
        let inspection = Instant::now();
        // Exhaustion stops NotDetermined waiting, not a final read-only state
        // check: an Authorized result can still admit the website deadline.
        let query_timeout = if budget.remaining.is_zero() { Duration::from_secs(3) } else { budget.remaining.min(Duration::from_secs(3)) };
        let native = tokio::time::timeout(query_timeout, native_diagnostic(app, fixture, case)).await;
        let native = match native {
            Ok(Ok(native)) => native,
            Ok(Err(failure)) => {
                let current = current_tab(runtime, id).await?;
                if has_camera_request(&current, fixture) {
                    record_native_diagnostic(json!({"camera_authorization_query_available":false,"actual_website_request_present":true}), case, "website_request_present_after_query_failure", collected);
                    return Ok(Some(current));
                }
                return Err(format!("OS camera prerequisite unavailable; website 8s deadline never started; diagnostic={failure}"));
            }
            Err(_) => {
                let current = current_tab(runtime, id).await?;
                if has_camera_request(&current, fixture) {
                    record_native_diagnostic(json!({"camera_authorization_query_available":false,"actual_website_request_present":true}), case, "website_request_present_after_query_timeout", collected);
                    return Ok(Some(current));
                }
                budget.consume(inspection.elapsed());
                return Err("OS camera prerequisite query exceeded its available system budget; website 8s deadline never started".into());
            }
        };
        let current = current_tab(runtime, id).await?;
        let actual_request = has_camera_request(&current, fixture);
        let step = budget.step(native.camera, actual_request);
        if last_state != Some(native.camera) || !matches!(step, SystemStep::Wait) {
            record_native_diagnostic(native.evidence, case, "system_prerequisite", collected);
        }
        last_state = Some(native.camera);
        match step {
            SystemStep::HandleWebsiteRequest => return Ok(Some(current)),
            SystemStep::StartWebsiteDeadline => return Ok(None),
            SystemStep::Reject(reason) => return Err(format!("OS camera prerequisite unmet: {reason}; website delegate lifecycle not verified; safe HTTP evidence={}", media_evidence(fixture, case))),
            SystemStep::Wait => {
                budget.consume(inspection.elapsed());
                if budget.remaining.is_zero() { continue; }
                let sleeping = Instant::now();
                tokio::time::sleep(budget.remaining.min(Duration::from_millis(250))).await;
                budget.consume(sleeping.elapsed());
            }
        }
    }
}

// Public AVFoundation constants and enum from AVMediaFormat/AVCaptureDevice.h.
// These queries never call requestAccessForMediaType or create capture inputs.
#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {
    static AVMediaTypeVideo: *const objc2_foundation::NSString;
    static AVMediaTypeAudio: *const objc2_foundation::NSString;
}
fn av_authorization(media_type: *const objc2_foundation::NSString) -> (AvAuthorization, Value) {
    let Some(class) = objc2::runtime::AnyClass::get(c"AVCaptureDevice") else { return (AvAuthorization::Unavailable, json!({"available":false})); };
    let Some(media_type) = (unsafe { media_type.as_ref() }) else { return (AvAuthorization::Unavailable, json!({"available":false})); };
    let responds: bool = unsafe { objc2::msg_send![class, respondsToSelector: objc2::sel!(authorizationStatusForMediaType:)] };
    if !responds { return (AvAuthorization::Unavailable, json!({"available":false})); }
    let value: isize = unsafe { objc2::msg_send![class, authorizationStatusForMediaType: media_type] };
    let (state, public_enum) = match value {
        0 => (AvAuthorization::NotDetermined, "AVAuthorizationStatusNotDetermined"),
        1 => (AvAuthorization::Restricted, "AVAuthorizationStatusRestricted"),
        2 => (AvAuthorization::Denied, "AVAuthorizationStatusDenied"),
        3 => (AvAuthorization::Authorized, "AVAuthorizationStatusAuthorized"),
        _ => (AvAuthorization::Unavailable, "UnknownAVAuthorizationStatus"),
    };
    (state, json!({"available":true,"public_enum":public_enum,"value":value}))
}
async fn emit_native_diagnostic(app: &tauri::AppHandle, fixture: &Fixture, case: &str, phase: &str, collected: &mut Vec<Value>) -> Result<(), String> {
    let native = native_diagnostic(app, fixture, case).await?;
    record_native_diagnostic(native.evidence, case, phase, collected);
    Ok(())
}
fn record_native_diagnostic(mut evidence: Value, case: &str, phase: &str, collected: &mut Vec<Value>) {
    evidence["case"] = json!(case);
    evidence["phase"] = json!(phase);
    eprintln!("MACOS_BROWSER_COMPATIBILITY_SMOKE_MEDIA_NATIVE {evidence}");
    collected.push(evidence);
}
async fn native_diagnostic(app: &tauri::AppHandle, fixture: &Fixture, case: &str) -> Result<NativeDiagnostic, String> {
    use objc2::{MainThreadMarker, msg_send, rc::Retained, runtime::{AnyClass, AnyObject}};
    use objc2_app_kit::{NSApplication, NSView};
    use objc2_foundation::NSURL;
    use tauri::Manager;
    let handle = app.clone();
    let address = fixture.address;
    let expected_case = case.to_owned();
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let result = (|| -> Result<NativeDiagnostic, String> {
            let mtm = MainThreadMarker::new().ok_or("Native diagnostic is not on AppKit main thread")?;
            let main = handle.get_window("main").ok_or("Own fixture main window is absent")?;
            let raw = main.ns_window().map_err(|_| "Own fixture NSWindow unavailable")?;
            let window = unsafe { raw.cast::<objc2_app_kit::NSWindow>().as_ref() }.ok_or("Own fixture NSWindow is null")?;
            let root = window.contentView().ok_or("Own fixture content view is absent")?;
            let wk = AnyClass::get(c"WKWebView").ok_or("System WKWebView public class unavailable")?;
            // Walk only this fixture's owned content subtree. Never enumerate
            // other apps or windows, and never record URLs/DOM/private selectors.
            let mut stack: Vec<(Retained<NSView>, usize)> = vec![(root, 0)];
            let mut visited = 0;
            let mut candidates = Vec::new();
            while let Some((view, depth)) = stack.pop() {
                visited += 1;
                if visited > 512 || depth > 32 { return Err("Own fixture native subtree exceeded its diagnostic budget".into()); }
                let is_wk: bool = unsafe { msg_send![&*view, isKindOfClass: wk] };
                if is_wk && !view.isHiddenOrHasHiddenAncestor() {
                    let native_url: Option<Retained<NSURL>> = unsafe { msg_send![&*view, URL] };
                    let matches = native_url.and_then(|url|url.absoluteString()).and_then(|url|url::Url::parse(&url.to_string()).ok())
                        .is_some_and(|url|url.scheme() == "http" && url.host_str() == Some("127.0.0.1") && url.port() == Some(address.port())
                            && url.path() == "/media-permission" && url.query_pairs().any(|(key,value)|key == "case" && value == expected_case));
                    if matches {
                        let delegate: Option<Retained<AnyObject>> = unsafe { msg_send![&*view, UIDelegate] };
                        let class = delegate.as_ref().map(|delegate|delegate.class().name().to_string_lossy().into_owned());
                        let responds = delegate.as_ref().is_some_and(|delegate| unsafe {
                            msg_send![&**delegate, respondsToSelector: objc2::sel!(webView:requestMediaCapturePermissionForOrigin:initiatedByFrame:type:decisionHandler:)]
                        });
                        candidates.push(json!({"ui_delegate_present":delegate.is_some(),"ui_delegate_class":class,
                            "responds_to_public_media_selector":responds}));
                    }
                }
                for child in view.subviews().iter() { stack.push((child, depth + 1)); }
            }
            let (camera, camera_evidence) = av_authorization(unsafe { AVMediaTypeVideo });
            let (_, microphone_evidence) = av_authorization(unsafe { AVMediaTypeAudio });
            Ok(NativeDiagnostic { camera, evidence: json!({"scope":"own_main_window_media_view_public_introspection","ns_application_active":NSApplication::sharedApplication(mtm).isActive(),
                "main_window_key":window.isKeyWindow(),"visible_media_views":candidates,
                "camera_authorization":camera_evidence,"microphone_authorization":microphone_evidence}) })
        })();
        let _ = tx.send(result);
    }).map_err(|_| "Own fixture native diagnostic dispatch failed")?;
    tokio::time::timeout(Duration::from_secs(3), rx).await.map_err(|_| "Own fixture native diagnostic main-thread timeout")?
        .map_err(|_| "Own fixture native diagnostic response lost")?
}
fn application_metadata() -> Value {
    use objc2_foundation::{NSBundle, NSString};
    let bundle = NSBundle::mainBundle();
    let has_usage = |key| bundle.objectForInfoDictionaryKey(&NSString::from_str(key))
        .and_then(|value| value.downcast::<NSString>().ok()).is_some_and(|value| !value.to_string().trim().is_empty());
    json!({"main_bundle_is_app":bundle.bundlePath().to_string().ends_with(".app"),
        "bundle_identifier":bundle.bundleIdentifier().map(|value|value.to_string()),
        "camera_usage_description_present":has_usage("NSCameraUsageDescription"),
        "microphone_usage_description_present":has_usage("NSMicrophoneUsageDescription")})
}
fn media_evidence(fixture: &Fixture, case: &str) -> Value {
    json!(fixture.snapshot().into_iter().filter(|receipt| receipt.path.starts_with("/media-") && receipt.case.as_deref() == Some(case))
        .map(|receipt| json!({"path":receipt.path,"probe":receipt.media_probe})).collect::<Vec<_>>())
}
fn bounds() -> BrowserSurfaceBounds { BrowserSurfaceBounds { x:20.0,y:40.0,width:840.0,height:540.0 } }
fn error(error: impl std::fmt::Display) -> String { error.to_string() }
async fn wait_user_ready(fixture: &Fixture, case: &str) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if fixture.snapshot().iter().any(|receipt| receipt.path == "/media-user-start" && receipt.case.as_deref() == Some(case)
            && receipt.media_probe.as_ref().is_some_and(|probe|probe["trusted"] == true && probe["visibility_state"] == "visible" && probe["document_has_focus"] == true)) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            let receipts = fixture.snapshot();
            let last = receipts.iter().rev().find(|receipt|receipt.path.starts_with("/media-") && receipt.case.as_deref() == Some(case) && receipt.media_probe.as_ref().is_some_and(|probe|probe["visibility_state"].is_string()));
            let reason = match last.and_then(|receipt|receipt.media_probe.as_ref()) {
                None => "NO_MEDIA_DOCUMENT_PROBE",
                Some(probe) if probe["visibility_state"] == "hidden" => "DOCUMENT_HIDDEN_WITHOUT_ACCEPTED_START",
                Some(probe) if probe["document_has_focus"] == false => "DOCUMENT_UNFOCUSED_WITHOUT_ACCEPTED_START",
                Some(probe) if probe["trusted"] == false => "UNTRUSTED_START_CLICK",
                Some(_) => "NO_ACCEPTED_START_CLICK",
            };
            return Err(format!("Media user-ready prerequisite timed out after 60 seconds before an accepted Start HTTP witness; reason={reason}; request initiation through this gate remains unproven; safe HTTP evidence={}", media_evidence(fixture, case)));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
async fn wait_permission(runtime: &Arc<dyn BrowserRuntime>, id: &str, fixture: &Fixture, case: &str, user_ready: bool) -> Result<BrowserTabSnapshot, String> {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let current = runtime.snapshot().await.map_err(error)?.tabs.into_iter().find(|tab| tab.target.tab_id == id).ok_or("Permission tab disappeared")?;
        if !current.permission_requests.is_empty() { return Ok(current); }
        let evidence = media_evidence(fixture, case);
        let gate = if user_ready { "trusted visible focused Start accepted" } else { "automatic page timer" };
        if fixture.snapshot().iter().any(|receipt| receipt.path == "/media-denied" && receipt.case.as_deref() == Some(case)) {
            return Err(format!("Website getUserMedia rejected before a pending native request; trigger={gate}; safe HTTP evidence={evidence}"));
        }
        if !current.blocked_permissions.is_empty() { return Err(format!("Native media request was denied before becoming user-owned: {:?}; safe HTTP evidence={evidence}", current.blocked_permissions)); }
        if Instant::now() >= deadline { return Err(format!("Real website media permission delegate did not produce a request within 8 seconds after {gate}; no root cause asserted; safe HTTP evidence={evidence}")); }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
async fn wait_denied(fixture: &Fixture, case: &str) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if fixture.snapshot().iter().any(|receipt| receipt.path == "/media-denied" && receipt.case.as_deref() == Some(case)) { return Ok(()); }
        if Instant::now() >= deadline { return Err(format!("{case} lost the website's denied completion HTTP witness")); }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[cfg(test)]
mod system_wait_budget_tests {
    use super::*;
    #[test]
    fn os_wait_longer_than_eight_seconds_does_not_start_website_deadline() {
        let mut budget = SystemWaitBudget::new();
        budget.consume(Duration::from_secs(20));
        assert_eq!(budget.step(AvAuthorization::NotDetermined, false), SystemStep::Wait);
        assert_eq!(budget.remaining, Duration::from_secs(100));
        assert_eq!(budget.step(AvAuthorization::Authorized, false), SystemStep::StartWebsiteDeadline);
    }
    #[test]
    fn denied_restricted_and_unavailable_fail_immediately_without_consuming_budget() {
        let budget = SystemWaitBudget::new();
        assert_eq!(budget.step(AvAuthorization::Denied, false), SystemStep::Reject("OS_CAMERA_DENIED"));
        assert_eq!(budget.step(AvAuthorization::Restricted, false), SystemStep::Reject("OS_CAMERA_RESTRICTED"));
        assert_eq!(budget.step(AvAuthorization::Unavailable, false), SystemStep::Reject("OS_CAMERA_AUTHORIZATION_UNAVAILABLE"));
        assert_eq!(budget.remaining, SYSTEM_WAIT_BUDGET);
    }
    #[test]
    fn system_wait_budget_is_shared_across_cases_and_saturates() {
        let mut budget = SystemWaitBudget::new();
        budget.consume(Duration::from_secs(70));
        assert_eq!(budget.step(AvAuthorization::NotDetermined, false), SystemStep::Wait);
        budget.consume(Duration::from_secs(51));
        assert_eq!(budget.used(), SYSTEM_WAIT_BUDGET);
        assert_eq!(budget.step(AvAuthorization::NotDetermined, false), SystemStep::Reject("OS_CAMERA_NOT_DETERMINED_BUDGET_EXHAUSTED"));
        assert_eq!(budget.step(AvAuthorization::Authorized, false), SystemStep::StartWebsiteDeadline);
    }
    #[test]
    fn actual_website_request_outweighs_cached_os_status_and_exhausted_budget() {
        let mut budget = SystemWaitBudget::new();
        budget.consume(SYSTEM_WAIT_BUDGET);
        for state in [AvAuthorization::NotDetermined, AvAuthorization::Denied, AvAuthorization::Restricted, AvAuthorization::Unavailable] {
            assert_eq!(budget.step(state, true), SystemStep::HandleWebsiteRequest);
        }
    }
}
