//! Native Workspace/run-guard integration, not a model or product testing mode.
use super::windows::script_dialogs as dialogs;
use nomifun_browser_platform::{
    run_guard::{BrowserInputState, RunAdmissionError},
    runtime::*,
    workspace::BrowserWorkspaceService,
};
use std::sync::Arc;
use tauri::Manager;

pub(super) async fn verify(app: &tauri::AppHandle, url: &str) -> Result<serde_json::Value, String> {
    let service = Arc::new(BrowserWorkspaceService::new(Arc::new(
        super::host::DesktopBrowserHost::new(app.clone()),
    )));
    let workspace = service
        .ensure(
            BrowserWorkspaceKey {
                user_id: "dialog-user".into(),
                conversation_id: "dialog-conversation".into(),
            },
            "native-webview2-v2".into(),
            BrowserProfile::Ephemeral,
        )
        .await
        .map_err(|e| e.to_string())?;
    let result = async {
        let run = workspace.begin_run().await.map_err(|e|e.to_string())?;
        run.require_explicit_finish();
        workspace.agent_command(&run, BrowserTabCommand::Create {url:url.into()}).await.map_err(|e|e.to_string())?;
        let initial_deadline = tokio::time::Instant::now()+std::time::Duration::from_secs(5);
        let tab = loop {
            let snapshot = workspace.snapshot().await.map_err(|e|e.to_string())?.runtime.ok_or("Missing created dialog runtime")?;
            if let Some(tab) = snapshot.tabs.first().filter(|tab|tab.lifecycle==BrowserTabLifecycle::Ready && tab.url==url) {break tab.clone();}
            if tokio::time::Instant::now()>=initial_deadline {return Err("Dialog runtime initial navigation did not settle".into());}
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        };
        let view = app.get_webview(&tab.target.tab_id).ok_or("Native dialog runtime view missing")?;
        // The production host installed its handler before the first navigation.
        if dialogs::subscribe(&view).await.map_err(|e|e.to_string())?.is_none() {return Err("Production dialog handler was not installed".into());}
        workspace.set_surface(BrowserSurfaceBounds {x:20.0,y:60.0,width:900.0,height:600.0},true,Default::default()).await.map_err(|e|e.to_string())?;
        super::evaluate(&view,"document.body.innerHTML='<button id=dialog-action>Dialog action</button>';window.dialogClicks=0;document.getElementById('dialog-action').onclick=e=>{window.dialogTrusted=e.isTrusted;window.dialogClicks++;window.firstReply=confirm('First dialog');window.secondReply=prompt('Second dialog','default')};true").await?;
        let observed = workspace.observe(&run,None).await.map_err(|e|e.to_string())?;
        let element = observed.elements.iter().find(|element|element.name=="Dialog action").ok_or("Dialog action was not observed")?.reference.clone();
        let first = tokio::time::timeout(std::time::Duration::from_secs(5), workspace.act(&run,BrowserAction::click(element.clone()))).await.map_err(|_|"First dialog held the tool operation gate")?.map_err(|e|e.to_string())?;
        let BrowserActionOutcome::AwaitingDialog {dialog:first} = first.outcome else {return Err("Click was falsely reported as completed".into());};
        if first.message != "First dialog" {return Err("Wrong first dialog result".into());}
        if workspace.act(&run,BrowserAction::click(element)).await.unwrap_err()!=WorkspaceError::DialogPending {return Err("Pending click was replayed".into());}
        if workspace.observe(&run,None).await.unwrap_err()!=WorkspaceError::DialogPending {return Err("Observation blocked behind paused input".into());}
        if workspace.user_command(BrowserTabCommand::Reload {target:first.target.clone()}).await.unwrap_err()!=WorkspaceError::Admission(RunAdmissionError::UserInputLocked) {return Err("User acted while dialog input was owned by Agent".into());}
        let forged_user = BrowserTabCommand::Dialog {target:first.target.clone(),request_id:first.request_id.clone(),accept:true,text:None};
        if workspace.user_command(forged_user.clone()).await.unwrap_err()!=WorkspaceError::Admission(RunAdmissionError::UserInputLocked) {return Err("User answered an Agent dialog".into());}
        if workspace.agent_command(&run,forged_user).await.unwrap_err()!=WorkspaceError::UnsupportedAction {return Err("Agent used the user dialog channel".into());}
        let second = workspace.respond_dialog(&run,BrowserDialogReply {target:first.target.clone(),request_id:first.request_id.clone(),accept:true,text:None}).await.map_err(|e|e.to_string())?;
        let BrowserActionOutcome::AwaitingDialog {dialog:second} = second.outcome else {return Err("Chained dialog was reported as completed".into());};
        if second.message!="Second dialog" || second.request_id==first.request_id {return Err("Chained dialog identity was lost".into());}
        if workspace.respond_dialog(&run,BrowserDialogReply {target:first.target,request_id:first.request_id,accept:false,text:None}).await.unwrap_err()!=WorkspaceError::StaleTarget {return Err("Prior dialog answered a later dialog".into());}
        let completed = workspace.respond_dialog(&run,BrowserDialogReply {target:second.target,request_id:second.request_id,accept:true,text:Some("Agent 中文".into())}).await.map_err(|e|e.to_string())?;
        if !matches!(completed.outcome,BrowserActionOutcome::Completed) {return Err("Original input did not complete after final response".into());}
        if super::evaluate(&view,"dialogClicks===1 && dialogTrusted && firstReply===true && secondReply==='Agent 中文'").await?!=true {return Err("Native dialog chain did not preserve the single trusted input".into());}
        let observed = workspace.observe(&run,None).await.map_err(|e|e.to_string())?;
        let element = observed.elements.iter().find(|element|element.name=="Dialog action").ok_or("Second dialog action missing")?.reference.clone();
        let waiting = workspace.act(&run,BrowserAction::click(element)).await.map_err(|e|e.to_string())?;
        let BrowserActionOutcome::AwaitingDialog {dialog:stopped} = waiting.outcome else {return Err("Stop fixture was not waiting for a dialog".into());};
        run.cancel();
        tokio::time::timeout(std::time::Duration::from_secs(5),workspace.settle_run(&run)).await.map_err(|_|"Stop did not drain the dialog/input chain")?.map_err(|e|e.to_string())?;
        if workspace.snapshot().await.map_err(|e|e.to_string())?.run.input_state!=BrowserInputState::AgentRunning {return Err("Settled input unlocked before terminal completion".into());}
        workspace.finish_run(&run).await.map_err(|e|e.to_string())?;
        if workspace.snapshot().await.map_err(|e|e.to_string())?.run.input_state!=BrowserInputState::UserReady {return Err("Completed run kept user input locked".into());}
        if super::evaluate(&view,"dialogClicks===2 && firstReply===false && secondReply===null").await?!=true {return Err("Stop did not dismiss subsequent dialogs in the same callback".into());}
        // Ordinary-user command path, with the native view occluded exactly as
        // the renderer does for its tab-local dialog. This invokes native JS;
        // renderer form interaction is covered separately, not simulated here.
        let mut events = dialogs::subscribe(&view).await.map_err(|e|e.to_string())?.ok_or("Missing dialog subscription")?;
        let before_revision = workspace.snapshot().await.map_err(|e|e.to_string())?.runtime.unwrap().revision;
        let child = view.clone();
        let mut user_page = tokio::spawn(async move {super::evaluate(&child,"prompt('User website prompt','默认')").await});
        let user_dialog = tokio::time::timeout(std::time::Duration::from_secs(3),async {
            loop {
                if let Some(dialog) = events.borrow_and_update().clone() {break Ok::<_,String>(dialog);}
                events.changed().await.map_err(|e|e.to_string())?;
            }
        }).await.map_err(|_|"User dialog was not restored after Agent stop")??;
        let published=workspace.snapshot().await.map_err(|e|e.to_string())?.runtime.unwrap();
        if published.tabs.iter().find(|tab|tab.target.tab_id==user_dialog.target.tab_id).and_then(|tab|tab.script_dialog.as_ref())!=Some(&user_dialog) || published.revision<=before_revision {return Err("User dialog snapshot/revision was not published".into());}
        workspace.set_surface(BrowserSurfaceBounds {x:20.0,y:60.0,width:900.0,height:600.0},false,Default::default()).await.map_err(|e|e.to_string())?;
        let user_reply = BrowserTabCommand::Dialog {target:user_dialog.target,request_id:user_dialog.request_id,accept:true,text:Some("用户 中文".into())};
        workspace.user_command(user_reply.clone()).await.map_err(|e|e.to_string())?;
        let answer = tokio::time::timeout(std::time::Duration::from_secs(3),&mut user_page).await.map_err(|_|"User reply did not release native JS")?.map_err(|e|e.to_string())??;
        if answer!="用户 中文" || workspace.snapshot().await.map_err(|e|e.to_string())?.runtime.unwrap().tabs.iter().any(|tab|tab.script_dialog.is_some()) {return Err("User reply did not update the native page/snapshot".into());}
        if workspace.user_command(user_reply).await.unwrap_err()!=WorkspaceError::StaleTarget {return Err("Repeated user reply was accepted".into());}
        workspace.set_surface(BrowserSurfaceBounds {x:20.0,y:60.0,width:900.0,height:600.0},true,Default::default()).await.map_err(|e|e.to_string())?;
        let child = view.clone();
        let mut before_run = tokio::spawn(async move {super::evaluate(&child,"confirm('User dialog before Agent begins')").await});
        tokio::time::timeout(std::time::Duration::from_secs(3),async {
            loop {
                if events.borrow_and_update().is_some() {break Ok::<_,String>(());}
                events.changed().await.map_err(|e|e.to_string())?;
            }
        }).await.map_err(|_|"User dialog before run did not appear")??;
        let next = tokio::time::timeout(std::time::Duration::from_secs(5),workspace.begin_run()).await.map_err(|_|"Agent begin deadlocked behind a user dialog")?.map_err(|e|e.to_string())?;
        if tokio::time::timeout(std::time::Duration::from_secs(3),&mut before_run).await.map_err(|_|"Agent begin did not release user JS")?.map_err(|e|e.to_string())??!=false || workspace.snapshot().await.map_err(|e|e.to_string())?.runtime.unwrap().tabs.iter().any(|tab|tab.script_dialog.is_some()) {return Err("Agent begin did not cancel the prior user dialog".into());}
        next.require_explicit_finish();
        if workspace.respond_dialog(&run,BrowserDialogReply {target:stopped.target.clone(),request_id:stopped.request_id.clone(),accept:true,text:None}).await.unwrap_err()!=WorkspaceError::Admission(RunAdmissionError::StaleRun) {return Err("Old run retained dialog authority".into());}
        if workspace.respond_dialog(&next,BrowserDialogReply {target:stopped.target,request_id:stopped.request_id,accept:true,text:None}).await.unwrap_err()!=WorkspaceError::StaleTarget {return Err("Stopped dialog leaked into a new run".into());}
        let observed = workspace.observe(&next,None).await.map_err(|e|e.to_string())?;
        let element = observed.elements.iter().find(|element|element.name=="Dialog action").ok_or("New run lost the same page")?.reference.clone();
        if !matches!(workspace.act(&next,BrowserAction::click(element)).await.map_err(|e|e.to_string())?.outcome,BrowserActionOutcome::AwaitingDialog {..}) {return Err("New run did not restore dialog handling".into());}
        tokio::time::timeout(std::time::Duration::from_secs(5),workspace.close()).await.map_err(|_|"Closing a runtime with pending input hung")?.map_err(|e|e.to_string())?;
        if app.get_webview(view.label()).is_some() {return Err("Closed dialog runtime retained its native view".into());}
        Ok(serde_json::json!({"single_trusted_click":true,"chained_dialogs":true,"pending_actions_not_replayed":true,"stop_drains_following_dialogs":true,"terminal_before_unlock":true,"stale_run_rejected":true,"same_page_new_run":true,"pending_runtime_close":true,"user_dialog_after_stop":true,"user_reply_while_native_occluded":true,"dialog_snapshot_published_and_cleared":true,"agent_cannot_use_user_channel":true,"agent_begin_cancels_user_dialog":true}))
    }.await;
    if let Err(error) = &result {
        eprintln!("DIALOG_RUNTIME_STAGE_FAIL {error}");
    }
    let closed = service.shutdown().await.map_err(|e| e.to_string());
    result.and_then(|result| closed.map(|_| result))
}
