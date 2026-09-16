//! Exact-tab close while page script or browser work waits for a dialog.
use nomifun_browser_platform::{
    run_guard::BrowserInputState,
    runtime::*,
    workspace::{BrowserWorkspace, BrowserWorkspaceService},
};
use std::{sync::Arc, time::Duration};
use tauri::Manager;
async fn find_dialog(workspace: &BrowserWorkspace, message: &str) -> Result<BrowserDialog, String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let snapshot = workspace
            .snapshot()
            .await
            .map_err(|e| e.to_string())?
            .runtime
            .ok_or("Missing close fixture runtime")?;
        if let Some(dialog) = snapshot
            .tabs
            .iter()
            .filter_map(|tab| tab.script_dialog.as_ref())
            .find(|dialog| {
                dialog.message == message
                    || (message == "beforeunload" && dialog.kind == BrowserDialogKind::BeforeUnload)
            })
        {
            return Ok(dialog.clone());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!("Close fixture dialog missing: {message}"));
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
async fn create(workspace: &Arc<BrowserWorkspace>, url: &str) -> Result<BrowserTabTarget, String> {
    let snapshot = workspace
        .user_command(BrowserTabCommand::Create { url: url.into() })
        .await
        .map_err(|e| e.to_string())?;
    let id = snapshot.active_tab_id.ok_or("Missing created tab")?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let snapshot = workspace
            .snapshot()
            .await
            .map_err(|e| e.to_string())?
            .runtime
            .unwrap();
        if let Some(tab) = snapshot
            .tabs
            .iter()
            .find(|tab| tab.target.tab_id == id && tab.lifecycle == BrowserTabLifecycle::Ready)
        {
            return Ok(tab.target.clone());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("Close fixture page did not finish loading".into());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
pub(super) async fn verify(
    app: &tauri::AppHandle,
    base: &str,
) -> Result<serde_json::Value, String> {
    let service = Arc::new(BrowserWorkspaceService::new(Arc::new(
        super::host::DesktopBrowserHost::new(app.clone()),
    )));
    let workspace = service
        .ensure(
            BrowserWorkspaceKey {
                user_id: "tab-close-user".into(),
                conversation_id: "tab-close-conversation".into(),
            },
            "native-webview2-v2".into(),
            BrowserProfile::Ephemeral,
        )
        .await
        .map_err(|e| e.to_string())?;
    let result=async {
        workspace.user_command(BrowserTabCommand::Create {url:format!("{base}dialog-initial")}).await.map_err(|e|e.to_string())?;
        let initial=find_dialog(&workspace,"Initial document prompt").await?;
        let closed=tokio::time::timeout(Duration::from_secs(5),workspace.user_command(BrowserTabCommand::Close {target:initial.target.clone()})).await.map_err(|_|"User close waited for the initial prompt")?.map_err(|e|e.to_string())?;
        if !closed.tabs.is_empty() || app.get_webview(&initial.target.tab_id).is_some() {return Err("Initial-dialog close did not remove the real native tab".into());}
        let first=create(&workspace,base).await?;
        let second=create(&workspace,base).await?;
        workspace.set_surface(BrowserSurfaceBounds {x:20.0,y:60.0,width:900.0,height:600.0},true,Default::default()).await.map_err(|e|e.to_string())?;
        let first_view=app.get_webview(&first.tab_id).ok_or("Missing first close target")?;
        let second_view=app.get_webview(&second.tab_id).ok_or("Missing retained page")?;
        let run=workspace.begin_run().await.map_err(|e|e.to_string())?;
        run.require_explicit_finish();
        workspace.agent_command(&run,BrowserTabCommand::Activate {target:first.clone()}).await.map_err(|e|e.to_string())?;
        super::evaluate(&first_view,"document.body.innerHTML='<button id=close-source>Close blocked input</button>';document.getElementById('close-source').onclick=()=>confirm('Close this input');true").await?;
        let observed=workspace.observe(&run,Some(first.tab_id.clone())).await.map_err(|e|e.to_string())?;
        let element=observed.elements.iter().find(|element|element.name=="Close blocked input").ok_or("Missing close input control")?.reference.clone();
        let action=workspace.act(&run,BrowserAction::click(element)).await.map_err(|e|e.to_string())?;
        let BrowserActionOutcome::AwaitingDialog {dialog:blocked}=action.outcome else {return Err("Close input was not suspended at its native dialog".into());};
        let retained=second_view.clone();
        let other=tokio::spawn(async move {super::evaluate(&retained,"confirm('Keep the other page dialog')").await});
        let other_dialog=find_dialog(&workspace,"Keep the other page dialog").await?;
        let mut stale=blocked.target.clone(); stale.document_generation+=1;
        if workspace.agent_command(&run,BrowserTabCommand::Close {target:stale}).await.err()!=Some(WorkspaceError::StaleTarget) {return Err("Stale close was admitted".into());}
        let closed=tokio::time::timeout(Duration::from_secs(5),workspace.agent_command(&run,BrowserTabCommand::Close {target:blocked.target.clone()})).await.map_err(|_|"Agent tab close waited on its blocked input")?.map_err(|e|e.to_string())?;
        if closed.tabs.iter().any(|tab|tab.target.tab_id==first.tab_id) || app.get_webview(&first.tab_id).is_some() {return Err("Agent close retained the suspended native page".into());}
        let stale_command=tokio::time::timeout(Duration::from_secs(1),super::windows::protocol_call(&first_view,"Runtime.evaluate",serde_json::json!({"expression":"true"}))).await.map_err(|_|"A command against the closed native view hung")?;
        if stale_command.is_ok(){return Err("Closed native view accepted another renderer command".into());}
        if closed.tabs.iter().find(|tab|tab.target.tab_id==second.tab_id).and_then(|tab|tab.script_dialog.as_ref())!=Some(&other_dialog) {return Err("Closing one tab answered the other page's dialog".into());}
        if workspace.snapshot().await.map_err(|e|e.to_string())?.run.input_state!=BrowserInputState::AgentRunning {return Err("Closing a tab released the Agent run".into());}
        workspace.respond_dialog(&run,BrowserDialogReply {target:other_dialog.target,request_id:other_dialog.request_id,accept:false,text:None}).await.map_err(|e|e.to_string())?;
        if other.await.map_err(|e|e.to_string())??!=false {return Err("Other dialog did not retain its independent response".into());}
        let observed=workspace.observe(&run,Some(second.tab_id.clone())).await.map_err(|e|e.to_string())?;
        let button=observed.elements.iter().find(|element|element.name=="验证点击").ok_or("Remaining page stopped being actionable")?.reference.clone();
        workspace.act(&run,BrowserAction::click(button)).await.map_err(|e|e.to_string())?;
        super::evaluate(&second_view,"window.onbeforeunload=e=>{e.preventDefault();e.returnValue=''};true").await?;
        workspace.finish_run(&run).await.map_err(|e|e.to_string())?;
        let target=workspace.snapshot().await.map_err(|e|e.to_string())?.runtime.unwrap().tabs[0].target.clone();
        workspace.user_command(BrowserTabCommand::Navigate {target,url:format!("{base}dialog-initial")}).await.map_err(|e|e.to_string())?;
        let leave=find_dialog(&workspace,"beforeunload").await?;
        let closed=tokio::time::timeout(Duration::from_secs(5),workspace.user_command(BrowserTabCommand::Close {target:leave.target})).await.map_err(|_|"User close waited behind beforeunload navigation")?.map_err(|e|e.to_string())?;
        if !closed.tabs.is_empty() || app.get_webview(&second.tab_id).is_some() {return Err("User close retained the pending navigation page".into());}
        let reopened=create(&workspace,base).await?;
        if reopened.tab_id==second.tab_id {return Err("Closed page identity was reused".into());}
        Ok(serde_json::json!({"user_initial_dialog_close":true,"agent_paused_input_close":true,"other_dialog_unchanged":true,"run_not_cancelled":true,"stale_close_rejected":true,"closed_view_protocol_rejected":true,"user_beforeunload_close":true,"new_tab_after_close":true}))
    }.await;
    if let Err(error) = &result {
        eprintln!("DIALOG_CLOSE_STAGE_FAIL {error}");
    }
    let cleanup = service.shutdown().await.map_err(|e| e.to_string());
    let mut result = result.and_then(|result| cleanup.map(|_| result))?;
    verify_parallel_close(app, base).await?;
    result["parallel_close_settles"] = serde_json::json!(true);
    Ok(result)
}

async fn verify_parallel_close(app: &tauri::AppHandle, base: &str) -> Result<(), String> {
    let runtime = super::host::DesktopBrowserHost::new(app.clone())
        .create(CreateBrowserRuntime {
            key: BrowserWorkspaceKey {
                user_id: "parallel-close".into(),
                conversation_id: "parallel-close".into(),
            },
            runtime_generation: 1,
            profile: BrowserProfile::Ephemeral,
            user_input_enabled: true,
        })
        .await
        .map_err(|e| e.to_string())?;
    let result = async {
        let initial = runtime
            .execute(
                BrowserTabCommand::Create { url: base.into() },
                Default::default(),
            )
            .await
            .map_err(|e| e.to_string())?;
        let id = initial.active_tab_id.ok_or("Missing parallel close tab")?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let target =
            loop {
                let snapshot = runtime.snapshot().await.map_err(|e| e.to_string())?;
                if let Some(tab) = snapshot.tabs.iter().find(|tab| {
                    tab.target.tab_id == id && tab.lifecycle == BrowserTabLifecycle::Ready
                }) {
                    break tab.target.clone();
                }
                if tokio::time::Instant::now() >= deadline {
                    return Err("Parallel close page did not load".into());
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            };
        let view = app.get_webview(&id).ok_or("Parallel close view missing")?;
        let child = view.clone();
        let mut paused = tokio::spawn(async move {
            super::evaluate(&child, "confirm('Parallel close dialog')").await
        });
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            if runtime
                .snapshot()
                .await
                .map_err(|e| e.to_string())?
                .tabs
                .iter()
                .any(|tab| tab.script_dialog.is_some())
            {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                return Err("Parallel close dialog did not open".into());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let (one, two) = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(
                runtime.execute(
                    BrowserTabCommand::Close {
                        target: target.clone()
                    },
                    Default::default()
                ),
                runtime.execute(BrowserTabCommand::Close { target }, Default::default())
            )
        })
        .await
        .map_err(|_| "Concurrent native closes waited on one another")?;
        if one.is_err() && two.is_err() {
            return Err(format!("Both concurrent closes failed: {one:?}; {two:?}"));
        }
        for result in [one, two] {
            if let Err(error) = result {
                if error != WorkspaceError::StaleTarget {
                    return Err(format!("Unexpected concurrent close failure: {error}"));
                }
            }
        }
        let _ = tokio::time::timeout(Duration::from_secs(3), &mut paused)
            .await
            .map_err(|_| "Concurrent close left paused native JS")?
            .map_err(|e| e.to_string())?;
        if app.get_webview(view.label()).is_some()
            || !runtime
                .snapshot()
                .await
                .map_err(|e| e.to_string())?
                .tabs
                .is_empty()
        {
            return Err("Concurrent close retained a native page".into());
        }
        Ok(())
    }
    .await;
    let closed = runtime.close().await.map_err(|e| e.to_string());
    result.and(closed)
}
