//! Production BrowserRuntime on a real WebView2 child, with independent HTTP witnesses.
//! Run via browser_workspace_smoke --navigation-only. No existing user data is read.
use super::navigation_fixture::Fixture;
use nomifun_browser_platform::runtime::*;
use nomifun_browser_platform::run_guard::RunAdmissionError;
use nomifun_browser_platform::workspace::BrowserResourceService;
use serde_json::{Value, json};
use base64::Engine;
use std::{collections::BTreeSet, sync::Arc, time::{Duration, Instant}};
use tauri::Manager;
use tokio_util::sync::CancellationToken;

fn message(error: impl std::fmt::Display) -> String { error.to_string() }

pub(super) async fn verify(app: &tauri::AppHandle) -> Result<Value, String> {
    let fixture = Fixture::new()?;
    let factory = super::host::DesktopBrowserHost::new(app.clone());
    let runtime = factory.create(CreateBrowserRuntime {
        key: BrowserResourceKey { principal_id: "navigation-conformance".into(),
            agent_session_id: "disposable-session".into(), resource_binding_id: "native-navigation".into() },
        runtime_generation: 1, profile: BrowserProfile::Ephemeral, user_input_enabled: true,
    }).await.map_err(message)?;
    let result = tokio::time::timeout(Duration::from_secs(140), scenarios(app, &fixture, &runtime)).await
        .map_err(|_| "Native navigation conformance timed out".to_owned()).and_then(|result| result);
    if result.is_err() {
        if let Ok(snapshot) = runtime.snapshot().await {
            for tab in snapshot.tabs {
                if let Ok(report) = runtime.navigation_diagnostics(tab.target).await {
                    eprintln!("BROWSER_NAVIGATION_FAILURE_EVIDENCE {}", serde_json::to_string(&report).unwrap());
                }
            }
        }
    }
    let closed = runtime.close().await.map_err(message);
    drop(runtime);
    let cleanup = closed.and(factory.shutdown().await.map_err(message));
    match (result, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(format!("Navigation fixture cleanup failed: {error}")),
        (Err(error), Err(cleanup)) => Err(format!("{error}; cleanup: {cleanup}")),
    }
}

async fn scenarios(app: &tauri::AppHandle, fixture: &Fixture, runtime: &Arc<dyn BrowserRuntime>) -> Result<Value, String> {
    surface(runtime, true, 1060.0).await?;
    stage("initial_and_passive_updates");
    let id = create(runtime, &fixture.url("/nav/initial?fixture_secret=address#private_fragment")).await?;
    let first = settled(runtime, &id, "/nav/initial").await?;
    exactly(fixture, "/nav/initial", 1)?;
    let initial_report = report(runtime, &id).await?;
    let environment = initial_report.runtime.clone().ok_or("Missing effective native environment evidence")?;
    if environment.profile_persistence != Some(BrowserProfilePersistence::Ephemeral)
        || environment.profile_identity_token.as_ref().is_none_or(|token| token.len() != 64)
        || environment.browser_version.as_ref().is_none_or(String::is_empty)
        || environment.environment_overrides_present.is_none() {
        return Err("Native environment did not verify effective Runtime and ephemeral Profile".into());
    }
    require_finished(&first)?;
    require_host_commands(&initial_report, 1)?;
    assert_redacted(&initial_report)?;
    let first_sequence = load(&first)?.navigation_sequence;
    // Snapshot, diagnostics, resize and show/hide are passive; use server
    // witnesses as well as the navigation owner, so two wrong views cannot agree.
    for (visible, width) in [(false, 1000.0), (true, 1040.0), (false, 1060.0), (true, 1060.0)] {
        surface(runtime, visible, width).await?;
        let _ = runtime.snapshot().await.map_err(message)?;
        let _ = report(runtime, &id).await?;
    }
    tokio::time::sleep(Duration::from_millis(400)).await;
    exactly(fixture, "/nav/initial", 1)?;
    if load(&tab(runtime, &id).await?)?.navigation_sequence != first_sequence {
        return Err("Passive surface or metadata operations created another navigation".into());
    }

    stage("explicit_reload");
    let reload_tab = tab(runtime, &id).await?;
    command(runtime, BrowserTabCommand::Reload { expected_navigation_sequence:load(&reload_tab)?.navigation_sequence,
        target:reload_tab.target }).await?;
    wait_count(fixture, "/nav/initial", 2).await?;
    let reloaded = wait_tab(runtime, &id, |tab| tab.lifecycle == BrowserTabLifecycle::Ready
        && tab.load.as_ref().is_some_and(|load| load.phase == BrowserNavigationPhase::Finished && load.navigation_sequence > first_sequence)).await?;
    exactly(fixture, "/nav/initial", 2)?;
    require_host_commands(&report(runtime, &id).await?, 2)?;

    stage("redirects");
    let before = report(runtime, &id).await?;
    navigate(runtime, &id, &fixture.url("/nav/redirect-a?fixture_secret=redirect")).await?;
    let redirected = settled(runtime, &id, "/nav/final").await?;
    for path in ["/nav/redirect-a", "/nav/redirect-b", "/nav/final"] { exactly(fixture, path, 1)?; }
    let redirect_report = report(runtime, &id).await?;
    let events = since(&redirect_report, &before);
    let redirects: Vec<_> = events.iter().filter(|event| event.kind == BrowserNavigationEventKind::Redirect).collect();
    let native_attempts: BTreeSet<_> = events.iter().filter(|event| matches!(event.kind,
        BrowserNavigationEventKind::Started | BrowserNavigationEventKind::Redirect | BrowserNavigationEventKind::Committed | BrowserNavigationEventKind::Finished))
        .filter_map(|event| event.navigation_sequence).collect();
    if redirects.len() != 2 || native_attempts.len() != 1 || redirects.iter().any(|event|
        event.navigation_sequence != Some(load(&redirected).unwrap().navigation_sequence)) {
        return Err("Two HTTP redirects were not retained as one native attempt".into());
    }
    require_host_delta(&redirect_report, &before, 1)?;
    assert_redacted(&redirect_report)?;

    stage("page_initiated_navigation");
    let before_script = report(runtime, &id).await?;
    navigate(runtime, &id, &fixture.url("/nav/script-reload")).await?;
    wait_count(fixture, "/nav/script-reload", 2).await?;
    let script_tab = settled(runtime, &id, "/nav/script-reload").await?;
    let view = app.get_webview(&id).ok_or("Missing navigation fixture child")?;
    if evaluate(&view, "document.getElementById('state').textContent").await? != "reload-count=1" {
        return Err("Page-script reload did not settle in its second real document".into());
    }
    exactly(fixture, "/nav/script-reload", 2)?;
    let script_report = report(runtime, &id).await?;
    require_host_delta(&script_report, &before_script, 1)?;
    require_distinct_native_attempts(&script_report, &before_script, 2)?;
    if load(&script_tab)?.navigation_sequence <= load(&reloaded)?.navigation_sequence {
        return Err("Page-script reload did not advance native attempt identity".into());
    }
    for path in ["/nav/script-replace", "/nav/meta"] {
        let before = report(runtime, &id).await?;
        let expected = fixture.count("/nav/replacement") + 1;
        navigate(runtime, &id, &fixture.url(path)).await?;
        wait_count(fixture, "/nav/replacement", expected).await?;
        settled(runtime, &id, "/nav/replacement").await?;
        exactly(fixture, path, 1)?;
        exactly(fixture, "/nav/replacement", expected)?;
        let after = report(runtime, &id).await?;
        require_host_delta(&after, &before, 1)?;
        require_distinct_native_attempts(&after, &before, 2)?;
    }

    stage("automatic_navigation_pause");
    let loop_evidence = pause_loop(app, fixture, runtime, &id, "/nav/loop", &["/nav/loop"]).await?;
    let aba_evidence = pause_loop(app, fixture, runtime, &id, "/nav/aba-a", &["/nav/aba-a", "/nav/aba-b"]).await?;
    let post_resume_evidence=post_resume(fixture,runtime,&id,&view).await?;

    stage("resources_and_page_environment");
    let worker_witness=super::worker_witness::Witness::install(&view).await?;
    let before_resources = report(runtime, &id).await?;
    navigate(runtime, &id, &fixture.url("/nav/resources")).await?;
    let resource_tab = settled(runtime, &id, "/nav/resources").await?;
    require_finished(&resource_tab)?;
    wait_count(fixture, "/nav/frame", 2).await?;
    wait_count(fixture, "/nav/fetch", 16).await?;
    wait_count(fixture, "/nav/pixel", 12).await?;
    let resource_expression="fixtureFrames.length === 2 && fixtureWorker !== null ? ({root:{ua:navigator.userAgent,isTauri:'isTauri' in window,internals:'__TAURI_INTERNALS__' in window,ipc:'ipc' in window,dialog:[typeof alert,typeof confirm,typeof prompt]},frames:fixtureFrames,worker:fixtureWorker}) : null";
    let nested_expression="fixtureFrames.length === 2 && fixtureWorker !== null && fixtureNestedWorker !== null ? ({root:{ua:navigator.userAgent,isTauri:'isTauri' in window,internals:'__TAURI_INTERNALS__' in window,ipc:'ipc' in window,dialog:[typeof alert,typeof confirm,typeof prompt]},frames:fixtureFrames,worker:fixtureWorker,nested_worker:fixtureNestedWorker}) : null";
    let identity = match wait_value(&view,nested_expression).await {
        Ok(identity)=>identity,
        Err(error)=>{
            let baseline_visibility=resource_visibility(app,&view).await?;
            let baseline_witness=worker_witness.snapshot(&fixture.url("/nav/worker.js")).await;
            let baseline_counts=json!({"frame":fixture.count("/nav/frame"),"worker":fixture.count("/nav/worker.js"),
                "nested_worker":fixture.count("/nav/nested-worker.js"),"nested_execution":fixture.count("/nav/nested-worker-executed"),
                "fetch":fixture.count("/nav/fetch"),"pixel":fixture.count("/nav/pixel")});
            // The original classic script keeps its top-level const worker.
            // A separate controlled variant adds only a window strong reference.
            // Its result is diagnosis and cannot convert a baseline failure to pass.
            navigate(runtime,&id,&fixture.url("/nav/resources-strong-worker")).await?;
            settled(runtime,&id,"/nav/resources-strong-worker").await?;
            let variant=wait_value(&view,resource_expression).await;
            let variant_result=match variant {Ok(_)=>json!({"received":true}),Err(error)=>json!({"received":false,"error":error})};
            let variant_witness=worker_witness.snapshot(&fixture.url("/nav/worker.js")).await;
            // This third script is explicitly diagnostic: a controlled HTTP
            // side effect precedes postMessage so execution and delivery have
            // separate witnesses. The original identity assertion is unchanged.
            navigate(runtime,&id,&fixture.url("/nav/resources-worker-executed")).await?;
            settled(runtime,&id,"/nav/resources-worker-executed").await?;
            let diagnostic=wait_value(&view,resource_expression).await;
            let diagnostic_result=match diagnostic {Ok(_)=>json!({"received":true}),Err(error)=>json!({"received":false,"error":error})};
            let diagnostic_native=worker_witness.snapshot(&fixture.url("/nav/worker-diagnostic.js")).await;
            let diagnostic_visibility=resource_visibility(app,&view).await?;
            let executed=json!({"script_http_requests":fixture.count("/nav/worker-diagnostic.js"),
                "worker_execution_http_side_effects":fixture.count("/nav/worker-executed"),"message":diagnostic_result,
                "native":diagnostic_native,"visibility":diagnostic_visibility});
            let before_intervention=fixture.count("/nav/worker-executed");
            let intervention=worker_witness.resume_owned_worker(&fixture.url("/nav/worker-diagnostic.js"),
                &fixture.url("/nav/resources-worker-executed"),fixture).await;
            let intervention=match intervention {Ok(value)=>value,Err(error)=>json!({"available":false,"error":error})};
            let intervention=json!({"control":intervention,"executed_http_before":before_intervention,
                "executed_http_after":fixture.count("/nav/worker-executed")});
            surface(runtime,false,1060.0).await?;
            let plain=super::worker_plain::verify(app,fixture).await;
            surface(runtime,true,1060.0).await?;
            let plain=match plain {Ok(value)=>value,Err(error)=>json!({"available":false,"error":error})};
            return Err(format!("Original frame/worker witness failed: {error}; baseline_counts={baseline_counts}; baseline_native={baseline_witness}; baseline_visibility={baseline_visibility}; strong_reference_variant={variant_result}; variant_native={variant_witness}; execution_diagnostic={executed}; controlled_debugger_intervention={intervention}; plain_embedded_constructor={plain}"));
        }
    };
    let resource_worker_witness=worker_witness.snapshot(&fixture.url("/nav/worker.js")).await;
    drop(worker_witness);
    wait_count(fixture,"/nav/nested-worker-executed",1).await?;
    exactly(fixture,"/nav/nested-worker.js",1)?;
    exactly(fixture,"/nav/nested-worker-executed",1)?;
    let ua = identity["root"]["ua"].as_str().ok_or("Fixture JavaScript UA missing")?;
    let mut contexts = vec![&identity["root"], &identity["worker"],&identity["nested_worker"]];
    contexts.extend(identity["frames"].as_array().ok_or("Frame environment evidence missing")?);
    if contexts.iter().any(|context| context["isTauri"] != false || context["internals"] != false || context["ipc"] != false || context["ua"] != ua) {
        return Err(format!("External page/frame/worker host marker or identity mismatch: {identity}"));
    }
    if identity["root"]["dialog"] != json!(["function", "function", "function"]) {
        return Err("External document lost standard dialog APIs".into());
    }
    if fixture.snapshot().iter().filter(|receipt| ["/nav/resources", "/nav/frame", "/nav/fetch", "/nav/pixel", "/nav/worker.js", "/nav/nested-worker.js", "/nav/nested-worker-executed"].contains(&receipt.path.as_str()))
        .any(|receipt| receipt.user_agent != ua || receipt.method != "GET") {
        return Err("HTTP resource/frame/worker identity differs from navigator.userAgent".into());
    }
    exactly(fixture, "/nav/resources", 1)?;
    let resource_report = report(runtime, &id).await?;
    require_host_delta(&resource_report, &before_resources, 1)?;
    require_distinct_native_attempts(&resource_report, &before_resources, 1)?;
    if resource_report.load.as_ref().and_then(|load| load.navigation_control.as_ref()).is_none_or(|control| control.loop_suspected) {
        return Err("Ordinary frames, images and XHR were mistaken for main-document loop".into());
    }

    stage("native_popup");
    navigate(runtime, &id, &fixture.url("/nav/popup-source")).await?;
    settled(runtime, &id, "/nav/popup-source").await?;
    let popup_witness=super::popup_pause_witness::Witness::install(&view).await?;
    let initial_popup_event=popup_witness.count();
    trusted_user_ready_popup_click(&view,"#popup","popup").await?;
    let popup = wait_popup(runtime, &id).await?;
    settled(runtime, &popup.target.tab_id, "/nav/popup").await?;
    exactly(fixture, "/nav/popup-source", 1)?;
    exactly(fixture, "/nav/popup", 1)?;
    let popup_view = app.get_webview(&popup.target.tab_id).ok_or("Missing native popup child")?;
    let popup_report = report(runtime, &popup.target.tab_id).await?;
    if popup_report.runtime.as_ref().and_then(|runtime| runtime.profile_identity_token.as_ref()) != environment.profile_identity_token.as_ref() {
        return Err("Popup did not use the same verified native Profile".into());
    }
    let popup_environment = evaluate(&popup_view, "({isTauri:'isTauri' in window,internals:'__TAURI_INTERNALS__' in window,ipc:'ipc' in window,ua:navigator.userAgent})").await?;
    if popup_environment["isTauri"] != false || popup_environment["internals"] != false || popup_environment["ipc"] != false || popup_environment["ua"] != ua {
        return Err(format!("Popup leaked host markers or changed identity: {popup_environment}"));
    }
    command(runtime, BrowserTabCommand::Close { target: tab(runtime, &popup.target.tab_id).await?.target }).await?;
    command(runtime, BrowserTabCommand::Activate { target: tab(runtime, &id).await?.target }).await?;
    require_user_initiated_popup_events(&popup_witness,initial_popup_event,"/nav/popup")?;
    let popup_hold=popup_hold(app,fixture,runtime,&id,&view,&popup_witness).await?;
    popup_witness.close().await?;
    let first_download=direct_initial_download(app,fixture,runtime,&id).await?;

    stage("http_status_and_document_outcome");
    let before_http_status = tab(runtime,&id).await?;
    let status_command=runtime.execute(BrowserTabCommand::Navigate{target:before_http_status.target,
        url:fixture.url("/nav/forbidden")},CancellationToken::new()).await;
    if let Err(error)=status_command {if error!=WorkspaceError::PageFailed {return Err(message(error));}}
    let http_status=wait_tab(runtime,&id,|tab|tab.load.as_ref().is_some_and(|load|
        matches!(load.phase,BrowserNavigationPhase::Finished|BrowserNavigationPhase::Failed)
            && load.response_status==Some(403))).await?;
    exactly(fixture,"/nav/forbidden",1)?;
    if load(&http_status)?.content_state!=BrowserContentState::CurrentDocument
        || evaluate(&view,"document.querySelector('h1')?.textContent").await? != "HTTP 403 fixture document" {
        return Err("HTTP 403 response lost its real rendered document or lacked accurate status proof".into());
    }

    stage("failure_stop_and_late_completion");
    let tls_fixture=super::navigation_tls::Fixture::new().await?;
    let tls_id=create_failure(runtime,&tls_fixture.url()).await?;
    let tls_failed=wait_tab(runtime,&tls_id,|tab|tab.lifecycle==BrowserTabLifecycle::Failed).await?;
    let tls_load=load(&tls_failed)?;
    let tls_witness=tls_fixture.witness()?;
    eprintln!("NAVIGATION_TLS_WITNESS {}",tls_witness);
    if tls_load.phase!=BrowserNavigationPhase::Failed || tls_load.content_state!=BrowserContentState::None
        || tls_load.problem.as_ref().is_none_or(|problem|problem.safe_reason!=BrowserFailureReason::Tls
            || problem.native_domain_class!=BrowserNativeDomainClass::WebView2) {
        return Err(format!("Real untrusted TLS certificate did not report native TLS failure without a current document; witness={tls_witness}"));
    }
    if tls_witness["tls_client_hellos"].as_u64().is_none_or(|count|count==0)
        || tls_witness["http_requests"]!=0 || tls_witness["completed_http_documents"]!=0
        || tls_witness["server_no_credentials_failures"]!=0 {
        return Err("TLS failure lacks a real ClientHello witness or incorrectly trusted the certificate".into());
    }
    let tls_problem=tls_load.problem.clone();
    let tls_state=json!({"phase":tls_load.phase,"content_state":tls_load.content_state});
    command(runtime,BrowserTabCommand::Close{target:tls_failed.target}).await?;
    let tls_cleanup=tls_fixture.close()?;
    eprintln!("NAVIGATION_TLS_VERIFIED {}",json!({"native_problem":tls_problem,"witness":tls_witness,
        "cleanup":tls_cleanup,"native_state":tls_state}));

    // .invalid is reserved for negative name resolution. A system proxy can
    // return a different outcome; retain the exact native classification rather
    // than substituting an unrelated port/error and claiming DNS coverage.
    let dns_id=create_failure(runtime,"http://nomifun-native-conformance.invalid/").await?;
    let dns_tab=wait_tab(runtime,&dns_id,|tab|tab.load.as_ref().is_some_and(|load|
        matches!(load.phase,BrowserNavigationPhase::Finished|BrowserNavigationPhase::Failed|BrowserNavigationPhase::Cancelled))).await?;
    let dns_load=load(&dns_tab)?;
    let dns_reason=dns_load.problem.as_ref().map(|problem|problem.safe_reason);
    let dns_verified=dns_load.phase==BrowserNavigationPhase::Failed && dns_reason==Some(BrowserFailureReason::Dns);
    if dns_load.phase==BrowserNavigationPhase::Failed && (dns_tab.lifecycle!=BrowserTabLifecycle::Failed || dns_load.problem.is_none()) {
        return Err("Negative name-resolution probe reported inaccurate native failure state".into());
    }
    let dns_outcome=json!({"reserved_invalid_domain":true,"dns_failure_observed":dns_verified,
        "native_phase":dns_load.phase,"native_lifecycle":dns_tab.lifecycle,"native_problem":dns_load.problem,
        "environment_limitation":if dns_verified{None}else{Some("This environment returned a different native outcome; DNS failure was not proved")}});
    command(runtime,BrowserTabCommand::Close{target:dns_tab.target}).await?;

    let failed_id = create_failure(runtime, &fixture.url("/nav/disconnect?fixture_secret=failure")).await?;
    let failed = wait_tab(runtime, &failed_id, |tab| tab.lifecycle == BrowserTabLifecycle::Failed).await?;
    let failure = load(&failed)?;
    if failure.phase != BrowserNavigationPhase::Failed || failure.problem.is_none() || failure.content_state != BrowserContentState::None {
        return Err("First failed navigation fabricated Ready or user content".into());
    }
    let failed_view=app.get_webview(&failed_id).ok_or("Missing retained failed native controller")?;
    if native_visibility(&failed_view).await? != (false,false) {
        return Err("Initial no-content failure was not masked at controller and HWND".into());
    }
    // Recover the same selected slot. No resize, menu open/close or tab change
    // can serve as an accidental visibility repair.
    let pending_runtime=runtime.clone();let pending_target=failed.target.clone();
    let recovery_url=fixture.url("/nav/recovery-slow");
    let pending_recovery=tokio::spawn(async move{pending_runtime.execute(
        BrowserTabCommand::Navigate{target:pending_target,url:recovery_url},CancellationToken::new()).await});
    wait_count(fixture,"/nav/recovery-slow",1).await?;
    let recovering=wait_tab(runtime,&failed_id,|tab|tab.load.as_ref().is_some_and(|load|
        matches!(load.phase,BrowserNavigationPhase::Requested|BrowserNavigationPhase::Provisional)
            && load.content_state==BrowserContentState::None)).await?;
    if native_visibility(&failed_view).await?!=(false,false) || recovering.target.tab_id!=failed_id {
        return Err("Failed-view recovery exposed the old native error document while no-content navigation was pending".into());
    }
    tokio::time::sleep(Duration::from_millis(180)).await;
    if load(&tab(runtime,&failed_id).await?)?.content_state!=BrowserContentState::None
        || native_visibility(&failed_view).await?!=(false,false) {
        return Err("Pending no-content recovery did not keep the native controller and HWND masked".into());
    }
    pending_recovery.await.map_err(message)?.map_err(message)?;
    let recovered=settled(runtime,&failed_id,"/nav/recovery-slow").await?;
    if recovered.target.tab_id!=failed_id || native_visibility(&failed_view).await? != (true,true)
        || evaluate(&failed_view,"document.querySelector('h1')?.textContent").await? != "/nav/recovery-slow" {
        return Err("Same-slot native recovery failed to restore the actual current page's visibility".into());
    }
    let recovery_input=trusted_native_text(&failed_view,"#recovery-input","fixtureRecoveryInputTrusted").await?;
    let recovery_screenshot=capture_fixture(runtime,&failed_id).await?;
    if recovery_screenshot["captured"]!=true {return Err("Same-slot recovery lacks a native current-document screenshot".into());}
    let crash_view=failed_view.clone();
    let mut crashed_command=tokio::spawn(async move {super::windows::protocol_call(&crash_view,"Page.crash",json!({})).await});
    let crashed=wait_tab(runtime,&failed_id,|tab|tab.lifecycle==BrowserTabLifecycle::Crashed).await?;
    if crashed.target.document_generation<=recovered.target.document_generation
        || native_visibility(&failed_view).await? != (false,false) {
        return Err("Native crash did not revoke document authority and mask the current controller/HWND".into());
    }
    navigate(runtime,&failed_id,&fixture.url("/nav/recovery")).await?;
    let recovered_crash=settled(runtime,&failed_id,"/nav/recovery").await?;
    if recovered_crash.target.tab_id!=failed_id || native_visibility(&failed_view).await? != (true,true)
        || evaluate(&failed_view,"document.querySelector('h1')?.textContent").await? != "/nav/recovery" {
        return Err("Same-slot recovery after native crash failed without a layout mutation".into());
    }
    let crash_screenshot=capture_fixture(runtime,&failed_id).await?;
    if crash_screenshot["captured"]!=true {return Err("Crash recovery lacks a current-document native screenshot".into());}
    command(runtime, BrowserTabCommand::Close { target: recovered_crash.target }).await?;
    if tokio::time::timeout(Duration::from_secs(2),&mut crashed_command).await.is_err() {
        // The destructive test command has no useful result after its owned
        // native controller is positively closed. Retire only this waiter.
        crashed_command.abort();let _=crashed_command.await;
    }
    let (stopped_id, pending_create) = create_in_flight(runtime, &fixture.url("/nav/slow")).await?;
    wait_count(fixture, "/nav/slow", 1).await?;
    let stop_tab = tab(runtime, &stopped_id).await?;
    command(runtime, BrowserTabCommand::StopLoading { expected_navigation_sequence:load(&stop_tab)?.navigation_sequence,
        target:stop_tab.target }).await?;
    let stopped = wait_tab(runtime, &stopped_id, |tab| tab.lifecycle == BrowserTabLifecycle::Stopped).await?;
    if load(&stopped)?.phase != BrowserNavigationPhase::Cancelled || load(&stopped)?.cancellation_reason != Some(BrowserCancellationReason::UserStop)
        || load(&stopped)?.content_state != BrowserContentState::None {
        return Err("Stop before first document did not produce cancelled/no-content evidence".into());
    }
    settle_interrupted(pending_create).await?;
    command(runtime, BrowserTabCommand::Close { target: stopped.target }).await?;

    let (paused_id, pending_pause_create) = create_in_flight(runtime, &fixture.url("/nav/slow-pause")).await?;
    wait_count(fixture, "/nav/slow-pause", 1).await?;
    let pause_tab = tab(runtime, &paused_id).await?;
    command(runtime, BrowserTabCommand::PauseNavigation { expected_navigation_sequence:load(&pause_tab)?.navigation_sequence,
        target:pause_tab.target }).await?;
    let paused_loading = wait_tab(runtime, &paused_id, |tab| tab.lifecycle == BrowserTabLifecycle::Stopped
        && tab.load.as_ref().is_some_and(|load| load.phase == BrowserNavigationPhase::Cancelled
            && load.navigation_control.as_ref().is_some_and(|control| control.paused))).await?;
    if load(&paused_loading)?.content_state != BrowserContentState::None {
        return Err("Pause of first in-flight navigation fabricated a user document".into());
    }
    settle_interrupted(pending_pause_create).await?;
    command(runtime, BrowserTabCommand::Close { target:tab(runtime, &paused_id).await?.target }).await?;

    // Retain a valid old document when the next provisional load fails.
    let original = tab(runtime, &id).await?;
    let failed_navigation = runtime.execute(BrowserTabCommand::Navigate { target: original.target.clone(),
        url:fixture.url("/nav/disconnect") }, CancellationToken::new()).await;
    if let Err(error) = failed_navigation { if error != WorkspaceError::PageFailed { return Err(message(error)); } }
    let retained = wait_tab(runtime, &id, |tab| tab.lifecycle == BrowserTabLifecycle::Failed).await?;
    let failed_load = load(&retained)?;
    if failed_load.phase != BrowserNavigationPhase::Failed || failed_load.content_state == BrowserContentState::CurrentDocument
        || match failed_load.content_state {
            BrowserContentState::RetainedDocument => failed_load.content_url != Some(original.url.clone()),
            BrowserContentState::None => failed_load.content_url.is_some(),
            BrowserContentState::CurrentDocument => true,
        } {
        return Err("Failed navigation claimed a current user document or mislabeled attempted URL as retained content".into());
    }
    navigate(runtime, &id, &fixture.url("/nav/race-final")).await?;
    let prior = settled(runtime, &id, "/nav/race-final").await?;
    stage("provisional_wire_guard");
    let prior_navigation_sequence=load(&prior)?.navigation_sequence;
    let pending_runtime=runtime.clone();
    let pending_target=prior.target.clone();
    let provisional_url=fixture.url("/nav/provisional-guard");
    let pending_guard=tokio::spawn(async move {pending_runtime.execute(
        BrowserTabCommand::Navigate {target:pending_target,url:provisional_url},CancellationToken::new()).await});
    wait_count(fixture, "/nav/provisional-guard", 1).await?;
    let provisional=wait_tab(runtime, &id, |tab| tab.load.as_ref().is_some_and(|load|
        load.phase==BrowserNavigationPhase::Provisional && load.navigation_sequence>prior_navigation_sequence)).await?;
    if provisional.target.document_generation!=prior.target.document_generation {
        return Err("Slow B did not preserve A document identity before ContentLoading; cannot prove provisional wire guard".into());
    }
    if load(&provisional)?.content_state!=BrowserContentState::RetainedDocument
        || native_visibility(&view).await?!=(true,true) {
        return Err("A readable retained document was hidden during a successor's provisional navigation".into());
    }
    let guard_report=report(runtime, &id).await?;
    for command in [
        BrowserTabCommand::StopLoading {target:provisional.target.clone(),expected_navigation_sequence:prior_navigation_sequence},
        BrowserTabCommand::Reload {target:provisional.target.clone(),expected_navigation_sequence:prior_navigation_sequence},
    ] {
        if !matches!(runtime.execute(command,CancellationToken::new()).await,Err(WorkspaceError::StaleTarget)) {
            return Err("Old A wire sequence controlled B while the document target was unchanged".into());
        }
    }
    let guarded_after=tab(runtime, &id).await?;
    if load(&guarded_after)?.phase!=BrowserNavigationPhase::Provisional
        || load(&guarded_after)?.navigation_sequence!=load(&provisional)?.navigation_sequence {
        return Err("Rejected provisional wire control changed B phase or attempt".into());
    }
    exactly(fixture, "/nav/provisional-guard", 1)?;
    require_host_delta(&report(runtime, &id).await?,&guard_report,0)?;
    command(runtime,BrowserTabCommand::StopLoading {target:guarded_after.target.clone(),
        expected_navigation_sequence:load(&guarded_after)?.navigation_sequence}).await?;
    settle_interrupted(pending_guard).await?;
    navigate(runtime, &id, &fixture.url("/nav/race-final")).await?;
    let prior=settled(runtime, &id, "/nav/race-final").await?;
    let stale_receipt = super::windows::navigation::receipt_for_target(&view,prior.target.clone(),
        Some(load(&prior)?.navigation_sequence)).await.map_err(message)?;
    // Exercise actual overlapping production Runtime commands. The native
    // owner, not callback arrival order, decides the current attempt.
    let pending_runtime = runtime.clone();
    let pending_target = prior.target.clone();
    let slow_url = fixture.url("/nav/slow-late");
    let pending_navigation = tokio::spawn(async move {pending_runtime.execute(
        BrowserTabCommand::Navigate{target:pending_target,url:slow_url},CancellationToken::new()).await});
    wait_count(fixture, "/nav/slow-late", 1).await?;
    navigate(runtime, &id, &fixture.url("/nav/final")).await?;
    let race_final = settled(runtime, &id, "/nav/final").await?;
    settle_interrupted(pending_navigation).await?;
    let sequence = load(&race_final)?.navigation_sequence;
    tokio::time::sleep(Duration::from_millis(1600)).await;
    let after_late = tab(runtime, &id).await?;
    if load(&after_late)?.navigation_sequence != sequence || after_late.url != race_final.url || after_late.lifecycle != BrowserTabLifecycle::Ready {
        return Err("Late predecessor response overwrote the succeeding document".into());
    }
    let stale_navigation_sequence=load(&prior)?.navigation_sequence;
    for stale in [BrowserTabCommand::StopLoading { target: prior.target.clone(),expected_navigation_sequence:stale_navigation_sequence },
        BrowserTabCommand::Reload { target: prior.target,expected_navigation_sequence:stale_navigation_sequence }] {
        if !matches!(runtime.execute(stale, CancellationToken::new()).await, Err(WorkspaceError::StaleTarget)) {
            return Err("An old document target changed the succeeding navigation".into());
        }
    }
    let before_stale_dispatch = report(runtime, &id).await?;
    let final_requests = fixture.count("/nav/final");
    if super::windows::stop_navigation(&view, stale_receipt).await.is_ok() {
        return Err("A delayed native Stop receipt stopped its successor".into());
    }
    let stale_reload = super::windows::NavigationDispatch { source:BrowserNavigationSource::UserCommand,
        url:None, action:BrowserNavigationActionType::Reload, receipt:Arc::new(std::sync::Mutex::new(None)),
        rejected_before_unload:Arc::new(std::sync::atomic::AtomicBool::new(false)),
        cancel:CancellationToken::new(), closing:CancellationToken::new() };
    if super::windows::protocol_navigation(&view, "Page.reload", json!({}), stale_reload, Some(stale_receipt)).await.is_ok() {
        return Err("A delayed native Reload receipt reloaded its successor".into());
    }
    tokio::time::sleep(Duration::from_millis(150)).await;
    require_host_delta(&report(runtime, &id).await?, &before_stale_dispatch, 0)?;
    exactly(fixture, "/nav/final", final_requests)?;
    exactly(fixture, "/nav/slow-late", 1)?;

    stage("hidden_late_commit");
    let late_runtime=runtime.clone();
    let late_target=tab(runtime,&id).await?.target;
    let late_url=fixture.url("/nav/late-hidden");
    let pending_hidden=tokio::spawn(async move {late_runtime.execute(
        BrowserTabCommand::Navigate{target:late_target,url:late_url},CancellationToken::new()).await});
    wait_count(fixture,"/nav/late-hidden",1).await?;
    surface(runtime,false,1060.0).await?;
    if native_visibility(&view).await? != (false,false) {return Err("Panel hide did not conceal native controller and HWND".into());}
    settle_interrupted(pending_hidden).await?;
    settled(runtime,&id,"/nav/late-hidden").await?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    if native_visibility(&view).await? != (false,false) {return Err("Late successful commit resurrected a hidden native panel".into());}
    exactly(fixture,"/nav/late-hidden",1)?;
    surface(runtime,true,1060.0).await?;
    if native_visibility(&view).await? != (true,true) {return Err("Explicit panel show failed after hidden late commit".into());}

    stage("bounded_diagnostics");
    let before_capacity = report(runtime, &id).await?;
    for n in 0..40 {
        navigate(runtime, &id, &fixture.url(&format!("/nav/bounded?case={n}&fixture_secret=capacity"))).await?;
        settled(runtime, &id, "/nav/bounded").await?;
    }
    exactly(fixture, "/nav/bounded", 40)?;
    let final_report = report(runtime, &id).await?;
    require_host_delta(&final_report, &before_capacity, 40)?;
    if final_report.trace.events.len() > MAX_NAVIGATION_EVENTS || final_report.trace.dropped == 0
        || serde_json::to_vec(&final_report.trace.events).map_err(message)?.len() > MAX_NAVIGATION_TRACE_BYTES {
        return Err("Navigation ring did not retain independent counters while bounding events".into());
    }
    assert_redacted(&final_report)?;
    if !matches!(runtime.navigation_diagnostics(first.target).await, Err(WorkspaceError::StaleTarget)) {
        return Err("Navigation diagnostics accepted a stale document target".into());
    }
    let screenshot=capture_fixture(runtime,&id).await?;
    stage("workspace_interrupt_admission");
    surface(runtime,false,1060.0).await?;
    let workspace_interrupt = workspace_interrupt(app,fixture).await?;
    let main_receipts = fixture.snapshot().iter().filter(|receipt| receipt.destination == "document").count();
    let failed_recovery=json!({"pending_no_content_controller_hidden":true,"pending_no_content_hwnd_hidden":true,"controller_visible":true,"hwnd_visible":true,"trusted_native_input_after_commit":recovery_input,"screenshot":recovery_screenshot});
    let mut evidence=json!({
        "platform":"windows_webview2", "production_native_runtime":true,
        "independent_http_document_requests":main_receipts,
        "single_initial_and_explicit_reload":true, "passive_updates_do_not_navigate":true,
        "redirects_one_native_attempt":true, "script_reload_replace_meta":true,
        "loop_pause_resume":loop_evidence, "aba_pause_resume":aba_evidence,
        "post_pause_resume":post_resume_evidence,
        "resource_traffic_not_main_navigation":true,
        "page_frame_worker_identity_consistent":true, "external_host_markers_absent":true,
        "worker_baseline":{"window_strong_reference_intervention":false,"native_witness":resource_worker_witness},
        "nested_dedicated_worker":{"script_http_requests":1,"execution_http_side_effects":1,"message_relay_and_native_identity":true},
        "standard_dialog_api_present":true, "popup_one_request_and_clean_environment":true,
        "http_403":{"status":403,"native_phase":load(&http_status)?.phase,"native_lifecycle":http_status.lifecycle,
            "rendered_document_verified":true,"counted_as_normal_site_access":false},
        "initial_failure_no_fake_ready":true, "initial_stop_no_content":true,
        "untrusted_tls":{"native_problem":tls_problem,"witness":tls_witness,"cleanup":tls_cleanup},
        "dns":dns_outcome,
        "same_slot_failed_view_recovery":failed_recovery,
        "same_slot_crashed_view_recovery":{"controller_visible":true,"hwnd_visible":true,"screenshot":crash_screenshot},
        "hidden_panel_late_commit_cannot_resurrect":true,
        "pause_cancels_current_first_load":true,
        "workspace_interrupt_admission":workspace_interrupt,
        "failure_content_state_explicit":true, "late_predecessor_cannot_overwrite":true,
        "provisional_wire_sequence_guard_with_unchanged_document":true,
        "overlap_source":"production_runtime_predecessor_and_successor",
        "stale_stop_reload_rejected":true, "delayed_native_receipt_rejected_before_dispatch":true,
        "bounded_redacted_diagnostics":true,
        "routing_loss_fault_injection":"unit_owner_tests_required",
        "native_download":"separate_native_conformance_required",
        "profile_restart_isolation":"separate_persistent_profile_conformance_required",
        "real_sites":"not_exercised_by_local_fixture", "effective_environment":environment, "navigation_report":final_report,
        "final_native_fixture_screenshot":screenshot,
    });
    evidence["popup_pause_trusted_user_ready"]=popup_hold;
    evidence["first_direct_download_without_document"]=first_download;
    evidence["retained_document_visible_during_pending_navigation"]=json!(true);
    Ok(evidence)
}

async fn trusted_user_ready_popup_click(view:&tauri::Webview,selector:&str,target:&str)->Result<Value,String> {
    if native_visibility(view).await? != (true,true) {
        return Err("Popup gesture requires an actually visible native opener".into());
    }
    if !native_hwnd_enabled(view).await? {return Err("Popup gesture was attempted under an Agent/native user input lock".into());}
    let before=evaluate(view,"({nonce:fixtureDocumentNonce,count:fixturePopupClicks.length})").await?;
    // Existing fixture input uses real browser Input.dispatchMouseEvent. It
    // does not invoke JS click/window.open or change the Runtime input owner.
    super::native_fixture_click(view,selector).await?;
    let after=evaluate(view,"({nonce:fixtureDocumentNonce,count:fixturePopupClicks.length,last:fixturePopupClicks.at(-1)})").await?;
    if before["nonce"]!=after["nonce"] || after["count"].as_u64()!=before["count"].as_u64().map(|count|count+1)
        || after["last"]["target"]!=target || after["last"]["trusted"]!=true {
        return Err(format!("Native UserReady popup input lacks an exact trusted DOM witness: {after}"));
    }
    Ok(json!({"native_user_input_enabled":true,"controller_and_hwnd_visible":true,"trusted_dom_click":true,"target":target}))
}

async fn native_hwnd_enabled(view:&tauri::Webview)->Result<bool,String> {
    let (tx,rx)=tokio::sync::oneshot::channel();
    view.with_webview(move|platform|{
        let result=(||->::windows::core::Result<bool>{
            let mut hwnd=::windows::Win32::Foundation::HWND::default();
            unsafe{platform.controller().ParentWindow(&mut hwnd)?;
                Ok(::windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled(hwnd).as_bool())}
        })().map_err(message);let _=tx.send(result);
    }).map_err(message)?;
    rx.await.map_err(message)?
}

fn require_user_initiated_popup_events(witness:&super::popup_pause_witness::Witness,start:usize,path:&str)->Result<Value,String> {
    let evidence=witness.since(start);
    let events=evidence["native_events"].as_array().ok_or("Native popup event witness missing")?;
    if events.len()!=1 || events[0]["path"]!=path || events[0]["is_user_initiated"]!=true || events[0]["host_handled"]!=true {
        return Err(format!("Popup request did not reach the native host with real user activation: {evidence}"));
    }
    Ok(evidence)
}

async fn trusted_native_text(view:&tauri::Webview,selector:&str,flag:&str)->Result<Value,String> {
    if native_visibility(view).await?!=(true,true){return Err("Recovered document is not actually visible for native typing".into());}
    if !native_hwnd_enabled(view).await? {return Err("Visible document remains locked against native user input".into());}
    super::native_fixture_click(view,selector).await?;
    super::windows::protocol_call(view,"Input.insertText",json!({"text":"native-fixture-entry"})).await?;
    let value=evaluate(view,&format!("({{value:document.querySelector({selector:?})?.value,trusted:window[{flag:?}],focused:document.activeElement===document.querySelector({selector:?})}})")).await?;
    if value["value"]!="native-fixture-entry" || value["trusted"]!=true || value["focused"]!=true {
        return Err(format!("Visible current document did not receive native trusted input: {value}"));
    }
    Ok(json!({"native_trusted_input":true,"actual_input_value_verified":true,"current_input_focus_verified":true}))
}

async fn direct_initial_download(app:&tauri::AppHandle,fixture:&Fixture,runtime:&Arc<dyn BrowserRuntime>,original:&str)->Result<Value,String> {
    stage("first_direct_download_no_document");
    let id=create(runtime,&fixture.url("/nav/download")).await?;
    let view=app.get_webview(&id).ok_or("Missing first direct-download controller")?;
    let deadline=Instant::now()+Duration::from_secs(8);
    let picker=loop {
        if let Some(picker)=super::windows::user_downloads::inspect(&view).await?.0 {picker.opened().await?;break picker;}
        if Instant::now()>=deadline {return Err("First direct attachment did not present its native Save chooser".into());}
        tokio::time::sleep(Duration::from_millis(15)).await;
    };
    let receipt=picker.exit_receipt().await?;
    let target=tab(runtime,&id).await?;
    if load(&target)?.content_state!=BrowserContentState::None || native_visibility(&view).await?!=(false,false) {
        return Err("First direct attachment fabricated a document or exposed empty/error native content".into());
    }
    exactly(fixture,"/nav/download",1)?;
    let snapshot=runtime.snapshot().await.map_err(message)?;
    let download=snapshot.downloads.iter().find(|download|download.tab_id==id&&download.state==BrowserDownloadState::Choosing&&download.can_cancel)
        .ok_or("First direct attachment lacked its cancellable native download owner")?;
    command(runtime,BrowserTabCommand::CancelDownload{target:target.target,download_id:download.id.clone()}).await?;
    tokio::time::timeout(Duration::from_secs(5),receipt.wait()).await.map_err(|_|"First direct-download chooser did not exit after explicit cancel")?.map_err(message)?;
    super::windows::user_downloads::cancel_and_wait(&view,false).await?;
    command(runtime,BrowserTabCommand::Close{target:tab(runtime,&id).await?.target}).await?;
    command(runtime,BrowserTabCommand::Activate{target:tab(runtime,original).await?.target}).await?;
    Ok(json!({"initial_main_attachment_get":1,"content_state_none":true,"controller_and_hwnd_hidden":true,
        "native_save_chooser_presented_independently":true,"explicit_cancel_awaited_native_chooser_exit":true,"saved_file_bytes":"not_claimed"}))
}

async fn writable_production_blank_popup(app:&tauri::AppHandle,runtime:&Arc<dyn BrowserRuntime>,id:&str,view:&tauri::Webview,
    witness:&super::popup_pause_witness::Witness)->Result<Value,String> {
    stage("production_writable_blank_popup_visibility");
    let start=witness.count();trusted_user_ready_popup_click(view,"#writable-popup","writable-popup").await?;
    let child=wait_popup(runtime,id).await?;
    let child_view=app.get_webview(&child.target.tab_id).ok_or("Missing production writable blank popup controller")?;
    let deadline=Instant::now()+Duration::from_secs(5);
    let ready=loop {
        let tab=tab(runtime,&child.target.tab_id).await?;
        let body=evaluate(&child_view,"({written:document.getElementById('written-native')?.textContent,input:!!document.getElementById('written-input')})").await;
        let visibility=native_visibility(&child_view).await?;let enabled=native_hwnd_enabled(&child_view).await?;
        if tab.load.as_ref().is_some_and(|load|load.content_state==BrowserContentState::CurrentDocument)
            && body.as_ref().is_ok_and(|body|body["written"]=="Written by opener"&&body["input"]==true)
            && visibility==(true,true) && enabled {break tab;}
        if Instant::now()>=deadline {return Err(format!("Production writable about:blank popup lacks usable native document: load={:?},body={body:?},visibility={visibility:?},enabled={enabled}",tab.load));}
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    if native_visibility(&child_view).await?!=(true,true) {return Err("Production writable about:blank popup remained masked after real document creation".into());}
    let input=trusted_native_text(&child_view,"#written-input","fixtureBlankInputTrusted").await?;
    let native_event=require_user_initiated_popup_events(witness,start,"blank")?;
    command(runtime,BrowserTabCommand::Close{target:ready.target}).await?;
    command(runtime,BrowserTabCommand::Activate{target:tab(runtime,id).await?.target}).await?;
    Ok(json!({"production_host_popup":true,"about_blank_is_real_user_document":true,"non_http_document_written_by_opener":true,
        "controller_visible":true,"hwnd_visible":true,"trusted_native_input":input,"native_request":native_event}))
}

async fn popup_hold(app:&tauri::AppHandle,fixture:&Fixture,runtime:&Arc<dyn BrowserRuntime>,id:&str,view:&tauri::Webview,
    witness:&super::popup_pause_witness::Witness)->Result<Value,String> {
    stage("trusted_user_ready_popup_pause");
    // The prior positive named child has been really closed. Repeating its
    // name now must test fresh popup admission, matching the actual GUI leak.
    control(runtime,id,true).await?;
    let paused=tab(runtime,id).await?;
    if load(&paused)?.navigation_control.as_ref().is_none_or(|control|!control.paused){return Err("Popup opener pause was not acknowledged".into());}
    let retained_nonce=evaluate(view,"fixtureDocumentNonce").await?;
    let before=runtime.snapshot().await.map_err(message)?;
    let expected_tabs:BTreeSet<_>=before.tabs.into_iter().map(|tab|tab.target.tab_id).collect();
    let expected_labels:BTreeSet<_>=app.webviews().keys().cloned().collect();
    let named_before=fixture.count("/nav/popup");let blank_before=fixture.count("/nav/popup-blank");
    let mut rejected=Vec::new();
    for (selector,target,path) in [("#script-popup","script-popup","/nav/popup"),("#blank-popup","blank-popup","/nav/popup-blank")] {
        let before_report=report(runtime,id).await?;
        let start=witness.count();
        let click=trusted_user_ready_popup_click(view,selector,target).await?;
        let deadline=Instant::now()+Duration::from_secs(3);
        while witness.count()==start {
            if Instant::now()>=deadline {return Err("Trusted paused popup lacks a readable native host request; pause admission was not proved".into());}
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let event=require_user_initiated_popup_events(witness,start,path)?;
        // Sample actual runtime, native registrations and HTTP receipts over
        // a bounded 600ms window; any page observed in that window is failure.
        // This is not proof about arbitrary delayed tasks or shorter transients.
        let until=Instant::now()+Duration::from_millis(600);
        loop {
            let current=runtime.snapshot().await.map_err(message)?;
            let tabs:BTreeSet<_>=current.tabs.into_iter().map(|tab|tab.target.tab_id).collect();
            let labels:BTreeSet<_>=app.webviews().keys().cloned().collect();
            if tabs!=expected_tabs || labels!=expected_labels || current.active_tab_id.as_deref()!=Some(id)
                || fixture.count("/nav/popup")!=named_before || fixture.count("/nav/popup-blank")!=blank_before {
                return Err(format!("Trusted paused popup escaped native admission: event={event}; named_get_before={named_before}; named_get_after={}; blank_get_before={blank_before}; blank_get_after={}",fixture.count("/nav/popup"),fixture.count("/nav/popup-blank")));
            }
            if Instant::now()>=until {break;}
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let current_parent=tab(runtime,id).await?;
        if evaluate(view,"fixtureDocumentNonce").await?!=retained_nonce || current_parent.target!=paused.target
            || load(&current_parent)?.navigation_control.as_ref().is_none_or(|control|!control.paused) {
            return Err("Rejected popup changed the retained opener document or released its pause".into());
        }
        let after_report=report(runtime,id).await?;
        let policies:Vec<_>=since(&after_report,&before_report).into_iter().filter(|event|
            event.kind==BrowserNavigationEventKind::Action && event.source==BrowserNavigationSource::PageNavigation
                && event.navigation_sequence.is_none() && event.document_generation.is_none()
                && event.action.as_ref().is_some_and(|action|!action.main_frame && action.policy==BrowserNavigationPolicy::Paused
                    && action.user_initiated==Some(true))
                && event.url.as_deref().and_then(|url|url::Url::parse(url).ok()).is_some_and(|url|url.path()==path)).collect();
        if policies.len()!=1 {return Err("Paused popup lacks exact native host denial Action witness; GET zero alone does not prove admission".into());}
        rejected.push(json!({"input":click,"native_request":event,"host_pause_denial_action":policies[0],"native_request_document_association":"unavailable","parent_document_snapshot_unchanged":true,"http_get_delta":0,"runtime_tabs_unchanged":true,"native_views_unchanged":true,"retained_opener_nonce_unchanged":true}));
    }
    control(runtime,id,false).await?;
    let resumed_event=witness.count();trusted_user_ready_popup_click(view,"#script-popup","script-popup").await?;
    let resumed=wait_popup(runtime,id).await?;
    settled(runtime,&resumed.target.tab_id,"/nav/popup").await?;
    exactly(fixture,"/nav/popup",named_before+1)?;
    let resumed_native=require_user_initiated_popup_events(witness,resumed_event,"/nav/popup")?;
    command(runtime,BrowserTabCommand::Close{target:tab(runtime,&resumed.target.tab_id).await?.target}).await?;
    command(runtime,BrowserTabCommand::Activate{target:tab(runtime,id).await?.target}).await?;
    control(runtime,id,true).await?;
    navigate(runtime,id,&fixture.url("/nav/popup-source")).await?;
    settled(runtime,id,"/nav/popup-source").await?;
    if load(&tab(runtime,id).await?)?.navigation_control.as_ref().is_none_or(|control|control.paused){return Err("Explicit navigation did not release popup hold".into());}
    let explicit_event=witness.count();trusted_user_ready_popup_click(view,"#blank-popup","blank-popup").await?;
    let explicit=wait_popup(runtime,id).await?;
    settled(runtime,&explicit.target.tab_id,"/nav/popup-blank").await?;
    exactly(fixture,"/nav/popup-blank",blank_before+1)?;
    let explicit_native=require_user_initiated_popup_events(witness,explicit_event,"/nav/popup-blank")?;
    command(runtime,BrowserTabCommand::Close{target:tab(runtime,&explicit.target.tab_id).await?.target}).await?;
    command(runtime,BrowserTabCommand::Activate{target:tab(runtime,id).await?.target}).await?;
    let writable=writable_production_blank_popup(app,runtime,id,view,witness).await?;
    Ok(json!({"closed_named_target_recreation_and_blank_target":rejected,"resume_permits_named_popup":resumed_native,
        "explicit_navigation_permits_blank_popup":explicit_native,"production_writable_about_blank_popup":writable,"no_agent_input_lock_or_script_click_used":true}))
}

async fn capture_fixture(runtime:&Arc<dyn BrowserRuntime>,id:&str)->Result<Value,String> {
    runtime.lock_user_input().await.map_err(message)?;
    let captured=runtime.automation().ok_or("Missing native screenshot port")?
        .screenshot(Some(id.into()),CancellationToken::new()).await;
    runtime.unlock_user_input().await.map_err(message)?;
    let captured=match captured {Ok(captured)=>captured,Err(error)=>return Ok(json!({"captured":false,"error_code":error.code()}))};
    let directory=std::env::temp_dir().join(format!("nomifun-browser-navigation-{}",uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&directory).map_err(message)?;
    let path=directory.join("final-native-fixture.png");
    let png=base64::engine::general_purpose::STANDARD.decode(&captured.png_base64).map_err(message)?;
    std::fs::write(&path,png).map_err(message)?;
    let receipt=json!({"captured":true,"path":path,"width":captured.width,"height":captured.height,
        "target":captured.target,"source":"owned_native_screenshot_port"});
    eprintln!("NAVIGATION_NATIVE_SCREENSHOT {}",receipt);
    Ok(receipt)
}

async fn native_visibility(view:&tauri::Webview)->Result<(bool,bool),String> {
    let (tx,rx)=tokio::sync::oneshot::channel();
    view.with_webview(move|platform|{
        let result=(||->::windows::core::Result<(bool,bool)>{
            let mut visible=::windows::core::BOOL::default();
            let mut hwnd=::windows::Win32::Foundation::HWND::default();
            unsafe {platform.controller().IsVisible(&mut visible)?;platform.controller().ParentWindow(&mut hwnd)?;
                Ok((visible.as_bool(),::windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(hwnd).as_bool()))}
        })().map_err(message);
        let _=tx.send(result);
    }).map_err(message)?;
    rx.await.map_err(message)?
}

async fn resource_visibility(app:&tauri::AppHandle,view:&tauri::Webview)->Result<Value,String> {
    let (controller_visible,container_visible)=native_visibility(view).await?;
    let main=app.get_window("main").ok_or("Missing controlled native main window")?;
    let main_visible=main.is_visible().map_err(message)?;
    let main_minimized=main.is_minimized().map_err(message)?;
    let (tx,rx)=tokio::sync::oneshot::channel();
    view.with_webview(move|platform| {
        let result=(||->::windows::core::Result<Vec<bool>> {
            let mut hwnd=::windows::Win32::Foundation::HWND::default();
            unsafe {platform.controller().ParentWindow(&mut hwnd)?;}
            let mut ancestors=Vec::new();
            for _ in 0..8 {
                let Ok(parent)= (unsafe {::windows::Win32::UI::WindowsAndMessaging::GetParent(hwnd)}) else {break};
                if parent.0.is_null() {break;}
                ancestors.push(unsafe {::windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(parent)}.as_bool());
                hwnd=parent;
            }
            Ok(ancestors)
        })().map_err(message);let _=tx.send(result);
    }).map_err(message)?;
    let ancestor_visible=rx.await.map_err(message)??;
    let document=evaluate(view,"({visibility:document.visibilityState,hidden:document.hidden})").await?;
    Ok(json!({"controller_visible":controller_visible,"container_visible":container_visible,
        "ancestor_windows_visible":ancestor_visible,"main_window_visible":main_visible,"main_window_minimized":main_minimized,
        "document":document,"visibility_intervention":false}))
}

async fn workspace_interrupt(app:&tauri::AppHandle,fixture:&Fixture)->Result<Value,String> {
    let service=BrowserResourceService::new(Arc::new(super::host::DesktopBrowserHost::new(app.clone())));
    let authority=super::browser_resource_fixture::authority("navigation-workspace","interrupt-admission","native-interrupt");
    let key=authority.key();
    let workspace=service.ensure(authority,BrowserProfile::Ephemeral).await.map_err(message)?;
    let result=async {
        workspace.set_surface(BrowserSurfaceBounds{x:20.0,y:60.0,width:1060.0,height:640.0},true,CancellationToken::new()).await.map_err(message)?;
        let mut cases=Vec::new();
        for (path,pause) in [("/nav/workspace-stop",false),("/nav/workspace-pause",true)] {
            let pending_workspace=workspace.clone();
            let url=fixture.url(path);
            let pending=tokio::spawn(async move {pending_workspace.user_command(BrowserTabCommand::Create{url}).await});
            wait_count(fixture,path,1).await?;
            let snapshot=workspace.snapshot().await.map_err(message)?.runtime.ok_or("Pending Workspace Create has no runtime")?;
            let target=snapshot.tabs.into_iter().find(|tab|tab.load.as_ref().is_some_and(|load|
                load.requested_url.as_ref().is_some_and(|url|url.ends_with(path))))
                .ok_or("Pending Workspace Create has no current native attempt")?;
            let expected_navigation_sequence=load(&target)?.navigation_sequence;
            let control=if pause {BrowserTabCommand::PauseNavigation{target:target.target.clone(),expected_navigation_sequence}}
                else {BrowserTabCommand::StopLoading{target:target.target.clone(),expected_navigation_sequence}};
            let started=Instant::now();
            let interrupted=tokio::time::timeout(Duration::from_secs(2),workspace.user_command(control)).await
                .map_err(|_|"Workspace interrupt waited behind the slow original navigation")?.map_err(message)?;
            settle_interrupted(pending).await?;
            let tab=interrupted.tabs.into_iter().find(|tab|tab.target.tab_id==target.target.tab_id)
                .ok_or("Workspace interrupt retired the owned initial tab")?;
            let native_load=load(&tab)?;
            if tab.lifecycle!=BrowserTabLifecycle::Stopped || native_load.phase!=BrowserNavigationPhase::Cancelled
                || native_load.content_state!=BrowserContentState::None
                || native_load.navigation_control.as_ref().is_none_or(|control|control.paused!=pause) {
                return Err("Workspace interrupt returned without exact native cancelled/hold proof".into());
            }
            exactly(fixture,path,1)?;
            cases.push(json!({"command":if pause{"pause_navigation"}else{"stop_loading"},
                "settled_ms":started.elapsed().as_millis(),"initial_tab_retained":true,"original_operation_joined":true}));
            workspace.user_command(BrowserTabCommand::Close{target:tab.target}).await.map_err(message)?;
        }
        Ok::<Value,String>(json!({"cases":cases,"real_user_command_coordinator":true}))
    }.await;
    let closed=service.close(&key).await.map_err(message);
    match (result,closed) {(Ok(value),Ok(()))=>Ok(value),(Err(error),Ok(()))=>Err(error),
        (Ok(_),Err(cleanup))=>Err(format!("Workspace fixture cleanup: {cleanup}")),
        (Err(error),Err(cleanup))=>Err(format!("{error}; Workspace fixture cleanup: {cleanup}"))}
}

async fn post_resume(fixture:&Fixture,runtime:&Arc<dyn BrowserRuntime>,id:&str,view:&tauri::Webview)->Result<Value,String> {
    stage("post_resume_without_releasing_old_requests");
    navigate(runtime,id,&fixture.url("/nav/post-source")).await?;
    settled(runtime,id,"/nav/post-source").await?;
    control(runtime,id,true).await?;
    let deadline=Instant::now()+Duration::from_secs(4);
    let paused_ordinal=loop {
        let value=evaluate(view,"fixturePostOrdinal").await?.as_u64().ok_or("Missing controlled POST ordinal")?;
        if value>=6 {break value;}
        if Instant::now()>=deadline {return Err("Paused page did not produce the controlled POST navigation attempts".into());}
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    exactly(fixture,"/nav/post-receive",0)?;
    control(runtime,id,false).await?;
    wait_count(fixture,"/nav/post-receive",1).await?;
    settled(runtime,id,"/nav/post-receive").await?;
    let posted:Vec<_>=fixture.snapshot().into_iter().filter(|receipt|receipt.path=="/nav/post-receive").collect();
    if posted.iter().any(|receipt|receipt.method!="POST" || receipt.post_attempt.is_none_or(|ordinal|ordinal<=paused_ordinal)) {
        return Err("Resume released a POST produced while the page was held".into());
    }
    Ok(json!({"paused_post_attempts_not_forwarded":true,"resume_marker":paused_ordinal,
        "forwarded_only_future_attempts":posted.into_iter().filter_map(|receipt|receipt.post_attempt).collect::<Vec<_>>(),
        "request_body_not_recorded_or_replayed":true}))
}

async fn pause_loop(app: &tauri::AppHandle, fixture: &Fixture, runtime: &Arc<dyn BrowserRuntime>, id: &str, start: &str, paths: &[&str]) -> Result<Value, String> {
    let before = report(runtime, id).await?;
    navigate(runtime, id, &fixture.url(start)).await?;
    wait_tab(runtime, id, |tab| tab.load.as_ref().and_then(|load| load.navigation_control.as_ref())
        .is_some_and(|control| control.loop_suspected)).await?;
    // Target generations can change between a read and Pause. Retry only this
    // idempotent control on StaleTarget; never replay a website mutation.
    control(runtime, id, true).await?;
    let paused = wait_tab(runtime, id, |tab| tab.load.as_ref().and_then(|load| load.navigation_control.as_ref())
        .is_some_and(|control| control.paused)).await?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let stable = tab(runtime, id).await?;
    let view = app.get_webview(id).ok_or("Missing paused native document")?;
    let held_document = evaluate(&view, "({path:location.pathname,title:document.title,body:document.querySelector('h1')?.textContent,nonce:fixtureDocumentNonce})").await?;
    if !paths.contains(&held_document["path"].as_str().unwrap_or(""))
        || held_document["title"] != held_document["path"] || held_document["body"] != held_document["path"]
        || held_document["nonce"].as_str().is_none_or(|nonce|nonce.len()!=36) {
        return Err("Pause did not preserve the actually readable fixture document".into());
    }
    let paused_count: usize = paths.iter().map(|path| fixture.count(path)).sum();
    tokio::time::sleep(Duration::from_millis(850)).await;
    let after_count: usize = paths.iter().map(|path| fixture.count(path)).sum();
    if paused_count != after_count { return Err(format!("Native Pause allowed future main requests at {start}: before={paused_count}, after={after_count}, receipts={:?}",
        fixture.snapshot().into_iter().filter(|receipt|paths.contains(&receipt.path.as_str())).map(|receipt|(receipt.path,receipt.destination,receipt.method)).collect::<Vec<_>>())); }
    if tab(runtime, id).await?.target.document_generation != stable.target.document_generation
        || evaluate(&view, "({path:location.pathname,title:document.title,body:document.querySelector('h1')?.textContent,nonce:fixtureDocumentNonce})").await? != held_document {
        return Err("Held main document changed while native Pause was active".into());
    }
    if load(&paused)?.content_state == BrowserContentState::None { return Err("Pause lost the fixture's retained document".into()); }
    let held = report(runtime, id).await?;
    if !held.trace.events.iter().any(|event| event.kind == BrowserNavigationEventKind::Action
        && event.action.as_ref().is_some_and(|action| action.policy == BrowserNavigationPolicy::Paused)) {
        return Err("Paused timer navigation did not leave a native policy witness".into());
    }
    require_host_delta(&held, &before, 1)?;
    control(runtime, id, false).await?;
    wait_tab(runtime, id, |tab| tab.load.as_ref().and_then(|load| load.navigation_control.as_ref())
        .is_some_and(|control| !control.paused)).await?;
    let deadline = Instant::now() + Duration::from_secs(3);
    while paths.iter().map(|path| fixture.count(path)).sum::<usize>() == paused_count {
        if Instant::now() >= deadline { return Err("Resume failed to permit the retained page's next timer navigation".into()); }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    // An explicit host navigation releases the hold and leaves the loop.
    control(runtime, id, true).await?;
    navigate(runtime, id, &fixture.url("/nav/initial")).await?;
    let recovered = settled(runtime, id, "/nav/initial").await?;
    if load(&recovered)?.navigation_control.as_ref().is_none_or(|control| control.paused) {
        return Err("Explicit navigation did not release native Pause".into());
    }
    Ok(json!({"warning_observed":true,"paused_request_count":paused_count,"future_requests_blocked":true,
        "blocked_policy_witness":true,"readable_document_preserved":true,
        "resume_permits_page_timer":true,"explicit_navigation_releases_hold":true}))
}

fn stage(name: &str) { eprintln!("BROWSER_NAVIGATION_STAGE {name}"); }
fn load(tab: &BrowserTabSnapshot) -> Result<&BrowserLoadSummary, String> { tab.load.as_ref().ok_or_else(|| "Native navigation summary missing".into()) }
fn host_count(report: &BrowserNavigationReport) -> u64 { report.trace.counters.user_commands + report.trace.counters.agent_commands }
fn require_host_commands(report: &BrowserNavigationReport, expected: u64) -> Result<(), String> {
    if host_count(report) != expected { return Err(format!("Host command count {} differs from {expected}", host_count(report))); } Ok(())
}
fn require_host_delta(after: &BrowserNavigationReport, before: &BrowserNavigationReport, expected: u64) -> Result<(), String> {
    if host_count(after).saturating_sub(host_count(before)) != expected { return Err("Repeated host dispatch or missing command witness".into()); } Ok(())
}
fn since<'a>(after: &'a BrowserNavigationReport, before: &BrowserNavigationReport) -> Vec<&'a BrowserNavigationEvent> {
    let last = before.trace.events.last().map_or(0, |event| event.sequence);
    after.trace.events.iter().filter(|event| event.sequence > last).collect()
}
fn require_distinct_native_attempts(after: &BrowserNavigationReport, before: &BrowserNavigationReport, expected: usize) -> Result<(), String> {
    let attempts: BTreeSet<_> = since(after, before).iter().filter(|event| event.kind == BrowserNavigationEventKind::Started)
        .filter_map(|event| event.navigation_sequence).collect();
    if attempts.len() != expected { return Err(format!("Expected {expected} native attempts, got {}", attempts.len())); } Ok(())
}
fn assert_redacted(report: &BrowserNavigationReport) -> Result<(), String> {
    let encoded = serde_json::to_string(report).map_err(message)?;
    if ["fixture_secret", "private_fragment", "?case=", "?token="].iter().any(|secret| encoded.contains(secret)) {
        return Err("Navigation diagnostic exposed fixture query or fragment secrets".into());
    } Ok(())
}
fn require_finished(tab: &BrowserTabSnapshot) -> Result<(), String> {
    if tab.lifecycle != BrowserTabLifecycle::Ready || load(tab)?.phase != BrowserNavigationPhase::Finished
        || load(tab)?.content_state != BrowserContentState::CurrentDocument { return Err("Ready lacks successful current-document navigation proof".into()); } Ok(())
}
fn exactly(fixture: &Fixture, path: &str, expected: usize) -> Result<(), String> {
    let actual = fixture.count(path);
    if actual != expected { return Err(format!("HTTP witness {path} expected {expected} requests, got {actual}")); } Ok(())
}
async fn command(runtime: &Arc<dyn BrowserRuntime>, command: BrowserTabCommand) -> Result<BrowserRuntimeSnapshot, String> {
    runtime.execute(command, CancellationToken::new()).await.map_err(message)
}
async fn create(runtime: &Arc<dyn BrowserRuntime>, url: &str) -> Result<String, String> {
    command(runtime, BrowserTabCommand::Create { url: url.into() }).await?.active_tab_id.ok_or_else(|| "Created page has no active tab".into())
}
async fn create_failure(runtime: &Arc<dyn BrowserRuntime>, url: &str) -> Result<String, String> {
    let before: BTreeSet<_> = runtime.snapshot().await.map_err(message)?.tabs.into_iter().map(|tab| tab.target.tab_id).collect();
    let result = runtime.execute(BrowserTabCommand::Create { url:url.into() }, CancellationToken::new()).await;
    if let Err(error) = result { if error != WorkspaceError::PageFailed { return Err(message(error)); } }
    let new_tabs: Vec<_> = runtime.snapshot().await.map_err(message)?.tabs.into_iter()
        .filter(|tab| !before.contains(&tab.target.tab_id)).collect();
    if new_tabs.len() != 1 { return Err("Initial navigation failure did not retain its one owned page for recovery".into()); }
    Ok(new_tabs.into_iter().next().unwrap().target.tab_id)
}
async fn create_in_flight(runtime: &Arc<dyn BrowserRuntime>, url: &str) -> Result<(String, tokio::task::JoinHandle<Result<BrowserRuntimeSnapshot, WorkspaceError>>), String> {
    let before: BTreeSet<_> = runtime.snapshot().await.map_err(message)?.tabs.into_iter().map(|tab| tab.target.tab_id).collect();
    let command_runtime = runtime.clone();
    let url = url.to_owned();
    let pending = tokio::spawn(async move { command_runtime.execute(BrowserTabCommand::Create {url}, CancellationToken::new()).await });
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(tab) = runtime.snapshot().await.map_err(message)?.tabs.into_iter().find(|tab|
            !before.contains(&tab.target.tab_id) && tab.load.as_ref().is_some_and(|load|
                matches!(load.phase, BrowserNavigationPhase::Requested | BrowserNavigationPhase::Provisional))) {
            return Ok((tab.target.tab_id, pending));
        }
        if Instant::now() >= deadline { return Err("Pending Create exposed no owned provisional tab".into()); }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
async fn settle_interrupted(pending: tokio::task::JoinHandle<Result<BrowserRuntimeSnapshot, WorkspaceError>>) -> Result<(), String> {
    match pending.await.map_err(message)? {
        Ok(_) | Err(WorkspaceError::PageFailed) | Err(WorkspaceError::Admission(RunAdmissionError::Cancelled)) => Ok(()),
        Err(error) => Err(message(error)),
    }
}
async fn navigate(runtime: &Arc<dyn BrowserRuntime>, id: &str, url: &str) -> Result<(), String> {
    command(runtime, BrowserTabCommand::Navigate { target: tab(runtime, id).await?.target, url: url.into() }).await.map(|_| ())
}
async fn control(runtime: &Arc<dyn BrowserRuntime>, id: &str, paused: bool) -> Result<(), String> {
    for _ in 0..20 {
        let snapshot = tab(runtime, id).await?;
        let expected_navigation_sequence=load(&snapshot)?.navigation_sequence;
        let target=snapshot.target;
        let command = if paused { BrowserTabCommand::PauseNavigation { target,expected_navigation_sequence } }
            else { BrowserTabCommand::ResumeNavigation { target,expected_navigation_sequence } };
        match runtime.execute(command, CancellationToken::new()).await {
            Ok(_) => return Ok(()), Err(WorkspaceError::StaleTarget) => {}, Err(error) => return Err(message(error)),
        }
    }
    Err("Could not acquire current target for native navigation control".into())
}
async fn tab(runtime: &Arc<dyn BrowserRuntime>, id: &str) -> Result<BrowserTabSnapshot, String> {
    runtime.snapshot().await.map_err(message)?.tabs.into_iter().find(|tab| tab.target.tab_id == id).ok_or_else(|| "Native tab disappeared".into())
}
async fn wait_tab(runtime: &Arc<dyn BrowserRuntime>, id: &str, predicate: impl Fn(&BrowserTabSnapshot) -> bool) -> Result<BrowserTabSnapshot, String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let tab = tab(runtime, id).await?;
        if predicate(&tab) { return Ok(tab); }
        if Instant::now() >= deadline { return Err(format!("Native tab did not reach expected state: {}", serde_json::to_string(&tab.load.as_ref().map(BrowserLoadSummary::agent_projection)).unwrap())); }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
async fn settled(runtime: &Arc<dyn BrowserRuntime>, id: &str, path: &str) -> Result<BrowserTabSnapshot, String> {
    wait_tab(runtime, id, |tab| tab.lifecycle == BrowserTabLifecycle::Ready && tab.load.as_ref().is_some_and(|load|
        load.phase == BrowserNavigationPhase::Finished && load.content_url.as_deref().and_then(|url| url::Url::parse(url).ok()).is_some_and(|url| url.path() == path))).await
}
async fn report(runtime: &Arc<dyn BrowserRuntime>, id: &str) -> Result<BrowserNavigationReport, String> {
    runtime.navigation_diagnostics(tab(runtime, id).await?.target).await.map_err(message)
}
async fn wait_count(fixture: &Fixture, path: &str, minimum: usize) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while fixture.count(path) < minimum {
        if Instant::now() >= deadline { return Err(format!("HTTP witness {path} did not reach {minimum} requests")); }
        tokio::time::sleep(Duration::from_millis(20)).await;
    } Ok(())
}
async fn evaluate(view: &tauri::Webview, expression: &str) -> Result<Value, String> {
    let result = super::windows::protocol_call(view, "Runtime.evaluate", json!({"expression":expression,"awaitPromise":true,"returnByValue":true})).await?;
    if result.get("exceptionDetails").is_some() { return Err("Navigation fixture evaluation failed".into()); }
    result["result"].get("value").cloned().ok_or_else(|| "Navigation fixture evaluation missing value".into())
}
async fn wait_value(view: &tauri::Webview, expression: &str) -> Result<Value, String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let value = evaluate(view, expression).await?;
        if !value.is_null() { return Ok(value); }
        if Instant::now() >= deadline {
            let detail=evaluate(view,"({frame_count:typeof fixtureFrames==='undefined'?null:fixtureFrames.length,worker_received:typeof fixtureWorker!=='undefined'&&fixtureWorker!==null,nested_worker_received:typeof fixtureNestedWorker!=='undefined'&&fixtureNestedWorker!==null,worker_error:typeof fixtureWorkerError==='undefined'?null:fixtureWorkerError,secure_context:isSecureContext,nonce_api:typeof crypto.randomUUID})").await?;
            let targets=super::windows::protocol_call(view,"Target.getTargets",json!({})).await.ok();
            let workers:Vec<_>=targets.as_ref().and_then(|value|value["targetInfos"].as_array()).into_iter().flatten()
                .filter(|target|target["type"]=="worker" && target["url"].as_str().and_then(|url|url::Url::parse(url).ok())
                    .is_some_and(|url|url.path()=="/nav/worker.js"))
                .map(|target|json!({"kind":"controlled_worker","attached":target["attached"].as_bool(),"path":"/nav/worker.js"})).collect();
            return Err(format!("Frame/worker environment witness missing: {detail}; owned_worker_targets={workers:?}"));
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
async fn surface(runtime: &Arc<dyn BrowserRuntime>, visible: bool, width: f64) -> Result<(), String> {
    runtime.surface().ok_or("Missing native surface port")?.set_surface(
        BrowserSurfaceBounds { x:20.0, y:60.0, width, height:640.0 }, visible, CancellationToken::new()).await.map_err(message)
}
async fn wait_popup(runtime: &Arc<dyn BrowserRuntime>, opener: &str) -> Result<BrowserTabSnapshot, String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(tab) = runtime.snapshot().await.map_err(message)?.tabs.into_iter().find(|tab| tab.target.tab_id != opener) { return Ok(tab); }
        if Instant::now() >= deadline { return Err("Trusted popup action created no native child".into()); }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
