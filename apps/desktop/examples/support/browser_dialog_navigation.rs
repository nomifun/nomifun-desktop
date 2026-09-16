//! Production host installation across initial documents, navigation and popup.
use nomifun_browser_platform::{
    runtime::*,
    workspace::{BrowserWorkspace, BrowserWorkspaceService},
};
use std::{sync::Arc, time::Duration};
use tauri::Manager;

async fn dialog(workspace: &BrowserWorkspace, message: &str) -> Result<BrowserDialog, String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let snapshot = workspace
            .snapshot()
            .await
            .map_err(|e| e.to_string())?
            .runtime
            .ok_or("Missing dialog runtime")?;
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
            return Err(format!(
                "Production dialog missing: {message}; snapshot={snapshot:?}"
            ));
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
fn reply(dialog: BrowserDialog, accept: bool, text: Option<&str>) -> BrowserDialogReply {
    BrowserDialogReply {
        target: dialog.target,
        request_id: dialog.request_id,
        accept,
        text: text.map(str::to_owned),
    }
}
async fn element(
    workspace: &Arc<BrowserWorkspace>,
    run: &nomifun_browser_platform::run_guard::BrowserRunGuard,
    name: &str,
) -> Result<BrowserElementRef, String> {
    let observed = workspace
        .observe(run, None)
        .await
        .map_err(|e| e.to_string())?;
    observed
        .elements
        .into_iter()
        .find(|element| element.name == name)
        .map(|element| element.reference)
        .ok_or_else(|| format!("Missing native element: {name}"))
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
                user_id: "navigation-dialog-user".into(),
                conversation_id: "navigation-dialog-conversation".into(),
            },
            "native-webview2-v2".into(),
            BrowserProfile::Ephemeral,
        )
        .await
        .map_err(|e| e.to_string())?;
    let result=async {
        let initial_url=format!("{base}dialog-initial");
        tokio::time::timeout(Duration::from_secs(5),workspace.user_command(BrowserTabCommand::Create {url:initial_url.clone()})).await.map_err(|_|"Initial page dialog blocked the user command gate")?.map_err(|e|e.to_string())?;
        let initial=dialog(&workspace,"Initial document prompt").await?;
        let root_id=initial.target.tab_id.clone();
        let view=app.get_webview(&root_id).ok_or("Missing initial native page")?;
        workspace.set_surface(BrowserSurfaceBounds {x:20.0,y:60.0,width:900.0,height:600.0},true,Default::default()).await.map_err(|e|e.to_string())?;
        workspace.user_command(BrowserTabCommand::Dialog {target:initial.target,request_id:initial.request_id,accept:true,text:Some("初始确认".into())}).await.map_err(|e|e.to_string())?;
        if super::evaluate(&view,"initialCompleted && initialAnswer==='初始确认'").await?!=true {return Err("Initial native prompt response did not reach the page".into());}
        if super::evaluate(&view,"typeof Notification==='function' && Function.prototype.toString.call(Notification).includes('[native code]')").await?!=true {return Err("Application notification plugin replaced the native website API".into());}
        let run=workspace.begin_run().await.map_err(|e|e.to_string())?;
        run.require_explicit_finish();
        let arm=element(&workspace,&run,"Arm leave confirmation").await?;
        workspace.act(&run,BrowserAction::click(arm)).await.map_err(|e|e.to_string())?;
        let target=workspace.snapshot().await.map_err(|e|e.to_string())?.runtime.unwrap().tabs.iter().find(|tab|tab.target.tab_id==root_id).unwrap().target.clone();
        tokio::time::timeout(Duration::from_secs(5),workspace.agent_command(&run,BrowserTabCommand::Navigate {target,url:base.into()})).await.map_err(|_|"beforeunload blocked the navigation tool gate")?.map_err(|e|e.to_string())?;
        let leave=dialog(&workspace,"beforeunload").await?;
        workspace.respond_dialog(&run,reply(leave,false,None)).await.map_err(|e|e.to_string())?;
        if super::evaluate(&view,"armTrusted && location.pathname==='/dialog-initial'").await?!=true {return Err("Cancelling beforeunload failed to preserve the actual page".into());}
        let target=workspace.snapshot().await.map_err(|e|e.to_string())?.runtime.unwrap().tabs.iter().find(|tab|tab.target.tab_id==root_id).unwrap().target.clone();
        workspace.agent_command(&run,BrowserTabCommand::Navigate {target,url:base.into()}).await.map_err(|e|e.to_string())?;
        let leave=dialog(&workspace,"beforeunload").await?;
        workspace.respond_dialog(&run,reply(leave,true,None)).await.map_err(|e|e.to_string())?;
        let deadline=tokio::time::Instant::now()+Duration::from_secs(5);
        loop {
            if super::evaluate(&view,"location.pathname==='/' && document.readyState==='complete'").await.unwrap_or_default()==true {break;}
            if tokio::time::Instant::now()>=deadline {return Err("Accepting beforeunload did not navigate the real page".into());}
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let target=workspace.snapshot().await.map_err(|e|e.to_string())?.runtime.unwrap().tabs.iter().find(|tab|tab.target.tab_id==root_id).unwrap().target.clone();
        workspace.agent_command(&run,BrowserTabCommand::Navigate {target,url:initial_url}).await.map_err(|e|e.to_string())?;
        let initial=dialog(&workspace,"Initial document prompt").await?;
        workspace.respond_dialog(&run,reply(initial,true,Some("Agent initial answer"))).await.map_err(|e|e.to_string())?;
        if super::evaluate(&view,"initialAnswer==='Agent initial answer'").await?!=true {return Err("Agent navigation did not answer the next document's prompt".into());}

        // A timer has no owning input task. Observation must still surface the
        // dialog, retain any blocked read, and let the same run answer it.
        let child=view.clone();
        let trigger=tokio::spawn(async move {super::evaluate(&child,"setTimeout(()=>{window.asyncAnswer=confirm('Async observation dialog')},0);true").await});
        let asynchronous=dialog(&workspace,"Async observation dialog").await?;
        let observed=tokio::time::timeout(Duration::from_secs(5),workspace.observe(&run,None)).await.map_err(|_|"Observation blocked behind async dialog")?.map_err(|e|e.to_string())?;
        if observed.script_dialog.as_ref()!=Some(&asynchronous) || !observed.elements.is_empty() {return Err("Observation did not explicitly return the paused dialog".into());}
        workspace.respond_dialog(&run,reply(asynchronous,false,None)).await.map_err(|e|e.to_string())?;
        trigger.await.map_err(|e|e.to_string())??;
        if super::evaluate(&view,"asyncAnswer===false").await?!=true {return Err("Async dialog response was not delivered".into());}

        let child=view.clone();
        let trigger=tokio::spawn(async move {super::evaluate(&child,"setTimeout(()=>alert('Async screenshot dialog'),0);true").await});
        let screenshot_dialog=dialog(&workspace,"Async screenshot dialog").await?;
        if tokio::time::timeout(Duration::from_secs(5),workspace.screenshot(&run,None)).await.map_err(|_|"Screenshot blocked behind async dialog")?.err()!=Some(WorkspaceError::DialogPending) {return Err("Screenshot returned a fabricated capture while a dialog was pending".into());}
        workspace.respond_dialog(&run,reply(screenshot_dialog,true,None)).await.map_err(|e|e.to_string())?;
        trigger.await.map_err(|e|e.to_string())??;

        let popup_button=element(&workspace,&run,"Open dialog popup").await?;
        workspace.act(&run,BrowserAction::click(popup_button)).await.map_err(|e|e.to_string())?;
        let popup=dialog(&workspace,"Popup initial confirmation").await?;
        if popup.target.tab_id==root_id {return Err("Popup dialog reused the opener identity".into());}
        let child=app.get_webview(&popup.target.tab_id).ok_or("Missing native popup dialog page")?;
        workspace.respond_dialog(&run,reply(popup,true,None)).await.map_err(|e|e.to_string())?;
        if super::evaluate(&child,"popupLoaded && popupAnswer===true && !!opener").await?!=true || super::evaluate(&view,"popupTrusted").await?!=true {return Err("Popup initial dialog did not preserve its native opener and trusted click".into());}
        workspace.finish_run(&run).await.map_err(|e|e.to_string())?;
        Ok(serde_json::json!({"production_install_before_first_document":true,"initial_user_prompt":true,"beforeunload_cancel_preserves_page":true,"beforeunload_accept_navigates":true,"agent_initial_document_prompt":true,"async_dialog_observation":true,"async_dialog_screenshot_settlement":true,"popup_initial_dialog":true,"native_opener_preserved":true}))
    }.await;
    if let Err(error) = &result {
        eprintln!("DIALOG_NAVIGATION_STAGE_FAIL {error}");
    }
    let closed = service.shutdown().await.map_err(|e| e.to_string());
    result.and_then(|result| closed.map(|_| result))
}
