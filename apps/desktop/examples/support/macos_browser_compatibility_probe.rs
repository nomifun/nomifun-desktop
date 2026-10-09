//! Exercise the real desktop host through its typed ports, never arbitrary JS.
use super::{fixture::Fixture, macos::host::DesktopBrowserHost};
use nomifun_browser_platform::runtime::*;
use objc2::msg_send;
use serde_json::{Value, json};
use std::{sync::Arc, time::{Duration, Instant}};
use tauri::Manager;
use tokio_util::sync::CancellationToken;

fn error(value: impl std::fmt::Display) -> String { value.to_string() }

pub(crate) async fn verify(app: &tauri::AppHandle, fixture: &Fixture, hold_ui: bool, live_sites: bool) -> Result<Value, String> {
    let factory = DesktopBrowserHost::new(app.clone());
    let created = tokio::time::timeout(Duration::from_secs(10), factory.create(CreateBrowserRuntime {
        key: BrowserResourceKey { principal_id: "compatibility-fixture".into(),
            agent_session_id: "disposable-session".into(), resource_binding_id: "native-compatibility".into() },
        runtime_generation: 1, profile: BrowserProfile::Ephemeral, user_input_enabled: true,
    })).await;
    let runtime = match created {
        Ok(Ok(runtime)) => runtime,
        result => {
            let shutdown = factory.shutdown().await.map_err(error);
            let failure = match result { Ok(Err(value)) => error(value), _ => "Native fixture creation timed out".into() };
            return match shutdown { Ok(()) => Err(failure), Err(cleanup) => Err(format!("{failure}; cleanup: {cleanup}")) };
        }
    };
    let result = tokio::time::timeout(Duration::from_secs(if hold_ui && live_sites { 210 } else { 120 }), async {
        let mut report = scenarios(app, fixture, &runtime, hold_ui).await?;
        report["core_checks_passed"] = json!(true);
        if live_sites {
            report["live_sites"] = match super::live::verify(&runtime).await {
                Ok(report) => report,
                Err(error) => json!({"entry_checks_passed":false,"error":error,"login":"not_covered","actual_video_playback":"not_covered"}),
            };
        }
        Ok::<_, String>(report)
    }).await
        .map_err(|_| "Compatibility scenario timed out".to_owned()).and_then(|value| value);
    if result.is_err() {
        if let Ok(snapshot) = runtime.snapshot().await {
            for tab in snapshot.tabs {
                if let Ok(report) = runtime.navigation_diagnostics(tab.target).await {
                    eprintln!("MACOS_BROWSER_COMPATIBILITY_SMOKE_EVIDENCE {}", serde_json::to_string(&report).unwrap());
                }
            }
        }
    }
    let closed = runtime.close().await.map_err(error);
    drop(runtime);
    let shutdown = factory.shutdown().await.map_err(error);
    match (result, closed.and(shutdown)) {
        (Ok(mut report), Ok(())) => {
            report["attachment_picker"] = super::downloads::verify(app, fixture).await?;
            Ok(report)
        },
        (Err(failure), Ok(())) => Err(failure),
        (Ok(_), Err(cleanup)) => Err(format!("Native fixture cleanup failed: {cleanup}")),
        (Err(failure), Err(cleanup)) => Err(format!("{failure}; cleanup: {cleanup}")),
    }
}

async fn scenarios(app: &tauri::AppHandle, fixture: &Fixture, runtime: &Arc<dyn BrowserRuntime>, hold_ui: bool) -> Result<Value, String> {
    eprintln!("MACOS_BROWSER_COMPATIBILITY_SMOKE_STAGE first_stop");
    surface(runtime).await?;
    // Create acknowledges dispatch; the server intentionally never commits a document.
    let stopped_id = create(runtime, &fixture.url("/slow-first?token=stop-secret")).await?;
    wait_receipt(fixture, "/slow-first", 1).await?;
    let pending = tab(runtime, &stopped_id).await?;
    if pending.load.as_ref().is_none_or(|load| load.content_state != BrowserContentState::None) {
        return Err("First external request incorrectly retained bootstrap as user content".into());
    }
    command(runtime, BrowserTabCommand::StopLoading { target: pending.target }).await?;
    let stopped = wait_tab(runtime, &stopped_id, |tab| tab.lifecycle == BrowserTabLifecycle::Stopped).await?;
    let load = stopped.load.as_ref().ok_or("Missing stopped-page evidence")?;
    if load.phase != BrowserNavigationPhase::Cancelled || load.content_state != BrowserContentState::None
        || load.cancellation_reason != Some(BrowserCancellationReason::UserStop) {
        return Err("First Stop did not retain explicit cancelled/no-content state".into());
    }
    wait_visibility(app, false).await?;
    runtime.lock_user_input().await.map_err(error)?;
    let unavailable = observe(runtime, &stopped_id).await;
    if !matches!(unavailable, Err(WorkspaceError::PageStopped)) {
        return Err(format!("Stopped page did not fail promptly before DOM observation: {unavailable:?}"));
    }

    // Recovery uses the same Page and existing attachment while native input is locked.
    eprintln!("MACOS_BROWSER_COMPATIBILITY_SMOKE_STAGE ua_recovery");
    let root_url = fixture.url("/ua-root?token=fixture-query-only#fragment-secret");
    navigate(runtime, &stopped_id, &root_url).await?;
    let root = finished(runtime, &stopped_id).await?;
    wait_visibility(app, true).await?;
    let root_report = report(runtime, &root.target).await?;
    let identity = root_report.identity.clone().ok_or("Missing compatibility identity evidence")?;
    let expected_ua = identity.effective_user_agent.clone();
    if !expected_ua.contains("Version/") || !expected_ua.contains("Safari/") {
        return Err("Actual WK identity is missing its desktop Safari compatibility tokens".into());
    }
    wait_javascript_ua(fixture, &["root", "frame"], &expected_ua).await?;
    wait_receipt(fixture, "/ua-image", 1).await?;
    wait_receipt(fixture, "/ua-fetch", 2).await?;
    for receipt in fixture.snapshot() {
        if ["/slow-first", "/ua-root", "/ua-frame", "/ua-image", "/ua-fetch", "/ua-report"].contains(&receipt.path.as_str())
            && receipt.user_agent != expected_ua {
            return Err(format!("HTTP UA differs from native identity for {}", receipt.path));
        }
    }
    let root_observed = observe(runtime, &stopped_id).await.map_err(error)?;
    if !root_observed.content.contains("Compatibility UA root") || !root_observed.content.contains(&expected_ua) {
        return Err("Real Agent observation did not read fixture content and navigator UA".into());
    }
    assert_redacted(&serde_json::to_value(&root_observed).map_err(error)?)?;
    let stable_sequence = root.load.as_ref().unwrap().navigation_sequence;
    tokio::time::sleep(Duration::from_millis(500)).await;
    if tab(runtime, &stopped_id).await?.load.as_ref().unwrap().navigation_sequence != stable_sequence {
        return Err("An idle UA fixture was reloaded by host metadata synchronization".into());
    }

    // A real default anchor action may open a WK popup. Semantic DOM input does
    // not create trusted user activation; --hold-ui can prove a physical click.
    eprintln!("MACOS_BROWSER_COMPATIBILITY_SMOKE_STAGE popup");
    let popup = if hold_ui {
        runtime.release_pressed_input().await.map_err(error)?;
        runtime.unlock_user_input().await.map_err(error)?;
        eprintln!("MACOS_BROWSER_COMPATIBILITY_SMOKE_HOLD Click Open compatibility popup in the disposable native window");
        let result = wait_popup(runtime, &stopped_id, Duration::from_secs(60)).await;
        runtime.lock_user_input().await.map_err(error)?;
        result?
    } else {
        let link = root_observed.elements.iter().find(|element| element.name == "Open compatibility popup")
            .ok_or("Missing standard popup anchor")?.reference.clone();
        runtime.automation().ok_or("Missing automation port")?
            .act(BrowserAction::click(link), CancellationToken::new()).await.map_err(error)?;
        wait_popup(runtime, &stopped_id, Duration::from_secs(3)).await?
    };
    let popup_evidence = if let Some(popup_id) = popup {
        let popup = finished(runtime, &popup_id).await?;
        wait_javascript_ua(fixture, &["popup"], &expected_ua).await?;
        if fixture.snapshot().iter().any(|receipt| receipt.path == "/popup" && receipt.user_agent != expected_ua) {
            return Err("Popup's first request used a different identity".into());
        }
        let popup_report = report(runtime, &popup.target).await?;
        if popup_report.identity != Some(identity.clone()) { return Err("Popup identity policy differs from opener".into()); }
        let observed = observe(runtime, &popup_id).await.map_err(error)?;
        if !observed.content.contains("Compatibility popup ready") { return Err("Popup did not retain its real document".into()); }
        command(runtime, BrowserTabCommand::Close { target: tab(runtime, &popup_id).await?.target }).await?;
        json!({"opened":true,"first_request_and_javascript_ua":true,"physical_click":hold_ui})
    } else {
        if hold_ui { return Err("Physical popup link was not clicked or did not create a native page".into()); }
        json!({"opened":false,"observation":"No popup observed after a semantic anchor click; trusted user activation was not supplied"})
    };
    command(runtime, BrowserTabCommand::Activate { target: tab(runtime, &stopped_id).await?.target }).await?;
    surface(runtime).await?;

    // A two-hop server redirect belongs to one host request and one native attempt.
    eprintln!("MACOS_BROWSER_COMPATIBILITY_SMOKE_STAGE redirects");
    let before_redirect = trace_sequence(runtime, &stopped_id).await?;
    navigate(runtime, &stopped_id, &fixture.url("/redirect-a?token=redirect-secret")).await?;
    let redirected = finished(runtime, &stopped_id).await?;
    let redirect_report = report(runtime, &redirected.target).await?;
    if fixture.snapshot().iter().any(|receipt| ["/redirect-a", "/redirect-b", "/redirect-final"].contains(&receipt.path.as_str())
        && receipt.user_agent != expected_ua) {
        return Err("A server redirect request used a different compatibility identity".into());
    }
    let redirect_events: Vec<_> = redirect_report.trace.events.iter().filter(|event| event.sequence > before_redirect).collect();
    let redirects: Vec<_> = redirect_events.iter().filter(|event| event.kind == BrowserNavigationEventKind::Redirect).collect();
    let redirect_attempt = redirected.load.as_ref().unwrap().navigation_sequence;
    if redirects.len() < 2 || redirects.iter().any(|event| event.navigation_sequence != Some(redirect_attempt))
        || redirect_events.iter().filter(|event| event.kind == BrowserNavigationEventKind::Requested).count() != 1 {
        return Err("Native redirect callbacks did not preserve a single navigation attempt".into());
    }
    assert_redacted(&serde_json::to_value(&redirect_report).map_err(error)?)?;
    let response_events: Vec<_> = redirect_report.trace.events.iter()
        .filter(|event| event.kind == BrowserNavigationEventKind::Response).collect();
    if response_events.is_empty() || response_events.iter().any(|event|
        event.navigation_sequence.is_some() || event.document_generation.is_some()) {
        return Err("WK response without WKNavigation was attributed to the latest attempt".into());
    }
    let redirected_observed = observe(runtime, &stopped_id).await.map_err(error)?;
    if !redirected_observed.content.contains("Compatibility redirects settled") {
        return Err("Redirected document was not observable".into());
    }

    // Page script reload uses a new native attempt, without another host request.
    eprintln!("MACOS_BROWSER_COMPATIBILITY_SMOKE_STAGE page_reload");
    let before_reload = trace_sequence(runtime, &stopped_id).await?;
    navigate(runtime, &stopped_id, &fixture.url("/reload")).await?;
    wait_receipt(fixture, "/reload", 2).await?;
    let redirect_attempt = redirected.load.as_ref().unwrap().navigation_sequence;
    let reloaded = wait_tab(runtime, &stopped_id, |tab| tab.load.as_ref().is_some_and(|load|
        load.phase == BrowserNavigationPhase::Finished && load.navigation_sequence >= redirect_attempt.saturating_add(2))).await?;
    let reloaded_observed = observe(runtime, &stopped_id).await.map_err(error)?;
    if !reloaded_observed.content.contains("reload-count=1") { return Err("Page-initiated reload did not settle".into()); }
    let reload_report = report(runtime, &reloaded.target).await?;
    let reload_events: Vec<_> = reload_report.trace.events.iter().filter(|event| event.sequence > before_reload).collect();
    let started_attempts: std::collections::BTreeSet<_> = reload_events.iter()
        .filter(|event| event.kind == BrowserNavigationEventKind::Started).filter_map(|event| event.navigation_sequence).collect();
    if started_attempts.len() < 2 || reload_events.iter().filter(|event|
        event.kind == BrowserNavigationEventKind::Requested && event.source == BrowserNavigationSource::AgentCommand).count() != 1
        || !reload_events.iter().any(|event| event.kind == BrowserNavigationEventKind::Started && event.source == BrowserNavigationSource::PageNavigation) {
        return Err("Page reload was misattributed to repeated host navigation".into());
    }

    // A fresh tab has no document to retain when the main resource fails.
    eprintln!("MACOS_BROWSER_COMPATIBILITY_SMOKE_STAGE connection_failure");
    let failed_id = create(runtime, &fixture.url("/disconnect?token=failure-secret")).await?;
    let failed = wait_tab(runtime, &failed_id, |tab| tab.lifecycle == BrowserTabLifecycle::Failed).await?;
    if failed.load.as_ref().is_none_or(|load| load.phase != BrowserNavigationPhase::Failed
        || load.content_state != BrowserContentState::None || load.problem.is_none()) {
        return Err("Initial connection failure did not report safe no-content evidence".into());
    }
    wait_visibility(app, false).await?;
    let failed_observe = observe(runtime, &failed_id).await;
    if !matches!(failed_observe, Err(WorkspaceError::PageFailed)) {
        return Err(format!("Failed page did not fail promptly before DOM observation: {failed_observe:?}"));
    }
    // A later layout update cannot make the old native failure surface visible.
    surface(runtime).await?;
    wait_visibility(app, false).await?;
    assert_redacted(&serde_json::to_value(report(runtime, &failed.target).await?).map_err(error)?)?;

    // 403 is a successfully received website document, not a host error page.
    eprintln!("MACOS_BROWSER_COMPATIBILITY_SMOKE_STAGE http_403");
    let forbidden_url = fixture.url("/forbidden?token=forbidden-secret#fragment-secret");
    navigate(runtime, &failed_id, &forbidden_url).await?;
    let forbidden = finished(runtime, &failed_id).await?;
    let visible_forbidden_url = forbidden.load.as_ref().and_then(|load| load.content_url.clone())
        .ok_or("HTTP 403 document has no content address")?;
    wait_visibility(app, true).await?;
    let forbidden_observed = observe(runtime, &failed_id).await.map_err(error)?;
    if !forbidden_observed.content.contains("Compatibility HTTP 403 site content") {
        return Err("HTTP 403 document was replaced by a host failure surface".into());
    }
    let forbidden_report = report(runtime, &forbidden.target).await?;
    if !forbidden_report.trace.events.iter().any(|event| event.kind == BrowserNavigationEventKind::Response && event.response_status == Some(403)) {
        return Err("Main-document HTTP 403 response was absent from native diagnostics".into());
    }

    // A subsequent provisional failure preserves and correctly identifies the
    // existing 403 document, while its old observation references are obsolete.
    eprintln!("MACOS_BROWSER_COMPATIBILITY_SMOKE_STAGE retained_document");
    navigate(runtime, &failed_id, &fixture.url("/disconnect?token=failure-secret")).await?;
    let retained = wait_tab(runtime, &failed_id, |tab| tab.lifecycle == BrowserTabLifecycle::Failed).await?;
    if retained.load.as_ref().is_none_or(|load| load.content_state != BrowserContentState::RetainedDocument
        || load.content_url.as_deref() != Some(visible_forbidden_url.as_str())) {
        return Err("New request failure lost the existing document's true address".into());
    }
    wait_visibility(app, true).await?;
    let retained_observed = observe(runtime, &failed_id).await.map_err(error)?;
    if !retained_observed.content.contains("Compatibility HTTP 403 site content")
        || retained_observed.load.as_ref().is_none_or(|load| load.phase != BrowserNavigationPhase::Failed)
        || retained_observed.content_url.as_deref() != Some(fixture.url("/forbidden").as_str()) {
        return Err("Observation did not distinguish retained document from failed request".into());
    }
    let final_report = report(runtime, &retained.target).await?;
    assert_redacted(&serde_json::to_value(&final_report).map_err(error)?)?;
    assert_redacted(&serde_json::to_value(&retained_observed).map_err(error)?)?;
    let mut stale = retained.target.clone();
    stale.document_generation = stale.document_generation.saturating_sub(1);
    if !matches!(runtime.navigation_diagnostics(stale).await, Err(WorkspaceError::StaleTarget)) {
        return Err("Diagnostics accepted an outdated document target".into());
    }
    runtime.release_pressed_input().await.map_err(error)?;
    runtime.unlock_user_input().await.map_err(error)?;
    Ok(json!({
        "scope":"real_macos_desktop_host", "ephemeral_profile":true,
        "window_inner_size":{"width":880,"height":600}, "browser_surface_bounds":{"x":20,"y":40,"width":840,"height":540}, "identity":identity,
        "runtime":final_report.runtime,
        "first_stop_without_content":true,"stop_and_failure_surface_mask":true,
        "same_tab_recovery":true,"http_and_javascript_ua":{"root":true,"iframe":true,"fetch":true,"image":true},
        "popup":popup_evidence,"redirect_single_attempt":true,"page_reload_distinguished":true,
        "failed_observation_returns_promptly":true,"http_403_document_retained":true,
        "provisional_failure_retains_old_document":true,"diagnostics_redacted_and_target_checked":true,
        "unassociated_response_samples":true,"navigation_report":final_report,
    }))
}

async fn command(runtime: &Arc<dyn BrowserRuntime>, command: BrowserTabCommand) -> Result<BrowserRuntimeSnapshot, String> {
    runtime.execute(command, CancellationToken::new()).await.map_err(error)
}
async fn create(runtime: &Arc<dyn BrowserRuntime>, url: &str) -> Result<String, String> {
    let snapshot = command(runtime, BrowserTabCommand::Create { url: url.into() }).await?;
    snapshot.active_tab_id.ok_or_else(|| "Created native page has no active target".into())
}
async fn navigate(runtime: &Arc<dyn BrowserRuntime>, id: &str, url: &str) -> Result<(), String> {
    command(runtime, BrowserTabCommand::Navigate { target: tab(runtime, id).await?.target, url: url.into() }).await.map(|_| ())
}
async fn tab(runtime: &Arc<dyn BrowserRuntime>, id: &str) -> Result<BrowserTabSnapshot, String> {
    runtime.snapshot().await.map_err(error)?.tabs.into_iter().find(|tab| tab.target.tab_id == id)
        .ok_or_else(|| "Owned native fixture tab is missing".into())
}
async fn wait_tab(runtime: &Arc<dyn BrowserRuntime>, id: &str, predicate: impl Fn(&BrowserTabSnapshot) -> bool) -> Result<BrowserTabSnapshot, String> {
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        let tab = tab(runtime, id).await?;
        if predicate(&tab) { return Ok(tab); }
        if Instant::now() >= deadline { return Err(format!("Native tab did not settle; lifecycle={:?}, phase={:?}", tab.lifecycle, tab.load.as_ref().map(|load| load.phase))); }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
async fn finished(runtime: &Arc<dyn BrowserRuntime>, id: &str) -> Result<BrowserTabSnapshot, String> {
    wait_tab(runtime, id, |tab| tab.load.as_ref().is_some_and(|load| load.phase == BrowserNavigationPhase::Finished)).await
}
async fn surface(runtime: &Arc<dyn BrowserRuntime>) -> Result<(), String> {
    runtime.surface().ok_or("Missing native surface port")?.set_surface(
        BrowserSurfaceBounds { x:20.0,y:40.0,width:840.0,height:540.0 }, true, CancellationToken::new()).await.map_err(error)
}
async fn observe(runtime: &Arc<dyn BrowserRuntime>, id: &str) -> Result<BrowserObservation, WorkspaceError> {
    tokio::time::timeout(Duration::from_secs(3), runtime.automation().ok_or(WorkspaceError::UnsupportedAction)?
        .observe(Some(id.into()), CancellationToken::new())).await.map_err(|_| WorkspaceError::NativeCommandFailed)?
}
async fn report(runtime: &Arc<dyn BrowserRuntime>, target: &BrowserTabTarget) -> Result<BrowserNavigationReport, String> {
    runtime.navigation_diagnostics(target.clone()).await.map_err(error)
}
async fn trace_sequence(runtime: &Arc<dyn BrowserRuntime>, id: &str) -> Result<u64, String> {
    Ok(report(runtime, &tab(runtime, id).await?.target).await?.trace.events.last().map_or(0, |event| event.sequence))
}
async fn wait_receipt(fixture: &Fixture, path: &str, expected: usize) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while fixture.count(path) < expected {
        if Instant::now() >= deadline { return Err(format!("Fixture request {path} did not arrive")); }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    Ok(())
}
async fn wait_javascript_ua(fixture: &Fixture, cases: &[&str], expected: &str) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let receipts = fixture.snapshot();
        if cases.iter().all(|case| receipts.iter().any(|receipt| receipt.path == "/ua-report"
            && receipt.case.as_deref() == Some(*case) && receipt.javascript_user_agent.as_deref() == Some(expected)
            && receipt.user_agent == expected)) { return Ok(()); }
        if Instant::now() >= deadline { return Err("Fixture JavaScript and HTTP UA did not agree with native identity".into()); }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
async fn wait_popup(runtime: &Arc<dyn BrowserRuntime>, opener: &str, timeout: Duration) -> Result<Option<String>, String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(tab) = runtime.snapshot().await.map_err(error)?.tabs.iter().find(|tab| tab.target.tab_id != opener) {
            return Ok(Some(tab.target.tab_id.clone()));
        }
        if Instant::now() >= deadline { return Ok(None); }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
fn assert_redacted(value: &Value) -> Result<(), String> {
    let text = serde_json::to_string(value).map_err(error)?;
    for secret in ["fixture-query-only", "fragment-secret", "popup-secret", "redirect-secret", "failure-secret", "forbidden-secret", "stop-secret"] {
        if text.contains(secret) { return Err("A fixture query/fragment secret escaped metadata projection".into()); }
    }
    Ok(())
}

/// Inspect only this disposable NSWindow's actual WK children on the AppKit
/// thread, without a JS bridge or screenshot inference of native visibility.
async fn native_visible_count(app: &tauri::AppHandle) -> Result<usize, String> {
    let handle = app.clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let result = (|| -> Result<usize, String> {
            let window = handle.get_window("main").ok_or("Missing disposable native window")?;
            let raw = window.ns_window().map_err(error)?;
            let window = unsafe { raw.cast::<objc2_app_kit::NSWindow>().as_ref() }.ok_or("Missing NSWindow")?;
            let content = window.contentView().ok_or("Missing NSWindow content")?;
            let class = objc2::runtime::AnyClass::get(c"WKWebView").ok_or("WKWebView class is unavailable")?;
            let mut count = 0;
            for view in content.subviews() {
                let webkit: bool = unsafe { msg_send![&*view, isKindOfClass: class] };
                if webkit && !view.isHidden() { count += 1; }
            }
            Ok(count)
        })();
        let _ = tx.send(result);
    }).map_err(error)?;
    rx.await.map_err(error)?
}
async fn wait_visibility(app: &tauri::AppHandle, visible: bool) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let count = native_visible_count(app).await?;
        if count == usize::from(visible) { return Ok(()); }
        if Instant::now() >= deadline { return Err(format!("Native WK mask showed {count} children, expected {}", usize::from(visible))); }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
