//! One real Host candidate held after claim/publication and before binding.
//! Native hold -> candidate release/retire -> Pause ACK -> Resume, without sleeps.
use super::{host::{DesktopBrowserHost, PopupInflightProbe}, navigation_fixture::Fixture};
use nomifun_browser_platform::runtime::*;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tauri::Manager;
use tokio_util::sync::CancellationToken;

fn message(error: impl std::fmt::Display) -> String { error.to_string() }

pub(super) async fn verify(app: &tauri::AppHandle) -> Result<Value, String> {
    let fixture = Fixture::new()?;
    let probe = Arc::new(PopupInflightProbe::default());
    let factory = DesktopBrowserHost::for_popup_inflight_conformance(app.clone(), probe.clone());
    let runtime = factory.create(CreateBrowserRuntime {
        key: BrowserResourceKey { principal_id: "popup-inflight-conformance".into(),
            agent_session_id: uuid::Uuid::now_v7().to_string(), resource_binding_id: "native-popup".into() },
        runtime_generation: 1, profile: BrowserProfile::Ephemeral, user_input_enabled: true,
    }).await.map_err(message)?;
    let result = tokio::time::timeout(Duration::from_secs(35), scenario(app, &fixture, &runtime, &probe)).await
        .map_err(|_| "Claimed popup Pause/Resume conformance timed out".to_owned()).and_then(|result| result);
    let close = runtime.close().await.map_err(message);
    drop(runtime);
    let cleanup = close.and(factory.shutdown().await.map_err(message));
    match (result, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(format!("Popup inflight cleanup failed: {error}")),
        (Err(error), Err(cleanup)) => Err(format!("{error}; cleanup: {cleanup}")),
    }
}

async fn scenario(app: &tauri::AppHandle, fixture: &Fixture, runtime: &Arc<dyn BrowserRuntime>,
    probe: &Arc<PopupInflightProbe>) -> Result<Value, String> {
    runtime.surface().ok_or("Missing native popup surface")?.set_surface(
        BrowserSurfaceBounds { x: 20.0, y: 60.0, width: 1000.0, height: 600.0 }, true, Default::default()
    ).await.map_err(message)?;
    let mut changes = runtime.changes().ok_or("Missing production revision watch")?;
    let created = runtime.execute(BrowserTabCommand::Create { url: fixture.url("/nav/popup-source") },
        CancellationToken::new()).await.map_err(message)?;
    let id = created.active_tab_id.ok_or("Missing production popup opener")?;
    let parent = wait_tab(runtime, &mut changes, &id, |tab| tab.lifecycle == BrowserTabLifecycle::Ready
        && tab.load.as_ref().is_some_and(|load| load.content_state == BrowserContentState::CurrentDocument)).await?;
    let view = app.get_webview(&id).ok_or("Missing production popup opener controller")?;
    let nonce = super::evaluate(&view, "fixtureDocumentNonce").await?;
    let native_before = native_state(&view).await?;
    if native_before != (true, true, true) { return Err(format!("Popup opener is not visible UserReady: {native_before:?}")); }
    // A controlled local handler, using the native window API as the product
    // does. No page-world browser API replacement or synthetic DOM click.
    super::evaluate(&view, r#"(()=>{
        const old=document.getElementById('script-popup'); const button=old.cloneNode(true);old.replaceWith(button);
        window.inflightPopupReply=null;
        button.onclick=event=>{
            const child=window.open('/nav/popup','native-inflight-claimed');
            window.inflightPopupReply={returned:true,childWasNull:child===null,trusted:event.isTrusted};
            dispatchEvent(new Event('fixture-inflight-popup-returned'));
        }; return true;
    })()"#).await?;
    let witness = super::popup_pause_witness::Witness::install(&view).await?;
    let ticket = probe.arm(parent.target.clone()).map_err(message)?;
    let lifecycle = async {
        let mut candidate = ticket.entered.await.map_err(|_| "Claimed candidate stage was not reached")?;
        if candidate.opener != parent.target { return Err("Candidate gate captured a different opener".into()); }
        let candidate_view = app.get_webview(&candidate.child.tab_id).ok_or("Claimed native child was not published")?;
        let published = runtime.snapshot().await.map_err(message)?;
        if published.tabs.len() != 2 || !published.tabs.iter().any(|tab| tab.target == candidate.child)
            || fixture.count("/nav/popup") != 0 {
            return Err("Candidate was not held after claim/publication and before the first HTTP request".into());
        }
        native_controller_available(&candidate_view).await?;
        let pause = runtime.execute(BrowserTabCommand::PauseNavigation {
            target: parent.target.clone(), expected_navigation_sequence: sequence(&parent)?,
        }, CancellationToken::new());
        tokio::pin!(pause);
        // Positive native hold notification establishes the interleaving. The
        // Host's complete Pause ACK must still wait for this owned candidate.
        loop {
            let current = find_tab(runtime.snapshot().await.map_err(message)?, &id)?;
            if current.load.as_ref().and_then(|load| load.navigation_control.as_ref())
                .is_some_and(|control| control.paused) { break; }
            tokio::select! {
                biased;
                result=&mut pause=>return Err(format!("Pause returned before its held candidate could retire: {result:?}")),
                changed=changes.changed()=>changed.map_err(|_| "Native hold revision watch closed")?,
            }
        }
        tokio::select! {
            biased;
            result=&mut pause=>return Err(format!("Pause ACK crossed an unreleased claimed candidate: {result:?}")),
            _=std::future::ready(())=>{},
        }
        native_controller_available(&candidate_view).await?;
        if !matches!(candidate.closed.try_recv(), Err(tokio::sync::oneshot::error::TryRecvError::Empty)) {
            return Err("Native Pause destroyed a claimed candidate before its Host owner retired it".into());
        }
        require_native_request(&witness, 0)?;
        if witness.count() != 1 { return Err("More than one native request reached the held candidate scope".into()); }
        // Do not have the fixture call settle_view: that could repair a missing
        // Host acknowledgement. The original script must resume as denied while
        // the claimed candidate is still held and its native Close is pending.
        let returned = super::evaluate(&view, r#"new Promise(resolve=>{
            if(window.inflightPopupReply){resolve(window.inflightPopupReply);return;}
            addEventListener('fixture-inflight-popup-returned',()=>resolve(window.inflightPopupReply),{once:true});
        })"#).await?;
        if returned["returned"] != true || returned["childWasNull"] != true || returned["trusted"] != true {
            return Err(format!("Native deferral did not settle the original trusted request as denied: {returned}"));
        }
        ticket.release.send(()).map_err(|_| "Claimed candidate release was lost")?;
        let paused = pause.await.map_err(message)?;
        let native_closed = candidate.closed.try_recv()
            .map_err(|_| "Pause ACK arrived without its exact claimed child native Close ACK")?;
        if native_closed.tab_id != candidate.child.tab_id || native_closed.runtime_generation != candidate.child.runtime_generation
            || app.get_webview(&candidate.child.tab_id).is_some() || paused.tabs.len() != 1
            || paused.tabs.iter().any(|tab| tab.target.tab_id == candidate.child.tab_id) {
            return Err("Pause ACK did not follow exact native candidate closure and registry retirement".into());
        }
        let retained = find_tab(paused, &id)?;
        if retained.target != parent.target || super::evaluate(&view, "fixtureDocumentNonce").await? != nonce
            || fixture.count("/nav/popup") != 0 {
            return Err("Cancelling the claimed popup changed its retained opener or released its HTTP request".into());
        }
        let resumed = runtime.execute(BrowserTabCommand::ResumeNavigation {
            target: retained.target.clone(), expected_navigation_sequence: sequence(&retained)?,
        }, CancellationToken::new()).await.map_err(message)?;
        if resumed.tabs.len() != 1 || resumed.active_tab_id.as_deref() != Some(id.as_str())
            || fixture.count("/nav/popup") != 0 || app.get_webview(&candidate.child.tab_id).is_some()
            || witness.count() != 1 {
            return Err("Resume released the cancelled candidate or old native popup request".into());
        }
        let report = runtime.navigation_diagnostics(parent.target.clone()).await.map_err(message)?;
        if report.trace.counters.user_commands != 1 || report.trace.counters.agent_commands != 0 {
            return Err("Pause/Resume replayed a host navigation".into());
        }
        Ok::<_, String>((candidate.child, native_closed, returned))
    };
    let (click, lifecycle) = tokio::join!(super::native_fixture_click(&view, "#script-popup"), lifecycle);
    click?;
    let (retired, native_closed, returned) = lifecycle?;
    if native_state(&view).await? != (true, true, true) { return Err("Opener did not remain visible UserReady after Pause/Resume".into()); }
    // The probe is consumed. A fresh trusted request with the same window name
    // must take the normal production Host path and issue exactly one new GET.
    super::native_fixture_click(&view, "#script-popup").await?;
    let fresh = loop {
        let snapshot = runtime.snapshot().await.map_err(message)?;
        if let Some(tab) = snapshot.tabs.iter().find(|tab| tab.target.tab_id != id
            && tab.lifecycle == BrowserTabLifecycle::Ready && tab.url.ends_with("/nav/popup")) { break tab.clone(); }
        changes.changed().await.map_err(|_| "Fresh popup revision watch closed")?;
    };
    if fresh.target.tab_id == retired.tab_id || fixture.count("/nav/popup") != 1 || witness.count() != 2
        || !fixture.snapshot().iter().any(|receipt| receipt.path == "/nav/popup" && receipt.method == "GET" && receipt.destination == "document") {
        return Err("Fresh Resume request reused the retired child or violated its exact HTTP count".into());
    }
    let positive = require_native_request(&witness, 1)?;
    runtime.execute(BrowserTabCommand::Close { target: fresh.target }, CancellationToken::new()).await.map_err(message)?;
    let first = require_native_request(&witness, 0)?;
    witness.close().await?;
    Ok(json!({"production_host_claimed_published_candidate":true,
        "stage_barrier":"exact_opener_single_use_debug_oneshot", "ordering_without_sleep":true,
        "native_hold_observed_before_candidate_release":true,"pause_ack_waited_for_owned_candidate":true,
        "native_close_ack":native_closed,"retired_candidate":retired,"native_original_request":first,
        "original_window_open_returned":returned,"deferral_settlement_confirmed_while_candidate_still_owned":true,
        "native_close_ack_preceded_pause_ack":true,
        "candidate_native_and_runtime_registries_removed_before_pause_ack":true,
        "resume_old_request_http_get_delta":0,"fresh_after_resume_http_get_delta":1,
        "fresh_native_request":positive,"retained_opener_nonce_unchanged":true,
        "native_request_document_association":"unavailable","no_agent_lock_or_dom_click":true}))
}

fn find_tab(snapshot: BrowserRuntimeSnapshot, id: &str) -> Result<BrowserTabSnapshot, String> {
    snapshot.tabs.into_iter().find(|tab| tab.target.tab_id == id).ok_or_else(|| "Exact popup fixture tab is unavailable".into())
}
fn sequence(tab: &BrowserTabSnapshot) -> Result<u64, String> {
    tab.load.as_ref().map(|load| load.navigation_sequence).ok_or_else(|| "Missing native navigation sequence".into())
}
async fn wait_tab(runtime: &Arc<dyn BrowserRuntime>, changes: &mut tokio::sync::watch::Receiver<u64>, id: &str,
    ready: impl Fn(&BrowserTabSnapshot) -> bool) -> Result<BrowserTabSnapshot, String> {
    loop {
        let tab = find_tab(runtime.snapshot().await.map_err(message)?, id)?;
        if ready(&tab) { return Ok(tab); }
        changes.changed().await.map_err(|_| "Native tab revision watch closed")?;
    }
}
fn require_native_request(witness: &super::popup_pause_witness::Witness, index: usize) -> Result<Value, String> {
    let output = witness.since(index);
    let event = output["native_events"].as_array().and_then(|events| events.first()).ok_or("Native popup request witness missing")?;
    if event["path"] != "/nav/popup" || event["is_user_initiated"] != true || event["host_handled"] != true {
        return Err(format!("Popup did not reach its exact native Host with activation: {output}"));
    }
    Ok(event.clone())
}
async fn native_controller_available(view: &tauri::Webview) -> Result<(), String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    view.with_webview(move |platform| {
        let result = unsafe { platform.controller().CoreWebView2().and_then(|core| core.Settings()) }
            .map(|_| ()).map_err(message);
        let _ = tx.send(result);
    }).map_err(message)?;
    rx.await.map_err(|_| "Native candidate availability callback lost")?
}
async fn native_state(view: &tauri::Webview) -> Result<(bool, bool, bool), String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    view.with_webview(move |platform| {
        let result = (|| -> ::windows::core::Result<_> {
            let mut visible = ::windows::core::BOOL::default();
            let mut hwnd = ::windows::Win32::Foundation::HWND::default();
            unsafe {
                platform.controller().IsVisible(&mut visible)?;
                platform.controller().ParentWindow(&mut hwnd)?;
                Ok((visible.as_bool(), ::windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(hwnd).as_bool(),
                    ::windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled(hwnd).as_bool()))
            }
        })().map_err(message);
        let _ = tx.send(result);
    }).map_err(message)?;
    rx.await.map_err(|_| "Native popup presentation callback lost")?
}
