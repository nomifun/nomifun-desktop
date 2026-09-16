//! Permission conformance uses a disposable localhost page and emulated location.
//! It never reads the user's actual location, camera, microphone or clipboard.
use nomifun_browser_platform::{
    runtime::*,
    workspace::{BrowserWorkspace, BrowserWorkspaceService},
};
use serde_json::{Value, json};
use std::sync::Arc;
use tauri::Manager;
use tokio_util::sync::CancellationToken;
fn message(error: impl std::fmt::Display) -> String {
    error.to_string()
}

async fn visibility(view: &tauri::Webview) -> Result<(bool, bool), String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    view.with_webview(move |platform| {
        let result = (|| -> windows::core::Result<(bool, bool)> {
            let mut visible = windows::core::BOOL::default();
            let mut hwnd = windows::Win32::Foundation::HWND::default();
            unsafe {
                platform.controller().IsVisible(&mut visible)?;
                platform.controller().ParentWindow(&mut hwnd)?;
                Ok((
                    visible.as_bool(),
                    windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(hwnd).as_bool(),
                ))
            }
        })()
        .map_err(message);
        let _ = tx.send(result);
    })
    .map_err(message)?;
    rx.await.map_err(message)?
}

async fn tab(workspace: &Arc<BrowserWorkspace>, id: &str) -> Result<BrowserTabSnapshot, String> {
    workspace
        .snapshot()
        .await
        .map_err(message)?
        .runtime
        .ok_or("Missing permission runtime")?
        .tabs
        .into_iter()
        .find(|tab| tab.target.tab_id == id)
        .ok_or("Missing permission tab".into())
}
async fn refresh(workspace: &Arc<BrowserWorkspace>, view: &tauri::Webview) -> Result<(), String> {
    let target = tab(workspace, view.label()).await?.target;
    let generation = target.document_generation;
    workspace
        .user_command(BrowserTabCommand::Reload { target })
        .await
        .map_err(message)?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let current = tab(workspace, view.label()).await?;
        if current.target.document_generation > generation
            && current.lifecycle == BrowserTabLifecycle::Ready
        {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("Permission retry refresh did not complete".into());
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}
async fn request(
    workspace: &Arc<BrowserWorkspace>,
    view: &tauri::Webview,
) -> Result<(BrowserTabTarget, String), String> {
    super::evaluate(view, "window.geoResult=undefined;true").await?;
    super::native_fixture_click(view, "#geo").await?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let current = tab(workspace, view.label()).await?;
        if current.permission_requests.len() == 1 {
            let request = &current.permission_requests[0];
            if request.kind != "geolocation"
                || !request.origin.starts_with("http://127.0.0.1:")
                || request.origin.contains("/popup")
            {
                return Err(format!(
                    "Permission did not expose its exact safe origin: {request:?}"
                ));
            }
            return Ok((current.target, request.request_id.clone()));
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "Visible UserReady page did not receive a permission request: {:?}; result={}",
                current.permission_requests,
                super::evaluate(view, "window.geoResult").await?
            ));
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}
async fn result(view: &tauri::Webview, expected: &str, timeout: u64) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout);
    loop {
        let value = super::evaluate(view, "window.geoResult").await?;
        if value == expected {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            let state=super::evaluate(view,"({visibility:document.visibilityState,focused:document.hasFocus(),url:location.href})").await?;
            return Err(format!(
                "Permission callback did not settle: expected={expected}, actual={value}, page={state}"
            ));
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}
pub(super) async fn verify(
    app: &tauri::AppHandle,
    url: &str,
    timeout_only: bool,
) -> Result<Value, String> {
    let service =
        BrowserWorkspaceService::new(Arc::new(super::host::DesktopBrowserHost::new(app.clone())));
    let key = BrowserWorkspaceKey {
        user_id: "permission-fixture".into(),
        conversation_id: "permission-fixture".into(),
    };
    let workspace = service
        .ensure_user(key.clone(), BrowserProfile::Ephemeral)
        .await
        .map_err(message)?;
    let evidence=async {
        workspace.user_command(BrowserTabCommand::Create {url:url.into()}).await.map_err(message)?;
        let target=super::wait_workspace_page(&workspace,url).await?;
        let view=app.get_webview(&target.tab_id).ok_or("Missing permission view")?;
        let bounds=BrowserSurfaceBounds {x:20.0,y:60.0,width:1000.0,height:600.0};
        workspace.set_surface(bounds,true,CancellationToken::new()).await.map_err(message)?;
        super::windows::protocol_call(&view,"Emulation.setGeolocationOverride",json!({"latitude":12.0,"longitude":34.0,"accuracy":1.0})).await?;
        let (target,id)=request(&workspace,&view).await?;
        if timeout_only {
            result(&view,"denied-1",32).await?;
            if !tab(&workspace,view.label()).await?.permission_requests.is_empty() {return Err("Expired permission remained in snapshot".into());}
            return Ok(json!({"native_permission_timeout_denied":true}));
        }
        if !matches!(workspace.user_command(BrowserTabCommand::Permission {target:target.clone(),request_id:"not-this-request".into(),allow:true}).await,Err(WorkspaceError::StaleTarget)) {
            return Err("Unknown permission request was accepted".into());
        }
        workspace.user_command(BrowserTabCommand::Permission {target:target.clone(),request_id:id.clone(),allow:false}).await.map_err(message)?;
        result(&view,"denied-1",3).await.map_err(|error|format!("User denial: {error}"))?;
        if !matches!(workspace.user_command(BrowserTabCommand::Permission {target,request_id:id,allow:true}).await,Err(WorkspaceError::StaleTarget)) {
            return Err("Completed permission request was reusable".into());
        }
        let (target,id)=request(&workspace,&view).await?;
        workspace.user_command(BrowserTabCommand::Permission {target,request_id:id,allow:true}).await.map_err(message)?;
        result(&view,"allowed",3).await?;
        // The one allowed fixture call is finished. Later calls are denied
        // before location access; retire the mock provider before lifecycle checks.
        super::windows::protocol_call(&view,"Emulation.clearGeolocationOverride",json!({})).await?;
        if !tab(&workspace,view.label()).await?.permission_requests.is_empty() {return Err("Granted permission remained pending".into());}
        let (repeat_target,repeat_id)=request(&workspace,&view).await?;
        workspace.user_command(BrowserTabCommand::Permission {target:repeat_target,request_id:repeat_id,allow:false}).await.map_err(message)?;
        result(&view,"denied-1",3).await.map_err(|error|format!("Denial after grant without hiding: {error}"))?;
        let (target,id)=request(&workspace,&view).await?;
        let run=workspace.begin_run().await.map_err(message)?;
        result(&view,"denied-1",3).await.map_err(|error|format!("Run admission: {error}"))?;
        if !matches!(workspace.agent_command(&run,BrowserTabCommand::Permission {target,request_id:id,allow:true}).await,Err(WorkspaceError::UnsupportedAction)) {
            return Err("Agent was allowed to answer a human permission request".into());
        }
        let observed=workspace.observe(&run,None).await.map_err(message)?;
        let geo=observed.elements.iter().find(|el|el.name=="Request geolocation").ok_or("Missing Agent permission control")?.reference.clone();
        super::evaluate(&view,"window.geoResult=undefined;true").await?;
        workspace.act(&run,BrowserAction::click(geo)).await.map_err(message)?;
        result(&view,"denied-1",3).await.map_err(|error|format!("Agent request: {error}"))?;
        if !tab(&workspace,view.label()).await?.permission_requests.is_empty() {return Err("Agent permission created a user prompt".into());}
        workspace.finish_run(&run).await.map_err(message)?;
        refresh(&workspace,&view).await?;
        let (target,id)=request(&workspace,&view).await?;
        workspace.user_command(BrowserTabCommand::Reload {target:target.clone()}).await.map_err(message)?;
        let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(3);
        loop {
            let fresh=tab(&workspace,view.label()).await?;
            if fresh.target.document_generation>target.document_generation && fresh.lifecycle==BrowserTabLifecycle::Ready {
                if !fresh.permission_requests.is_empty() {return Err("Navigation preserved an old permission request".into());}
                break;
            }
            if tokio::time::Instant::now()>=deadline {return Err("Permission fixture reload did not reach a new ready document".into());}
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        if !matches!(workspace.user_command(BrowserTabCommand::Permission {target,request_id:id,allow:true}).await,Err(WorkspaceError::StaleTarget)) {
            return Err("Old document's permission decision crossed navigation".into());
        }
        let created=workspace.user_command(BrowserTabCommand::Create {url:url.into()}).await.map_err(message)?;
        let closing_id=created.active_tab_id.ok_or("Missing permission-close tab")?;
        let closing_view=app.get_webview(&closing_id).ok_or("Missing permission-close view")?;
        let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(3);
        while tab(&workspace,&closing_id).await?.lifecycle!=BrowserTabLifecycle::Ready {
            if tokio::time::Instant::now()>=deadline {return Err("Permission-close fixture did not load".into());}
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let (closing_target,_)=request(&workspace,&closing_view).await?;
        workspace.user_command(BrowserTabCommand::Close {target:closing_target}).await.map_err(message)?;
        if app.get_webview(&closing_id).is_some() {return Err("Pending permission prevented native tab closure".into());}
        let source=tab(&workspace,view.label()).await?.target;
        workspace.user_command(BrowserTabCommand::Activate {target:source}).await.map_err(message)?;
        eprintln!("BROWSER_PERMISSION_CORE_EVIDENCE {}",json!({"user_prompt":true,"deny_and_allow":true,"no_persisted_grant":true,
            "stale_request_rejected":true,"run_start_denies":true,"agent_cannot_grant":true,"navigation_invalidates":true,"close_pending_request":true}));
        let (target,id)=request(&workspace,&view).await?;
        workspace.set_surface(bounds,false,CancellationToken::new()).await.map_err(message)?;
        if visibility(&view).await?.1 {return Err("Permission cleanup left the native HWND visible".into());}
        if !tab(&workspace,view.label()).await?.permission_requests.is_empty() {return Err("Hidden permission request remained pending".into());}
        if workspace.user_command(BrowserTabCommand::Permission {target,request_id:id,allow:true}).await.is_ok() {return Err("Hidden permission request remained grantable".into());}
        let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(2);
        while visibility(&view).await?.0 {
            if tokio::time::Instant::now()>=deadline {return Err("Permission drain retained the compositor indefinitely".into());}
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        workspace.set_surface(bounds,true,CancellationToken::new()).await.map_err(message)?;
        // Geolocation defers delivery to an inactive document. The native
        // request is already denied; verify its page callback after showing.
        if let Err(error)=result(&view,"denied-1",3).await {
            return Err(format!("Hidden prompt: {error}; native_requests={:?}",tab(&workspace,view.label()).await?.permission_requests));
        }
        super::evaluate(&view,"window.geoResult=undefined;true").await?;
        super::native_fixture_click(&view,"#geo").await?;
        result(&view,"denied-1",3).await?;
        if !tab(&workspace,view.label()).await?.permission_requests.is_empty() {return Err("Cancelled permission re-prompted without a new document".into());}
        refresh(&workspace,&view).await?;
        request(&workspace,&view).await?; // Refresh permits a fresh human decision.
        Ok(json!({"user_prompt":true,"deny_and_allow":true,"no_persisted_grant":true,"stale_request_rejected":true,"hide_denies":true,"refresh_allows_new_decision":true,
            "run_start_denies":true,"agent_cannot_grant":true,"navigation_invalidates":true,"close_pending_request":true}))
    }.await;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match service.close(&key).await {
            Ok(()) => break,
            Err(error) if tokio::time::Instant::now() >= deadline => {
                return Err(format!("Permission fixture cleanup: {error}"));
            }
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(25)).await,
        }
    }
    evidence
}
