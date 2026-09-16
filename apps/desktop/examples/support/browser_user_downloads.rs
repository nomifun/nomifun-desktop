//! Same-WebView native attachment response, Save dialog and terminal cleanup.
use nomifun_browser_platform::{runtime::*, workspace::BrowserWorkspaceService};
use std::sync::Arc;
use tauri::Manager;

#[derive(Clone, Copy)]
pub(super) enum Check { Lifecycle, Save, ActiveCancel }

pub(super) async fn verify(
    app: &tauri::AppHandle,
    url: &str,
    check: Check,
) -> Result<serde_json::Value, String> {
    let service =
        BrowserWorkspaceService::new(Arc::new(super::host::DesktopBrowserHost::new(app.clone())));
    let key = BrowserWorkspaceKey {
        user_id: "download-fixture".into(),
        conversation_id: "download-fixture".into(),
    };
    let workspace = service
        .ensure(
            key.clone(),
            "native-download-fixture".into(),
            BrowserProfile::Ephemeral,
        )
        .await
        .map_err(|e| e.to_string())?;
    let root = tempfile::Builder::new()
        .prefix("nomifun-user-download-")
        .tempdir()
        .map_err(|e| e.to_string())?;
    let select=!matches!(check,Check::Lifecycle);
    let result=tokio::time::timeout(std::time::Duration::from_secs(240),async {
        workspace.user_command(BrowserTabCommand::Create {url:url.into()}).await.map_err(|e|e.to_string())?;
        let target=super::wait_workspace_page(&workspace,url).await?;
        let view=app.get_webview(&target.tab_id).ok_or("Missing download view")?;
        let bounds=BrowserSurfaceBounds {x:20.,y:60.,width:1000.,height:600.};
        workspace.set_surface(bounds,true,Default::default()).await.map_err(|e|e.to_string())?;
        for phase in match check {Check::Save=>vec!["save"],Check::ActiveCancel=>vec!["active_close"],Check::Lifecycle=>vec!["cancel","hide","agent","navigate","close"]} {
            eprintln!("BROWSER_USER_DOWNLOAD_PHASE {phase}");
            super::native_fixture_click(&view,if phase=="active_close" {"#slow"}else{"#download"}).await?;
            let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(8);
            let picker=loop {
                if let Some(picker)=super::windows::user_downloads::inspect(&view).await?.0 {
                    picker.opened().await?;break picker;
                }
                if tokio::time::Instant::now()>=deadline {return Err("Native download did not open the Save picker".into());}
                tokio::time::sleep(std::time::Duration::from_millis(15)).await;
            };
            let receipt=picker.exit_receipt().await?;
            if select {
                let destination=root.path().join(if phase=="save" {"下载 验收.txt"}else{"中止 验收.txt"});
                eprintln!("BROWSER_USER_DOWNLOAD_SAVE_READY {}",serde_json::json!({"path":destination}));
                let chosen=picker.finished().await?.ok_or("Native download save was cancelled")?;
                if chosen != vec![destination.clone()] {return Err("Save did not select the exact temporary fixture destination".into());}
                let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(10);
                loop {
                    let (_, pending, completed, active)=super::windows::user_downloads::inspect(&view).await?;
                    if (phase=="save" && pending==0 && completed==1) || (phase=="active_close" && active==1) {break;}
                    if tokio::time::Instant::now()>=deadline {return Err("Native download did not reach completed cleanup".into());}
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
                if phase=="save" {
                    if std::fs::read_to_string(destination).map_err(|e|e.to_string())? != "Native WebView download 中文\n" {return Err("Downloaded bytes differ from the HTTP response".into());}
                } else {
                    let run=workspace.begin_run().await.map_err(|e|e.to_string())?;
                    if super::windows::user_downloads::inspect(&view).await?.3!=1 {return Err("Agent entry cancelled an already-authorized user transfer".into());}
                    workspace.finish_run(&run).await.map_err(|e|e.to_string())?;
                    service.close(&key).await.map_err(|e|e.to_string())?;
                    if app.get_webview(&target.tab_id).is_some() {return Err("Closing an active transfer retained its native view".into());}
                }
            } else {
                match phase {
                    "cancel"=>{
                        let snapshot=workspace.snapshot().await.map_err(|e|e.to_string())?.runtime.ok_or("Missing runtime")?;
                        let download=snapshot.downloads.last().ok_or("Missing download status")?;
                        if download.state!=BrowserDownloadState::Choosing || !download.can_cancel {return Err("Save picker status is not cancellable".into());}
                        let result=workspace.user_command(BrowserTabCommand::CancelDownload {target:snapshot.tabs[0].target.clone(),download_id:download.id.clone()}).await.map_err(|e|e.to_string())?;
                        let entry=result.downloads.iter().find(|entry|entry.id==download.id).ok_or("Cancelled history disappeared")?;
                        if entry.state!=BrowserDownloadState::Cancelled || entry.can_cancel {return Err("Cancellation command returned before terminal status".into());}
                    },
                    "hide"=>workspace.set_surface(bounds,false,Default::default()).await.map_err(|e|e.to_string())?,
                    "agent"=>{
                        let run=workspace.begin_run().await.map_err(|e|e.to_string())?;
                        tokio::time::timeout(std::time::Duration::from_millis(100),receipt.clone().wait()).await.map_err(|_|"Agent entered before Save picker exit")?.map_err(|e|e.to_string())?;
                        super::native_fixture_click(&view,"#download").await?;
                        if super::windows::user_downloads::inspect(&view).await?.0.is_some() {return Err("Agent-locked browser opened a user Save picker".into());}
                        workspace.finish_run(&run).await.map_err(|e|e.to_string())?;
                    },
                    "navigate"=>{
                        let current=workspace.snapshot().await.map_err(|e|e.to_string())?.runtime.ok_or("Missing download runtime")?.tabs[0].target.clone();
                        let next=format!("{url}?new");
                        workspace.user_command(BrowserTabCommand::Navigate {target:current,url:next.clone()}).await.map_err(|e|e.to_string())?;
                        super::wait_workspace_page(&workspace,&next).await?;
                    },
                    "close"=>service.close(&key).await.map_err(|e|e.to_string())?,
                    _=>unreachable!(),
                }
                tokio::time::timeout(std::time::Duration::from_secs(5),receipt.clone().wait()).await.map_err(|_|"Download picker did not exit")?.map_err(|e|e.to_string())?;
                if phase!="close" {
                    super::windows::user_downloads::cancel_and_wait(&view,false).await?;
                    if super::windows::user_downloads::inspect(&view).await?.1!=0 {return Err("Cancelled download retained in-flight work".into());}
                }
                if phase=="hide" {workspace.set_surface(bounds,true,Default::default()).await.map_err(|e|e.to_string())?;}
            }
            receipt.wait().await.map_err(|e|e.to_string())?;
        }
        Ok(serde_json::json!({"native_download_events":true,"saved_http_bytes":matches!(check,Check::Save),"active_transfer_survives_agent_then_closes":matches!(check,Check::ActiveCancel),"cancellation_matrix":!select}))
    }).await.unwrap_or_else(|_|Err("User download fixture timed out before cleanup".into()));
    let cleanup = service.close(&key).await.map_err(|e| e.to_string());
    cleanup?;
    root.close().map_err(|e| e.to_string())?;
    result
}
