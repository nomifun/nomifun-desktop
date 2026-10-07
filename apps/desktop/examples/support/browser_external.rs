//! One local test page is dispatched through the real default-browser handler.
use nomifun_browser_platform::{runtime::*, workspace::BrowserResourceService};
use std::sync::{Arc, atomic::Ordering};

pub(super) async fn verify_downloads(app: &tauri::AppHandle, url: &str) -> Result<serde_json::Value, String> {
    let service = BrowserResourceService::new(Arc::new(super::host::DesktopBrowserHost::new(app.clone())));
    let authority = super::browser_resource_fixture::authority("downloads-folder-fixture", "downloads-folder-fixture", "fixture");
    let key = authority.key();
    let workspace = service.ensure(authority, BrowserProfile::Ephemeral).await.map_err(|e| e.to_string())?;
    let result = async {
        let before = workspace.user_command(BrowserTabCommand::Create { url: url.into() }).await.map_err(|e| e.to_string())?;
        let target = super::wait_workspace_page(&workspace, url).await?;
        let generation = before.runtime_generation;
        let command = || BrowserTabCommand::OpenDownloads { runtime_generation: generation };
        if workspace.user_command(BrowserTabCommand::OpenDownloads { runtime_generation: generation + 1 }).await != Err(WorkspaceError::StaleTarget) {
            return Err("Stale workspace opened a folder".into());
        }
        let run = workspace.begin_run().await.map_err(|e| e.to_string())?;
        if workspace.user_command(command()).await.is_ok() || workspace.agent_command(&run, command()).await != Err(WorkspaceError::UnsupportedAction) {
            return Err("Agent run allowed a folder handoff".into());
        }
        workspace.finish_run(&run).await.map_err(|e| e.to_string())?;
        let after = workspace.user_command(command()).await.map_err(|e| e.to_string())?;
        if after.tabs.len() != 1 || after.tabs[0].target != target || after.tabs[0].url != url {
            return Err("Opening Downloads replaced the embedded page".into());
        }
        Ok(serde_json::json!({"os_folder_handoff_acknowledged":true,"native_tab_preserved":true,"stale_and_running_rejected":true}))
    }.await;
    service.close(&key).await.map_err(|e| e.to_string())?;
    result
}

pub(super) async fn verify(app: &tauri::AppHandle, url: &str) -> Result<serde_json::Value, String> {
    let service=BrowserResourceService::new(Arc::new(super::host::DesktopBrowserHost::new(app.clone())));
    let authority=super::browser_resource_fixture::authority("external-fixture","external-fixture","fixture");
    let key=authority.key();
    let workspace=service.ensure(authority,BrowserProfile::Ephemeral).await.map_err(|e|e.to_string())?;
    let result=async {
        workspace.user_command(BrowserTabCommand::Create {url:url.into()}).await.map_err(|e|e.to_string())?;
        let target=super::wait_workspace_page(&workspace,url).await?;
        workspace.set_surface(BrowserSurfaceBounds{x:20.,y:60.,width:1000.,height:600.},true,Default::default()).await.map_err(|e|e.to_string())?;
        let command=BrowserTabCommand::OpenExternal {target:target.clone()};
        let run=workspace.begin_run().await.map_err(|e|e.to_string())?;
        if workspace.user_command(command.clone()).await.is_ok() || workspace.agent_command(&run,command.clone()).await!=Err(WorkspaceError::UnsupportedAction) {return Err("Run authority allowed external browser launch".into());}
        workspace.finish_run(&run).await.map_err(|e|e.to_string())?;
        let other=url::Url::parse(url).and_then(|url|url.join("/user-downloads")).map_err(|e|e.to_string())?;
        workspace.user_command(BrowserTabCommand::Create {url:other.into()}).await.map_err(|e|e.to_string())?;
        if workspace.user_command(command.clone()).await!=Err(WorkspaceError::NotActionable) {return Err("Inactive tab opened externally".into());}
        workspace.user_command(BrowserTabCommand::Activate {target:target.clone()}).await.map_err(|e|e.to_string())?;
        let mut stale=target.clone();stale.document_generation=stale.document_generation.saturating_sub(1);
        if workspace.user_command(BrowserTabCommand::OpenExternal {target:stale}).await.is_ok() {return Err("Stale target opened externally".into());}
        let before=super::EXTERNAL_PAGE_REQUESTS.load(Ordering::SeqCst);
        if before!=1 {return Err(format!("Unexpected fixture request count before handoff: {before}"));}
        // The dropdown occludes the native view; this must not revoke a user
        // action on the still-current tab or mistakenly unlock a running Agent.
        workspace.set_surface(BrowserSurfaceBounds{x:20.,y:60.,width:1000.,height:600.},false,Default::default()).await.map_err(|e|e.to_string())?;
        eprintln!("BROWSER_EXTERNAL_FIXTURE_URL {url}");
        let snapshot=workspace.user_command(command).await.map_err(|e|e.to_string())?;
        if snapshot.tabs.len()!=2 || snapshot.active_tab_id.as_ref()!=Some(&target.tab_id) || !snapshot.tabs.iter().any(|tab|tab.target==target && tab.url==url) {return Err("System-browser handoff replaced the embedded page".into());}
        let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(15);
        while super::EXTERNAL_PAGE_REQUESTS.load(Ordering::SeqCst)==before {
            if tokio::time::Instant::now()>=deadline {return Err("OS accepted handoff but the external browser never requested the local page".into());}
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        Ok(serde_json::json!({"native_source_and_os_dispatch":true,"external_page_requested":true,"embedded_tab_preserved":true,"run_inactive_and_stale_rejected":true}))
    }.await;
    service.close(&key).await.map_err(|e|e.to_string())?;
    result
}
