//! Real WebView2 handoff between browser calls in one ongoing Agent Session.
//! Every controller and profile belongs to this disposable smoke fixture.
use nomifun_browser_platform::{
    run_guard::{BrowserInputState, RunAdmissionError},
    runtime::*,
    workspace::BrowserResourceService,
};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tauri::Manager;
#[path = "browser_interaction_process.rs"]
mod browser_process;

pub(super) async fn verify(app: &tauri::AppHandle, url: &str) -> Result<Value, String> {
    let service = Arc::new(BrowserResourceService::new(Arc::new(
        super::host::DesktopBrowserHost::new(app.clone()),
    )));
    let authority = super::browser_resource_fixture::authority(
        "interaction-user", "ongoing-agent-session", "native-interaction-fixture",
    );
    let workspace = service.ensure(authority.clone(), BrowserProfile::Ephemeral)
        .await.map_err(|error| error.to_string())?;
    let result = async {
        workspace.user_command(BrowserTabCommand::Create { url: url.into() })
            .await.map_err(|error| error.to_string())?;
        let target = tokio::time::timeout(Duration::from_secs(5), super::wait_workspace_page(&workspace, url))
            .await.map_err(|_| "Interaction page did not become ready")??;
        let view = app.get_webview(&target.tab_id).ok_or("Interaction native view missing")?;
        workspace.set_surface(BrowserSurfaceBounds {x:20.,y:60.,width:1000.,height:600.}, true, Default::default())
            .await.map_err(|error| error.to_string())?;
        super::evaluate(&view, "window.interactionClicks=0;window.interactionTrusted=false;const b=document.createElement('button');b.id='interaction-dialog';b.textContent='Interaction dialog';b.onclick=e=>{interactionClicks++;interactionTrusted=e.isTrusted;window.interactionFirst=confirm('Interaction first');window.interactionSecond=prompt('Interaction second','default')};document.body.prepend(b);true").await?;

        let observed_call = workspace.begin_interaction().await.map_err(|error| error.to_string())?;
        observed_call.require_explicit_finish();
        let observed = workspace.observe(&observed_call, None).await.map_err(|error| error.to_string())?;
        if super::context_menus_enabled(&view).await? {
            return Err("Native browser user policy stayed enabled during Agent observation".into());
        }
        let button = observed.elements.iter().find(|element| element.name == "验证点击")
            .ok_or("Retained-reference button was not observed")?.reference.clone();
        if workspace.user_command(BrowserTabCommand::Activate {target:target.clone()}).await.err()
            != Some(WorkspaceError::Admission(RunAdmissionError::UserInputLocked)) {
            return Err("User command was admitted during observation ownership".into());
        }
        if !workspace.finish_interaction(&observed_call).await.map_err(|error| error.to_string())?
            || workspace.snapshot().await.map_err(|error| error.to_string())?.run.input_state != BrowserInputState::UserReady {
            return Err("Completed observation kept user input locked".into());
        }
        if !super::context_menus_enabled(&view).await? {
            return Err("Native browser user policy was not restored between browser calls".into());
        }
        workspace.user_command(BrowserTabCommand::Activate {target:target.clone()})
            .await.map_err(|error| format!("User command between browser calls: {error}"))?;
        let action_call = workspace.begin_interaction().await.map_err(|error| error.to_string())?;
        action_call.require_explicit_finish();
        let action = workspace.act(&action_call, BrowserAction::click(button))
            .await.map_err(|error| format!("Reference from preceding observation call: {error}"))?;
        if !matches!(action.outcome, BrowserActionOutcome::Completed)
            || super::evaluate(&view, "smoke.clicks===1 && document.getElementById('result').dataset.lastClickTrusted==='true'").await? != true {
            return Err("Retained reference did not deliver one trusted native click".into());
        }
        if !workspace.finish_interaction(&action_call).await.map_err(|error| error.to_string())? {
            return Err("Completed click was falsely retained as pending work".into());
        }

        let dialog_observe = workspace.begin_interaction().await.map_err(|error| error.to_string())?;
        dialog_observe.require_explicit_finish();
        let observed = workspace.observe(&dialog_observe, None).await.map_err(|error| error.to_string())?;
        let button = observed.elements.iter().find(|element| element.name == "Interaction dialog")
            .ok_or("Dialog button was not observed")?.reference.clone();
        if !workspace.finish_interaction(&dialog_observe).await.map_err(|error| error.to_string())? {
            return Err("Dialog observation did not hand input to the user".into());
        }
        let dialog_call = workspace.begin_interaction().await.map_err(|error| error.to_string())?;
        dialog_call.require_explicit_finish();
        let waiting = workspace.act(&dialog_call, BrowserAction::click(button))
            .await.map_err(|error| error.to_string())?;
        let BrowserActionOutcome::AwaitingDialog {dialog:first} = waiting.outcome else {
            return Err("Trusted input did not retain its native dialog continuation".into());
        };
        if first.message != "Interaction first"
            || workspace.finish_interaction(&dialog_call).await.map_err(|error| error.to_string())?
            || workspace.snapshot().await.map_err(|error| error.to_string())?.run.input_state != BrowserInputState::AgentRunning {
            return Err("Pending native dialog released browser ownership".into());
        }
        if workspace.user_command(BrowserTabCommand::CloseAll {runtime_generation:workspace.runtime_generation()}).await.err()
            != Some(WorkspaceError::Admission(RunAdmissionError::UserInputLocked)) {
            return Err("User close bypassed pending Agent dialog ownership".into());
        }
        if service.close_idle(workspace.key().clone(),workspace.runtime_generation()).await.err()
            != Some(WorkspaceError::Admission(RunAdmissionError::UserInputLocked)) {
            return Err("User runtime destruction bypassed pending Agent dialog ownership".into());
        }
        let second = workspace.respond_dialog(&dialog_call, BrowserDialogReply {
            target:first.target,request_id:first.request_id,accept:true,text:None,
        }).await.map_err(|error| error.to_string())?;
        let BrowserActionOutcome::AwaitingDialog {dialog:second} = second.outcome else {
            return Err("Chained dialog lost the original browser call".into());
        };
        if second.message != "Interaction second"
            || workspace.finish_interaction(&dialog_call).await.map_err(|error| error.to_string())? {
            return Err("Chained native dialog released ownership too early".into());
        }
        let completed = workspace.respond_dialog(&dialog_call, BrowserDialogReply {
            target:second.target,request_id:second.request_id,accept:true,text:Some("handoff 中文".into()),
        }).await.map_err(|error| error.to_string())?;
        if !matches!(completed.outcome, BrowserActionOutcome::Completed)
            || !workspace.finish_interaction(&dialog_call).await.map_err(|error| error.to_string())?
            || workspace.snapshot().await.map_err(|error| error.to_string())?.run.input_state != BrowserInputState::UserReady {
            return Err("Final dialog response did not release browser input".into());
        }
        if super::evaluate(&view, "interactionClicks===1 && interactionTrusted && interactionFirst===true && interactionSecond==='handoff 中文'").await? != true {
            return Err("Handoff canceled or replayed the original native dialog callback".into());
        }
        workspace.user_command(BrowserTabCommand::Activate {target:target.clone()})
            .await.map_err(|error| error.to_string())?;


        verify_retirement_preserves_user_editor(app, &service, &workspace, &view, url).await?;

        // The production automation owns this view's sole frame subscription.
        // A direct read-only protocol request observes native liveness without
        // trying to become a second auto-attach owner.
        super::windows::protocol_call(&view, "Page.getFrameTree", json!({})).await?;
        let browser_process = browser_process::Witness::observe(&view).await?;
        let old_generation = workspace.runtime_generation();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            match service.close_idle(workspace.key().clone(), old_generation).await {
                Ok(()) => break,
                Err(WorkspaceError::NativeCommandFailed) if tokio::time::Instant::now() < deadline =>
                    tokio::time::sleep(Duration::from_millis(50)).await,
                Err(error) => return Err(format!("User close between browser calls: {error}")),
            }
        }
        if app.get_webview(&target.tab_id).is_some()
            || super::windows::protocol_call(&view, "Page.getFrameTree", json!({})).await.is_ok()
            || !matches!(workspace.begin_interaction().await, Err(WorkspaceError::WorkspaceClosed)) {
            return Err("User close retained a native page or its old interaction authority".into());
        }
        // Do not let test-only view handles extend native lifetime.
        drop(view);
        let browser_process_exited = browser_process.wait_for_exit(Duration::from_secs(5)).await?;
        if !browser_process_exited {
            eprintln!("BROWSER_INTERACTION_PROCESS_WITNESS {}", json!({
                "isolated_browser_process_exited_after_user_close":false,
                "browser_process_exit_observation_deadline_ms":5000,
            }));
            return Err("User close destroyed the native controller but its isolated browser process did not exit within 5000ms".into());
        }
        let replacement = service.ensure(authority.clone(), BrowserProfile::Ephemeral)
            .await.map_err(|error| error.to_string())?;
        if replacement.runtime_generation() <= old_generation {
            return Err("Same-Session browser rebuild reused the retired runtime generation".into());
        }
        let rebuilt_call = replacement.begin_interaction().await.map_err(|error| error.to_string())?;
        rebuilt_call.require_explicit_finish();
        replacement.agent_command(&rebuilt_call, BrowserTabCommand::Create {url:url.into()})
            .await.map_err(|error| format!("Agent browser call after user close: {error}"))?;
        let rebuilt = tokio::time::timeout(Duration::from_secs(5), super::wait_workspace_page(&replacement, url))
            .await.map_err(|_| "Same-Session rebuilt page did not become ready")??;
        if rebuilt.tab_id == target.tab_id {
            return Err("Rebuilt browser reused the destroyed native controller".into());
        }
        replacement.observe(&rebuilt_call, None).await.map_err(|error| error.to_string())?;
        if !replacement.finish_interaction(&rebuilt_call).await.map_err(|error| error.to_string())? {
            return Err("Rebuilt browser interaction failed to hand off".into());
        }
        Ok(json!({
            "observe_to_act_reference_survives_handoff":true,
            "trusted_native_click_once":true,
            "user_commands_between_calls":true,
            "native_user_policy_restored_between_calls":true,
            "pending_dialog_retains_ownership":true,
            "pending_dialog_blocks_user_close_and_destroy":true,
            "chained_dialog_retains_original_callback":true,
            "final_dialog_response_unlocks":true,
            "user_close_destroys_native_controller":true,
            "isolated_browser_process_observed_alive_before_close":true,
            "isolated_browser_process_exited_after_user_close":browser_process_exited,
            "browser_process_exit_observation_deadline_ms":5000,
            "same_session_agent_recreates_browser":true,
            "trusted_user_edit_survives_hidden_completed_and_successor_turns":true,
            "unrelated_task_only_native_carrier_automatically_retired":true,
        }))
    }.await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let closed = loop {
        match service.shutdown().await {
            Ok(()) => break Ok(()),
            Err(error) if tokio::time::Instant::now() >= deadline => break Err(format!("Interaction fixture cleanup: {error}")),
            Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    };
    result.and_then(|value| closed.map(|_| value))
}

/// Exercise actual native DOM/input and carrier destruction together. The
/// Browser owner's explicit task references stand in for its already-tested
/// canonical completion caller; no model or Session history is replayed.
async fn verify_retirement_preserves_user_editor(
    app: &tauri::AppHandle, service: &Arc<BrowserResourceService>,
    retained: &Arc<nomifun_browser_platform::workspace::BrowserResource>,
    view: &tauri::Webview, url: &str,
) -> Result<(), String> {
    super::evaluate(view, "window.lifecycleEditTrusted=false;const e=document.createElement('textarea');e.id='lifecycle-retained-editor';e.oninput=event=>window.lifecycleEditTrusted=event.isTrusted;document.body.prepend(e);true").await?;
    super::native_fixture_click(view, "#lifecycle-retained-editor").await?;
    super::windows::protocol_call(view, "Input.insertText", json!({"text":"retain unsaved 用户编辑"})).await?;
    let unchanged = "document.getElementById('lifecycle-retained-editor').value==='retain unsaved 用户编辑' && lifecycleEditTrusted";
    if super::evaluate(view, unchanged).await? != true { return Err("Native editor fixture did not receive trusted input".into()); }
    let original = retained.snapshot().await.map_err(|error|error.to_string())?.runtime.ok_or("Missing retained native runtime")?;
    retained.workspace().retain_task("native-retention-turn-one").await.map_err(|error|error.to_string())?;
    retained.set_surface(BrowserSurfaceBounds{x:20.,y:60.,width:1000.,height:600.}, false, Default::default())
        .await.map_err(|error|error.to_string())?;
    if retained.workspace().try_retire_after_task("native-retention-turn-one").await.map_err(|error|error.to_string())? {
        return Err("A completed task discarded a hidden user editor".into());
    }
    retained.workspace().retain_task("native-retention-turn-two").await.map_err(|error|error.to_string())?;
    let run = retained.begin_interaction().await.map_err(|error|error.to_string())?;
    retained.observe(&run, None).await.map_err(|error|error.to_string())?;
    retained.finish_interaction(&run).await.map_err(|error|error.to_string())?;
    retained.workspace().request_retirement_after_turn("native-retention-turn-two", false).await.map_err(|error|error.to_string())?;

    let task = service.ensure(super::browser_resource_fixture::authority(
        "interaction-user", "completed-task-only-session", "native-interaction-fixture"), BrowserProfile::Ephemeral)
        .await.map_err(|error|error.to_string())?;
    task.workspace().retain_task("native-task-only-completed").await.map_err(|error|error.to_string())?;
    let task_run = task.begin_interaction().await.map_err(|error|error.to_string())?;
    task.agent_command(&task_run, BrowserTabCommand::Create {url:url.into()}).await.map_err(|error|error.to_string())?;
    let task_target = tokio::time::timeout(Duration::from_secs(5), super::wait_workspace_page(&task,url))
        .await.map_err(|_|"Task-only native page did not load")??;
    let task_view = app.get_webview(&task_target.tab_id).ok_or("Missing task-only native controller")?;
    let task_process = browser_process::Witness::observe(&task_view).await?;
    if !task.finish_interaction(&task_run).await.map_err(|error|error.to_string())? {
        return Err("Task-only native interaction did not settle".into());
    }
    task.workspace().request_retirement_after_turn("native-task-only-completed", true).await.map_err(|error|error.to_string())?;
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if service.get_for_agent_session("interaction-user", "completed-task-only-session").await.map_err(|error|error.to_string())?.is_none() {
                return Ok::<_,String>(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.map_err(|_|"Completed task-only native carrier did not automatically retire")??;
    if app.get_webview(&task_target.tab_id).is_some() { return Err("Automatic retirement retained the task-only controller".into()); }
    drop(task_view);
    if !task_process.wait_for_exit(Duration::from_secs(5)).await? { return Err("Task-only Browser process stayed alive after automatic retirement".into()); }
    let after = retained.snapshot().await.map_err(|error|error.to_string())?.runtime.ok_or("Completed sibling destroyed the user editor")?;
    if after.runtime_generation != original.runtime_generation || after.tabs != original.tabs
        || super::evaluate(view, unchanged).await? != true {
        return Err("Cross-Turn/sibling retirement changed the original unsaved user editor".into());
    }
    retained.set_surface(BrowserSurfaceBounds{x:20.,y:60.,width:1000.,height:600.}, true, Default::default())
        .await.map_err(|error|error.to_string())?;
    Ok(())
}
