//! Production host installation across initial documents, navigation and popup.
use nomifun_browser_platform::{
    runtime::*,
    workspace::{BrowserResource, BrowserResourceService},
};
use std::{sync::Arc, time::Duration};
use tauri::Manager;

async fn dialog(workspace: &BrowserResource, message: &str) -> Result<BrowserDialog, String> {
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
    workspace: &Arc<BrowserResource>,
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
    let service = Arc::new(BrowserResourceService::new(Arc::new(
        super::host::DesktopBrowserHost::new(app.clone()),
    )));
    let workspace = service
        .ensure(
            super::browser_resource_fixture::authority(
                "navigation-dialog-user",
                "navigation-dialog-conversation",
                "native-webview2-v2",
            ),
            BrowserProfile::Ephemeral,
        )
        .await
        .map_err(|e| e.to_string())?;
    let result=async {
        let initial_url=format!("{base}dialog-initial");
        // Establish the one layout lease before native creation. After the
        // prompt reply, no layout/show operation may repair document visibility.
        workspace.set_surface(BrowserSurfaceBounds {x:20.0,y:60.0,width:900.0,height:600.0},true,Default::default()).await.map_err(|e|e.to_string())?;
        eprintln!("DIALOG_NAVIGATION_STAGE initial_create");
        tokio::time::timeout(Duration::from_secs(5),workspace.user_command(BrowserTabCommand::Create {url:initial_url.clone()})).await.map_err(|_|"Initial page dialog blocked the user command gate")?.map_err(|e|e.to_string())?;
        let initial=dialog(&workspace,"Initial document prompt").await?;
        eprintln!("DIALOG_NAVIGATION_STAGE initial_reply");
        let root_id=initial.target.tab_id.clone();
        let view=app.get_webview(&root_id).ok_or("Missing initial native page")?;
        workspace.user_command(BrowserTabCommand::Dialog {target:initial.target,request_id:initial.request_id,accept:true,text:Some("初始确认".into())}).await.map_err(|e|e.to_string())?;
        if super::evaluate(&view,"initialCompleted && initialAnswer==='初始确认'").await?!=true {return Err("Initial native prompt response did not reach the page".into());}
        let initial_visible_input=initial_prompt_input(&workspace,&view,&root_id).await?;
        if super::evaluate(&view,"typeof Notification==='function' && Function.prototype.toString.call(Notification).includes('[native code]')").await?!=true {return Err("Application notification plugin replaced the native website API".into());}
        let run=workspace.begin_run().await.map_err(|e|e.to_string())?;
        run.require_explicit_finish();
        let arm=element(&workspace,&run,"Arm leave confirmation").await?;
        eprintln!("DIALOG_NAVIGATION_STAGE arm_beforeunload");
        workspace.act(&run,BrowserAction::click(arm)).await.map_err(|e|e.to_string())?;
        let before_leave=workspace.snapshot().await.map_err(|e|e.to_string())?.runtime.unwrap().tabs.into_iter().find(|tab|tab.target.tab_id==root_id).ok_or("Missing beforeunload source tab")?;
        let retained_nonce=super::evaluate(&view,"window.fixtureBeforeUnloadNonce=crypto.randomUUID()").await?;
        let target=before_leave.target.clone();
        eprintln!("DIALOG_NAVIGATION_STAGE navigate_for_stay");
        tokio::time::timeout(Duration::from_secs(5),workspace.agent_command(&run,BrowserTabCommand::Navigate {target,url:base.into()})).await.map_err(|_|"beforeunload blocked the navigation tool gate")?.map_err(|e|e.to_string())?;
        let leave=dialog(&workspace,"beforeunload").await?;
        eprintln!("DIALOG_NAVIGATION_STAGE stay_reply");
        workspace.respond_dialog(&run,reply(leave,false,None)).await.map_err(|e|e.to_string())?;
        let rejected_deadline=tokio::time::Instant::now()+Duration::from_secs(5);
        let rejected=loop {
            let tab=workspace.snapshot().await.map_err(|e|e.to_string())?.runtime.ok_or("Missing rejected navigation runtime")?
                .tabs.into_iter().find(|tab|tab.target.tab_id==root_id).ok_or("Missing rejected navigation tab")?;
            if tab.load.as_ref().is_some_and(|load|load.phase==BrowserNavigationPhase::Cancelled) {break tab;}
            if tokio::time::Instant::now()>=rejected_deadline{return Err("Rejected beforeunload did not reach a native cancelled state".into());}
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        let rejected_load=rejected.load.as_ref().ok_or("Missing beforeunload cancellation facts")?;
        if rejected.lifecycle!=BrowserTabLifecycle::Stopped || rejected.target.document_generation!=before_leave.target.document_generation
            || rejected.script_dialog.is_some() || rejected_load.cancellation_reason!=Some(BrowserCancellationReason::NavigationRejected)
            || rejected_load.content_state!=BrowserContentState::RetainedDocument || rejected_load.content_url.as_deref()!=Some(initial_url.as_str()) {
            return Err("Rejected beforeunload lost precise cancellation or retained document authority".into());
        }
        if super::evaluate(&view,"armTrusted && initialCompleted && initialAnswer==='初始确认' && location.pathname==='/dialog-initial'").await?!=true
            || super::evaluate(&view,"fixtureBeforeUnloadNonce").await?!=retained_nonce {return Err("Cancelling beforeunload failed to preserve the actual document and nonce".into());}
        let target=workspace.snapshot().await.map_err(|e|e.to_string())?.runtime.unwrap().tabs.iter().find(|tab|tab.target.tab_id==root_id).unwrap().target.clone();
        eprintln!("DIALOG_NAVIGATION_STAGE navigate_for_leave");
        workspace.agent_command(&run,BrowserTabCommand::Navigate {target,url:base.into()}).await.map_err(|e|e.to_string())?;
        let leave=dialog(&workspace,"beforeunload").await?;
        eprintln!("DIALOG_NAVIGATION_STAGE leave_reply");
        workspace.respond_dialog(&run,reply(leave,true,None)).await.map_err(|e|e.to_string())?;
        let deadline=tokio::time::Instant::now()+Duration::from_secs(5);
        loop {
            if super::evaluate(&view,"location.pathname==='/' && document.readyState==='complete'").await.unwrap_or_default()==true {break;}
            if tokio::time::Instant::now()>=deadline {return Err("Accepting beforeunload did not navigate the real page".into());}
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let target=workspace.snapshot().await.map_err(|e|e.to_string())?.runtime.unwrap().tabs.iter().find(|tab|tab.target.tab_id==root_id).unwrap().target.clone();
        eprintln!("DIALOG_NAVIGATION_STAGE next_initial_prompt");
        workspace.agent_command(&run,BrowserTabCommand::Navigate {target,url:initial_url}).await.map_err(|e|e.to_string())?;
        let initial=dialog(&workspace,"Initial document prompt").await?;
        workspace.respond_dialog(&run,reply(initial,true,Some("Agent initial answer"))).await.map_err(|e|e.to_string())?;
        if super::evaluate(&view,"initialAnswer==='Agent initial answer'").await?!=true {return Err("Agent navigation did not answer the next document's prompt".into());}

        // A timer has no owning input task. Observation must still surface the
        // dialog, retain any blocked read, and let the same run answer it.
        let child=view.clone();
        eprintln!("DIALOG_NAVIGATION_STAGE async_observation");
        let trigger=tokio::spawn(async move {super::evaluate(&child,"setTimeout(()=>{window.asyncAnswer=confirm('Async observation dialog')},0);true").await});
        let asynchronous=dialog(&workspace,"Async observation dialog").await?;
        let observed=tokio::time::timeout(Duration::from_secs(5),workspace.observe(&run,None)).await.map_err(|_|"Observation blocked behind async dialog")?.map_err(|e|e.to_string())?;
        if observed.script_dialog.as_ref()!=Some(&asynchronous) || !observed.elements.is_empty() {return Err("Observation did not explicitly return the paused dialog".into());}
        workspace.respond_dialog(&run,reply(asynchronous,false,None)).await.map_err(|e|e.to_string())?;
        trigger.await.map_err(|e|e.to_string())??;
        if super::evaluate(&view,"asyncAnswer===false").await?!=true {return Err("Async dialog response was not delivered".into());}

        let child=view.clone();
        eprintln!("DIALOG_NAVIGATION_STAGE async_screenshot");
        let trigger=tokio::spawn(async move {super::evaluate(&child,"setTimeout(()=>alert('Async screenshot dialog'),0);true").await});
        let screenshot_dialog=dialog(&workspace,"Async screenshot dialog").await?;
        if tokio::time::timeout(Duration::from_secs(5),workspace.screenshot(&run,None)).await.map_err(|_|"Screenshot blocked behind async dialog")?.err()!=Some(WorkspaceError::DialogPending) {return Err("Screenshot returned a fabricated capture while a dialog was pending".into());}
        workspace.respond_dialog(&run,reply(screenshot_dialog,true,None)).await.map_err(|e|e.to_string())?;
        trigger.await.map_err(|e|e.to_string())??;

        let popup_button=element(&workspace,&run,"Open dialog popup").await?;
        eprintln!("DIALOG_NAVIGATION_STAGE popup");
        workspace.act(&run,BrowserAction::click(popup_button)).await.map_err(|e|e.to_string())?;
        let popup=dialog(&workspace,"Popup initial confirmation").await?;
        if popup.target.tab_id==root_id {return Err("Popup dialog reused the opener identity".into());}
        let child=app.get_webview(&popup.target.tab_id).ok_or("Missing native popup dialog page")?;
        workspace.respond_dialog(&run,reply(popup,true,None)).await.map_err(|e|e.to_string())?;
        if super::evaluate(&child,"popupLoaded && popupAnswer===true && !!opener").await?!=true || super::evaluate(&view,"popupTrusted").await?!=true {return Err("Popup initial dialog did not preserve its native opener and trusted click".into());}
        workspace.finish_run(&run).await.map_err(|e|e.to_string())?;
        Ok(serde_json::json!({"production_install_before_first_document":true,"initial_user_prompt":true,"initial_user_prompt_native_visibility_and_input":initial_visible_input,"beforeunload_cancel_preserves_page":true,
            "beforeunload_cancel":{"native_phase":rejected_load.phase,"native_lifecycle":rejected.lifecycle,"reason":rejected_load.cancellation_reason,
                "retained_document_generation_and_nonce":true},
            "beforeunload_accept_navigates":true,"agent_initial_document_prompt":true,"async_dialog_observation":true,"async_dialog_screenshot_settlement":true,"popup_initial_dialog":true,"native_opener_preserved":true}))
    }.await;
    if let Err(error) = &result {
        eprintln!("DIALOG_NAVIGATION_STAGE_FAIL {error}");
        if let Ok(snapshot)=workspace.snapshot().await {
            if let Some(runtime)=snapshot.runtime {
                for tab in runtime.tabs {
                    // User-only Workspace diagnostics deliberately reject an
                    // Agent run. This controlled fixture can read the owned
                    // native port directly without granting a production tool.
                    if let Some(view)=app.get_webview(&tab.target.tab_id) {
                        if let Ok(report)=super::windows::navigation::report(&view,tab.target).await {
                            eprintln!("DIALOG_NAVIGATION_FAILURE_EVIDENCE {}",serde_json::to_string(&report.project_metadata()).unwrap());
                        }
                    }
                }
            }
        }
    }
    let closed = service.shutdown().await.map_err(|e| e.to_string());
    result.and_then(|result| closed.map(|_| result))
}

async fn native_presentation(view:&tauri::Webview)->Result<(bool,bool,bool),String> {
    let (tx,rx)=tokio::sync::oneshot::channel();
    view.with_webview(move|platform|{
        let result=(||->::windows::core::Result<(bool,bool,bool)>{
            let mut visible=::windows::core::BOOL::default();let mut hwnd=::windows::Win32::Foundation::HWND::default();
            unsafe{platform.controller().IsVisible(&mut visible)?;platform.controller().ParentWindow(&mut hwnd)?;
                Ok((visible.as_bool(),::windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(hwnd).as_bool(),
                    ::windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled(hwnd).as_bool()))}
        })().map_err(|e|e.to_string());let _=tx.send(result);
    }).map_err(|e|e.to_string())?;
    rx.await.map_err(|e|e.to_string())?
}

async fn initial_prompt_input(workspace:&Arc<BrowserResource>,view:&tauri::Webview,id:&str)->Result<serde_json::Value,String> {
    eprintln!("DIALOG_NAVIGATION_STAGE initial_reply_native_visibility_and_input");
    let deadline=tokio::time::Instant::now()+Duration::from_secs(5);
    let current=loop {
        let tab=workspace.snapshot().await.map_err(|e|e.to_string())?.runtime.ok_or("Missing initial dialog runtime")?
            .tabs.into_iter().find(|tab|tab.target.tab_id==id).ok_or("Initial prompt page lost its slot")?;
        let presentation=native_presentation(view).await?;
        let body=super::evaluate(view,"initialCompleted && initialAnswer==='初始确认' && !!document.getElementById('initial-native-input')").await?;
        if tab.load.as_ref().is_some_and(|load|load.content_state==BrowserContentState::CurrentDocument)
            && tab.script_dialog.is_none() && presentation==(true,true,true) && body==true {break tab.target;}
        if tokio::time::Instant::now()>=deadline{return Err(format!("Prompt reply did not restore an actually visible usable current document: load={:?},presentation={presentation:?}",tab.load));}
        tokio::time::sleep(Duration::from_millis(15)).await;
    };
    super::native_fixture_click(view,"#initial-native-input").await?;
    super::windows::protocol_call(view,"Input.insertText",serde_json::json!({"text":"initial-native-entry"})).await?;
    let proof=super::evaluate(view,"({value:document.getElementById('initial-native-input').value,trusted:initialNativeInputTrusted,focused:document.activeElement===document.getElementById('initial-native-input')})").await?;
    let after=workspace.snapshot().await.map_err(|e|e.to_string())?.runtime.ok_or("Missing initial prompt input runtime")?
        .tabs.into_iter().find(|tab|tab.target.tab_id==id).ok_or("Initial prompt input lost its page")?;
    if after.target!=current || native_presentation(view).await?!=(true,true,true)
        || proof["value"]!="initial-native-entry" || proof["trusted"]!=true || proof["focused"]!=true {
        return Err(format!("Initial prompt replied document did not receive one trusted native input: {proof}"));
    }
    Ok(serde_json::json!({"same_native_slot_and_document":true,"controller_visible":true,"hwnd_visible":true,"native_user_input_enabled":true,
        "trusted_native_input":true,"exact_input_value_and_focus_verified":true,"layout_or_show_after_reply":false}))
}
