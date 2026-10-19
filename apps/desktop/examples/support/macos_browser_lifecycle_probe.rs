//! Production Browser owner over isolated, real WK views. Task completion is
//! supplied by this component fixture; canonical Session settlement is tested
//! separately. No global input, arbitrary JS evaluation, or shared-process kill.
use super::{fixture::Fixture, macos::host::DesktopBrowserHost};
use nomifun_browser_platform::{
    product::{BrowserCapabilityAction, BrowserProviderDescriptor, BrowserResourceBinding, BrowserSessionAuthority},
    run_guard::{BrowserInputState, BrowserRunGuard, RunAdmissionError},
    runtime::*,
    workspace::{BrowserResource, BrowserResourceService},
};
use base64::Engine as _;
use objc2::msg_send;
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::Arc, time::Duration};
use tauri::Manager;
use tokio_util::sync::CancellationToken;

#[derive(Clone,Copy)]
pub(crate) enum Selection { Nonempty, Empty }

fn error(value: impl std::fmt::Display) -> String { value.to_string() }
fn authority(session: &str) -> BrowserSessionAuthority {
    BrowserSessionAuthority::new("wk-lifecycle-audit", session, BrowserCapabilityAction::all(),
        BrowserResourceBinding::new(format!("wk-lifecycle:{session}"), "managed-browser", "wk-lifecycle-audit",
            BrowserProviderDescriptor::managed("managed", "wk-lifecycle-audit").unwrap(),
            BrowserCapabilityAction::all().map(BrowserCapabilityAction::resource_operation)).unwrap()).unwrap()
}
fn bounds() -> BrowserSurfaceBounds { BrowserSurfaceBounds{x:20., y:40., width:840., height:540.} }

pub(crate) async fn verify(app: &tauri::AppHandle, fixture: &Fixture, evidence_dir:Option<&std::path::Path>, selection:Selection) -> Result<Value, String> {
    eprintln!("MACOS_BROWSER_COMPATIBILITY_SMOKE_STAGE lifecycle_retirement_and_modal_stop");
    let baseline = native_views(app).await?;
    if !baseline.is_empty() { return Err("Prior compatibility owner retained native WK views".into()); }
    let service = Arc::new(BrowserResourceService::new(Arc::new(DesktopBrowserHost::new(app.clone()))));
    let result = scenarios(app, fixture, &service, evidence_dir, selection).await;
    let shutdown = service.shutdown().await.map_err(error);
    let final_views = native_views(app).await?;
    eprintln!("MACOS_BROWSER_LIFECYCLE_CLEANUP {}",json!({"service_shutdown_acknowledged":shutdown.is_ok(),"owned_native_view_count":final_views.len(),"baseline_native_view_count":baseline.len()}));
    match (result, shutdown) {
        (Ok(mut report), Ok(())) if final_views == baseline => {
            report["component_checks_passed"] = json!(true);
            report["native_wk_views_after_shutdown"] = json!(final_views.len());
            report["original_factory_shutdown_acknowledged"] = json!(true);
            Ok(report)
        },
        (Err(failure), Ok(())) => Err(failure),
        (Ok(_), Ok(())) => Err("Browser shutdown retained this fixture's native WK views".into()),
        (Ok(_), Err(cleanup)) => Err(format!("Browser lifecycle shutdown failed: {cleanup}")),
        (Err(failure), Err(cleanup)) => Err(format!("{failure}; Browser lifecycle shutdown: {cleanup}")),
    }
}

async fn scenarios(app: &tauri::AppHandle, fixture: &Fixture, service: &Arc<BrowserResourceService>, evidence_dir:Option<&std::path::Path>, selection:Selection) -> Result<Value, String> {
    let retained = service.ensure(authority("retained-user-session"), BrowserProfile::Ephemeral).await.map_err(error)?;
    let case = uuid::Uuid::now_v7().to_string();
    retained.user_command(BrowserTabCommand::Create {url:fixture.url(&format!("/lifecycle?case={case}"))}).await.map_err(error)?;
    let original = finished(&retained).await?;
    let document_nonce = fixture.document_nonce("/lifecycle",&case)?;
    retained.set_surface(bounds(), true, CancellationToken::new()).await.map_err(error)?;
    let native_retained = native_views(app).await?;
    if native_retained.len() != 1 { return Err("Retained user workspace did not own one real WK view".into()); }
    let first = retained.begin_interaction().await.map_err(error)?;
    first.require_explicit_finish();
    let observed = retained.observe(&first,None).await.map_err(error)?;
    eprintln!("MACOS_BROWSER_LIFECYCLE_OBSERVATION {}",json!({"target":observed.target,"elements":observed.elements.iter().map(|element|json!({"name":element.name,"role":element.role})).collect::<Vec<_>>()}));
    let draft = observed.elements.iter().find(|element|element.name == "Retained draft" && element.role == "textbox").ok_or("Lifecycle draft control missing")?.reference.clone();
    action(&retained,&first,BrowserAction::Type {element:draft,text:"unsaved WK 中文 draft".into()},"draft-type",evidence_dir).await?;
    if !retained.finish_interaction(&first).await.map_err(error)? { return Err("Semantic draft work did not settle".into()); }
    retained.workspace().retain_task("native-retained-turn-one").await.map_err(error)?;
    retained.set_surface(bounds(), false, CancellationToken::new()).await.map_err(error)?;
    if retained.workspace().try_retire_after_task("native-retained-turn-one").await.map_err(error)? {
        return Err("Task completion retired a hidden user-owned document".into());
    }
    retained.workspace().retain_task("native-retained-turn-two").await.map_err(error)?;
    let rounds = match selection {
        Selection::Empty => empty_carrier_rounds(app,fixture,service,&native_retained).await?,
        Selection::Nonempty => nonempty_carrier_rounds(app,fixture,service,&native_retained).await?,
    };
    // The fixture owns its layout acknowledgement, as the production renderer
    // does when an Agent requests the already-retained user panel.
    retained.set_surface(bounds(),true,CancellationToken::new()).await.map_err(error)?;
    let successor = retained.begin_interaction().await.map_err(error)?;
    successor.require_explicit_finish();
    let after = retained.observe(&successor,None).await.map_err(error)?;
    if after.target != original.target || !after.content.contains("unsaved WK 中文 draft")
        || !after.content.contains(&document_nonce) || fixture.count_case("/lifecycle",&case) != 1 {
        return Err("Completed sibling or successor lost the retained document/draft or reloaded it".into());
    }
    if !retained.finish_interaction(&successor).await.map_err(error)? { return Err("Successor observation did not settle".into()); }
    retained.workspace().request_retirement_after_turn("native-retained-turn-two",false).await.map_err(error)?;
    retained.set_surface(bounds(),true,CancellationToken::new()).await.map_err(error)?;

    let stop = retained.begin_interaction().await.map_err(error)?;
    stop.require_explicit_finish();
    let observed = retained.observe(&stop,None).await.map_err(error)?;
    let button = observed.elements.iter().find(|element|element.name == "Retained Confirm" && element.role == "button").ok_or("Lifecycle Confirm control missing")?.reference.clone();
    let stopped_action = action(&retained,&stop,BrowserAction::click(button),"stop-confirm",evidence_dir).await?;
    let BrowserActionOutcome::AwaitingDialog{dialog} = stopped_action.outcome else { return Err("Real WK Confirm did not yield its original pending callback".into()); };
    if retained.finish_interaction(&stop).await.map_err(error)?
        || retained.snapshot().await.map_err(error)?.run.input_state != BrowserInputState::AgentRunning {
        return Err("Pending WK Confirm released its native owner".into());
    }
    if retained.user_command(BrowserTabCommand::CloseAll{runtime_generation:retained.runtime_generation()}).await.err()
        != Some(WorkspaceError::Admission(RunAdmissionError::UserInputLocked)) {
        return Err("User runtime close bypassed a pending Agent callback".into());
    }
    stop.cancel();
    retained.settle_run(&stop).await.map_err(error)?;
    retained.finish_run(&stop).await.map_err(error)?;
    let check = retained.begin_interaction().await.map_err(error)?;
    check.require_explicit_finish();
    let after_stop = retained.observe(&check,None).await.map_err(error)?;
    if !after_stop.content.contains("Confirm cancelled once:1") || !after_stop.content.contains("unsaved WK 中文 draft") {
        return Err("Exact Stop did not drain its original WK callback once while preserving the document".into());
    }
    if !retained.finish_interaction(&check).await.map_err(error)? { return Err("Post-Stop observation did not release native input".into()); }
    if native_views(app).await? != native_retained { return Err("Local Stop changed the retained native view inventory".into()); }

    // A late reply for the drained original callback cannot activate or mutate
    // a newly created page. This uses the ordinary user command and exact old
    // request ID, without replaying the original semantic action.
    let successor_case = uuid::Uuid::now_v7().to_string();
    retained.user_command(BrowserTabCommand::Create {url:fixture.url(&format!("/lifecycle?case={successor_case}"))}).await.map_err(error)?;
    let fresh = finished(&retained).await?;
    let fresh_nonce = fixture.document_nonce("/lifecycle",&successor_case)?;
    if retained.user_command(BrowserTabCommand::Dialog {target:dialog.target.clone(),request_id:dialog.request_id.clone(),accept:true,text:None}).await.err()
        != Some(WorkspaceError::NotActionable) { return Err("A late old-target reply bypassed the current-page ownership fence".into()); }
    if retained.user_command(BrowserTabCommand::Dialog {target:fresh.target.clone(),request_id:dialog.request_id.clone(),accept:true,text:None}).await.err()
        != Some(WorkspaceError::StaleTarget) { return Err("The successor page accepted the original cancelled dialog request ID".into()); }
    let after_late = retained.begin_interaction().await.map_err(error)?;
    after_late.require_explicit_finish();
    let fresh_observed = retained.observe(&after_late,None).await.map_err(error)?;
    if fresh_observed.target != fresh.target || !fresh_observed.content.contains(&fresh_nonce)
        || !fresh_observed.content.contains("Confirm untouched") || fixture.count_case("/lifecycle",&successor_case) != 1 {
        return Err("Late original callback reply changed the exact successor document".into());
    }
    if !retained.finish_interaction(&after_late).await.map_err(error)? { return Err("Late-reply verification did not settle".into()); }

    // Real application shutdown will call this same Browser service owner. Keep
    // another native Confirm pending so shutdown must drain the original work.
    let close = retained.begin_interaction().await.map_err(error)?;
    close.require_explicit_finish();
    let observed = retained.observe(&close,None).await.map_err(error)?;
    let button = observed.elements.iter().find(|element|element.name == "Retained Confirm" && element.role == "button").ok_or("Close Confirm control missing")?.reference.clone();
    let pending = action(&retained,&close,BrowserAction::click(button),"shutdown-confirm",evidence_dir).await?;
    if !matches!(pending.outcome,BrowserActionOutcome::AwaitingDialog{..}) { return Err("Shutdown fixture did not retain a real native dialog".into()); }
    Ok(json!({"scope":"production_browser_owner_real_wk_component","ephemeral_profiles":true,
        "case":match selection {Selection::Nonempty=>"nonempty_task_only",Selection::Empty=>"empty_after_native_close_all"},
        "canonical_mission_completion":"supplied_by_component_fixture","draft_input_fidelity":"semantic_dom_untrusted",
        "user_navigation_and_unsaved_draft_survive_hidden_completion_and_successor":true,
        "retained_document_nonce":document_nonce,"retained_document_main_requests":fixture.count_case("/lifecycle",&case),"automatic_retirement_rounds":rounds,
        "exact_stop_dialog_request_id":dialog.request_id,"exact_stop_original_callback_cancelled_once":true,
        "late_original_dialog_reply_rejected_with_successor_document_unchanged":true,
        "pending_dialog_holds_input_until_original_owner_settles":true,"shutdown_started_with_pending_native_dialog":true,
        "shared_webcontent_processes":"not_signalled_or_counted_as_app_owned"}))
}

// Mirror the production Agent automatic-open handshake. The original Create
// first registers its native owner and waits for layout. This fixture then
// acknowledges the trusted Agent surface through BrowserWorkspace, never by
// calling Runtime.surface directly or relabelling a human show.
async fn nonempty_carrier_rounds(app:&tauri::AppHandle, fixture:&Fixture, service:&Arc<BrowserResourceService>,
    native_retained:&BTreeSet<usize>) -> Result<Vec<Value>,String> {
    let mut rounds = Vec::new();
    for round in 0..3 {
        let session = format!("completed-task-{round}");
        let operation = format!("native-task-completed-{round}");
        let task = service.ensure(authority(&session),BrowserProfile::Ephemeral).await.map_err(error)?;
        task.workspace().retain_task(&operation).await.map_err(error)?;
        let run = task.begin_interaction().await.map_err(error)?;
        run.require_explicit_finish();
        let attachment = round as u64 + 1;
        task.retain_surface_attachment(attachment).await.map_err(error)?;
        let create_owner = task.clone();
        let create_run = run.clone();
        let url = fixture.url(&format!("/lifecycle?case={session}"));
        eprintln!("MACOS_BROWSER_LIFECYCLE_STAGE nonempty_agent_create_round={}",round+1);
        let create = tokio::spawn(async move {create_owner.agent_command(&create_run,BrowserTabCommand::Create {url}).await});
        let registered = tokio::time::timeout(Duration::from_secs(5),async {
            loop {
                if task.snapshot().await.map_err(error)?.runtime.is_some_and(|runtime|!runtime.tabs.is_empty()) {return Ok::<_,String>(());}
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.map_err(|_|"Original Agent Create did not register its native layout owner".to_owned()).and_then(|value|value);
        let shown = match registered {
            Ok(()) => task.set_agent_surface(bounds(),true,CancellationToken::new()).await.map_err(error),
            Err(failure) => Err(failure),
        };
        if shown.is_err() {run.cancel();}
        let original_create = create.await.map_err(error)?.map_err(error);
        shown?;
        original_create?;
        let page = finished(&task).await?;
        if fixture.count_case("/lifecycle",&session) != 1 {return Err("Task-only WK navigation was replayed".into());}
        let all_views = native_views(app).await?;
        let task_views = all_views.difference(native_retained).copied().collect::<BTreeSet<_>>();
        if all_views.len() != native_retained.len()+1 || task_views.len()!=1 || native_visible_views(app).await? != task_views {
            return Err("Agent-only task did not present one exact locked WK view".into());
        }
        task.set_agent_surface(bounds(),false,CancellationToken::new()).await.map_err(error)?;
        if !native_visible_views(app).await?.is_empty() {return Err("Agent-only hide ACK retained a visible native page".into());}
        task.release_surface_attachment(attachment).await;
        if !task.finish_interaction(&run).await.map_err(error)? {return Err("Hidden original task operation did not settle".into());}
        task.workspace().request_retirement_after_turn(&operation,true).await.map_err(error)?;
        tokio::time::timeout(Duration::from_secs(10),async {
            loop {
                if service.get_for_agent_session("wk-lifecycle-audit",&session).await.map_err(error)?.is_none() {return Ok::<_,String>(());}
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.map_err(|_|"Task-only WK workspace did not automatically retire")??;
        if native_views(app).await? != *native_retained {return Err("Task retirement removed the user view or retained its own native view".into());}
        if !matches!(task.begin_interaction().await,Err(WorkspaceError::WorkspaceClosed)) {return Err("Retired task workspace readmitted an interaction".into());}
        eprintln!("MACOS_BROWSER_LIFECYCLE_STAGE retired_task_round={}",round+1);
        rounds.push(json!({"round":round+1,"tab_id":page.target.tab_id,"runtime_generation":page.target.runtime_generation,
            "native_task_views_before":task_views.len(),"native_views_after":native_retained.len(),
            "original_create_dispatched_once":true,"server_main_document_requests":1,
            "trusted_agent_show_and_hide_acknowledged":true,"exact_attachment_released_before_input_handoff":true,
            "nonempty_task_only_automatically_retired_without_session_close":true,"old_workspace_admission_closed":true}));
    }
    Ok(rounds)
}

// This is the supported empty-carrier subset. It keeps the nonempty
// task-only case separate: real user page commands and shown/hide facts cannot
// be relabelled as an unexposed Agent-only page merely to make retirement pass.
async fn empty_carrier_rounds(app:&tauri::AppHandle, fixture:&Fixture, service:&Arc<BrowserResourceService>,
    native_retained:&BTreeSet<usize>) -> Result<Vec<Value>,String> {
    let mut rounds = Vec::new();
    for round in 0..3 {
        let session = format!("empty-carrier-{round}");
        let operation = format!("completed-empty-carrier-{round}");
        let resource = service.ensure(authority(&session),BrowserProfile::Ephemeral).await.map_err(error)?;
        resource.workspace().retain_task(&operation).await.map_err(error)?;
        resource.user_command(BrowserTabCommand::Create {url:fixture.url(&format!("/lifecycle?case={session}"))}).await.map_err(error)?;
        let page = finished(&resource).await?;
        resource.set_surface(bounds(),true,CancellationToken::new()).await.map_err(error)?;
        let views = native_views(app).await?;
        if views.len() != native_retained.len()+1 || views.difference(native_retained).count() != 1 {
            return Err("Empty-carrier case did not create one exact additional WK view".into());
        }
        resource.set_surface(bounds(),false,CancellationToken::new()).await.map_err(error)?;
        if resource.workspace().try_retire_after_task(&operation).await.map_err(error)? {
            return Err("Completed empty-carrier case prematurely retired its nonempty user page".into());
        }
        resource.user_command(BrowserTabCommand::CloseAll{runtime_generation:resource.runtime_generation()}).await.map_err(error)?;
        tokio::time::timeout(Duration::from_secs(10),async {
            loop {
                if service.get_for_agent_session("wk-lifecycle-audit",&session).await.map_err(error)?.is_none() { return Ok::<_,String>(()); }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.map_err(|_|"Closed and hidden empty WK carrier did not automatically retire")??;
        if native_views(app).await? != *native_retained { return Err("Empty-carrier retirement changed the retained user view".into()); }
        if !matches!(resource.begin_interaction().await,Err(WorkspaceError::WorkspaceClosed)) { return Err("Old empty-carrier authority readmitted work".into()); }
        eprintln!("MACOS_BROWSER_LIFECYCLE_STAGE empty_carrier_retired_round={}",round+1);
        rounds.push(json!({"round":round+1,"tab_id":page.target.tab_id,"runtime_generation":page.target.runtime_generation,
            "native_views_while_loaded":views.len(),"native_views_after":native_retained.len(),
            "nonempty_user_page_preserved_before_close_all":true,"native_user_close_all_acknowledged":true,
            "empty_carrier_automatically_retired_without_session_close":true,"task_only_nonempty_retirement":"separate_case_not_proven"}));
    }
    Ok(rounds)
}

async fn action(resource:&Arc<BrowserResource>, run:&BrowserRunGuard, action:BrowserAction, stage:&str,
    evidence_dir:Option<&std::path::Path>) -> Result<BrowserActionResult,String> {
    eprintln!("MACOS_BROWSER_LIFECYCLE_STAGE {stage}");
    match resource.act(run,action).await {
        Ok(result) => Ok(result),
        Err(failure) => {
            // Only this fixture's already-owned page is read, after a failed
            // action. The original action is never retried or reinterpreted.
            if let Some(directory) = evidence_dir {
                if let Ok(screenshot) = resource.screenshot(run,None).await {
                    if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(screenshot.png_base64) {
                        let _ = std::fs::write(directory.join(format!("lifecycle-{stage}-failed.png")),bytes);
                    }
                }
            }
            Err(format!("WK lifecycle {stage}: {failure}"))
        }
    }
}

async fn finished(resource:&Arc<BrowserResource>) -> Result<BrowserTabSnapshot,String> {
    tokio::time::timeout(Duration::from_secs(10),async {
        loop {
            let snapshot = resource.snapshot().await.map_err(error)?.runtime.ok_or("WK lifecycle runtime missing")?;
            let active = snapshot.active_tab_id;
            if let Some(tab) = snapshot.tabs.into_iter().find(|tab|Some(&tab.target.tab_id)==active.as_ref() && tab.load.as_ref().is_some_and(|load|load.phase==BrowserNavigationPhase::Finished)) { return Ok::<_,String>(tab); }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.map_err(|_|"WK lifecycle page did not finish")?
}

// Read only native children of this audit's own NSWindow on the AppKit thread.
// Pointer values remain transient; no retained native handle extends lifetime.
async fn native_views(app:&tauri::AppHandle) -> Result<BTreeSet<usize>,String> {
    native_inventory(app,false).await
}
async fn native_visible_views(app:&tauri::AppHandle) -> Result<BTreeSet<usize>,String> {
    native_inventory(app,true).await
}
async fn native_inventory(app:&tauri::AppHandle, visible_only:bool) -> Result<BTreeSet<usize>,String> {
    let app_handle = app.clone();
    let (tx,rx) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let result = (|| {
            let window = app_handle.get_window("main").ok_or("Missing owned lifecycle window")?;
            let raw = window.ns_window().map_err(error)?;
            let window = unsafe { raw.cast::<objc2_app_kit::NSWindow>().as_ref() }.ok_or("Missing lifecycle NSWindow")?;
            let parent = window.contentView().ok_or("Missing lifecycle content view")?;
            let class = objc2::runtime::AnyClass::get(c"WKWebView").ok_or("WKWebView unavailable")?;
            let mut views = BTreeSet::new();
            for view in parent.subviews() {
                let webkit: bool = unsafe {msg_send![&*view,isKindOfClass:class]};
                if webkit && (!visible_only || !view.isHidden()) { views.insert(objc2::rc::Retained::as_ptr(&view) as usize); }
            }
            Ok::<_,String>(views)
        })();
        let _ = tx.send(result);
    }).map_err(error)?;
    rx.await.map_err(error)?
}
