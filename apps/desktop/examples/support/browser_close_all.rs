//! Real close-all lifecycle on disposable profiles, never the user's browser.
use nomifun_browser_platform::{run_guard::BrowserInputState,runtime::*,workspace::BrowserWorkspaceService};
use serde_json::{json,Value};
use std::{sync::Arc,time::Duration};
use tauri::Manager;

pub async fn verify(app:&tauri::AppHandle,url:&str)->Result<Value,String> {
    let profile=tempfile::Builder::new().prefix("nomifun-close-all-").tempdir().map_err(|e|e.to_string())?;
    let service=BrowserWorkspaceService::new(Arc::new(super::host::DesktopBrowserHost::new(app.clone())));
    let workspace=service.ensure(BrowserWorkspaceKey{user_id:"close-all".into(),conversation_id:"selected".into()},"provider".into(),BrowserProfile::Persistent(profile.path().to_owned())).await.map_err(|e|e.to_string())?;
    let other=service.ensure(BrowserWorkspaceKey{user_id:"close-all".into(),conversation_id:"unrelated".into()},"provider".into(),BrowserProfile::Ephemeral).await.map_err(|e|e.to_string())?;
    let result:Result<Value,String>=async {
        let first=workspace.user_command(BrowserTabCommand::Create {url:url.into()}).await.map_err(|e|e.to_string())?;
        let first_id=first.active_tab_id.clone().ok_or("Missing first page")?;
        let generation=first.runtime_generation;
        let command=||BrowserTabCommand::CloseAll {runtime_generation:generation};
        let first_view=app.get_webview(&first_id).ok_or("Missing first native page")?;
        tokio::time::timeout(Duration::from_secs(5),super::wait_workspace_page(&workspace,url)).await.map_err(|_|"First page load timed out")??;
        super::evaluate(&first_view,"document.cookie='close_all_identity=owned-fixture; expires=Thu, 01 Jan 2099 00:00:00 GMT; path=/';localStorage.setItem('close_all_identity','owned-fixture');true").await?;
        let second=workspace.user_command(BrowserTabCommand::Create {url:url.into()}).await.map_err(|e|e.to_string())?;
        let second_id=second.active_tab_id.clone().ok_or("Missing second page")?;
        let second_view=app.get_webview(&second_id).ok_or("Missing second native page")?;
        let retained=other.user_command(BrowserTabCommand::Create {url:url.into()}).await.map_err(|e|e.to_string())?;
        let retained_id=retained.active_tab_id.clone().ok_or("Missing unrelated page")?;
        let retained_view=app.get_webview(&retained_id).ok_or("Missing unrelated native page")?;
        tokio::time::timeout(Duration::from_secs(5),super::wait_workspace_page(&other,url)).await.map_err(|_|"Unrelated page load timed out")??;
        let nonce=super::evaluate(&retained_view,"popupNonce").await?;
        if workspace.user_command(BrowserTabCommand::CloseAll {runtime_generation:generation+1}).await.err()!=Some(WorkspaceError::StaleTarget) {return Err("Stale close-all was accepted".into());}
        let run=workspace.begin_run().await.map_err(|e|e.to_string())?;
        if workspace.agent_command(&run,command()).await.err()!=Some(WorkspaceError::UnsupportedAction) {return Err("Agent acquired close-all".into());}
        if workspace.user_command(command()).await.err()!=Some(WorkspaceError::Admission(nomifun_browser_platform::run_guard::RunAdmissionError::UserInputLocked)) {return Err("User close-all bypassed an active run".into());}
        workspace.finish_run(&run).await.map_err(|e|e.to_string())?;
        let target=workspace.snapshot().await.map_err(|e|e.to_string())?.runtime.unwrap().tabs.into_iter().find(|tab|tab.target.tab_id==first_id).unwrap().target;
        workspace.user_command(BrowserTabCommand::Activate {target}).await.map_err(|e|e.to_string())?;
        workspace.set_surface(BrowserSurfaceBounds{x:20.0,y:60.0,width:900.0,height:600.0},true,Default::default()).await.map_err(|e|e.to_string())?;
        super::native_fixture_click(&first_view,"#unique").await?;
        let deadline=tokio::time::Instant::now()+Duration::from_secs(5);
        let tabs=loop {
            let snapshot=workspace.snapshot().await.map_err(|e|e.to_string())?.runtime.unwrap();
            if snapshot.tabs.len()==3 && snapshot.tabs.iter().all(|tab|tab.lifecycle==BrowserTabLifecycle::Ready) {break snapshot.tabs;}
            if tokio::time::Instant::now()>=deadline {return Err("Close-all fixture popup did not finish native registration".into());}
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        let dialog_view=second_view.clone();
        let dialog=tokio::spawn(async move{super::evaluate(&dialog_view,"confirm('close-all-native-dialog')").await});
        loop {
            let snapshot=workspace.snapshot().await.map_err(|e|e.to_string())?.runtime.unwrap();
            if snapshot.tabs.iter().any(|tab|tab.script_dialog.as_ref().is_some_and(|dialog|dialog.message=="close-all-native-dialog")) {break;}
            if tokio::time::Instant::now()>=deadline {return Err("Close-all fixture dialog did not appear".into());}
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let closed=tokio::time::timeout(Duration::from_secs(5),workspace.user_command(command())).await.map_err(|_|"Close-all did not settle the native dialog")?.map_err(|e|e.to_string())?;
        let _=tokio::time::timeout(Duration::from_secs(2),dialog).await.map_err(|_|"Closed page retained a protocol callback")?.map_err(|e|e.to_string())?;
        if !closed.tabs.is_empty() || closed.active_tab_id.is_some() || closed.runtime_generation!=generation {return Err("Close-all replaced the runtime or retained pages".into());}
        if tabs.iter().any(|tab|app.get_webview(&tab.target.tab_id).is_some()) {return Err("Closed native controllers remain registered".into());}
        if app.get_webview(&retained_id).is_none() || super::evaluate(&retained_view,"popupNonce").await?!=nonce {return Err("Close-all touched another conversation's native page".into());}
        if !profile.path().is_dir() {return Err("Closing page tabs deleted the Profile".into());}
        if workspace.snapshot().await.map_err(|e|e.to_string())?.run.input_state!=BrowserInputState::UserReady {return Err("Close-all left an Agent run lock".into());}
        let reopened=workspace.user_command(BrowserTabCommand::Create {url:url.into()}).await.map_err(|e|e.to_string())?;
        let new_id=reopened.active_tab_id.as_ref().ok_or("Missing reopened page")?;
        if tabs.iter().any(|tab|&tab.target.tab_id==new_id) || reopened.runtime_generation!=generation {return Err("Reopen reused a destroyed tab or replaced its runtime".into());}
        tokio::time::timeout(Duration::from_secs(5),super::wait_workspace_page(&workspace,url)).await.map_err(|_|"Reopened page load timed out")??;
        let fresh=app.get_webview(new_id).ok_or("Missing reopened native page")?;
        if super::evaluate(&fresh,"document.cookie.includes('close_all_identity=owned-fixture') && localStorage.getItem('close_all_identity')==='owned-fixture'").await?!=true {return Err("Close-all lost persistent Profile storage".into());}
        workspace.user_command(command()).await.map_err(|e|e.to_string())?;
        if !workspace.user_command(command()).await.map_err(|e|e.to_string())?.tabs.is_empty() {return Err("Empty close-all was not idempotent".into());}
        Ok(json!({"user_only":true,"stale_generation_rejected":true,"popup_and_dialog_closed":true,"native_close_confirmed":true,"same_runtime":true,"persistent_profile_storage_retained":true,"other_conversation_unchanged":true,"empty_idempotent":true}))
    }.await;
    service.shutdown().await.map_err(|e|e.to_string())?;
    super::cleanup_fixture_profile(&profile)?;
    let mut result=result?;
    verify_pending_creation(app,url).await?;
    result["pending_creation_settled"]=json!(true);
    Ok(result)
}

async fn verify_pending_creation(app:&tauri::AppHandle,url:&str)->Result<(),String> {
    let runtime=super::host::DesktopBrowserHost::new(app.clone()).create(CreateBrowserRuntime {
        key:BrowserWorkspaceKey{user_id:"close-all-create".into(),conversation_id:"pending".into()},
        runtime_generation:77,profile:BrowserProfile::Ephemeral,user_input_enabled:true,
    }).await.map_err(|e|e.to_string())?;
    let result:Result<(),String>=async {
        let mut slow=url::Url::parse(url).map_err(|e|e.to_string())?;
        slow.set_path("/slow-navigation");slow.set_query(Some("close-all-create"));
        super::DELAYED_NAVIGATION_STARTED.store(false,std::sync::atomic::Ordering::SeqCst);
        let close=async {
            let deadline=tokio::time::Instant::now()+Duration::from_secs(3);
            while !super::DELAYED_NAVIGATION_STARTED.load(std::sync::atomic::Ordering::SeqCst) {
                if tokio::time::Instant::now()>=deadline {return Err("Pending creation did not reach the fixture server".to_owned());}
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            let candidate=runtime.snapshot().await.map_err(|e|e.to_string())?.active_tab_id.ok_or("Pending creation has no retained candidate")?;
            let closed=runtime.execute(BrowserTabCommand::CloseAll{runtime_generation:77},Default::default()).await.map_err(|e|e.to_string())?;
            if !closed.tabs.is_empty() || app.get_webview(&candidate).is_some() {return Err("Close-all retained its pending creation".into());}
            Ok(())
        };
        let (created,closed)=tokio::time::timeout(Duration::from_secs(5),async {
            tokio::join!(runtime.execute(BrowserTabCommand::Create{url:slow.to_string()},Default::default()),close)
        }).await.map_err(|_|"Close-all deadlocked with native creation")?;
        closed?;
        if !matches!(created,Err(WorkspaceError::Admission(nomifun_browser_platform::run_guard::RunAdmissionError::Cancelled))) {
            return Err(format!("In-flight creation did not cancel: {created:?}"));
        }
        let recovered=runtime.execute(BrowserTabCommand::Create{url:url.into()},Default::default()).await.map_err(|e|e.to_string())?;
        if recovered.tabs.len()!=1 || recovered.runtime_generation!=77 {return Err("Runtime did not recover after closing pending creation".into());}
        Ok(())
    }.await;
    let deadline=tokio::time::Instant::now()+Duration::from_secs(5);
    loop {
        match runtime.close().await {
            Ok(())=>break,
            Err(error) if tokio::time::Instant::now()>=deadline=>return Err(format!("Pending-creation fixture cleanup: {error}")),
            Err(_)=>tokio::time::sleep(Duration::from_millis(25)).await,
        }
    }
    result
}
