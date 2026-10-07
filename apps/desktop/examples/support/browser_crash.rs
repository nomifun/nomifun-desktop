//! Fault injection targets only a disposable fixture renderer, never user data.
use nomifun_browser_platform::{
    run_guard::BrowserInputState, runtime::*, workspace::BrowserResourceService,
};
use serde_json::{Value, json};
use std::sync::Arc;
use tauri::Manager;
fn message(error: impl std::fmt::Display) -> String {
    error.to_string()
}

pub(super) async fn verify(app: &tauri::AppHandle, url: &str) -> Result<Value, String> {
    let service =
        BrowserResourceService::new(Arc::new(super::host::DesktopBrowserHost::new(app.clone())));
    let authority = super::browser_resource_fixture::authority(
        "crash-fixture",
        "crash-fixture",
        "native-crash-fixture",
    );
    let key = authority.key();
    let workspace = service
        .ensure(
            authority,
            BrowserProfile::Ephemeral,
        )
        .await
        .map_err(message)?;
    let mut crash_command = None;
    let result=async {
        workspace.user_command(BrowserTabCommand::Create {url:url.into()}).await.map_err(message)?;
        let target=super::wait_workspace_page(&workspace,url).await?;
        let view=app.get_webview(&target.tab_id).ok_or("Missing crash fixture child")?;
        let nonce=super::evaluate(&view,"popupNonce").await?;
        let run=workspace.begin_run().await.map_err(message)?;
        let observed=workspace.observe(&run,None).await.map_err(message)?;
        let button=observed.elements.iter().find(|element|element.name=="Push SPA route").ok_or("Missing pre-crash control")?.reference.clone();
        let child=view.clone();
        crash_command=Some(tokio::spawn(async move { super::windows::protocol_call(&child,"Page.crash",json!({})).await }));
        let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(5);
        let failed=loop {
            let snapshot=workspace.snapshot().await.map_err(message)?;
            let tab=snapshot.runtime.ok_or("Missing runtime after crash")?.tabs.into_iter().find(|tab|tab.target.tab_id==target.tab_id).ok_or("Crash removed the managed tab")?;
            if tab.lifecycle==BrowserTabLifecycle::Crashed { break tab; }
            if tokio::time::Instant::now()>=deadline { return Err(format!("Native ProcessFailed did not mark the tab crashed: {tab:?}")); }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        };
        if failed.target.document_generation<=target.document_generation { return Err("Crash preserved old document authority".into()); }
        let unavailable=tokio::time::timeout(std::time::Duration::from_millis(500),super::windows::protocol_call(&view,"Runtime.evaluate",json!({"expression":"true"}))).await.map_err(|_|"A new renderer command hung after confirmed process exit")?;
        if unavailable.is_ok() {return Err("A dead document accepted a new renderer operation".into());}
        if !matches!(workspace.act(&run,BrowserAction::click(button)).await,Err(WorkspaceError::StaleTarget)) { return Err("Pre-crash reference was not rejected".into()); }
        if !matches!(workspace.agent_command(&run,BrowserTabCommand::Reload {target:target.clone()}).await,Err(WorkspaceError::StaleTarget)) { return Err("Pre-crash navigation target was not rejected".into()); }
        if workspace.snapshot().await.map_err(message)?.run.input_state!=BrowserInputState::AgentRunning { return Err("Crash implicitly unlocked the browser".into()); }
        workspace.finish_run(&run).await.map_err(message)?;
        workspace.user_command(BrowserTabCommand::Reload {target:failed.target}).await.map_err(message)?;
        let recovered=super::wait_workspace_page(&workspace,url).await?;
        if recovered.tab_id!=target.tab_id || recovered.document_generation<=target.document_generation || super::evaluate(&view,"popupNonce").await?==nonce { return Err("Explicit recovery did not replace only the crashed document".into()); }
        let next=workspace.begin_run().await.map_err(message)?;
        let observed=workspace.observe(&next,None).await.map_err(message)?;
        let button=observed.elements.iter().find(|element|element.name=="Push SPA route").ok_or("Missing recovered control")?.reference.clone();
        workspace.act(&next,BrowserAction::click(button)).await.map_err(message)?;
        if super::evaluate(&view,"window.spaClickTrusted").await?!=true { return Err("Recovered document did not receive trusted input".into()); }
        workspace.finish_run(&next).await.map_err(message)?;
        Ok(json!({"native_process_failure_event":true,"old_targets_revoked":true,"no_implicit_unlock":true,"explicit_reload_same_tab":true,"fresh_input_after_reload":true}))
    }.await;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match service.close(&key).await {
            Ok(()) => break,
            Err(error) if tokio::time::Instant::now() >= deadline => {
                return Err(format!("Crash fixture cleanup: {error}"));
            }
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(25)).await,
        }
    }
    if let Some(mut command) = crash_command {
        if tokio::time::timeout(std::time::Duration::from_secs(2), &mut command)
            .await
            .is_err()
        {
            // Test-only destructive CDP command has no useful result after its
            // fixture controller is proven closed. Retire only this waiter.
            command.abort();
            let _ = command.await;
        }
    }
    result
}
