//! Same-machine identity comparison with a disposable persistent WK store.
//! Each arm starts from an empty store and identical controlled warm protocol;
//! this never copies user cookies or changes the production identity policy.
use super::{fixture::Fixture, macos::host::DesktopBrowserHost};
use nomifun_browser_macos::BrowserIdentityPolicy;
use nomifun_browser_platform::runtime::*;
use serde_json::{Value, json};
use std::{path::Path, sync::{Arc, Mutex}, time::{Duration, Instant}};
use tokio_util::sync::CancellationToken;

pub(crate) async fn verify(app: &tauri::AppHandle, fixture: &Fixture, hold_ui: bool,
    live_sites: bool, manual_sites: bool, evidence_dir: Option<&Path>) -> Result<Value, String> {
    let identifier = *uuid::Uuid::now_v7().as_bytes();
    let profile = BrowserProfile::WebKitPersistent { identifier };
    let mut reports = Vec::new();
    let started = Instant::now();
    for (name, policy) in [("safari17", BrowserIdentityPolicy::Safari17), ("native_webkit", BrowserIdentityPolicy::NativeWebKit)] {
        let directory = evidence_dir.map(|directory| directory.join(name));
        if let Some(directory) = &directory { std::fs::create_dir_all(directory).map_err(|_| "Identity evidence directory failed")?; }
        let host = DesktopBrowserHost::new_with_identity_policy(app.clone(), policy);
        let retained_runtime = Arc::new(Mutex::new(None));
        let timeout = Duration::from_secs(if manual_sites { 900 } else if live_sites { 360 } else { 90 });
        let outcome = tokio::time::timeout(timeout, arm(&host, fixture, &profile, hold_ui, live_sites,
            manual_sites, directory.as_deref(), retained_runtime.clone())).await
            .map_err(|_| "Identity arm timed out before cleanup".to_string()).and_then(|result| result);
        // An observation/surface timeout must cancel and drain its exact runtime
        // before native engine shutdown and profile removal, even after the arm
        // future was dropped. The outer watchdog only handles cleanup failure.
        let retained = retained_runtime.lock().unwrap().take();
        let emergency_close = if let Some(runtime) = retained {
            tokio::time::timeout(Duration::from_secs(15),runtime.close()).await
                .map_err(|_| "Identity emergency close timed out".to_string())
                .and_then(|result| result.map_err(|error| error.to_string()))
        } else { Ok(()) };
        let shutdown = tokio::time::timeout(Duration::from_secs(15),host.shutdown()).await
            .map_err(|_| "Identity host shutdown timed out".to_string())
            .and_then(|result| result.map_err(|error| error.to_string()));
        // The fixture owns this randomly allocated store; disposal is explicit
        // and occurs after every native context and engine has been released.
        let removal = tokio::time::timeout(Duration::from_secs(30),host.delete_persistent_profile(&profile)).await
            .map_err(|_| "Identity disposable store removal timed out".to_string())
            .and_then(|result| result.map_err(|error| error.to_string()));
        let mut report = outcome.unwrap_or_else(|error| json!({"fixture_checks_passed":false,"error":error}));
        report["variant"] = json!(name);
        report["finished_elapsed_ms"] = json!(started.elapsed().as_millis());
        report["shutdown_completed"] = json!(shutdown.is_ok());
        report["disposable_store_removed"] = json!(removal.is_ok());
        report["emergency_runtime_close_completed"] = json!(emergency_close.is_ok());
        if let Err(error) = emergency_close.and(shutdown).and(removal) { return Err(format!("Identity arm cleanup failed: {error}")); }
        reports.push(report);
    }
    let fixture_passed = reports.iter().all(|report| report["fixture_checks_passed"] == true);
    let live_passed = !live_sites || reports.iter().all(|report| report["live_sites"]["entry_checks_passed"] == true);
    let native_site_acceptance = live_sites && manual_sites && fixture_passed && reports.iter()
        .all(|report| report["live_sites"]["native_site_acceptance_completed"] == true);
    Ok(json!({"fixture_checks_passed":fixture_passed,"live_checks_passed":live_passed,"arms":reports,
        "native_site_acceptance_completed":native_site_acceptance,
        "same_machine_and_process":true,"same_fixture_origin":true,"same_disposable_profile_identifier":true,
        "initial_external_site_state":"empty_store_before_each_arm",
        "controlled_warm_state":"identical_cookie_localstorage_indexeddb_fixture_then_runtime_reopen",
        "external_browser_comparison":"requires_separate_Safari_same_machine_same_network_run",
        "production_policy_changed":false,"elapsed_seconds":started.elapsed().as_secs()}))
}

async fn create(host: &DesktopBrowserHost, profile: &BrowserProfile, generation: u64) -> Result<Arc<dyn BrowserRuntime>, String> {
    tokio::time::timeout(Duration::from_secs(10), host.create(CreateBrowserRuntime {
        key: BrowserResourceKey { principal_id:"identity-comparison-fixture".into(),
            agent_session_id:"disposable-identity-session".into(),resource_binding_id:"identity-comparison".into() },
        runtime_generation:generation,profile:profile.clone(),user_input_enabled:true,
    })).await.map_err(|_| "Identity runtime creation timed out".to_string())?
        .map_err(|error| error.to_string())
}

async fn arm(host: &DesktopBrowserHost, fixture: &Fixture, profile: &BrowserProfile, hold_ui: bool,
    live_sites: bool, manual_sites: bool, evidence_dir: Option<&Path>,
    retained_runtime: Arc<Mutex<Option<Arc<dyn BrowserRuntime>>>>) -> Result<Value, String> {
    let receipt_start = fixture.snapshot().len();
    let warm = create(host, profile, 1).await?;
    *retained_runtime.lock().unwrap() = Some(warm.clone());
    let warming = async {
        create_tab(&warm, &fixture.url("/identity-warm")).await?;
        wait_receipts(fixture, receipt_start, |receipts| receipts.iter().any(|receipt|
            receipt.path == "/storage-report" && receipt.case.as_deref() == Some("warm") && receipt.storage_ok == Some(true))).await
    }.await;
    let closed = warm.close().await.map_err(|error| error.to_string());
    if closed.is_ok() { retained_runtime.lock().unwrap().take(); }
    drop(warm);
    warming.and(closed)?;
    let runtime = create(host, profile, 2).await?;
    *retained_runtime.lock().unwrap() = Some(runtime.clone());
    let result = verify_runtime(&runtime, fixture, receipt_start, hold_ui, live_sites, manual_sites, evidence_dir).await;
    let closed = runtime.close().await.map_err(|error| error.to_string());
    if closed.is_ok() { retained_runtime.lock().unwrap().take(); }
    drop(runtime);
    match (result, closed) {
        (Ok(report), Ok(())) => Ok(report),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(format!("Identity runtime close failed: {error}")),
        (Err(error), Err(cleanup)) => Err(format!("{error}; close: {cleanup}")),
    }
}

async fn verify_runtime(runtime: &Arc<dyn BrowserRuntime>, fixture: &Fixture, receipt_start: usize,
    hold_ui: bool, live_sites: bool, manual_sites: bool, evidence_dir: Option<&Path>) -> Result<Value, String> {
    let id = create_tab(runtime, &fixture.url("/identity-check")).await?;
    runtime.surface().ok_or("Identity runtime has no surface")?.set_surface(
        BrowserSurfaceBounds { x:20.0,y:40.0,width:840.0,height:540.0 },true,CancellationToken::new())
        .await.map_err(|error| error.to_string())?;
    let current = finished(runtime, &id).await?;
    let report = measured_report(runtime,&current.target.tab_id).await?;
    let identity = report.identity.ok_or("Missing measured identity report")?;
    let expected = &identity.effective_user_agent;
    if expected.is_empty() { return Err("Native identity UA was not measured before external requests".into()); }
    wait_receipts(fixture, receipt_start, |receipts| ["root", "frame", "worker"].iter().all(|case|
        receipts.iter().any(|receipt| receipt.path == "/ua-report" && receipt.case.as_deref() == Some(*case)
            && receipt.javascript_user_agent.as_deref() == Some(expected.as_str()) && receipt.user_agent == *expected))
        && receipts.iter().any(|receipt| receipt.path == "/storage-report" && receipt.case.as_deref() == Some("check")
            && receipt.storage_ok == Some(true))
        && receipts.iter().any(|receipt| receipt.path == "/ua-image")
        && receipts.iter().filter(|receipt| receipt.path == "/ua-fetch").count() >= 2
        && receipts.iter().any(|receipt| receipt.path == "/identity-worker.js")).await?;
    if fixture.snapshot()[receipt_start..].iter().any(|receipt| receipt.user_agent != *expected) {
        return Err("Identity differs between first document, frame, worker or resource requests".into());
    }
    runtime.lock_user_input().await.map_err(|error| error.to_string())?;
    let observation = runtime.automation().ok_or("Identity runtime has no observation port")?
        .observe(Some(id.clone()),CancellationToken::new()).await.map_err(|error| error.to_string())?;
    if !observation.content.contains("Profile checks passed") || !observation.content.contains(expected) {
        return Err("Reopened runtime did not retain the controlled warm profile".into());
    }
    if hold_ui {
        runtime.release_pressed_input().await.map_err(|error| error.to_string())?;
        runtime.unlock_user_input().await.map_err(|error| error.to_string())?;
        eprintln!("MACOS_BROWSER_IDENTITY_AB_HOLD {}: physically click Open compatibility popup", identity.profile_id);
    } else {
        let link = observation.elements.iter().find(|element| element.name == "Open compatibility popup")
            .ok_or("Identity fixture popup anchor missing")?.reference.clone();
        runtime.automation().ok_or("Identity runtime has no action port")?
            .act(BrowserAction::click(link),CancellationToken::new()).await.map_err(|error| error.to_string())?;
    }
    let popup_deadline = Instant::now() + Duration::from_secs(if hold_ui { 60 } else { 3 });
    let mut popup_evidence = json!({"observed":false,"reason":"no_trusted_popup_observed","physical_click":hold_ui});
    loop {
        let snapshot = runtime.snapshot().await.map_err(|error| error.to_string())?;
        if let Some(popup) = snapshot.tabs.into_iter().find(|tab| tab.target.tab_id != id) {
            let popup = finished(runtime, &popup.target.tab_id).await?;
            wait_receipts(fixture, receipt_start, |receipts| receipts.iter().any(|receipt|
                receipt.path == "/ua-report" && receipt.case.as_deref() == Some("popup")
                    && receipt.javascript_user_agent.as_deref() == Some(expected.as_str()) && receipt.user_agent == *expected)).await?;
            let popup_report = measured_report(runtime,&popup.target.tab_id).await?;
            if popup_report.identity.as_ref() != Some(&identity) || fixture.snapshot()[receipt_start..].iter()
                .any(|receipt| receipt.path == "/popup" && receipt.user_agent != *expected) {
                return Err("Popup did not inherit the runtime identity from its first request".into());
            }
            if fixture.snapshot()[receipt_start..].iter().filter(|receipt| receipt.path == "/popup").count() != 1 {
                return Err("Popup initial document was requested more than once".into());
            }
            popup_evidence = json!({"observed":true,"same_identity_first_http_and_js":true,"physical_click":hold_ui,
                "navigation_report":popup_report});
            runtime.execute(BrowserTabCommand::Close {target:popup.target},CancellationToken::new()).await.map_err(|error| error.to_string())?;
            break;
        }
        if Instant::now() >= popup_deadline { break; }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let popup_observed = popup_evidence["observed"] == true;
    runtime.release_pressed_input().await.map_err(|error| error.to_string())?;
    runtime.unlock_user_input().await.map_err(|error| error.to_string())?;
    let final_report = runtime.navigation_diagnostics(tab(runtime, &id).await?.target).await.map_err(|error| error.to_string())?;
    let live = if live_sites { Some(super::live::verify(runtime, manual_sites, evidence_dir).await?) } else { None };
    Ok(json!({"fixture_checks_passed":popup_observed,"identity":identity,"runtime":final_report.runtime,
        "http_javascript_ua_agree":{"root":true,"iframe":true,"worker":true,"fetch":true,"image":true},
        "cookie_localstorage_indexeddb_after_runtime_reopen":true,"popup":popup_evidence,
        "navigation_report":final_report,"live_sites":live}))
}

async fn create_tab(runtime: &Arc<dyn BrowserRuntime>, url: &str) -> Result<String, String> {
    runtime.execute(BrowserTabCommand::Create {url:url.into()},CancellationToken::new()).await.map_err(|error| error.to_string())?
        .active_tab_id.ok_or_else(|| "Identity tab creation returned no active tab".into())
}

async fn tab(runtime: &Arc<dyn BrowserRuntime>, id: &str) -> Result<BrowserTabSnapshot, String> {
    runtime.snapshot().await.map_err(|error| error.to_string())?.tabs.into_iter().find(|tab| tab.target.tab_id == id)
        .ok_or_else(|| "Identity target disappeared".into())
}

async fn finished(runtime: &Arc<dyn BrowserRuntime>, id: &str) -> Result<BrowserTabSnapshot, String> {
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        let current = tab(runtime,id).await?;
        if current.load.as_ref().is_some_and(|load| load.phase == BrowserNavigationPhase::Finished) { return Ok(current); }
        if Instant::now() >= deadline { return Err("Identity document did not finish".into()); }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn measured_report(runtime: &Arc<dyn BrowserRuntime>, id: &str) -> Result<BrowserNavigationReport,String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let current = tab(runtime,id).await?;
        let report = runtime.navigation_diagnostics(current.target).await.map_err(|error| error.to_string())?;
        if report.identity.as_ref().is_some_and(|identity| !identity.effective_user_agent.is_empty()) { return Ok(report); }
        if Instant::now() >= deadline { return Err("Public native identity measurement remained unavailable".into()); }
        // Read-only diagnostic sampling never re-navigates the document.
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn wait_receipts(fixture: &Fixture, start: usize, predicate: impl Fn(&[super::fixture::Receipt]) -> bool) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        let receipts = fixture.snapshot();
        if predicate(&receipts[start..]) { return Ok(()); }
        if Instant::now() >= deadline { return Err("Controlled identity/profile evidence did not arrive".into()); }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
